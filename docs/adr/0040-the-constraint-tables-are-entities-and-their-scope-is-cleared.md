# ADR 0040 — A scope row is an entity with a change-log entry, and its deletion is written rather than cascaded

- Status: **Accepted**
- Date: 2026-10-22
- Decides: three details of `V16` that depart from [DATA-MODEL.md](../DATA-MODEL.md) §7's
  sketch of `constraint_instance` and `constraint_ref`, and the shape of `V16`'s one new
  column.
- Builds on: [ADR 0005](0005-constraints-as-data.md) (constraints are data, scope is
  normalised), [ADR 0010](0010-structure-deletion-semantics.md) (RESTRICT for ownership,
  never a default CASCADE), [ADR 0035](0035-student-group-overlap-is-a-three-state-answer.md)
  §4 (the column), `V5`/`V6`'s headers (an association is a first-class entity).
- Binds: `V16`, which is immutable, and `evara_db::repo::constraints`.

## Context

DATA-MODEL.md §7 has sketched these two tables since before any of them existed:

```sql
constraint_instance (id, school_id, type_key, level, weight REAL NOT NULL DEFAULT 1,
                     enabled, params, term_id NULL, timetable_id NULL, …)
constraint_ref (constraint_id -> constraint_instance ON DELETE CASCADE,
                role, entity_kind, entity_id,
                PRIMARY KEY(constraint_id, role, entity_id))
```

Three details of that sketch are no longer compatible with conventions this project adopted
after it was written. A migration is immutable and there are no down migrations, so the shape
`V16` ships with is the shape every future project has; the reasoning belongs somewhere a
reader will find it rather than in a commit message.

## Decision

### 1. `constraint_ref` is a first-class entity, not a bare junction row

It carries `id`, `created_at`, `updated_at` and `rev`, and the three change-log triggers. The
uniqueness the sketch expressed as a composite primary key survives as
`UNIQUE (constraint_id, role, entity_id)`.

Forced by two schema-level tests written since the sketch —
`every_entity_table_carries_the_common_metadata_columns` and
`every_entity_table_has_all_three_change_log_triggers` — and by the triggers themselves, which
read `NEW.id` and `NEW.rev`. A composite-key table with no `id` cannot be in the change feed
at all, and `CLAUDE.md` §3 is explicit that the feed is what keeps future synchronisation
possible.

`V5`'s `building_campus` and `V6`'s `room_campus` made the same choice for the same reason, and
their headers argue it in the same words: *an association is a first-class entity, not a bare
junction row*. This is that convention applied, not a new decision.

### 2. The scope edge is `RESTRICT`, and the repository clears the rows itself

ADR 0005 says scope references get *"a cascading delete"*. That was right when
`constraint_ref` had no triggers and is not right now: **SQLite fires a `DELETE` trigger for a
row removed by a foreign-key action only when `recursive_triggers` is on**, and this project
never enables it (`connection::configure`). A `CASCADE` would therefore remove scope rows with
no change-log entry — silently, and only for the rows reached by the cascade, which is the
worst possible shape for a gap in an audit trail.

So the edge is `RESTRICT`, a trigger refuses to delete an instance while any scope row
survives, and `Constraints::delete` removes them itself — one `DELETE` each, through
`repo::delete`, inside the instance's transaction. `delete_class_section` already does exactly
this, and `V15`'s comment states the principle: a row removed this way is *"a truthful
change-log entry"*, and a scope row with no instance *"is not history, it is a dangling
fact"*.

The user-visible behaviour ADR 0005 wanted is unchanged — deleting an instance takes its scope
with it — and the change feed now says so. The scope deletions share the instance's
transaction, so a revision conflict on the instance rolls them back and a failed delete cannot
leave a rule silently unscoped.

The rows are removed at `Expected::Any`, which is the first of
[ADR 0027](0027-revisions-are-checked-on-write.md) §4's three documented kinds: a cascade
inside another operation's transaction. The caller read the instance's revision, not each
scope row's, and demanding one would mean inventing it.

### 3. `weight` is an INTEGER, and there is no `timetable_id`

`CLAUDE.md` §3: *"Scores are integers… Weights become integer penalties at compile time, not
at scoring time."* `SOLVER.md` §3's *"a float in the database"* parenthetical and the sketch's
`REAL` are corrected rather than followed. `AvailabilityRule.weight` already chose `i64` over a
float with that reasoning written down, and the `entity!` macro derives `Eq`, which `f64` does
not implement — so the record type for a `REAL` column could not have been generated without
dropping a derive every other record has.

`timetable_id` is omitted because there is no `timetable` table until M7, and a nullable
column with no foreign key would be precisely the referential hole `constraint_ref` exists to
avoid. M7 adds it as one nullable `ADD COLUMN`, which is the cheap direction. A constraint
scoped to one timetable is a real requirement and nothing here prevents it.

### 4. `student_group.children_are_disjoint` has no `DEFAULT`

`INTEGER`, nullable, `CHECK (… IS NULL OR … IN (0, 1))`, and deliberately **no default
value**. ADR 0035 §4 gives the column three states and the third is *not stated*, which is
what every existing row reads as after the `ALTER TABLE`. A `DEFAULT 0` would have made every
school in every existing project assert that its cohorts' children share students; a
`DEFAULT 1` would have made them all assert the opposite and silently licensed concurrent
placements nobody declared.

`NULL` is not a missing answer to be filled in. It is the answer *we have not been told*, and
ADR 0035's whole argument is that it must stay distinguishable from both of the others.

## Consequences

- `change_log::TRACKED_TABLES` gains both tables, so the schema-level test that discovers
  entity tables and demands three triggers passes without an exemption.
- Deleting a configured constraint is two or more statements rather than one, and every one of
  them is in the change feed.
- A scope row's `entity_id` is polymorphic and so carries no foreign key — the same hole
  `availability.entity_id` has, which `V11`'s header calls *"the one place in this schema
  where a reference cannot be enforced by the database"*. `availability` closes it three ways;
  M5 wires two of them. The repository cannot create a scope row at all (ADR 0039 §4), so the
  third — *every repository that deletes a referable entity clears its scope rows* — would be
  eleven call sites clearing rows that cannot exist, which is code no test can hold honest.
  `Constraints::orphans` is the sweep that finds anything a newer build leaves behind, and the
  call sites arrive with the first narrowable type.
- The `V14` rewind test had to learn about `V15` and `V16`: rewinding to a schema version means
  undoing **every** later migration, including dropping these two tables and the new column.

## Alternatives rejected

**Keep the composite primary key and leave `constraint_ref` out of the change feed.** Rejected
by the tests and by ADR 0003's sync-ready schema requirement. A table whose rows carry a
school's configuration decisions is exactly a table a future synchronisation has to reconcile.

**`CASCADE` plus `PRAGMA recursive_triggers = ON`.** This would make the cascade fire the
triggers and the sketch's design work as written. Rejected: that pragma changes trigger
behaviour for the *whole* database, including every existing `RESTRICT` edge and every
change-log trigger written on the assumption that it is off. Changing a global pragma to
simplify one delete is a large blast radius for a small saving, and `connection::configure` is
the one place that would have to justify it to every future table.

**`REAL` weights, with integer conversion at scoring time.** Rejected by `CLAUDE.md` §3, and
on its own merits: a float weight read at scoring time means the conversion happens inside the
hot loop and the same timetable can score differently on two machines. Integer penalties
compiled once are reproducible, which `SOLVER.md` §5 requires of a seeded search.
