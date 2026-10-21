# ADR 0035 — Whether two student groups share students has three answers, and *indeterminate* is one of them

- Status: **Accepted**
- Date: 2026-10-21
- Decides: [OPEN-DECISIONS](../OPEN-DECISIONS.md) **D12**, open since Phase 0 and the
  blocker [`SCHEDULING-READINESS-AUDIT.md`](../SCHEDULING-READINESS-AUDIT.md) B2 names for
  `student-clash`.
- Builds on: [ADR 0006](0006-group-based-scheduling-first.md) (a full timetable must be
  buildable from `student_group.size` with no student records at all),
  [ADR 0018](0018-people-identity-and-deactivation.md) (leaving is a state, and history is
  kept), [ADR 0021](0021-project-level-people-and-school-membership.md) (people belong to
  the project, membership to the school),
  [ADR 0029](0029-a-lesson-is-a-course-a-section-and-its-occurrences.md) §4 (the groups are
  the scheduling population; the roster is an overlay),
  [ADR 0032](0032-timetable-generation-is-scoped-to-one-school.md) (generation is scoped to
  one school; cross-school sharing is reported).
- Binds: `student-clash` (M5), the `Problem` loader's audience compilation (M4), and
  `CONSTRAINTS.md`'s *"resolved to leaf student sets"* phrasing. **Requires one future
  nullable column**, specified in §4 and not written here.

## Context

`student-clash` is one of the three constraints without which a timetable is not a
timetable. Its input is the question *do these two groups share any student?*, and the
repository stores the shape of an answer without storing the answer.

### What is stored, checked rather than assumed

| Fact | Where |
| --- | --- |
| `student_group.parent_group_id`, `RESTRICT`, with a one-step cycle `CHECK` and a repository walk to `MAX_GROUP_DEPTH = 64` | `V10:129`, `V10:141`; `repo/people.rs:1232-1271` |
| A parent must belong to the **same school** as its child | `V12:172-176`, trigger pair |
| `student_group_member` is many-to-many, with its own `id` and audit columns | `V10:159-173` |
| A member must have **some** `student_school_enrolment` spell at the group's school — **`LEFT` included**, deliberately | `V13:474-499` |
| `student_group.size` is nullable; `NULL` means "membership is the truth"; `0` is legal | `V10:131-133` |
| A section may be taught to several groups | `class_group`, `V15:662` |
| `class_enrolment` has **no scheduling meaning** in Milestone 1 | `V15:716-726`, ADR 0029 §4 |
| A student may hold concurrent spells at **more than one school** | `V13:311`, one *current* spell per `(student, school)`, unlimited history |
| **No read walks the hierarchy.** `repo/people.rs` offers `student_group_members`, `groups_for_student` and a member count, and nothing transitive | `repo/people.rs:1124-1476` |

### Two things nothing enforces, which decide the whole design

**Containment is declared, not enforced.** No trigger and no repository check requires a
child group's members to be a subset of its parent's. `create_student_group` and
`update_student_group` verify that the parent exists, that the edge closes no cycle and that
the parent is in the same school (`repo/people.rs:1005-1110`) — nothing more. So
`parent_group_id` is an *assertion by the school* that this group is a part of that one, and
the member rows may be silent about it or may contradict it.

**Sibling disjointness is neither declared nor derivable, and the schema's own comment
assumes it.** `V10:124-126` reads: *"a cohort split into three teaching sets means those sets
clash with the cohort but not with each other."* That is true of three *classes* and false of
three *teaching sets of different subjects*:

```
Year 9  (120)
├── 9A, 9B, 9C          three classes      — pairwise disjoint, every child in exactly one
├── 9-Mat-1 … 9-Mat-4   maths sets         — pairwise disjoint
└── 9-Fre-1 … 9-Fre-3   French sets        — pairwise disjoint
```

Every one of those eleven groups is a child of Year 9. `9A` and `9B` are disjoint. `9-Mat-1`
and `9-Fre-1` overlap heavily — and the two pairs are **structurally identical** in the
stored data: same parent, same shape, nothing to tell them apart. `student_group.kind` does
not separate them either, since `9-Mat-1` and `9-Fre-1` are both `TEACHING_SET`.

Nor does rearranging the tree help. Interposing "Year 9 Classes", "Year 9 Maths Sets" and
"Year 9 French Sets" makes `9A`/`9B` siblings under one node and `9-Mat-1`/`9-Fre-1` cousins
— but the three interposed nodes are themselves siblings under Year 9, and *they* overlap
(each is the whole cohort viewed one way). A rule that made siblings disjoint would then
declare `9-Mat-1` and `9-Fre-1` disjoint through their parents. **A tree of containment
cannot express two different divisions of the same population**, and that is the limit this
record is about. It is also why the products in this category — aSc's divisions, Edval's
clash tables, TimeTabler's bands — each carry an explicit construct for it.

## Decision

**Overlap is a function of two groups returning one of three values. Only `Overlaps` is a
hard clash. `Disjoint` must be *proved*. Everything else is `Indeterminate`, which is never
silently converted to either, is not enforced, and is reported loudly with a remedy.**

### 1. The three values

```rust
enum Overlap { Overlaps, Disjoint, Indeterminate }   // conceptual — not implemented here
```

- **`Overlaps`** — at least one student is in both. A hard `student-clash`.
- **`Disjoint`** — no student is in both, proved from stored facts.
- **`Indeterminate`** — the stored data does not decide it. Not a clash, not a licence, and
  counted.

### 2. Effective membership, and who counts

`subtree(g)` is `g` together with every transitive descendant through `parent_group_id`.

`effective_members(g)` is the union of `student_group_member` rows over `subtree(g)`,
**restricted to students who could be in the room**: `student.is_active = 1` **and** a
`student_school_enrolment` spell at the group's school whose `status <> 'LEFT'`.

Two parts of that need their reasons on the record.

**The subtree union, because containment is the parent edge's meaning.** A student named in
`9-Mat-1` is in Year 9, whether or not a Year 9 member row exists. Without the union,
overlap between a group and a cousin of its child would be invisible, and a school that
enters members only at the leaves — which is the natural way to enter them — would get no
protection at its cohort level.

**The `LEFT` and `is_active` filter, because `student_group_member` deliberately keeps
history.** `V13:468-470` admits any spell *"LEFT included, for the reason ADR 0018 gives
throughout — membership of 9-MAT-A last year is the only evidence it happened"*. That is
right for the record and wrong for a room: a child who has left is not competing for a
timeslot, and counting them would manufacture clashes between groups that no longer share
anybody. The conjunction mirrors ADR 0021's treatment of a teacher's effective eligibility —
`teacher.is_active` **and** an active membership — computed in one place rather than assumed
at call sites. `class_enrolment`'s own trigger pair already made the same `<> 'LEFT'` choice
(`V15:780-812`).

`provably_empty(g)` holds when `g.size = 0`, `effective_members(g)` is empty, and no group in
`subtree(g)` declares a `size > 0`. An empty group competes for nobody.

### 3. The algorithm, in order, first match winning

```
overlap(A, B):
  1. if provably_empty(A) or provably_empty(B)                 -> Disjoint
  2. if A == B                                                 -> Overlaps
  3. if effective_members(A) ∩ effective_members(B) ≠ ∅        -> Overlaps      [fact]
  4. if A ∈ subtree(B) or B ∈ subtree(A)                       -> Overlaps      [containment]
  5. if ∃ P : A,B ∈ subtree(P), A≠P, B≠P,
       size(A), size(B), size(P) all declared,
       and size(A) + size(B) > size(P)                         -> Overlaps      [pigeonhole]
  6. if ∃ a ∈ path(A), b ∈ path(B) : a ≠ b,
       parent(a) = parent(b) = P,
       and P.children_are_disjoint is true                     -> Disjoint      [§4]
  7. otherwise                                                 -> Indeterminate
```

`path(X)` is `X` and its ancestors. The relation is symmetric and total, and every input is
a stored fact or the §4 declaration — there is no heuristic and no tie-break.

**Why facts precede declarations.** Rule 3 is before rule 6 on purpose. If a school declares
a parent's children disjoint and then puts one student in two of them, the member row is
what happened and the flag is what was intended. The pair is `Overlaps`, **and the
contradiction is reported** — pre-flight names the student, the two groups and the parent
whose declaration they break. Silently honouring either side would hide a data error that
matters.

**Rule 5 is arithmetic, not a guess.** Both groups are inside `P`, so their populations are
subsets of `P`'s; if the declared sizes sum past it they must intersect. It catches the
large-pair case and nothing else, and it costs one comparison. It finds a genuine overlap in
data that contains no member rows at all, which is exactly the ADR 0006 school.

**Rule 4 fires before any emptiness subtlety, because rule 1 already removed it.** A
non-empty descendant's students are in its ancestor by declaration.

### 4. The one future column, and why nothing else will do

```sql
-- FUTURE. Not in V10 and not written by this record.
ALTER TABLE student_group
    ADD COLUMN children_are_disjoint INTEGER
        CHECK (children_are_disjoint IS NULL OR children_are_disjoint IN (0, 1));
```

It is asked of the **parent**, once, and it means: *no two of my child groups share a
student.* `NULL` is "not stated" and yields `Indeterminate`; `1` feeds rule 6; `0` is an
explicit "they may overlap", which is worth distinguishing from silence because it tells
pre-flight to stop asking.

**Per parent, not per pair.** That is the property that makes it usable. A pair-wise table
over the groups a school attaches to sections is O(n²) questions — hundreds for a modest
school, which is a form nobody completes. One question per parent with children is a dozen
or so answers for a whole school, and it resolves the pairs quadratically.

**It composes to express two divisions of one cohort**, which §"Context" showed a tree alone
cannot:

```
Year 9                     children_are_disjoint = 0   -- classes and sets overlap
├── Year 9 Classes         children_are_disjoint = 1   -- 9A, 9B, 9C
├── Year 9 Maths Sets      children_are_disjoint = 1   -- 9-Mat-1 … 4
└── Year 9 French Sets     children_are_disjoint = 1   -- 9-Fre-1 … 3
```

- `9A` vs `9B` — separated under "Year 9 Classes", which declares `1` → **Disjoint** ✅
- `9-Mat-1` vs `9-Mat-2` → **Disjoint** ✅
- `9-Mat-1` vs `9A` — their paths separate under Year 9, which declares `0`, so rule 6 does
  not fire → **Indeterminate**, which is the honest answer: they very likely do overlap, and
  nothing stored proves it.
- `9-Mat-1` vs `9-Fre-1` → **Indeterminate**, same reasoning.
- `Year 9` vs `9A` → **Overlaps** by rule 4.

**It is required for the first solver, not optional.** `student-clash` is in the MVP hard set
and the three-state function degenerates without it: at a school with no member rows, every
pair that is not same-or-ancestor is `Indeterminate`, including `9A` against `9B` — the
commonest pair in any school. So the migration that M5 already needs for
`constraint_instance` / `constraint_ref` carries this column too. Specifying it now is what
keeps it one nullable `ALTER TABLE`: `student_group` is already in
`change_log::TRACKED_TABLES`, and the change-log triggers interpolate only the table name,
`id` and `rev` (`change_log.rs:106-134`), so there is no trigger work and no ADR 0020
rebuild. Existing projects read `NULL`, which is `Indeterminate`, which is the behaviour
this record defines for them.

### 5. A section's scheduling population is its groups, not a student set

```
audience(section)  = { g : class_group(section, g) }        -- groups, not students
conflict(A, B)    ⟺ ∃ g₁ ∈ audience(A), g₂ ∈ audience(B) : overlap(g₁, g₂) = Overlaps
```

**`Problem` holds groups, not expanded student sets.** `SOLVER.md` §2 currently says
`groups: Vec<GroupFact>  // expanded to leaf student sets` and `CONSTRAINTS.md` §3 says
`student-clash` applies to *"A student group (resolved to leaf student sets)"*. Both are
corrected by this record. An expansion to student sets cannot be the compiled form, because
ADR 0006 requires a school with **no student rows at all** to get a full timetable, and such
a school's leaf sets are all empty — which would make every pair disjoint and
`student-clash` silent for precisely the schools group-first scheduling was chosen for. The
compiled form is an **overlap relation over group indices**, which is derived from members
where they exist and from declarations where they do not.

**Overlap inside one section is not a conflict.** A section taught to a cohort and to one of
its sets has `Overlaps` within its own audience; those people are in one room together,
which is what a combined class is. The relation is only consulted *between* two activities.

**Multi-period activities.** An occurrence of `duration_periods = d` occupies `d`
consecutive slots (ADR 0033 §5), and the audience occupies every one of them.

**Detection is a bitset intersection, not a pairwise scan.** Over the groups a school
attaches to sections — hundreds at most — the loader builds

```
audience_mask(a)  : bitset over group indices     -- the groups attending a's section
clash_mask(a)     = audience_mask(a) ∪ ⋃ { overlaps_with[g] : g ∈ audience(a) }
```

and two activities conflict exactly when
`clash_mask(A) ∩ audience_mask(B) ≠ ∅`. Incremental evaluation keeps one occupancy bitset
per timeslot and tests the candidate's `clash_mask` against it — `O(|groups|/64)` words per
test, a handful at every size in the audit's §14.1 table. The relation is computed once per
solve and never inside the search loop.

### 6. `Indeterminate` is not enforced, and is never quiet

The three available treatments are each wrong on their own, which is why this clause exists.

- **Enforce it as a clash.** `9A` and `9B` would conflict at every school that has not
  answered §4's question, so Year 9 could hold one lesson at a time. No timetable.
- **Treat it as disjoint.** A child in two rooms at once, with both timetables reporting
  themselves clean. This is the failure mode the product exists to avoid, and the definition
  of done forbids it.
- **Block generation until every pair is resolved.** Also no timetable: the same `9A`/`9B`
  pair blocks every school on day one, and a pre-flight that refuses before a school has
  learned what it is being asked is a product that cannot be started.

So: **enforce what is proved, and make what is not proved impossible to miss.**

1. **Pre-flight states it in actionable arithmetic.** Not a list of pairs — a count plus the
   number of answers that would clear it:

   > *"431 pairs of this school's attending groups cannot be compared: Evara does not know
   > whether they share students. Answering "do these overlap?" on 7 parent groups — Year 9,
   > Year 10 … — resolves 412 of them. Until then, lessons for those groups may be
   > timetabled at the same time."*

2. **The generated timetable records the limit.** Its views and its provenance carry the
   count of activity pairs placed concurrently whose audiences were `Indeterminate`. A
   timetable is never described as student-clash-free; it is described as having no
   *confirmed* student clash, with the unverified count beside it.

3. **Contradictions are named, not resolved.** Rule 3 beating rule 6; a group declared
   `size = 0` that attends a class section; a parent declaring `size` smaller than a child's.
   Each is reported with the rows involved.

4. **No soft constraint rewards avoiding `Indeterminate` concurrency in the MVP.** It is a
   plausible later addition — the audit defers every soft type for the same reason — and
   adding it now would spend optimisation budget on a proxy for missing data rather than
   prompting the user to supply the data.

### 7. What is explicitly out of scope

**An individually enrolled student is not protected.** `class_enrolment` has no scheduling
meaning in Milestone 1 (ADR 0029 §4), so a student named on a section's roster contributes
nothing to `audience(section)` and generates no clash. The consequence is stated rather than
implied: a section taught to **no group** and 20 named students has an empty audience and
`student-clash` has nothing to say about it. Pre-flight reports exactly that, with the
remedy — *"Year 9 Mathematics A names 20 students and no attending group; its students are
not protected from clashes. Add a student group."* Nothing here double-counts anybody,
because nothing here counts the roster at all.

**Cross-school clashes are reported, not detected.** A student with concurrent `ENROLLED`
spells at two schools (`V13:311`) may be timetabled into conflicting lessons at each, and
ADR 0032 rule 3 is the record that says so and warns by name. Overlap is computed only among
groups of the school being solved — `student_group.school_id` is `NOT NULL` and a parent
must share its child's school (`V12:172-176`), so a subtree never crosses a school boundary
and the restriction is structural rather than remembered.

**Sectioning is Milestone 2.** Deriving groups from `course_request` (ADR 0006) will produce
sets whose disjointness is known *by construction*. When it arrives it should write
`children_are_disjoint` on the parents it creates, which is the point of the column being
data rather than an inference.

## Consequences

**Good.**

- `student-clash` becomes implementable with a specification rather than a guess, and the
  three-state result makes "we do not know" a first-class outcome instead of a silent
  default.
- ADR 0006 is honoured in the one place it was most at risk: a school with no student rows
  still gets real clash detection, from `size`, the hierarchy and one declaration per parent.
- Rules 3 and 5 extract genuine answers from partial data — a member row proves overlap
  without any declaration, and declared sizes prove it without any member row.
- Combined classes and split sets, which the audit called the place competitors
  differentiate, are expressible exactly, including two divisions of one cohort.
- The compiled form is bitsets over a few hundred groups, so the hot path is cheap and
  `validate_move` stays cheap later.
- The future schema change is one nullable column, no trigger work, no rebuild.

**Bad, and accepted.**

- **Until a school answers §4's question, two groups that genuinely overlap may be
  timetabled together.** This is a real hole. It is counted, named, remedied in a dozen
  clicks, and never papered over — but it is a hole, and a school that ignores the warning
  gets a timetable with a clash in it.
- The warning's wording is load-bearing. A count with no actionable second sentence would be
  dismissed, and a dismissed warning is worse than none — the same risk ADR 0032 records
  about its shared-resource warning.
- `student-clash` cannot ship before the `children_are_disjoint` column, so M5 carries a
  migration it would not otherwise have needed. One column, in a migration it was writing
  anyway.
- `effective_members` walks the subtree, so a 64-deep hierarchy is 64 reads. Irrelevant at
  the real depths (`MAX_GROUP_DEPTH` is a backstop against damage, not a design target) and
  the walk happens once per solve.

**Neutral.**

- `AttendingGroup.population` and the Class section screen's audience total are unchanged.
  They are a display of what has been entered, labelled as such, and
  [ADR 0034](0034-expected-attendance-is-explicit-and-room-capacity-waits-for-it.md) §3 is
  where the scheduling consequence of their imprecision is settled.
- `student_group.kind` keeps its five values and gains no scheduling meaning. It was the
  tempting shortcut for rule 6 and it cannot work: `9-Mat-1` and `9-Fre-1` are both
  `TEACHING_SET`.

## Alternatives rejected

**Treat siblings as disjoint (the reading `V10:124-126` assumes).** The cheapest answer and
the one the schema's own comment implies. Rejected because it is wrong for two different
divisions of one cohort — `9-Mat-1` and `9-Fre-1` — and wrong *silently*, producing exactly
the missed clash `student-clash` exists to prevent. The comment is the reason this record
exists rather than evidence against it.

**Treat every unproven pair as overlapping.** Sound and useless: `9A` and `9B` would clash,
so a year group could hold one lesson at a time and no school could generate.

**Expand groups to leaf student sets and intersect (`SOLVER.md` §2's current wording).**
Rejected on ADR 0006. A school with no student rows has empty leaf sets, so every pair would
compare disjoint and the constraint would be silent for precisely the schools group-first
scheduling was chosen for. Member rows are used where they exist — rule 3 — but they cannot
be the representation.

**A pairwise group-overlap table the user fills in (Edval's clash tables, literally).** The
most expressive and the worst form: O(n²) questions, no default that is safe, and a table
that goes stale every time a group is added. Rule 6's per-parent flag gets the same
expressiveness for a linear number of answers, and §4 shows the composition that recovers
the cases a flat flag would miss.

**Require containment with a trigger, so a child's members must be a subset of its
parent's.** Attractive — it would make rule 4 a fact rather than a declaration. Rejected
because it makes data entry order-dependent in a way nothing warns about: adding a student
to a teaching set before adding them to the cohort would be refused, and a school entering
leaves first (the natural order) would hit a wall. It also does not help the case that
matters, since sibling disjointness remains unprovable either way.

**Infer disjointness from `student_group.kind`.** Rejected above: both sides of the pair that
breaks the inference carry the same kind.

**Infer disjointness from sizes alone — if `Σ size(children) ≤ size(parent)` then the
children are disjoint.** Rejected as unsound in the wrong direction: three maths sets of 28
under a cohort of 120 satisfy it, and so do three French sets of 25, and the two families
overlap completely. The arithmetic proves overlap when it fails (rule 5) and proves nothing
when it holds.

**Defer `student-clash` from the MVP, as `room-capacity` is deferred by ADR 0034.**
Rejected on what a timetable is. A room whose size is unverified is a timetable with a
caveat; a child in two rooms at once is not a timetable. The audit's §5.3 puts
`student-clash` in the required-for-correctness set, and this record's job is to make it
implementable rather than to drop it.

## Truth table

`size` columns are `student_group.size`; *members* means `effective_members` per §2.
Every row is a case a fixture must cover.

| # | A | B | Relation | Evidence | Result | Rule |
| --- | --- | --- | --- | --- | --- | --- |
| 1 | 9A (28) | 9A (28) | same group | — | **Overlaps** | 2 |
| 2 | Empty (`size = 0`) | 9A (28) | unrelated | subtree declares nothing > 0, no members | **Disjoint** | 1 |
| 3 | Empty (`size = 0`) | Empty (same group) | same group | as above | **Disjoint** | 1 — emptiness precedes identity |
| 4 | Year 9 (120) | 9A (28) | A is parent of B | — | **Overlaps** | 4 |
| 5 | Year 9 (120) | 9-Mat-1 (28), grandchild via Year 9 Maths Sets | A is ancestor of B | — | **Overlaps** | 4 |
| 6 | Year 9 (120) | 9A (`size = 0`, no members) | A is parent of B | B provably empty | **Disjoint** | 1 |
| 7 | 9A (28) | 9B (28) | siblings under Year 9 Classes, `children_are_disjoint = 1` | — | **Disjoint** | 6 |
| 8 | 9A (28) | 9B (28) | siblings, parent's flag `NULL` | — | **Indeterminate** | 7 |
| 9 | 9A (28) | 9B (28) | siblings, flag `= 1`, **one shared member row** | the member row | **Overlaps** + contradiction reported | 3 before 6 |
| 10 | 9-Mat-1 (28) | 9-Fre-1 (25) | cousins; paths separate under Year 9, flag `= 0` | — | **Indeterminate** | 7 |
| 11 | 9-Mat-1 (28) | 9A (28) | paths separate under Year 9, flag `= 0` | — | **Indeterminate** | 7 |
| 12 | 9A-Reading (12) | 9B-Reading (12) | children of 9A and 9B, which are `Disjoint` by rule 6 | disjointness propagates to descendants | **Disjoint** | 6, matched on the ancestors |
| 13 | Set X (70) | Set Y (70) | both children of Year 9 (120), flag `NULL` | 70 + 70 > 120 | **Overlaps** | 5 |
| 14 | Set X (28) | Set Y (25) | both children of Year 9 (120), flag `NULL` | 53 ≤ 120 | **Indeterminate** | 7 — the arithmetic proves nothing |
| 15 | Set X (`size NULL`, 3 members) | Set Y (`size NULL`, 3 members), no shared member | siblings, flag `NULL` | partial rosters | **Indeterminate** | 7 — absent rows are not evidence of absence |
| 16 | Set X (`size NULL`, 3 members) | Set Y (`size NULL`, 3 members), one shared | siblings, flag `NULL` | the member row | **Overlaps** | 3 |
| 17 | 9A (28) | 10A (28) | different year groups, no common ancestor, no shared member | — | **Indeterminate** | 7 — unrelated is not disjoint |
| 18 | 9A | 9B | siblings, flag `= 1`, shared member whose only spell is `LEFT` | that student is filtered out by §2 | **Disjoint** | 6 |
| 19 | 9A | 9B | siblings, flag `= 1`, shared member who is `is_active = 0` | filtered out by §2 | **Disjoint** | 6 |
| 20 | Northgate 9A | Southmoor 9A | different schools | — | **not compared** | §7 — ADR 0032 rule 3 warns |

## Future acceptance tests

Fixtures are M2's (audit B12); these are the cases they must carry, beyond the twenty truth
table rows which are unit tests of the function itself. None exists yet.

| # | Fixture | Expectation |
| --- | --- | --- |
| 1 | **Group-only school**, sizes declared, zero `student` rows, flags answered | `student-clash` is fully effective. Proves ADR 0006 survives: no student record is needed for real clash detection. |
| 2 | **Group-only school**, flags all `NULL` | Generation succeeds; pre-flight reports the pair count **and** the parent count that would clear it; the timetable carries the unverified-pair count. No clash is enforced beyond same-group and ancestor pairs. |
| 3 | **Parent and child both attending different sections**, concurrent placement attempted | One hard violation, explained as *"Year 9 and 9A: 9A is part of Year 9."* |
| 4 | **Overlapping membership**, two sibling groups sharing 3 students, flag `NULL` | `Overlaps` by rule 3; the two sections are never concurrent; no declaration was needed. |
| 5 | **Declared-disjoint siblings contradicted by a member row** | `Overlaps`, the solve proceeds, and the contradiction is reported naming the student, both groups and the parent. |
| 6 | **Disjoint groups**, flag `= 1`, no shared members | Concurrent placement is legal and produces no violation and no warning. |
| 7 | **Incomplete membership**, partial rosters, no shared rows, flag `NULL` | `Indeterminate`. Asserts the absence of member rows never becomes a proof of disjointness. |
| 8 | **Pigeonhole**, two sets of 70 under a cohort of 120 | `Overlaps` with no member row anywhere in the fixture. |
| 9 | **Section with several attending groups**, one of which is the parent of another | The section's own internal overlap is not a violation; both groups still clash with other sections' activities. |
| 10 | **Section with `class_enrolment` rows and no `class_group`** | Empty audience, no clash protection, and a pre-flight finding naming the section and the remedy. |
| 11 | **Individually enrolled student also in an attending group** | Counted once, via the group. The roster contributes nothing; nothing is double-counted. Pairs with ADR 0029 §4's existing `coveredByGroup`. |
| 12 | **Two divisions of one cohort** modelled as §4's three-level tree | Classes mutually disjoint, sets mutually disjoint, class-against-set `Indeterminate`. The case a flat sibling rule gets wrong. |
| 13 | **Shared student across two schools**, concurrent `ENROLLED` spells, groups in both | Overlap is computed per school; the cross-school pair is never compared; ADR 0032 rule 3 warns by name. |
| 14 | **Deep hierarchy**, 8 levels, members entered only at the leaves | A cohort-level group overlaps a leaf 7 levels down. Asserts the subtree union, not a one-step parent read. |
| 15 | **Backward compatibility**: a project created before the §4 column | Every flag reads `NULL`; results are those of fixture 2; no re-entry, no re-materialisation, and a school that had entered members keeps rules 3 and 5. |
| 16 | **Custom rotating cycle** with the above | Unchanged. Overlap reads groups only, and never a day, a weekday or a period. |
