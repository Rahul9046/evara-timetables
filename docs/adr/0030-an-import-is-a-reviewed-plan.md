# ADR 0030 — An import is a reviewed plan, applied in one transaction

- Status: **Accepted**
- Date: 2026-10-15
- Implements: `ROADMAP.md`'s Phase 3, Stage D — *Excel roster imports*. The roadmap entry
  is one line; §8 below records where this delivery is wider than it, and why.
- Builds on: [ADR 0011](0011-stable-timeslot-identity.md) (a preview carries a fingerprint
  and the apply recomputes it), [ADR 0019](0019-multiple-schools-in-one-project.md) (every
  school-owned write names its school), [ADR 0023](0023-the-interface-scopes-to-one-school.md)
  (the interface scopes to one school and a switch discards per-school state),
  [ADR 0027](0027-revisions-are-checked-on-write.md) (a write states the revision it was
  built on), [ADR 0029](0029-a-lesson-is-a-course-a-section-and-its-occurrences.md) (one
  activity row is one occurrence).
- Companion: [ADR 0031](0031-import-identity-matching.md), which is the matching policy.
  This record is about where the engine lives and what it guarantees; that one is about how
  a row finds the record it is talking about.

## Context

A school adopting Evara already holds its roster somewhere — a workbook, an export from
another product, a list somebody keeps. Retyping four hundred students is not a migration
path, so an import is the feature that decides whether the application is adoptable at all.

It is also the feature a school uses **before it trusts Evara**, with a file it did not
write and cannot fully describe, against a project it has just created. Every failure mode
is therefore maximally expensive: a half-finished import leaves a state nobody can reason
about, a silent overwrite destroys data the school still believes is there, and a wrongly
merged identity produces two people sharing one timetable.

Five questions had no settled answer, and each one is cheap now and very expensive once
schools have imported.

## Decision

### 1. The feature is three crates, split by what each one is allowed to know

```
evara-core::import     vocabulary        SourceCell, ImportPlan, ImportKind, …
       ↑                                 no I/O, no SQL, no file formats
       ├── evara-import                  bytes → SourceTable, and the templates
       │                                 knows xlsx and csv; knows nothing about schools
       └── evara-db::import              SourceTable → ImportPlan → one transaction
                                         knows schools; knows nothing about worksheets
```

`evara-import` is a **new crate**, and the justification is that it has a dependency set
nothing else should inherit. It carries `calamine`, `csv` and `rust_xlsxwriter`; putting
those in `evara-db` would mean the crate whose charter is SQL also links three file-format
libraries, and putting them in `evara-core` would break *no I/O, no dependencies on other
crates* outright. It depends on `evara-core` alone, which is the layering rule
`CLAUDE.md` §2 states for `evara-integrations` and for the same reason.

The vocabulary is in `evara-core` because **two crates share it and neither can own it**.
`evara-import` parses into `SourceCell` and `evara-db` plans with it; a type one of them
owned would make the other depend on it, and `evara-import → evara-db` or
`evara-db → evara-import` are both wrong — the first puts a parser above the database, the
second puts file formats below it. `evara-core` is where a shared vocabulary goes.

### 2. The engine is in `evara-db`, and that is forced rather than convenient

`Database::write` is `pub(crate)` and hands out a `rusqlite::Transaction`. **An atomic
multi-entity write is therefore only expressible from inside `evara-db`.** A public
repository method opens its own transaction; composing twelve of them would be twelve
transactions and eleven places to fail half-way.

Two alternatives were considered and rejected on the code rather than on taste:

- **`evara-project`.** Its module documentation says in terms that it is for `.evara`,
  Evara's own portable format, and *not* for other formats. A spreadsheet is somebody
  else's format.
- **`evara-integrations`.** Depends on `evara-core` only, so it could not write at all, and
  its charter is post-Milestone-1 vendor adapters. A generic spreadsheet is not a vendor.

The engine composes writes through `pub(crate)` transaction-scoped helpers extracted from
the existing repository methods — `insert_teacher`, `update_teacher_row`, `insert_course`
and so on — rather than restating the column lists. That is the point of the extraction: a
column added to a repository's `INSERT` cannot reach the interactive path and miss the
import, because there is one `INSERT`. The 594 pre-existing `evara-db` tests passing
unchanged is what says the extraction was behaviour-preserving.

### 3. Preview writes nothing. Not "nothing that matters" — nothing

`Import::plan` runs inside `db.read`. It resolves every reference, validates every cell,
and returns an `ImportPlan`; it has no transaction and no write path.

`tests_import::previewing_writes_nothing_at_all` asserts this **over the change feed**
rather than over row counts, and the difference matters: a write that was rolled back would
leave no rows and would still have appended a `change_log` entry. The feed is the only
instrument that can tell "nothing happened" from "something happened and was undone".

### 4. Apply is one transaction. There is no partial import

`Import::apply` runs inside `db.write`. Every row's intent executes in that one
transaction, so a constraint failure on row 41 of 41 leaves none of the first 40 — no rows,
no consumed revisions, no change-log entries.

This is the reason §2's conclusion is not negotiable, and it is asserted directly:
`a_constraint_failure_rolls_the_whole_import_back` imports two valid departments and a
third the repository refuses, then asserts the department count is zero *and* that the feed
is exactly as long as it was.

The alternative — import what can be imported, report the rest — was rejected. A school
that corrects its file and imports again would then create everything twice, because the
first run's successes are indistinguishable from rows it never reached.

### 5. The plan the user approved is the plan that runs

Every `ImportPlan` carries an `ImportFingerprint`: FNV-1a over the kind, the school, and
every row's line, action and resolved target, sorted. Apply sends it back, and **apply
rebuilds the plan inside its own write transaction** and refuses `DbError::ReviewRequired`
if the two disagree.

The recomputation is the guarantee; the token only lets the disagreement be *named*. A row
somebody created in another window since the preview changes the plan, so the mismatch is
reported rather than applied — and the fix is to look again, which is why the interface
re-previews and says nothing was imported rather than reporting an error.

The findings' **wording is deliberately not material** to the fingerprint. A reworded
message must not invalidate a plan a user has already reviewed; what they agreed to is the
set of actions, not the prose.

The school is material, which is ADR 0023's backend half: a plan previewed against
Southmoor cannot be applied to Northgate, because the fingerprint would not match.
`a_fingerprint_from_a_different_school_does_not_apply_here` asserts it.

### 6. Nothing is created to fill a gap

A row naming a department, subject, course, section, year level, campus, room type or
student group that this school does not hold is **an error on that row**. No placeholder is
created, ever.

A placeholder would have no code anybody chose, no ordinal anybody chose, and would appear
in the school's own setup screens as though they had created it — and the school would then
have to find and remove it, having never been told it existed.

The consequence is that the imports have an order, and the type chooser says so:
`ImportKind::depends_on` is a sentence, shown before the file is chosen, so a school
importing enrolments into courses it has not created is told why rather than left to read
two hundred identical row errors.

### 7. An unmapped column is silence; a mapped blank column is an answer

Leaving a field unmapped means the import has **no opinion** about it: the stored value
survives. Mapping a column that happens to be blank means *there is no value* and the field
is cleared.

The distinction is what makes a second import able to *correct* the first, and it is the
only way "remove this teacher's email address" is expressible from a spreadsheet. Both
halves are asserted — `an_unmapped_column_is_left_alone_rather_than_blanked` and
`a_mapped_but_blank_column_clears_the_value_it_was_mapped_to`.

It is also what makes an unchanged row honestly unchanged. `Unchanged` means *the mapped
fields already say this*, so importing the same file twice consumes no revision and appends
nothing to the change feed — asserted, over the feed, in
`importing_the_same_teacher_file_twice_changes_nothing_the_second_time`.

### 8. What this delivers beyond the roadmap's line, and what it does not

The roadmap says *Excel roster imports* and lists *CSV import* under After Milestone 1.
Three differences, stated rather than buried:

- **CSV is included.** The same engine reads both; the format is decided in one function
  (`SourceFormat::of`) and the difference is forty lines of parser. Refusing CSV would have
  been a deliberate restriction with its own code and its own error message, and schools
  exporting from another product more often get a CSV than a workbook. The After-M1 entry
  is corrected accordingly.
- **Ten kinds, not three.** The roster is teachers, students and enrolments; the brief asks
  for departments, subjects, courses, class sections, class teachers, class groups and
  activities as well. One generic engine covers all ten — the per-kind code is one function
  each in `kinds.rs` — so building three and leaving seven would not have been less work,
  only less useful.
- **`.xls` is refused**, by name, with the remedy. The legacy binary format needs a second
  parser and a second set of quirks, and "save as .xlsx" is a thing every spreadsheet can
  do. The refusal names the file's OLE2 signature so a renamed `.xls` is caught too.

Not delivered, and deliberately: no formula evaluation (see §9), no `.xls`, no import of
availability, the time model, rooms, buildings or resources, and no vendor-specific column
dictionaries — those are `evara-integrations`, after Milestone 1.

### 9. A formula is never evaluated, and a cached result is used only when the file stored one

`calamine` has no formula engine, which is the behaviour this decision wants rather than a
limitation to work around. A cell holding a formula reads as whatever the writing
application stored as its last computed value.

Where the workbook stored **no** value, the cell becomes
`SourceCell::Unsupported` — reported as "this cell holds a formula and the workbook did not
store its result; Evara never calculates one" — and the row is an error. The alternatives
were to treat it as blank, which would silently import a gap, or to evaluate it, which
means executing arbitrary expressions out of an untrusted file.

Macros are refused at the extension: `.xlsm` and `.xltm` are `ImportFileError::MacroWorkbook`
with the remedy, and nothing in the stack can run one.

### 10. Identifiers survive as text, and a number is refused rather than guessed

`SourceCell` keeps both the text a cell displayed and its numeric interpretation.
`SourceCell::code()` **refuses a numeric cell** with `CellProblem::NumericCode`, because by
the time a code arrives as a number its leading zeros are already gone and cannot be
recovered: student `00124` has become `124`, which is a different student.

The message names the remedy — format the column as text and export again — and the
downloadable workbook template already does it (`set_num_format("@")` on every code
column), which is why the workbook template is the one offered first.

Dates are `YYYY-MM-DD` and nothing else. `01/09/2026` is the first of September in Britain
and the ninth of January in the United States; there is no reading of that file which is
safe to guess, so it is an error naming the one format Evara reads.

## Consequences

- **A preview can be slow on a large file and costs nothing.** It is a read; the limits
  (8 MiB, 20,000 rows, 128 columns, 4,096 characters a cell) exist so a damaged or hostile
  file cannot ask for an unbounded allocation, and a file longer than the row limit is
  **truncated *and says so*** — `truncated_at` is reported and the interface shows the row
  it stopped at. Silent truncation would be the worst available failure of this feature.
- **The backend holds the parsed file for the session.** The interface sends the bytes once
  and then previews as often as it likes. That is mostly a safety property rather than an
  optimisation: if the interface re-sent the file each time, the file *applied* could differ
  from the file *reviewed* and the fingerprint would not notice, because it is computed over
  the plan. One import at a time, replaced when another file is chosen, forgotten when the
  project closes.
- **Writing a file is a save dialogue, and the backend takes no content.** `import_template`
  and `import_report` prepare bytes and return the name to suggest; `import_save` takes a
  path and nothing else. There is no way for the interface to make the backend write
  something the backend did not generate, and the only authority the write has is a path
  that came from an OS dialogue the page cannot script — the arrangement `project_create`
  has had since Phase 1B.
- **Reading a file is `<input type="file">`, not a path.** `SchoolLogoPanel`'s security
  decision, unchanged and for the same three reasons: no filesystem capability, no path to
  traverse, and it works under Vitest.
- **The report is written by the backend.** One renderer, from the backend's own record of
  what happened rather than from a summary the interface hands back, so the file a school
  keeps cannot disagree with what Evara did. Values beginning `=`, `+`, `-` or `@` are
  prefixed with an apostrophe there and **only** there: the parser does not edit what it
  reads — the file says what it says — so the report is where a cell that Excel would run
  as a formula is made inert.
- **No migration.** Stage D adds no table, no column and no trigger. It is the first stage
  of Phase 2 that is pure behaviour, which is a property worth keeping: an import that
  needed its own schema would be an import that had invented a concept.

## Alternatives rejected

- **Import directly, row by row, reporting failures.** Rejected by §4: no partial imports.
- **One transaction per row.** Same objection, and worse — it makes "import again after
  correcting the file" unsafe, which is the workflow schools actually use.
- **Compare the fingerprint in the interface.** A check-then-act race, and the same mistake
  ADR 0011 rejected for the grid.
- **Guess the date format from the file.** A column of `01/09`–`12/09` values is consistent
  under both readings, so the guess would be right for months and silently wrong for a
  school in the other hemisphere.
- **Create missing references as placeholders, flagged for review.** Rejected by §6. The
  review never happens, and the placeholder is indistinguishable from a record the school
  made.
- **Hold the parsed file in the interface and send it with each call.** Rejected by the
  consequence above: it makes substituting the file between review and apply expressible.
