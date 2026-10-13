# ADR 0020 — A table can be rebuilt in place, under deferred foreign keys

- Status: **Accepted**
- Date: 2026-10-13
- Amends: [ADR 0013](0013-buildings-shared-across-campuses.md) and
  [ADR 0014](0014-rooms-shared-across-campuses.md), which recorded the rebuild as
  unavailable; and corrects the same claim in the comments of `V9__cycle_kind.sql`.
- Required by: [ADR 0019](0019-multiple-schools-in-one-project.md)

## Context

`V12` had to remove a table-level `CHECK` and `UNIQUE` from `school`. SQLite can drop
neither with `ALTER TABLE`, so the table has to be rebuilt — and this project had already
concluded, twice, that rebuilding a table was impossible here.

ADR 0013 put it this way, and ADR 0014 and `V9` repeat it: the documented twelve-step
rebuild requires `PRAGMA foreign_keys = OFF`; that pragma is **a no-op inside a
transaction**; refinery runs every pending migration inside one transaction
(`set_grouped(true)`); therefore the procedure is unavailable, and so `building.campus_id`
and `room.campus_id` survive as columns that cannot be dropped, and `cycle`'s weekly-length
rule is a trigger pair rather than a table `CHECK`.

Every part of that reasoning is correct. It is also incomplete.

## Decision

**Rebuild the table in place, under `PRAGMA defer_foreign_keys`.**

The rebuild does not need enforcement *disabled*. It needs it *postponed* to COMMIT — and
`defer_foreign_keys` is honoured inside a transaction where `foreign_keys` is not.

```sql
PRAGMA defer_foreign_keys = ON;
CREATE TABLE school_rebuild AS SELECT * FROM school;   -- copy out
DROP TABLE school;                                      -- implicit delete, deferred
CREATE TABLE school (…without the constraint…) STRICT;  -- SAME NAME
INSERT INTO school (…) SELECT … FROM school_rebuild;    -- same identifiers
DROP TABLE school_rebuild;
```

### Why keeping the name is the whole trick

Fourteen tables reference `school (id)`. Because nothing is renamed, **not one of their
foreign-key clauses is rewritten** — the hazard the twelve-step procedure exists to manage.
SQLite resolves a foreign key's parent by name at DML time, so the window in which those
tables point at a table that does not exist is simply never observed: no DML touches them
inside it, and by COMMIT the rows are back with identical identifiers.

### Deferred is not disabled

This is the property the whole approach rests on, and it was tested directly: run the
rebuild and deliberately **do not restore the rows**. The COMMIT is refused and the project
is untouched. A rebuild that lost its data therefore cannot commit —

**a stronger guarantee than the twelve-step procedure**, where enforcement is genuinely off
and only a later `foreign_key_check` would notice. `tests_multi_school.rs` asserts it, on
the bundled engine, as `a_rebuild_that_loses_its_rows_cannot_commit`.

### Evidence

Verified on the exact engine Evara ships — SQLite **3.50.2** via `libsqlite3-sys` 0.35 —
and asserted as Rust tests so it stays verified:

```text
foreign_keys = OFF inside a txn -> 1   (ignored; ADR 0013's premise confirmed)
defer_foreign_keys = ON inside a txn -> 1   (honoured)
```

Against a fixture built from the real `V1`–`V11` files and seeded across 21 tables: every
identifier, revision, timestamp and logo byte preserved; `foreign_key_check` empty;
`integrity_check` ok; a second school insertable; "Grade 1" in both schools; a duplicate
within one school still refused.

### Rejected alternatives, both tested

**`PRAGMA legacy_alter_table` plus a rename-swap.** `ALTER TABLE school RENAME TO
school_old` with the legacy pragma on does leave referencing clauses naming `school`, as
intended — but `DROP TABLE school_old` then fails with a foreign-key error, **with or
without deferred enforcement**. Not pursued further: the in-place rebuild needs no rename,
and `legacy_alter_table` is connection state that does not reset at COMMIT, so leaving it
on would change `ALTER TABLE` semantics for every later migration in the same batch.

**`PRAGMA writable_schema` surgery on the stored DDL.** Edits the `CREATE TABLE` text and
moves no data at all, which is superficially attractive. Refused outright by a defensive
build (`table sqlite_master may not be modified`), and unsound regardless: a `UNIQUE`
constraint creates an **implicit index** (`sqlite_autoindex_school_1`) which is its own
`sqlite_master` row that no amount of editing the table's DDL removes. The rebuilt table
would still have had an index enforcing one row.

**The twelve-step rebuild in a step outside refinery's transaction.** Works, and was
tested. Rejected because it needs application code in the open path, outside the batch that
guarantees all-or-nothing — so a failure can leave a committed intermediate state that the
application must then detect and handle. The in-place rebuild has no such state to detect.

### Change-log behaviour

The rebuild **emits no change-log entries**, which is correct: a schema change is not an
edit to a school. `DROP TABLE`'s implicit delete does not fire triggers, and the
replacement table has none yet when its rows are restored.

The table's three change-log triggers go with the dropped table and are recreated by
`change_log::install_all`, which runs on every open immediately after migrations with
`CREATE TRIGGER IF NOT EXISTS` — so they are back before the application can write a row.
`a_failure_after_the_rebuild_rolls_the_whole_batch_back` asserts that a rollback restores
them too.

**One consequence to carry forward:** a future synchronisation must read the schema version,
never the change feed, to notice that a rebuild happened.

### The safety copy

Because this is the first rebuild in the product's life, the open path takes a verified copy
of the project first and **refuses the upgrade if it cannot be made or verified**.

`VACUUM INTO`, not a file copy. A project runs in WAL mode, so its committed state is split
between the `.evaraproj` file and its `-wal` sidecar; copying the main file alone yields a
database that opens cleanly and silently lacks the most recent work, which is worse than no
backup. `VACUUM INTO` runs in a read transaction and writes a complete, checkpointed
database, and SQLite refuses to write it if the target exists — which is also the "never
overwrite an existing backup" rule, enforced by the engine rather than by a `try_exists`
race.

The copy is verified before the migration proceeds — it opens, `integrity_check` is ok, it
looks like an Evara project, and it holds the same number of school rows — and is deleted
again if it is not sound, because leaving a broken file named "backup" is how someone comes
to trust one. It is named `<project>.pre-vN.evaraproj`, keeps the project extension because
it **is** a project (the recovery is to open it, not to rename it first), and takes the next
free `-2`, `-3` … name rather than overwriting the evidence of an earlier attempt. Nothing
in Evara ever deletes it.

## Consequences

- `V12` can do what it needs to, inside the existing migration machinery, with the
  all-or-nothing guarantee intact.
- **A table rebuild is now an available technique.** That changes what is possible for
  `availability.school_id`'s `NOT NULL`, among others.
- **Do not reopen shipped designs on the strength of this.** `building.campus_id`,
  `room.campus_id` and `cycle`'s trigger pair all work, a released migration is immutable
  (`CLAUDE.md`), and "we could now do it differently" is not a reason to migrate a
  populated database. The option exists; using it needs its own justification.
- A rebuild is invisible to the change feed, as recorded above.
- `backup::FIRST_REBUILDING_MIGRATION` names the version this protection applies below. A
  later migration that rebuilds a table should take its own copy, which means deciding
  deliberately rather than inheriting this one's constant.

## Alternatives considered

Covered above, each tested rather than reasoned about: `legacy_alter_table` rename-swap
(fails), `writable_schema` surgery (refused, and unsound), and the twelve-step procedure
outside refinery's transaction (works, worse failure profile). Doing nothing was not
available: ADR 0019 needs the constraint gone, and no `ALTER TABLE` can remove it.
