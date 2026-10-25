# ADR 0050 — A repair produces a new version and is never applied in place

- Status: **Accepted** (2026-10-25)
- Date: 2026-10-25
- Decides: M9B decision D4, and the lineage question V18 answers
- Builds on: [ADR 0042](0042-a-stored-timetable-is-a-version-and-a-save-refuses-stale-inputs.md)
  (a save always inserts; several timetables coexist for one `(cycle, term)` and that is the
  versioning), [ADR 0046](0046-an-edit-changes-the-version-it-is-open-on.md) (an edit changes the
  version it is open on, and duplicating first is the user's decision),
  [ADR 0047](0047-a-cross-session-guard-is-an-exact-placement-list.md) (the arrangement stamp),
  [ADR 0020](0020-in-place-table-rebuild.md) (what altering a `CHECK` costs)
- Binds: `V18__timetable_repair.sql`, `evara_db::Timetables::{save_repaired, delete,
  descendant_count}`, `DbError::HasDescendants`, `timetable.based_on_timetable_id`,
  `solve_run.{kind, scope}`

## Context

ADR 0046 permits an **edit** to change the version it is open on, because one edit is one row in
one transaction and the user can see the single placement they are changing. A repair is
hundreds of rows at once, computed from a snapshot the user has then spent time reviewing.

Two of M9B's mandatory guarantees are about that gap: a failed, cancelled or stale repair must
not be able to modify a saved timetable, and historical generation and evaluation records must
remain immutable.

## Decision

**Approving a repair *inserts* a new timetable carrying the repaired arrangement, its locks, its
manual flags and a `REPAIR` run record naming the timetable it was based on. The original is
untouched. There is no in-place apply.**

This makes both guarantees **structural rather than guarded**: no code path exists that writes
into the baseline, so there is nothing to get wrong; and the original's `solve_run` and
`GENERATION` scoring are not reachable from the repair's write. It also needs no amendment to
ADR 0042 — a save always inserts, and discarding the original is the user's separate decision.

Four consequences are fixed by this record.

**`V18` is additive: three columns and one index, no table rebuilt.**
`timetable.based_on_timetable_id` is the lineage, `solve_run.kind` is `GENERATION | REPAIR`, and
`solve_run.scope` is the movable set as JSON. Defaults make every existing row correct with no
data migration.

**`timetable_evaluation.kind` is NOT widened.** Its `CHECK` admits only
`('GENERATION', 'REVALIDATION')`, a `CHECK` cannot be altered in SQLite, and widening it would
mean ADR 0020's twelve-step rebuild on a `STRICT` table. A repair writes **`GENERATION`**,
because that *is* the new version's original, immutable scoring — ADR 0042 §5's meaning exactly.
`solve_run.kind` is the discriminator, and it sits with the seed, the engine and the counters,
which is where "how did this come to be" already lives.

**No eighth `UnplacedCause`.** A repair's `solve_run_unscheduled` rows cover the **movable set
only** — what this run attempted. A frozen occurrence that was already unplaced appears in the
preview as still-unplaced and is absent from the table, because the run never tried it. An
`OutOfRepairScope` variant would have needed a new value in `solve_run_unscheduled.cause`'s
`CHECK`, which is the same rebuild.

**Lineage is `ON DELETE RESTRICT`, with a typed pre-check.** `Timetables::delete` counts
repaired descendants as its first statement inside the transaction and refuses
`DbError::HasDescendants { timetables }`. The foreign key would refuse the deletion anyway, but
on the final statement and as an opaque driver failure wrapped in `InternalError` — a useless
message for a legitimate refusal. The key stays as the backstop for a future writer that forgets
the check.

**`is_manual` is cleared on a moved occurrence and carried on an unmoved one.** M9A established
`is_manual` as the record of who placed a lesson. A placement a human made and the repair did
not move is still the human's; one the solver has just re-placed is not. That is the single flag
transition a repair makes.

## Consequences

Each repair adds a row to the timetable list. ADR 0042's versioning is already row-based and the
user names each one, so this is the shape the Duplicate affordance already taught.

`save_repaired` is a sibling of `save_generated` rather than a parameter on it. The two differ in
what they write (flags, lineage), what they guard (the arrangement stamp as well as the
watermark and the source revision) and what they refuse — and `save_generated`'s hard-coded
`EntryFlags { is_locked: false, is_manual: false }` is a correct decision for a generation that
would be a bug if made configurable.

`duplicate` copies `based_on_timetable_id` rather than pointing at the row it copies: a duplicate
of a repair descends from whatever that repair was made from, and claiming otherwise would
invent a lineage step that never happened.

## Alternatives rejected

| Alternative | Why not |
| --- | --- |
| **In-place apply to the baseline** | Every write path into a live timetable is a path a stale session could take. It would need the arrangement stamp as a hard guard on a multi-row write, and a partial failure mid-apply would leave a timetable that is neither the original nor the repair |
| **Both, the user's choice** | Two write paths, one of which is only safe when several guards hold. Offer it in a later milestone if users ask for it, with the diff as evidence that they understood what they were replacing |
| **A `draft` flag on `timetable`** | A lifecycle column. `TIMETABLE-PERSISTENCE.md` §12 records the published/archived lifecycle as deliberately out of the MVP, and a draft flag is the first half of one |
| **Encode lineage in the name** (*"Draft A (repaired)"*) | A string, not a reference: it cannot be followed, it breaks on rename, and no foreign key can check it |
| **`ON DELETE SET NULL` on the lineage** | It rewrites rows without bumping `rev`, which `CLAUDE.md` §3 forbids and which already made `subject.department_id` a `RESTRICT` (R10, ADR 0025 §6). It would also silently destroy the lineage the column exists to record |
| **`ON DELETE CASCADE`** | Deleting one timetable would delete every repair derived from it. Not a defensible non-default |
