# ADR 0047 — A cross-session guard is an exact placement list, not a digest

- Status: **Accepted** (2026-10-24)
- Date: 2026-10-24
- Decides: M9A required correction 2
- Builds on: [ADR 0027](0027-revisions-are-checked-on-write.md) (the revision **is** the
  predicate; read-compare-write is not a concurrency control),
  [ADR 0042](0042-a-stored-timetable-is-a-version-and-a-save-refuses-stale-inputs.md) §4 (a
  save is refused against the change-log watermark, compared inside the writing transaction),
  [ADR 0030](0030-an-import-is-a-reviewed-plan.md) (never compare a precondition in the
  interface)
- Binds: `evara_db::repo::timetables::{ArrangementStamp, arrangement_stamp}`,
  `DbError::ArrangementChanged`, every editing write, `commands::edit_session`

## Context

M9A's brief said the editor would carry `expectedRev` on each write, per ADR 0027, and would
check the input watermark, per ADR 0042. It also said that was necessary and **not sufficient**.
It was right, and the gap is sharper than it first looks.

Consider two windows open on the same timetable. Window A has picked up occurrence *X* and is
being shown a panel of tinted drop targets — computed from one snapshot of the arrangement.
Window B moves occurrence *Y* into one of those cells. Window A now drops *X*.

Every existing guard passes:

- *X*'s row is at the revision A read it at, so `expectedRev` matches;
- nothing referential breaks — `UNIQUE (timetable_id, activity_id)` still holds;
- the timetable row's own revision does not move when an entry changes;
- the input watermark does not move either, because `timetable_entry` is in
  `SOLVE_OUTPUT_TABLES` — it is an output, so a change to it is deliberately not a change of
  input.

So the write succeeds, and **every verdict Window A has shown the user is now false**. Nothing
else catches this. It is not a hypothetical: two windows on one campus file is the ordinary way
a timetabler works on two year levels at once.

## Decision

**Every editing write carries an `ArrangementStamp`: the exact `(entry id, rev)` list of the
timetable's placements, sorted by identifier.** It is compared inside the writing transaction,
and a difference is refused as `DbError::ArrangementChanged` — deliberately not `Conflict`,
because the remedy is different.

Three properties of the shape:

1. **It is exact, not a summary.** See the alternatives below; every summary of this list loses
   the case that matters.
2. **Every write returns a fresh stamp**, so the session has no window between its own write and
   its new precondition. A sequence of edits does not trip its own freshness check — there is a
   test named for that.
3. **The comparison is the predicate**, inside the transaction, in `evara-db`. Never in the
   command layer and never in the interface: that is ADR 0027's rule and ADR 0030's, and both
   apply here for the same reason — a comparison made outside the writing transaction is the
   same race with a longer window.

## Alternatives considered

### A count-and-sum digest — rejected, it collides on a real sequence

Storing `(count, Σrev)` is the obvious compression and it fails on an ordinary pair of actions.
Unschedule an entry sitting at `rev = 1` and place a new one: the count returns to what it was
and the total is unchanged, because the removed `1` is replaced by a created `1`. The digest
says nothing moved. There is a test named for exactly that case.

Any fixed-width summary has some colliding sequence; this one's collision is a *likely* sequence,
which is what disqualifies it rather than merely making it imperfect.

### A scoped change-log watermark — rejected, it cannot see a deletion

`change_log` records `(entity_table, entity_id)`. A query scoped to this timetable's entries can
therefore find changes to entries that **still exist**, and cannot find the deletion of one it
has never heard of: to notice that entry *Z* was removed, the query would have to already know
*Z* existed, which is the list this record is about.

An unschedule in another window would pass unnoticed — the case that matters most, because the
cell it freed is the one Window A is about to be told it may drop into.

### A revision on the `timetable` row, bumped by every entry write — rejected

It would work, and it would make every edit contend on one row across every window and every
scope, turning two people working on two year levels into a queue. It would also be a revision
that does not describe the row it is on: ADR 0042's `timetable.rev` is the precondition for a
rename and a delete, and overloading it with "something about the arrangement changed" makes
both of those fail for an unrelated reason.

### Widening the input watermark to include `timetable_entry` — rejected

`SOLVE_OUTPUT_TABLES` exists to keep "an input moved" and "the result changed" apart. Collapsing
them would make a save refuse because a *different* timetable had been edited, and would make
`InputsChanged` the message for a case whose remedy is not the same.

## Consequences

- **The screen has four refusals to tell apart, not three**, and they get four different
  remedies: `locked` → offer Unlock; `conflict` → this placement moved, reload; `arrangementChanged`
  → another placement moved, **reopen**; `inputsChanged` → a scheduling input moved, reopen.
  Nothing was written in any of them, and each message says that first.
- **`arrangementChanged` and `inputsChanged` end the session rather than the edit.** The compiled
  problem, the verdicts on screen and the undo stack were all computed from one snapshot, so they
  are stale together; a retry would answer with the old one. The editor stops offering writes and
  offers **Reopen**.
- **The stamp carries a difference count** (`entriesChanged`), so a message can say how much
  moved rather than only that something did.
- **Cost is one indexed read of `(id, rev)` per write.** A realistic timetable is a few thousand
  entries; this is the cheapest part of an editing transaction and is not worth compressing, which
  is the whole argument above restated as a measurement.
