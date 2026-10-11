# 0015 — School working days are a calendar default, stored as a seven-bit set

**Status:** Accepted
**Date:** 2026-10-11
**Supersedes:** nothing. Extends [0011](0011-stable-timeslot-identity.md) §6 and
[0003](0003-sqlite-document-model.md).

## Context

A school needs to state which weekdays it normally teaches on. Monday to Friday across
most of Europe and the Americas, Sunday to Thursday across much of the Middle East, six
days in parts of South Asia. The application had no way to record it.

Two things made this delicate rather than routine.

First, `CLAUDE.md` is categorical that **nothing in scheduling may read a weekday**, and
ADR 0011 §6 makes `cycle_day.ordinal` a day's only scheduling identity. A setting called
"working days" is the most natural place in the product for that rule to start leaking —
by inferring Monday to Friday from a five-day cycle, by deriving a cycle day from a date's
weekday, or by generating calendar mappings to match the week.

Second, the same screen was being simplified: "Calendar weeks spanned" (`cycle.week_count`)
was removed from Cycles and Cycle Days in the same change, which raised the question of
whether a column that is no longer collected should be dropped.

## Decision

### 1. A working day is a calendar fact, never a scheduling identity

`WorkingDays` lives in a new `evara_core::calendar` module, deliberately separate from
`evara_core::time`, and the separation is the point:

- a **cycle day** is a position in a rotation and has no weekday;
- a **working day** is a fact about the institution's week.

Three consequences are binding:

- **The cycle's length is unrelated.** Nothing reads `cycle.day_count` to decide a working
  week, and nothing reads the working week to decide a cycle's length. A six-day rotation
  across a five-day working week is an ordinary school.
- **A five-day week does not mean Monday to Friday.** The set is stored, never inferred
  from a count.
- **An explicit `calendar_day` row always wins.** The working week is a default pattern.

### 2. Disagreement is reported, never resolved by writing

`TimeModel::non_working_instructional_days` returns mapped teaching dates — a
`calendar_day` with a non-null `cycle_day_id` — whose weekday is not a working day. It
reads and nothing else.

Changing a school's working days creates, deletes and rewrites no calendar mapping. A
school really might teach one Saturday for an open day, and a build that "tidied" such a
row away would destroy a deliberate decision that the timetable depends on. Most findings
are a date entered a day out, which is exactly why the user and not the application
decides. Automatic calendar generation is explicitly out of scope here.

This is the one place a weekday is read, and it is read from a real date on an explicit
calendar row, compared against a stated fact about the institution. No cycle day is
derived, inferred or altered.

### 3. A column on `school`, not a `school_working_day` table

ADR 0013 established that "an association is a first-class entity, not a bare junction
row". That governs building-to-campus, where both ends are real rows with their own
identities, their own audit columns and their own place in the change feed.

There is no second entity here. A weekday is not a row; the working week is a set-valued
attribute of the single school a project describes. A table would add an entity, a
deletion rule and seven rows per school to express one fact.

### 4. A seven-bit set in one INTEGER

Bit 0 Monday through bit 6 Sunday. `DEFAULT 31` is Monday to Friday.

This is the only encoding that makes **"a school must have at least one working day" a
property of the schema**: a non-empty set is exactly `1..=127`, so
`CHECK (working_days BETWEEN 1 AND 127)` is the whole rule. The alternatives cannot do
that:

- *Seven boolean columns* would need a trigger to enforce "at least one", and seven columns
  for one concept makes the set awkward to iterate in both Rust and TypeScript.
- *A comma-separated `TEXT`* could not be checked meaningfully at all.

In both of those, the repository and the form would be the only things standing between a
school and a week it never teaches in.

The cost is that `31` is not self-describing to someone reading the file in a SQLite
browser. ADR 0003's readability argument is about identifiers — values copied between
tables and quoted in bug reports. This value is read through one accessor and never
cross-referenced, so the invariant is worth more than the legibility.

**The bit layout stops at the repository.** Across IPC and in an export the value is an
ascending list of ISO weekday numbers, `[1, 2, 3, 4, 5]`, which is what an interface wants
to render and matches the 1..7 that `cycle_day.weekday_hint` already used. `FromColumn for
WorkingDays` is the only code that knows about bits.

### 5. An out-of-range stored value is corruption, not a default

`WorkingDays::from_bits` refuses anything outside `1..=127`, zero included, and the
decoder reports it rather than substituting Monday to Friday. Same rule as `parse_id`
(ADR 0011) and for the same reason: an invented default makes a damaged file look like a
working one, and a school that silently claims to teach Monday to Friday when its file
says otherwise is worse than one that refuses to open.

### 6. `cycle.week_count` stays in the schema

The field is gone from the forms and the table. The column is not, and will not be dropped
by this change:

- a released migration is immutable and there are no down migrations;
- `ALTER TABLE ... DROP COLUMN` on a `STRICT` table with the column's `CHECK` is not a
  safe operation inside refinery's single transaction, for the same reason ADR 0013 kept
  `building.campus_id`: `PRAGMA foreign_keys` is a no-op inside a transaction, so the
  documented table-rebuild procedure is unavailable, and a rebuild of `cycle` with foreign
  keys on would interact with `cycle_day` and `timeslot`'s `RESTRICT` edges;
- `CycleInput` still carries it, so a save must supply a value.

The interface therefore preserves the stored value on edit and sends `1` for a new cycle,
which means **no existing cycle record changes**. A future cleanup is reasonable but is its
own migration with its own tests — see the Fix 6 report.

## Consequences

**Good.**

- "At least one working day" cannot be violated by any code path, including a future one.
- An existing project upgrades with no data migration: SQLite returns `DEFAULT 31` for rows
  written before the column existed, so every existing school keeps its `rev`, its
  `updated_at` and its change-log history untouched. The schema changed; the school did not.
- `school` has been in `change_log::TRACKED_TABLES` since V3 and the triggers name no
  columns, so the change feed covered the new column before it existed.
- The cycle/weekday separation is now stated in a module whose header exists to defend it,
  rather than being a convention spread across comments.

**Bad, or at least accepted.**

- `31` is opaque in a SQLite browser.
- `cycle.week_count` is now a column nothing collects — dead weight in the schema and one
  more field a `CycleInput` has to carry.
- The advisory scans every instructional `calendar_day` row rather than a range. A school
  year is a few hundred rows, so this is cheap, but it is a full scan and will want an
  index if calendar volumes ever grow.
- `WorkingDays` has a hand-written `Serialize`, `Deserialize` and `ts_rs::TS`. That is
  three impls to keep in agreement, and the price of the wire format being independent of
  the storage layout. A round-trip test over all 127 possible weeks guards it.

## Alternatives rejected

| Alternative | Why not |
| --- | --- |
| `school_working_day` table | No second entity to associate with; cannot enforce "at least one" without a trigger |
| Seven boolean columns | Needs a trigger for the invariant; awkward to iterate; seven columns for one fact |
| Comma-separated `TEXT` | Unverifiable by a `CHECK`; readable, but nothing else |
| Derive the week from `cycle.day_count` | Exactly the inference ADR 0011 §6 forbids; wrong for Sunday-to-Thursday and for every six-day school |
| Generate `calendar_day` rows from the week | Out of scope, and would overwrite deliberate mappings the timetable depends on |
| Expose the bitmask over IPC | Pushes a storage decision into the view layer |
| Drop `cycle.week_count` now | Unsafe table rebuild inside refinery's transaction; not needed to retire the field from the UI |
