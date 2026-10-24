# Architecture Decision Records

One file per decision. A record is written when a decision is hard to reverse, when it
rejects a reasonable alternative, or when a future contributor would otherwise be
tempted to undo it without knowing why.

| ADR | Decision | Status |
| --- | --- | --- |
| [0001](0001-local-first-desktop.md) | Local-first Tauri desktop app, no server, no authentication | Accepted |
| [0002](0002-rust-in-process-solver.md) | Rust in-process solver; do not adopt Timefold or a solver service | Accepted |
| [0003](0003-sqlite-document-model.md) | SQLite canonical store, one file per project, sync-ready schema | Accepted — *"multiple schools means multiple files"* superseded by [0019](0019-multiple-schools-in-one-project.md) |
| [0004](0004-evara-project-format.md) | `*.evara` is a versioned ZIP of JSON, separate from external interchange | Accepted |
| [0005](0005-constraints-as-data.md) | Constraints are data; code provides an evaluator registry | Accepted |
| [0006](0006-group-based-scheduling-first.md) | Group-based scheduling first; request/sectioning schema preserved | Accepted |
| [0007](0007-rusqlite-refinery.md) | rusqlite + refinery, bundled SQLite, one connection factory | Accepted |
| [0008](0008-installation-identity-and-project-lifecycle.md) | Installation identity, settings database, `.evaraproj` | Accepted |
| [0009](0009-typescript-type-generation.md) | `ts-rs` for generated TypeScript types | Accepted |
| [0010](0010-structure-deletion-semantics.md) | RESTRICT ownership, SET NULL classification; opaque driver errors | Accepted — one deliberate departure, for `subject.department_id`, in [0025](0025-teacher-qualifications-across-school-owned-subjects.md) §6; [0029](0029-a-lesson-is-a-course-a-section-and-its-occurrences.md) §7 applies the rule unchanged to `V15` and says why that departure does not carry over |
| [0011](0011-stable-timeslot-identity.md) | Timeslot logical identity; materialisation never deletes | Accepted |
| [0012](0012-records-are-the-ipc-contract.md) | Repository records are the IPC contract; no parallel DTO layer | Accepted |
| [0013](0013-buildings-shared-across-campuses.md) | A building may serve many campuses; `building_campus` is the association set | Accepted |
| [0014](0014-rooms-shared-across-campuses.md) | A room may serve many campuses; one identity, so occupancy is global | Accepted |
| [0015](0015-school-working-days.md) | School working days are a calendar default, stored as a seven-bit set; `cycle.week_count` retired from the UI only | Accepted |
| [0016](0016-school-logos-in-the-project.md) | School logos are BLOBs in the project, validated from their bytes; Evara's mark is the fallback | Accepted |
| [0017](0017-weekly-and-custom-cycles.md) | A cycle is weekly or custom; a weekly cycle holds all seven weekdays and no school's calendar | Accepted |
| [0018](0018-people-identity-and-deactivation.md) | People keep their identity; leaving is a state, not a deletion | Accepted — amended by [0021](0021-project-level-people-and-school-membership.md), which gives *leaving* two scopes |
| [0019](0019-multiple-schools-in-one-project.md) | A project may describe several schools, each owning its own academic structure | Accepted |
| [0020](0020-in-place-table-rebuild.md) | A table can be rebuilt in place under deferred foreign keys, inside a migration | Accepted |
| [0021](0021-project-level-people-and-school-membership.md) | People belong to the project; membership, eligibility and enrolment belong to the school | Accepted |
| [0022](0022-resources-shared-across-campuses.md) | A resource may serve many campuses, and so many schools; one identity, so availability is global | Accepted |
| [0023](0023-the-interface-scopes-to-one-school.md) | The interface scopes to one school, chosen once; project-wide reads are composed, and a certain refusal is explained rather than offered | Accepted |
| [0024](0024-setup-is-ordered-by-what-a-school-knows.md) | School Setup is ordered by what a school knows rather than by dependency; a hidden field is carried, not dropped | Accepted |
| [0025](0025-teacher-qualifications-across-school-owned-subjects.md) | A qualification is a property of the person and names one school's subject; no project-wide subject catalogue | Accepted — §4's conjunction amended by [0028](0028-assigning-a-lesson-warns-about-qualifications.md), which keeps two of the three conditions |
| [0026](0026-teacher-departments-are-a-narrowing-of-membership.md) | A teacher's department is a narrowing of school membership, and is its own table — not `teacher.department_id` | Accepted |
| [0027](0027-revisions-are-checked-on-write.md) | A write states the revision it was built on; a stale one is refused, never merged | Accepted |
| [0028](0028-assigning-a-lesson-warns-about-qualifications.md) | Assigning a lesson requires employment and campus eligibility; a missing qualification warns | Accepted — amends [0025](0025-teacher-qualifications-across-school-owned-subjects.md) §4 |
| [0029](0029-a-lesson-is-a-course-a-section-and-its-occurrences.md) | A lesson is a course, a class section, and one row per occurrence; no table in the chain holds a `school_id` | Accepted |
| [0030](0030-an-import-is-a-reviewed-plan.md) | An import is a reviewed plan applied in one transaction; preview writes nothing and nothing is created to fill a gap | Accepted |
| [0031](0031-import-identity-matching.md) | A code matches, a name asks; reuse across schools is offered and never assumed | Accepted |
| [0032](0032-timetable-generation-is-scoped-to-one-school.md) | Generation is scoped to one school's grid; shared occupancy is compared in real time within a school and reported across schools | Accepted — moved from *Proposed* in M1, every premise re-verified and one strengthened |
| [0033](0033-an-activity-schedules-against-one-resolved-bell-schedule.md) | An activity schedules against exactly one bell schedule, resolved by campus compatibility; an ambiguous one is refused, never guessed | Accepted |
| [0034](0034-expected-attendance-is-explicit-and-room-capacity-waits-for-it.md) | Expected attendance is a stated number or it is unknown; `room-capacity` leaves the first constraint set | Accepted |
| [0035](0035-student-group-overlap-is-a-three-state-answer.md) | Whether two student groups share students has three answers, and *indeterminate* is one of them | Accepted |
| [0036](0036-one-bell-schedule-resolver-shared-by-validation-and-loading.md) | One bell-schedule resolver, shared by validation and loading | Accepted |
| [0037](0037-stored-row-diagnostics-belong-to-the-loader.md) | A finding about a stored row belongs to the loader, not to the validator; `Problem` is not widened to carry rejected rows | Accepted |
| [0038](0038-counting-unanswered-questions-is-not-evaluating-overlap.md) | Counting unanswered questions is not evaluating overlap; one shared hierarchy walk serves both | Accepted |
| [0039](0039-a-mandatory-constraint-lives-in-the-registry.md) | A rule whose absence would make a timetable wrong lives in the registry, not in a row; a stored row tunes it and may raise but not weaken it | Accepted — amends the scope of *data* in [0005](0005-constraints-as-data.md) |
| [0040](0040-the-constraint-tables-are-entities-and-their-scope-is-cleared.md) | A scope row is an entity with a change-log entry, and its deletion is written rather than cascaded | Accepted |
| [0041](0041-the-incremental-scorer-lives-with-the-evaluators.md) | The incremental scorer lives with the evaluators, and derives its delta from a property of the rules rather than hand-writing one | Accepted — implements [SOLVER.md](../SOLVER.md) §4 and discharges R1 |
| [0042](0042-a-stored-timetable-is-a-version-and-a-save-refuses-stale-inputs.md) | A stored timetable is a version among several, a save refuses inputs that moved since generation, and a revalidation never overwrites how a timetable was generated | Accepted — corrects six details of [DATA-MODEL.md](../DATA-MODEL.md) §7's sketch |
| [0043](0043-a-report-renders-a-rules-own-explanation.md) | A report renders the rule's own sentence by calling `explain()`, and a stored `violation` row carries no prose | Accepted |
| [0044](0044-the-display-vocabulary-sits-beside-the-problem.md) | The names a grid needs and a solve does not live beside the `Problem`, never inside it | Accepted — extends [0037](0037-stored-row-diagnostics-belong-to-the-loader.md) §2 to a report-only field |
| [0045](0045-a-lock-is-a-property-of-a-placement.md) | A lock is `timetable_entry.is_locked` — per version, never `activity.is_locked`; and the loader compiles no pins | Accepted — corrects five records that said `activity.is_locked` |
| [0046](0046-an-edit-changes-the-version-it-is-open-on.md) | An edit changes the version it is open on; duplicating first is the user's decision, and an undo is a new transaction rather than a rollback | Accepted — implements the guarantee [0042](0042-a-stored-timetable-is-a-version-and-a-save-refuses-stale-inputs.md) §5 was written for |
| [0047](0047-a-cross-session-guard-is-an-exact-placement-list.md) | An editing write carries the exact `(entry id, rev)` list of the arrangement; a digest collides and a scoped watermark cannot see a deletion | Accepted |

Statuses: **Proposed** (awaiting a decision), **Accepted**, **Superseded by NNNN**,
**Deprecated**. Accepted records are not edited to reflect a change of mind — a new
record supersedes them.

Decisions still open are tracked in [../OPEN-DECISIONS.md](../OPEN-DECISIONS.md) and
graduate to an ADR once settled.
