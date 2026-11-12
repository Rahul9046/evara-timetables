//! The project a round trip is run over, and the timetable history it carries.
//!
//! # Why the timetable layer is built here and not in `evara-db`
//!
//! `evara_db::fixtures::transfer` builds the structure — schools, campuses, a shared room,
//! people, curriculum, lessons, constraints, a logo, availability, and a pass of real updates
//! so the change feed grew by *use*. What it deliberately does not build is a stored
//! timetable, because a stored timetable needs a `SolveOutcome` and the only honest source of
//! one is the engine that produces them — which `evara-db` may not depend on as a library.
//!
//! This crate's **tests** may. `ARCHITECTURE.md` §3 rule 4 is about library edges, and
//! `evara-db`'s own manifest already records the argument for its `evara-solver`
//! dev-dependency: *a test may run the other crate rather than imitate it.* So the timetable
//! layer is a real generation, saved through the real repository, and everything a round trip
//! then asserts about `timetable_entry`, `solve_run`, `timetable_evaluation`, `violation`,
//! lineage, locks and manual placements is asserting about rows the product actually writes.
//!
//! # What the history contains, and why each piece is there
//!
//! | Piece | What it proves a transfer carries | Test |
//! | --- | --- | --- |
//! | a generated timetable, its run, its `GENERATION` scoring and findings | §1.4.1's regression: a generation evaluation is **immutable**, so findings that did not travel could never be put back — a transferred timetable would show a generation score beside an empty findings list, permanently | T-15, T-16, T-19 |
//! | a **second** timetable for the same `(cycle, term)` | several versions coexist and that *is* the versioning (ADR 0042) | T-15 |
//! | a duplicate of the first | `based_on_timetable_id`, so `lineage:repair` is declared and `delete` still refuses a parent | T-18 |
//! | a locked placement | `timetable_entry.is_locked` — per version, never per lesson (ADR 0045) | T-15 |
//! | a hand-moved placement | `is_manual`, which a regeneration must not be able to forge | T-15 |
//! | unplaced occurrences, where the solve leaves any | `solve_run_unscheduled` with its cause and diagnostic code | T-17 |
//!
//! Every name is synthetic, per `CLAUDE.md` rule 3.

use evara_constraints::{Registry, evaluate};
use evara_core::scheduling::repair::MovableSet;
use evara_core::scheduling::{Cancel, ProblemIdentity, SolveOutcome, SolveStatus, Violation};
use evara_db::fixtures::seed::Project;
use evara_db::fixtures::transfer::{self, Built, Shape};
use evara_db::{Assembly, EditGuard, RUN_GENERATION, RunProvenance, SchedulingSelection};
use evara_solver::{GenerateRequest, RepairRequest, SolveConfig, generate, repair};
use uuid::Uuid;

/// The version string the fixtures record.
pub(crate) const APP_VERSION: &str = "0.1.0-test";

/// The seed every fixture solve uses.
///
/// Fixed, because solving is deterministic and a fixture whose timetable changed between runs
/// would make a round-trip comparison flaky for a reason that has nothing to do with transfer.
pub(crate) const SEED: u64 = 42;

/// A project with structure, lessons and a real timetable history.
pub(crate) struct Fixture {
    /// The open project.
    pub project: Project,
    /// What the structural fixture built.
    pub built: Built,
    /// The generated timetable, newest version first.
    pub timetables: Vec<Uuid>,
    /// The duplicate, which carries the placements and no run of its own.
    pub duplicate: Option<Uuid>,
    /// The repaired version, which **names its parent** — V18's lineage edge.
    pub repaired: Option<Uuid>,
}

impl Fixture {
    /// Builds one at `shape`, with the timetable layer on top.
    pub(crate) fn build(shape: Shape) -> Self {
        Self::build_named(shape, "northgate")
    }

    /// Builds one with a chosen file stem.
    pub(crate) fn build_named(shape: Shape, stem: &str) -> Self {
        let (mut project, built) = transfer::build_named(shape, stem);

        // The grid and the lessons have to be on disk before the loader can compile them: a
        // problem assembled through the handle that wrote the rows would not prove anything
        // reached the file, which is the point every fixture in this workspace is built
        // around.
        project.reopen();

        let mut fixture = Self {
            project,
            built,
            timetables: Vec::new(),
            duplicate: None,
            repaired: None,
        };
        fixture.add_timetable_history();
        fixture
    }

    /// Generates, saves, duplicates and locks.
    fn add_timetable_history(&mut self) {
        let selection = self.built.main.selection();

        for (n, name) in ["Draft", "Second draft"].into_iter().enumerate() {
            let assembly = self
                .project
                .load(&selection)
                .expect("the fixture school assembles");
            let outcome = solve(&assembly, &selection, SEED + n as u64);

            // A run that produced no timetable is not a fixture problem to paper over — it
            // would mean the structural fixture is infeasible, and a round trip over an empty
            // timetable table would assert nothing. Reported where it happens.
            assert_ne!(
                outcome.status,
                SolveStatus::Failed,
                "the fixture school must be solvable for the timetable layer to exist:                  termination {:?}, findings {:?}",
                outcome.termination,
                outcome
                    .preflight
                    .findings
                    .iter()
                    .map(|finding| (finding.code, finding.severity()))
                    .collect::<Vec<_>>()
            );

            let violations = score(&assembly, &outcome);
            let saved = self
                .project
                .db()
                .timetables()
                .save_generated(
                    &assembly,
                    &outcome,
                    &violations,
                    &provenance(RUN_GENERATION),
                    name,
                )
                .expect("save the generated timetable");
            self.timetables.push(saved.timetable.id);
        }

        let parent = self.timetables[0];
        let parent_rev = self
            .project
            .db()
            .timetables()
            .get(parent)
            .expect("read the timetable")
            .rev;
        let duplicate = self
            .project
            .db()
            .timetables()
            .duplicate(parent, parent_rev, "Working copy")
            .expect("duplicate a timetable");
        self.duplicate = Some(duplicate.timetable.id);

        let watermark = self
            .project
            .load(&selection)
            .expect("the fixture school assembles")
            .watermark;
        self.lock_one_placement(parent, watermark);
        self.repair_one_version(&selection, self.timetables[1]);
    }

    /// Locks one placement on the first timetable.
    ///
    /// On the *first* version only, so a round trip has a version where the flag is set and a
    /// version where it is not — which is what makes "preserved exactly, per version" a thing
    /// a comparison can fail. A lock on the lesson would pin it in every version at once,
    /// which is why the column is on the entry (ADR 0045).
    fn lock_one_placement(&mut self, timetable_id: Uuid, watermark: evara_db::Watermark) {
        let entries = self
            .project
            .db()
            .timetables()
            .entries(timetable_id)
            .expect("read the entries");
        let Some(first) = entries.first().cloned() else {
            return;
        };

        let stamp = self
            .project
            .db()
            .timetables()
            .arrangement_stamp(timetable_id)
            .expect("read the arrangement stamp");
        self.project
            .db()
            .timetables()
            .set_lock(
                first.id,
                first.rev,
                true,
                EditGuard {
                    watermark,
                    stamp: &stamp,
                },
            )
            .expect("lock a placement");
    }

    /// Repairs the second version, producing the lineage edge.
    ///
    /// The edge is what `based_on_timetable_id` holds, and it comes from a **repair** rather
    /// than from a duplication: `duplicate` *copies* the column rather than setting it to its
    /// source — a duplicate of a repaired timetable is a sibling, not a child — so a fixture
    /// that duplicated to get lineage would be asserting against a column nothing had written.
    ///
    /// The repair is run over the second version so the first keeps its lock, which is what
    /// makes "preserved exactly, per version" a thing a comparison can fail. Repaired on the
    /// second because an unexpressible lock **blocks** a repair (ADR 0052), and the locked
    /// placement is on the first.
    fn repair_one_version(&mut self, selection: &SchedulingSelection, source: Uuid) {
        let assembly = self
            .project
            .load(selection)
            .expect("the fixture school assembles");
        let baseline = self
            .project
            .db()
            .timetables()
            .repair_baseline(source, &assembly)
            .expect("read the repair baseline");

        // Everything movable. A narrower scope would be a better test of scope arithmetic and
        // a worse fixture: what a round trip needs is the lineage edge and a second
        // `solve_run` whose `kind` is `REPAIR`, and the widest scope is the one most likely
        // to produce a saveable outcome from a timetable that may already be feasible.
        let movable = MovableSet::everything(assembly.problem.activities.len())
            .expect("every occurrence is movable");
        let rules = Registry::mandatory();
        let report = evara_db::AssemblyReport::of(&assembly);

        let outcome = repair(&RepairRequest {
            problem: &assembly.problem,
            identity: ProblemIdentity {
                school: assembly.problem.context.school.id,
                cycle: selection.cycle_id,
                term: selection.term_id,
            },
            constraints: &rules,
            preflight: report.findings.clone(),
            may_solve: report.may_solve(),
            unenforced_restrictions: 0,
            config: SolveConfig::seeded(SEED + 100),
            baseline: &baseline.arrangement.assignment,
            movable: &movable,
        });

        // A repair that produced nothing is not a fixture problem to paper over, and it is
        // also not a certainty: reported where it happens rather than silently skipped, so a
        // fixture that stopped carrying lineage is a red test and not a quiet gap.
        assert_ne!(
            outcome.outcome.status,
            SolveStatus::Failed,
            "the fixture's repair must produce a timetable: {:?}, defects {:?}",
            outcome.outcome.termination,
            outcome.defect
        );

        let violations =
            evaluate(&assembly.problem, &outcome.outcome.assignment, &rules).violations;
        let saved = self
            .project
            .db()
            .timetables()
            .save_repaired(
                &assembly,
                &evara_db::RepairSave {
                    source,
                    expected: baseline.source_rev,
                    guard: EditGuard {
                        watermark: assembly.watermark,
                        stamp: &baseline.stamp,
                    },
                    outcome: &outcome.outcome,
                    violations: &violations,
                    baseline: &baseline.arrangement.assignment,
                    movable: &movable,
                    flags: &baseline.flags,
                    run: &provenance(evara_db::RUN_REPAIR),
                    name: "Repaired",
                },
            )
            .expect("save the repaired timetable");
        self.repaired = Some(saved.timetable.id);
    }
}

/// The provenance a fixture run records.
///
/// Fixed timestamps rather than the clock, so two runs of the same test produce the same
/// bytes — which is what a round trip compares.
fn provenance(kind: &str) -> RunProvenance {
    RunProvenance {
        engine: String::from("construction"),
        settings: String::from("{}"),
        started_at: String::from("2026-11-12T09:00:00.000Z"),
        finished_at: String::from("2026-11-12T09:00:01.000Z"),
        kind: String::from(kind),
        scope: String::from("{}"),
    }
}

/// Runs the real solver over an assembly.
///
/// `Registry::mandatory()` rather than a compiled set from the project's own rows, for one
/// reason worth stating: evaluation runs every implemented type whatever the database holds,
/// and a `constraint_instance` row only *tunes* one. The mandatory set is therefore what the
/// product would use, and the fixture's two tuning rows are carried for the round trip to
/// preserve rather than for the solve to honour.
fn solve(assembly: &Assembly, selection: &SchedulingSelection, seed: u64) -> SolveOutcome {
    let rules = Registry::mandatory();
    let report = evara_db::AssemblyReport::of(assembly);

    generate(&GenerateRequest {
        problem: &assembly.problem,
        identity: ProblemIdentity {
            school: assembly.problem.context.school.id,
            cycle: selection.cycle_id,
            term: selection.term_id,
        },
        constraints: &rules,
        preflight: report.findings.clone(),
        may_solve: report.may_solve(),
        unenforced_restrictions: 0,
        config: SolveConfig::seeded(seed),
    })
}

/// Scores an outcome the way a save does.
fn score(assembly: &Assembly, outcome: &SolveOutcome) -> Vec<Violation> {
    let rules = Registry::mandatory();
    evaluate(&assembly.problem, &outcome.assignment, &rules).violations
}

/// A cancellation that fires after a chosen number of questions.
///
/// What the cancellation tests use. A counter rather than a clock, so the point a transfer
/// stops at is the same on every machine — the same reason the solver's execution limits are
/// counts.
pub(crate) struct CancelAfter {
    remaining: std::sync::atomic::AtomicUsize,
}

impl CancelAfter {
    /// Fires once it has been asked `questions` times.
    #[must_use]
    pub(crate) const fn new(questions: usize) -> Self {
        Self {
            remaining: std::sync::atomic::AtomicUsize::new(questions),
        }
    }
}

impl Cancel for CancelAfter {
    fn is_cancelled(&self) -> bool {
        use std::sync::atomic::Ordering;
        // `fetch_sub` would wrap at zero, so the countdown saturates instead: once it has
        // fired it keeps firing, which is what a cancellation means.
        let before = self.remaining.load(Ordering::SeqCst);
        if before == 0 {
            return true;
        }
        self.remaining.store(before - 1, Ordering::SeqCst);
        false
    }
}
