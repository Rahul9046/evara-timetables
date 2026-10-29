# M9B — Automated Timetable Repair and Partial Regeneration · Delivery report

- Branch: `feat/m9b-timetable-repair`, from `334a21d` (`= origin/develop`)
- Six commits, `ef9d671` … `0fc7db9`, one per approved stage B1–B6
- 71 files changed, +13 237 / −189
- `npm run check` **passes** — fmt, clippy with `-D warnings`, 1 709 Rust tests, types up to
  date, `tsc`, 1 270 frontend tests
- Phase: ROADMAP **Phase 8B**, which closes M1-14
- The architecture is [M9B-REPAIR-PROPOSAL.md](M9B-REPAIR-PROPOSAL.md); what shipped is
  [TIMETABLE-REPAIR.md](TIMETABLE-REPAIR.md)

---

## 1. The mandatory safeguard, and where it is enforced

> *"Do not rely on `ViolationKey` equality alone to prove scope preservation. Before accepting a
> repair outcome, compare the complete baseline and repaired assignments for every frozen
> occurrence … This comparison must be part of the Rust repair verification and must be enforced
> again when persisting the repaired timetable."*

Implemented as `evara_core::scheduling::repair::frozen_differences`, which walks **every frozen
occurrence** and compares the whole `Placement` — slot *and* room — plus the unplaced state, in
both directions. It returns `FrozenMismatch::{Moved, Dropped, Added}` so a failure is diagnosable
rather than merely detected.

| Where | Call | On mismatch |
| --- | --- | --- |
| **Solver**, before returning | `repair::verify` check V0 | `RepairDefect::FrozenPlacementsChanged`, `Termination::InternalCheckFailed`, `applyable = false` |
| **Solver**, locks narrowed | V0b, `locked_differences` against the pin | `RepairDefect::LockedPlacementsChanged` |
| **Database**, inside the writing transaction | `refuse_scope_violation`, first statement of `save_repaired` | `DbError::ScopeViolated { mismatches, detail }` — nothing written |
| **Command layer** | `unapplyable_reason` reads `RepairOutcome::applyable` | The apply is refused before any database work |
| **Interface** | `isScopeViolation` | *"a defect in Evara, not a problem with your timetable … our bug to fix, not your data to correct"* |

It lives in `evara-core` because `evara-db` may not depend on `evara-solver`, so that is the only
place both callers can share **one** implementation.

**Why it is not derived from the violation sets.** Two arrangements can carry identical violation
sets and still differ: a frozen occurrence moved to an equally-clashing slot, or two frozen
occurrences swapped, leaves every `ViolationKey` the same.
`a_swap_of_two_frozen_occurrences_is_caught` demonstrates it, and
`a_moved_frozen_placement_is_caught_including_a_room_only_change` covers the room-only case a
slot digest would miss.

The four additional verifications were also required and are implemented:

| | Check | Implemented as |
| --- | --- | --- |
| Frozen-only hard sets unchanged | V3, by `ViolationKey` | `RepairDefect::FrozenViolationsChanged { appeared, disappeared }` |
| No final hard violation involves a movable occurrence | V2 | `RepairDefect::HardViolationInvolvingMovable` |
| Locked placements unchanged | V0b | `locked_differences` |
| The final reference evaluation covers the complete arrangement | — | `evaluate(problem, &repaired, constraints)` over the whole assignment; `SolveOutcome::score` and `hard_violations` come from it |

---

## 2. The six stages

| Stage | Commit | What |
| --- | --- | --- |
| **B1** | `ef9d671` | `construct` takes a baseline and a movable set; the frozen-only working arrangement; the baseline-first probe; `repair_with` and its five verifications; the shared vocabulary in `evara-core` |
| **B2** | `a02e91c` | `V18`; `repair_baseline` as one read; `save_repaired` with three guards and the repeated scope check; `StoredPins` widened with lock causes; `delete`'s descendant refusal |
| **B3** | `5eec75d` | `evara_report::repair` — the five change categories, the three attribution buckets, `ScoreLine` |
| **B4** | `273d7d1` | `RepairSession` and its six hazards; eight Tauri commands; scope derivation in `evara-core`; 20 generated TypeScript types |
| **B5** | `e1bfccf` | The repair screen: radio-group scope picker, pre-flight, progress, preview, explicit apply |
| **B6** | `0fc7db9` | `repair_budget.rs`; five ADRs; `TIMETABLE-REPAIR.md`; `SOLVER.md` §7, `CONSTRAINTS.md`, `TIMETABLE-PERSISTENCE.md`, ROADMAP and `CLAUDE.md` amendments |

Each stage was verified before the next began: clippy clean, its own tests green, and the
previously passing suites unchanged.

---

## 3. Tests

**128 new tests.** Totals: **1 709 Rust** (2 ignored) and **1 270 frontend**.

| Where | New | Covers |
| --- | --- | --- |
| `evara-core` `scheduling::repair` | 24 | `MovableSet` validation, `frozen_only`, `frozen_differences`, `locked_differences`, the six scope derivations |
| `evara-solver` `tests_repair` | 24 | Scorer initialisation, movable-to-movable, self-clash, multi-period, determinism, cancellation, locks, the four buckets, reference-vs-incremental |
| `evara-solver` `repair_budget` | 5 | The measured figures, and the scope guarantee asserted on every measurement |
| `evara-db` `tests_repair` | 15 | The snapshot, the three guards, rollback, lineage, flags, unexpressible locks, school isolation, the V17→V18 upgrade |
| `evara-report` `repair::tests` | 12 | The five categories exhaustive and disjoint over every fixture; the whole-arrangement totals |
| `commands::repair_session` | 16 | The six hazards, pure |
| `commands::repair` | 8 | Remedies, keys, serialisation round-trip, distinct event names |
| `RepairTimetable.test.tsx` | 23 | Scope never defaulted, arrow keys, the two pre-run warnings, blocking vs advisory, the no-change wording, the separate-repair offer, three refusal remedies |

### Every required case, and where it lives

| Required | Test |
| --- | --- |
| Movable-to-movable conflicts | `two_movable_occurrences_that_clash_can_both_be_re_placed` |
| Pre-existing frozen hard violations | `a_pre_existing_frozen_clash_does_not_refuse_any_candidate`, `a_repair_of_a_timetable_with_out_of_scope_hard_violations_is_applyable` |
| Frozen placement equality | `the_scope_guarantee_holds_for_every_frozen_occurrence`, `a_swap_of_two_frozen_occurrences_is_caught`, `a_repair_that_moved_a_frozen_placement_is_refused_by_the_write` |
| Locked placement preservation | `a_locked_occurrence_inside_the_scope_is_never_moved`, `a_lock_is_verified_against_its_pin_as_well_as_against_the_baseline`, `a_repair_preserves_lock_and_manual_flags_it_did_not_move` |
| Unexpressible locks | `a_lock_outside_the_compiled_problem_is_reported_with_its_cause`, `an_unexpressible_lock_is_never_cleared_by_reading_it`, `every_unexpressible_cause_names_a_remedy` |
| Multi-period lessons | `a_multi_period_lesson_moves_as_one_unit_to_an_unbroken_run`; `evara-db`'s fixture has two doubles of six, so every DB test exercises it |
| Shared rooms, multiple campuses | Inherited: the clash rules compare **minutes**, never `timeslot_id`, and `room-clash` stays project-wide. `every_fixture_repairs_without_disturbing_a_frozen_placement` sweeps the catalogue, which includes the shared-room and two-campus fixtures |
| Custom cycles | The same sweep; nothing in the repair path reads a weekday, and `a_cycle_day_scope_matches_the_stored_identifier_and_never_a_weekday` pins it |
| School isolation | `a_repair_of_one_school_writes_nothing_of_anothers`, `an_edit_to_another_schools_timetable_is_not_this_repairs_conflict` |
| All scope types | The six derivation tests, plus `the_scope_guarantee_holds_for_every_frozen_occurrence` per occurrence |
| Baseline preference, determinism | `a_movable_occurrence_whose_placement_is_still_legal_does_not_move`, `a_repair_is_deterministic_over_one_timetable_scope_and_seed`, `a_repair_at_a_different_seed_is_still_scope_preserving` |
| Cancellation, partial outcomes | `a_cancelled_repair_is_not_applyable`, `a_cancelled_repair_still_preserved_every_frozen_placement`, `a_partial_repair_is_applyable`, `a_budget_limited_repair_is_applyable` |
| Cross-session arrangement changes | `a_repair_whose_baseline_was_edited_is_refused` |
| Input watermark changes | `a_repair_whose_inputs_moved_is_refused` |
| Project epoch invalidation | `repair_apply`'s epoch check, with `commands::repair_session`'s hazard table |
| Transaction rollback | `a_failed_repair_save_leaves_nothing_at_all` |
| Immutable historical provenance | `a_repair_leaves_the_sources_run_and_scoring_byte_identical` |
| Repair lineage, descendant deletion | `deleting_a_source_with_descendants_is_refused_readably` |
| Preview completeness, score correctness | `the_five_categories_are_exhaustive_over_every_fixture`, `the_totals_are_the_whole_arrangements_not_the_scopes`, `the_preview_reports_the_whole_arrangements_violations` |
| **V17 → V18 with data intact** | `a_v17_project_upgrades_to_v18_with_every_row_intact` |

### The two tests proven to be real gates

Both critical designs were **deliberately broken** to confirm the tests catch them:

| Break | Result |
| --- | --- |
| Initialise the working scorer from the full baseline | **19 of 24** solver repair tests fail |
| Gate the repair on whole-arrangement `score.is_feasible()` | **5** fail, including `a_repair_of_a_timetable_with_out_of_scope_hard_violations_is_applyable` |

Both were restored and the suite re-verified.

### The V17 → V18 migration test

A genuine rewind, not a re-run over an empty file: the index is dropped, all three columns are
dropped with `ALTER TABLE … DROP COLUMN`, and V18's history row is deleted — then the project is
reopened so V18 re-applies over a file already holding a real timetable, run and scoring. The
test asserts the schema really went back (`pragma_table_info` finds no `based_on_timetable_id`),
then that the timetable row, every entry, the run and every evaluation are **byte-identical**
afterwards, that the lineage reads `NULL` and the run reads `GENERATION` from their defaults, and
that the upgraded project can still take a repair.

Two existing rewind tests (`a_v16_project_upgrades_to_v17_with_the_new_tables_empty`,
`a_project_at_the_previous_schema_keeps_every_row_and_revision`) were extended through V18:
deleting only V17's history row would have skipped it and left `timetable` without the column.

---

## 4. Performance

Measured by `crates/evara-solver/src/repair_budget.rs`, which prints its figures and bounds them
only as hang detectors — a tight timing assertion on shared CI hardware is a flaky test.

| Measurement | Debug | Release | Proposal §6.2 budget |
| --- | --- | --- | --- |
| Clean whole-timetable repair, 1 600 occurrences | **126 ms** | **90 ms** | < 2 s |
| Whole timetable, 400 occurrences | 17.7 ms | 10.3 ms | — |
| One occurrence of 400 — a narrow scope | **2.5 ms** | **1.6 ms** | < 250 ms |
| Conflicted baseline, 500 occurrences, 3 280 violations, all resolved | **49 ms** | **37 ms** | < 10 s |
| Ten of 500 with the rest conflicted — applyable | 46 ms | 39 ms | — |

All inside budget, several by two orders of magnitude. The clean whole-timetable repair spends
**exactly 1 600 candidate evaluations** — one baseline probe per occurrence, and the seeded walk
never runs — and reproduces the baseline byte-for-byte, which is the baseline-first rule working
as designed. A narrow scope is genuinely **cheaper** than a wide one, not merely smaller, because
the frozen set is placed into the scorer rather than searched.

### What the budget work found, and did not smooth over

**The first fixture was unrepresentative, and it cost ~194 s.** 1 600 *single-occurrence*
sections. Attributed:

```text
reference evaluate      3.2 ms
build the scorer        3.5 ms
one probe              14.4 µs   (×1600 = 23 ms)
a whole GENERATION    193.99 s
```

The repair cost 194.25 s on the same problem, so **it adds essentially nothing**: the time is
M6's `prepare`, whose `conflict_degrees` is documented as quadratic in the **sections** — *"a
500-occurrence school has far fewer sections, and every occurrence of a section has the same
degree"*. 1 600 sections of one occurrence is its worst case and no real school's shape.
`editing_budget.rs` uses the same shape but never calls `prepare`, so it never paid it.

The budget now uses 200 sections of eight. The diagnostic is kept as the evidence and
`#[ignore]`d — a three-minute test in the ordinary suite is paid by everyone on every run. This
is **pre-existing M6 behaviour, not M9B's**, and it is not fixed here.

**The first "clean" baseline was not clean.** One occurrence per `(slot, room)` cell is legal
only when every section has one occurrence; with eight each it puts a section's eight into eight
rooms of the *same slot* — the same teacher and the same class in eight places at once. The
fixture would have been a worst case wearing a clean label, and its figure would have been about
resolving 5 600 self-inflicted clashes. `a_legal_arrangement` now derives its mapping from the
construction, and the caller asserts `score.hard == 0` with the reference evaluator **before**
anything is timed.

---

## 5. Records

**Five ADRs**, all Accepted:

| ADR | Decision |
| --- | --- |
| [0048](adr/0048-repair-scope-is-a-movable-set.md) | Repair scope is a movable set, never a set of pins — the scope limits what may move, not what is checked |
| [0049](adr/0049-a-repairs-verification-is-scope-aware.md) | A repair verifies V0–V3 rather than whole-arrangement feasibility; a pre-existing violation outside the scope is not a defect |
| [0050](adr/0050-a-repair-produces-a-new-version.md) | A repair inserts a new timetable and is never applied in place; `V18` is additive, lineage is `RESTRICT` with a typed refusal |
| [0051](adr/0051-baseline-preference-is-candidate-ordering.md) | Baseline preference is candidate ordering, not a constraint; `minimise-disruption` waits for local search |
| [0052](adr/0052-an-unexpressible-lock-blocks-a-repair.md) | An unexpressible lock blocks a repair and only warns an edit |

**`SOLVER.md` §7 amended in three places, not the two the proposal expected.** Both of its
mechanisms were wrong — pinning out-of-scope placements drops them, and the disruption penalty is
deferred — and so was its **scope list**: it offered *"a term, a year level, a teacher's load, an
explicit selection"*, and a **term** is not a repair scope at all, because a timetable is already
per `(cycle, term)`. A year level is a derived set and was not included.

Also amended: `CONSTRAINTS.md` (OPS-001 → local search), `TIMETABLE-PERSISTENCE.md` §12 (the
lineage limitation struck through as paid), ROADMAP Phase 8B (complete, closing M1-14), and
`CLAUDE.md` (three new rules). `CONSTRAINT-ENGINE.md` needed **no** change: the verification fix
interprets the reference report in `evara-report` and is not a new rule, level or evaluator.

### `V18`, and the two omissions that keep it additive

```sql
ALTER TABLE timetable ADD COLUMN based_on_timetable_id TEXT
    REFERENCES timetable (id) ON DELETE RESTRICT;
CREATE INDEX idx_timetable_based_on ON timetable (based_on_timetable_id);
ALTER TABLE solve_run ADD COLUMN kind TEXT NOT NULL DEFAULT 'GENERATION'
    CHECK (kind IN ('GENERATION', 'REPAIR'));
ALTER TABLE solve_run ADD COLUMN scope TEXT NOT NULL DEFAULT '{}'
    CHECK (json_valid(scope) AND json_type(scope) = 'object');
```

`timetable_evaluation.kind` is **not** widened — a repair writes `GENERATION`, because that *is*
the new version's original immutable scoring, and `solve_run.kind` is the discriminator. And no
eighth `UnplacedCause` is added — a repair's `solve_run_unscheduled` rows cover the movable set
only. Either change would have needed a `CHECK` altered, which SQLite cannot do, so `V18` would
have become a twelve-step rebuild on a `STRICT` table and engaged ADR 0020.

---

## 6. Changed surface

| Area | Files |
| --- | --- |
| `evara-core` | `scheduling/repair.rs` (new, with tests), `scheduling/mod.rs` |
| `evara-solver` | `repair.rs`, `tests_repair.rs`, `repair_budget.rs` (all new); `construct.rs`, `lib.rs` |
| `evara-db` | `migrations/V18__timetable_repair.sql`, `tests_repair.rs` (new); `repo/timetables.rs`, `error.rs`, `lib.rs`, two existing test files |
| `evara-report` | `repair.rs`, `repair/tests.rs` (new); `lib.rs` |
| `evara-cli` | `save.rs` — the two new `RunProvenance` fields |
| Desktop Rust | `commands/repair.rs`, `commands/repair_session.rs` and their tests (new); `commands/{mod,project,session,solve}.rs`, `lib.rs`, `bin/typegen.rs` |
| Desktop TS | `ipc/repair.ts`, `app/timetables/RepairTimetable.tsx` and its test (new); `app/timetables/Timetables.tsx` and its test, `timetables.css` |
| Generated | 20 new types in `packages/domain/src/generated/`, plus both barrels |
| Docs | `TIMETABLE-REPAIR.md`, five ADRs (new); `SOLVER.md`, `CONSTRAINTS.md`, `TIMETABLE-PERSISTENCE.md`, `ROADMAP.md`, `adr/README.md`, `CLAUDE.md` |

Two shared pieces were **generalised rather than copied**, because the alternative was a second
implementation of something worth getting right once: `CoalescingSink` now takes a
`ProgressReporter` so the repair and generation sessions share its interval arithmetic, and
`Timetables::{arrangement, pins}` now delegate to the same projection `repair_baseline` uses, so
an entry can never be readable as an arrangement and unreadable as a pin.

The generation path is **untouched**: M6's 86 solver tests and M7's save path pass unchanged, and
`generate_with` and `status_of` were deliberately not modified.

---

## 7. Verification

Run with `CARGO_BUILD_JOBS=2`, `RUST_TEST_THREADS=2`, `CARGO_INCREMENTAL=0`; heavy builds
sequential.

| Gate | Result |
| --- | --- |
| `cargo fmt --all --check` | clean |
| `cargo clippy --workspace --all-targets --all-features -- -D warnings` | clean, **no crate-level allows added** beyond the established `needless_pass_by_value` for a command module |
| `cargo test --workspace --all-features` | **1 709 passed**, 0 failed, 2 ignored |
| `npm run types:check` | up to date |
| `npm run typecheck` | clean |
| `npm run test:web` | **1 270 passed**, 53 files |
| `npm run check` (all of the above) | **exit 0** |
| `cargo build --workspace --all-features --release` | clean, 4 m 18 s |
| `npm run build --workspace=@evara/desktop` | clean, 93 modules |

### The security advisories are not suppressed

`cargo deny check advisories` **fails**, on the pre-existing `zip 8.6.0` advisories
RUSTSEC-2026-0367, -0368 and -0369, reached through `rust_xlsxwriter` → `evara-import`. These are
`origin/develop`'s: `deny.toml` is **untouched** and `Cargo.lock` is **byte-identical** to
`origin/develop`. Nothing was added to an ignore list and no dependency was changed.

---

## 8. Limitations

| Limitation | Status |
| --- | --- |
| **No minimal-disruption guarantee** | By design, and stated in the UI, the ADR and the record. Greedy construction cannot reconsider a displacement |
| **A positional scope does not close under the constraints** | By design. Repairing "Monday" may fail where repairing everything would succeed; reported with the reason, never worked around |
| **An unexpressible lock must be resolved before repairing** | By design (ADR 0052). Mitigated by naming each remedy |
| **A heavily conflicted baseline is slower**, and the cost is allocation | Unfixed. `Violation` formats its `detail` eagerly; making it lazy is M5's surface |
| **`conflict_degrees` is quadratic in the sections** | Unfixed, and **pre-existing M6 behaviour** — §4 measures that a generation pays it identically. Reached only by an unrealistic fixture shape |
| **No GUI smoke test** | Unfixed, as M9A. 23 component tests cover the screen; nothing drives the built binary |
| **A repair's statistics are not comparable with a generation's** | By design. The baseline probe spends one evaluation per in-scope occurrence; `solve_run.kind` lets a reader tell them apart |
| **Cross-school coordinated repair** | Out of scope, as ADR 0032 defers cross-school comparison for generation |

### One thing worth your attention before merging

`SOLVER.md` §7's **scope list** changed as well as its two mechanisms — the proposal's §8.2 had
flagged this, and it is now amended. If you want **term** or **year level** as repair scopes,
they are additive: `derive_movable` takes a `RepairScope` enum and each variant is a few lines,
but neither is implemented and neither is tested.

Nothing is merged. The PR is open against `develop` and awaits your review.
