# Importing a roster from a spreadsheet

The reference for what Evara reads out of a workbook, what it does with each row, and what
it refuses. Written for whoever is preparing the file as much as for whoever is reading the
code.

The decisions behind it are [ADR 0030](adr/0030-an-import-is-a-reviewed-plan.md) — an
import is a reviewed plan, applied in one transaction — and
[ADR 0031](adr/0031-import-identity-matching.md) — a code matches, a name asks.

**Evara never sends your file anywhere.** It is read on this computer, by this
application, and the bytes do not leave it. Nothing in the import path opens a socket.

---

## 1. The five promises

These hold for every kind of import, and the screen states them before the first button.

| | |
| --- | --- |
| **Nothing is written until you say so** | A preview reads the project and changes nothing in it. Asserted over the change feed, not over row counts: a write that was rolled back would still be in the feed. |
| **All or nothing** | If any row fails, the whole import is undone. There is no half-imported state to clean up, and no risk of creating everything twice when you correct the file and import again. |
| **Existing records are never overwritten silently** | Where the file disagrees with what Evara holds, the row is a question with four equally weighted answers. |
| **Nothing is invented** | A row naming a subject, course, year level or anything else Evara does not hold is reported. No placeholder record is created. |
| **Importing twice is safe** | Rows that already match are left exactly as they are, including their revisions. (With one caveat — see §6.) |

---

## 2. What can be imported, and in what order

Ten kinds. Each is a separate import, into one explicitly selected school.

| Kind | Needs first |
| --- | --- |
| Teachers | — |
| Students | — |
| Departments | — |
| Subjects | Departments, if a subject names one |
| Courses | Subjects |
| Class sections | Courses |
| Class teachers | Class sections, and teachers employed here |
| Class groups | Class sections, and this school's student groups |
| Class enrolments | Class sections, and students enrolled here |
| Activities | Class sections |

The order is a consequence of §1's fourth promise: nothing is invented, so what a row
refers to has to exist already. The type chooser says what each kind needs before you pick
a file, rather than letting you discover it as two hundred identical row errors.

---

## 3. The file

| | |
| --- | --- |
| **Formats** | `.xlsx` and `.csv` |
| **Not** `.xls` | The legacy binary format is refused by name, including a renamed one — it is detected from the file's own signature. Save as `.xlsx` instead. |
| **Not** `.xlsm` / `.xltm` | Macro-enabled workbooks are refused. Nothing in Evara can run a macro, and a file that expects one is a file whose values may be missing. |
| **Size** | Up to 8 MiB |
| **Rows** | Up to 20,000 data rows. A longer file is read up to that point **and says so**, naming the row it stopped at. Split it and import each part. |
| **Columns** | Up to 128 |
| **One cell** | Up to 4,096 characters |

A workbook may hold several worksheets; Evara reads the first and offers the rest, without
asking for the file again.

### The header row

Row 1 is the headings. Every row number Evara shows you — in the preview, in a message, in
the report — is **the row number your spreadsheet shows**, counting the header as row 1. An
off-by-one here would send somebody to correct the wrong line, so there isn't one.

A wholly blank line in the middle of a CSV still counts as a row, because your spreadsheet
counts it.

### Formulas

Evara **never calculates a formula**, which is the behaviour this wants rather than a
limitation. A formula cell reads as whatever your application stored as its last computed
result. Where the file stored no result — some exporters write the formula and no value —
the cell is reported as unsupported and the row is an error, rather than being read as
blank and silently importing a gap.

### Identifiers, and why the template matters

**A code column must be formatted as text.** This is the one thing most likely to go wrong,
and it goes wrong before Evara ever sees the file: a spreadsheet storing `00124` as a number
has already discarded the leading zeros, and `124` is a different student.

Evara refuses such a cell rather than guessing — "this arrived as a number, so any leading
zeros are already gone and cannot be recovered" — and names the remedy. **The downloadable
Excel template already formats every code column as text**, which is why it is the one
offered first. If you export from another system instead, format those columns yourself
before saving.

The same applies to `T-0007`: it survives because it is not a number, but `0007` would not.

### Dates

**`YYYY-MM-DD`, and nothing else.** `2026-09-01`.

`01/09/2026` is the first of September in Britain and the ninth of January in the United
States. There is no reading of that file which is safe to guess, so it is an error naming
the one format Evara reads. A real date in an Excel date-formatted cell is read correctly
whatever the display format, because the stored value is unambiguous.

### Yes/no columns

`Yes`, `No`, `Y`, `N`, `True`, `False`, `T`, `F`, `1`, `0`, `Active`, `Inactive`, in any
case. Anything else is an error rather than a guess — a typo in this column would otherwise
silently mean "no".

---

## 4. Mapping columns to fields

Evara suggests a mapping from your headings, matching them exactly once case, spacing and
punctuation are ignored. It never guesses at a near-miss: a suggestion you can see and
change is useful, and a fuzzy match you cannot see is not.

Change any of them. The three states are distinct and the difference matters:

- **A column mapped to a field** — Evara writes what is in it.
- **A field left unmapped** — Evara has *no opinion* about it, and whatever the record
  already holds survives untouched.
- **A column mapped to a field, with a blank cell** — Evara reads that as *there is no
  value*, and clears the field.

The second and third are what make a corrective second import possible: mapping an empty
Email column is how you say "this teacher has no email address", and that cannot be said by
leaving the column out.

One column cannot fill two fields, and a required field with no column stops the whole
plan with one sentence naming it.

---

## 5. What the preview tells you

Each row gets one outcome:

| | |
| --- | --- |
| **Create** | A new record. |
| **Update** | An existing record, changed — only after you have answered the question about it. |
| **Unchanged** | The mapped fields already say this. Nothing is written and no revision is used. |
| **Skipped** | You said to leave it. |
| **Needs a decision** | Evara found something that might be this record and will not decide for you. Blocks the import until answered. |
| **Not imported** | Something is wrong with the row. Blocks the import until the file is corrected. |

Warnings do not block anything.

### The four answers

Where a row needs a decision, all four are offered equally — there is no default, because
Evara has no opinion about which is right:

- **Use this file** — update the record from the row.
- **Keep what Evara has** — leave the record alone. A complete answer.
- **Create a separate record** — these are two different people (or things). Not offered
  where the code would collide.
- **Skip this row** — decide later.

Answering one row re-previews the whole file, because an answer can change another row's
outcome: creating a separate record claims a code the next row wanted.

### If the project changes while you are looking

A preview is a question about the project as it was. If anything relevant changes before
you import — somebody else's window, another import — the import is refused, **nothing is
written**, and the plan is re-read for you to review again. That is the normal, safe
outcome and not an error.

---

## 6. How a row finds its record

**A code matches. A name asks.**

- A **code** is matched within the selected school, which is what the project's uniqueness
  rules already make it.
- A **name** is evidence and never a match. Two members of staff can share a name; so can
  two children. A name match is a question.
- A row with **no code at all** is imported, with a warning: importing the same file again
  will create a second record rather than recognising this one. This is the caveat to
  "importing twice is safe" — **a code column is what makes an import repeatable**, and
  Evara will not invent one, because a generated code is an identifier your own system does
  not know.
- Two rows in one file claiming **one code** is reported, naming the row they collide with.
  Not last-one-wins: that would make the result depend on row order in a way the preview
  could not show.

### People already in the project at another school

Evara looks project-wide for the narrowest possible match — the **same code and both
names** — and offers *employ the same person here* or *create a separate identity*, naming
the school it found them at. Even that is a question: two schools in one trust can both
number their staff from `T-0001`.

Choosing to employ them here adds a membership. The person stays one person, with one
identity.

### Things an import will not change

- **Employment and enrolment.** A file that disagrees with a teacher's membership, or whose
  student is currently enrolled at another school, is reported — never written. Moving a
  child between schools closes one enrolment and opens another, with a leaving date a
  spreadsheet column does not carry.
- **A course's subject.** A course is an offering *of* a subject; repointing it would move
  every lesson built on it. Give the new one its own code and name.
- **An activity, downwards.** See below.

### Activities are counted

An activity row has nothing unique about it — four single periods of one class are four
identical records — so it is matched by counting. If the section already holds exactly what
the row asks for, nothing happens. If it holds fewer, the difference is added, with a
warning naming the arithmetic. **If it holds more, the row is a question and nothing is
deleted.**

An import never deletes a lesson. Choosing which of six to destroy, and destroying its
place in the timetable with it, is not something a spreadsheet cell does.

---

## 7. The report

After an import, the report lists **every row and everything Evara said about it**, one
line per finding, with the row number first.

It is a CSV, written by Evara from its own record of what happened. A value that a
spreadsheet would read as a formula — anything beginning `=`, `+`, `-` or `@` — is prefixed
with a single apostrophe, so opening the report cannot run something out of your file. That
is the only change made to any value: the preview and the report always agree.

---

## 8. For the implementer

| | |
| --- | --- |
| `crates/evara-core/src/import.rs` | The vocabulary: `SourceCell`, `SourceTable`, `ColumnMapping`, `ImportPlan`, `ImportSummary`, `ImportKind`. No I/O. |
| `crates/evara-import/` | Bytes → `SourceTable`, and the templates. `calamine`, `csv`, `rust_xlsxwriter`. Depends on `evara-core` only. |
| `crates/evara-db/src/import/` | `lookup.rs` (resolution reads), `kinds.rs` (one arm per kind), `mod.rs` (`plan`, `apply`, the intents), `report.rs` (the CSV). |
| `crates/evara-db/src/tests_import.rs` | The guarantees, asserted. Start with `previewing_writes_nothing_at_all` and `a_constraint_failure_rolls_the_whole_import_back`. |
| `apps/desktop/src-tauri/src/commands/import.rs` | Nine commands and the session. |
| `apps/desktop/src/app/setup/ImportData.tsx` | Section 18 of School Setup, five steps. |

Adding a kind means: a variant on `ImportKind`, a column table in
`evara-import/src/template.rs`, the same field list in `evara_db::import::evara_import_fields`
(held in step by `every_required_field_matches_the_template`), an arm in `kinds.rs`, an
`Intent` variant with its execution, and tests. Nothing in the interface changes: the type
chooser, the mapper and the preview are generic over `ImportKind`.
