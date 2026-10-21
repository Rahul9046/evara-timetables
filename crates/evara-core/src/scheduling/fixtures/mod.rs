//! Deterministic synthetic schools, and what each one is expected to prove.
//!
//! # Synthetic, always
//!
//! `CLAUDE.md` rule 3: *"No real school data, ever. Fixtures, tests, examples, screenshots
//! and bug-report attachments use synthetic data only. No real student, teacher, parent or
//! school names."* Every name in this module is invented. The teacher names are built from a
//! fixed list of initials and surnames that belong to nobody, the schools are named after
//! invented places, and no value here was copied from anywhere.
//!
//! # Why the generator is the fixture
//!
//! Nothing here is checked into `fixtures/` as JSON, and that is deliberate. A generator
//! that is a pure function of a seed **is** a fixture: `fixture("tiny", seed)` returns the
//! same school on every platform and every toolchain, so committing its output would add a
//! second copy that could drift from the first. ADR 0009 names that exact failure for
//! generated TypeScript — *"otherwise generated degrades into hand-maintained with extra
//! steps"* — and solves it with a CI staleness check, which is worth its cost there because
//! the output is an IPC contract a human reads. A fixture's output is read by tests, which
//! can call the generator.
//!
//! `evara fixture` can still write one out for inspection or for a benchmark harness; see
//! [`crate::scheduling::fixtures::catalogue`] and `docs/SCHEDULING-FIXTURES.md`.
//!
//! # What a fixture carries
//!
//! Three things, and the middle one is the point:
//!
//! 1. **The inputs** — one or more [`Problem`]s, with every stored fact the later milestones
//!    read.
//! 2. **The expected conclusions** — [`Expectations`], which record what M3's diagnostics,
//!    M4's resolution and M5's overlap evaluator must each decide, with the ADR rule that
//!    decides it.
//! 3. **A known-feasible arrangement**, where the fixture claims one.
//!
//! So a fixture is not merely data: it is a specification with its data attached. When M5
//! writes the overlap evaluator it runs it over these problems and compares against
//! [`Expectations::overlaps`]; when M3 writes the diagnostics it compares against
//! [`Expectations::diagnostics`]. **This milestone asserts the structure and the encoded
//! expectations; it does not implement the functions that would satisfy them.**
//!
//! # In memory, not database-backed
//!
//! A fixture is a `Problem` and never a `.evaraproj`. Building one needs no database, no
//! migration and no file, which is what lets `evara-core`, `evara-constraints` and
//! `evara-solver` all use them — none of those three may open a database. M4's loader tests
//! will need database-backed fixtures as well, and seeding a project from one of these
//! descriptions is the natural way to get them; that is M4's work and no part of it is done
//! here.
//!
//! # Serialised out, never read back
//!
//! The types in this module derive [`Serialize`] and **not** `Deserialize`, which is a
//! deliberate asymmetry rather than an omission. A fixture's prose — its name, its purpose,
//! the rule each expectation cites — is a compile-time constant, so the only way to obtain a
//! fixture is to call the generator, and a parsed one could disagree with the code that
//! describes it. Writing one out is useful (inspection, a benchmark harness, a diff between
//! two seeds) and reading one back would be a second source of truth.
//!
//! [`Problem`] itself round-trips in both directions, because that is the contract M4's
//! loader and M6's engine exchange and `SOLVER.md` §9 requires a JSON-serialisable
//! `Problem`/`Assignment` pair for the headless CLI.

pub mod builder;
pub mod catalogue;
pub mod rng;

use serde::Serialize;
use uuid::Uuid;

use super::diagnostics::{DiagnosticCode, EntityKind};
use super::index::{ActivityIx, GroupIx, SectionIx};
use super::outcome::Seed;
use super::placement::{Assignment, PlacementSpan};
use super::population::{ExpectedAttendance, Overlap};
use super::problem::Problem;

pub use catalogue::{FixtureKind, all_fixtures, fixture, fixture_names};

/// One synthetic school — or two, where the case is about sharing between them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SchedulingFixture {
    /// The stable kebab-case name this fixture is addressed by.
    ///
    /// Part of the contract with `evara fixture` and with any committed benchmark baseline,
    /// so these are added to rather than renamed — the same discipline a
    /// [`DiagnosticCode`]'s key gets.
    pub name: &'static str,
    /// The seed it was generated from.
    pub seed: Seed,
    /// One sentence on what the fixture is for.
    pub purpose: &'static str,
    /// The problems it contains.
    ///
    /// One for almost every fixture. **Two** where the case is cross-school: ADR 0032 rule 3
    /// is about an entity two schools both hold, and a single `Problem` cannot express that
    /// because a problem is scoped to one school by construction. The two are generated
    /// independently and share only the identities the fixture deliberately reuses, which is
    /// exactly the real situation.
    pub problems: Vec<Problem>,
    /// What the later milestones must conclude.
    pub expectations: Expectations,
}

impl SchedulingFixture {
    /// The first (usually only) problem.
    ///
    /// # Panics
    ///
    /// If the fixture has no problems, which no fixture in the catalogue does.
    #[must_use]
    pub fn problem(&self) -> &Problem {
        &self.problems[0]
    }

    /// Every way any of its problems refers to something it does not contain.
    ///
    /// Empty for every fixture in the catalogue, asserted by a test over all of them at
    /// every seed the test tries.
    #[must_use]
    pub fn integrity(&self) -> Vec<super::problem::IntegrityError> {
        self.problems.iter().flat_map(Problem::integrity).collect()
    }

    /// How many occurrences there are to place, across every problem.
    #[must_use]
    pub fn activity_count(&self) -> usize {
        self.problems.iter().map(Problem::activity_count).sum()
    }
}

/// What the milestones after M2 must conclude about a fixture.
///
/// Every field is a claim with a record behind it, so that a later milestone implements
/// rather than re-decides. An empty field means the fixture makes no claim of that kind —
/// **not** that the answer is trivially the default.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Expectations {
    /// Group pairs whose three-state overlap answer the fixture pins.
    pub overlaps: Vec<OverlapExpectation>,
    /// Sections whose expected attendance the fixture pins.
    pub attendance: Vec<AttendanceExpectation>,
    /// Findings pre-solve validation must produce, with the entity each is about.
    pub diagnostics: Vec<ExpectedDiagnostic>,
    /// Identities the fixture deliberately reuses across problems.
    pub shared_identities: Vec<SharedIdentity>,
    /// An arrangement the fixture claims is feasible, where it claims one.
    pub feasible: Option<KnownFeasible>,
    /// Anything a reader of the fixture should know that is not a machine-checkable claim.
    pub notes: Vec<String>,
}

/// The overlap answer one pair of groups must get.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OverlapExpectation {
    /// Which problem the groups are in.
    pub problem: usize,
    /// One group.
    pub a: GroupIx,
    /// The other.
    pub b: GroupIx,
    /// What [ADR 0035](../../../../docs/adr/0035-student-group-overlap-is-a-three-state-answer.md)
    /// §3 must conclude.
    pub overlap: Overlap,
    /// Which of §3's seven ordered rules decides it, as `"rule 4 — containment"`.
    ///
    /// Recorded because *which* rule fires is part of the specification, not an
    /// implementation detail: the order is load-bearing — rule 3 before rule 6 is what makes
    /// a member row beat a declaration — and a test that only checked the verdict would pass
    /// on an evaluator that got the right answer by the wrong route.
    pub rule: &'static str,
}

/// The attendance answer one section must get.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AttendanceExpectation {
    /// Which problem the section is in.
    pub problem: usize,
    /// Which section.
    pub section: SectionIx,
    /// What [ADR 0034](../../../../docs/adr/0034-expected-attendance-is-explicit-and-room-capacity-waits-for-it.md)
    /// §3's derivation must produce.
    pub expected: ExpectedAttendance,
    /// Why, in one phrase — "explicit value wins", "overlap indeterminate, so no sum".
    pub because: &'static str,
}

/// A finding pre-solve validation must produce.
///
/// Both the code **and** the affected entity, because a diagnostic that fired about the
/// wrong row would be worse than one that did not fire: it would send the user to fix
/// something that is not broken.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExpectedDiagnostic {
    /// Which problem.
    pub problem: usize,
    /// The code, by its stable key.
    pub code: DiagnosticCode,
    /// What kind of thing it must be about.
    pub subject_kind: EntityKind,
    /// The exact row it must be about.
    pub subject: Uuid,
    /// What the fixture is demonstrating.
    pub because: &'static str,
}

/// An identity the fixture deliberately reuses across problems.
///
/// The machine-checkable form of
/// [ADR 0021](../../../../docs/adr/0021-project-level-people-and-school-membership.md): a
/// person belongs to the project and a membership to the school, so a teacher employed by
/// two schools is **one** identity in both problems. A loader that minted a school-specific
/// identity would break this assertion, which is why it is an assertion.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SharedIdentity {
    /// What kind of thing is shared.
    pub kind: EntityKind,
    /// The one identity.
    pub id: Uuid,
    /// Which problems hold it, in ascending order.
    pub problems: Vec<usize>,
}

/// An arrangement the fixture claims breaks no hard rule.
///
/// # "By construction", and what that does and does not mean
///
/// The fixture *built* this arrangement: it chose the slots and rooms so that no teacher,
/// room or proven-overlapping group pair is double-booked, and so that every multi-period
/// occurrence sits in a genuine run of consecutive teaching periods of its own resolved bell
/// schedule. M2's tests verify those properties directly from the spans, using
/// [`PlacementSpan::clashes_in_time_with`] — the one implementation of the interval test —
/// rather than a second copy of any rule.
///
/// What it does **not** claim is that a solver would find it, or that it is the best
/// arrangement, or that the problem is feasible in some stronger sense. It is an existence
/// witness: a timetable exists for this problem, and here it is. That matters because the
/// alternative — a fixture labelled "feasible" with nothing to show — is a claim nobody can
/// check until M6, and `evara_core::lessons`' rule applies: never report that a demand *can*
/// be scheduled unless you can point at the arrangement.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KnownFeasible {
    /// Which problem it is an arrangement for.
    pub problem: usize,
    /// Every occurrence and the slots it covers, in activity-index order.
    pub spans: Vec<(ActivityIx, PlacementSpan)>,
}

impl KnownFeasible {
    /// The arrangement as an [`Assignment`], for a caller that wants to seed a solve from it.
    ///
    /// `activities` is the problem's activity count, so the assignment is the right length
    /// even for an arrangement that leaves something unplaced — which no catalogue fixture
    /// does, and which a test asserts.
    #[must_use]
    pub fn assignment(&self, activities: usize) -> Assignment {
        let mut assignment = Assignment::empty(activities);
        for (activity, span) in &self.spans {
            assignment.place(*activity, span.placement());
        }
        assignment
    }
}

#[cfg(test)]
mod tests;
