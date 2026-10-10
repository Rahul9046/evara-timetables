# ADR 0006 — Group-based scheduling first; request-based schema preserved from day one

- Status: **Accepted** (approved 2026-10-06)
- Date: 2026-10-06

## Context

School timetabling splits into two traditions that are different optimisation problems,
not variations of one:

- **Group-based** (aSc, TimeTabler, much of Europe and Australia): classes are formed
  before timetabling. The solver places lessons for known groups.
- **Request-based** (PowerScheduler, Veracross, Edval electives, much of the US):
  students submit individual course requests. The system must build sections, assign
  students to them (**sectioning**), construct elective lines, and schedule — usually as
  several interacting phases.

Evara must ultimately support both. The question was which to build first, and what that
choice costs the other.

## Decision

**Implement group-based scheduling in Milestone 1.** Add request-based sectioning in
Milestone 2.

**The data model carries both from day one.** The schema in
[DATA-MODEL.md](../DATA-MODEL.md) already includes everything the request-based path will
need, and these structures are to be preserved even though nothing reads them yet:

| Capability | Schema support |
| --- | --- |
| Individual student course requests | `course_request` |
| Weighted requests | `course_request.weight`, `.priority` |
| Alternate requests | `course_request.kind = 'ALTERNATE'`, `.alternate_of_id` |
| Elective line generation | `line`, `line_member`, evaluated as `same-period` |
| Sectioning | `class_section`, `enrolment` derived from requests |
| Class balancing | `class_section.max_size`, `section-balance` |
| Student allocation | `student`, `student_group_member`, `enrolment` |

The corresponding constraint types (`request-satisfied`, `request-weighted`,
`request-alternate`, `section-balance`, `section-max-size`) are registered in
[CONSTRAINTS.md](../CONSTRAINTS.md) as `M2+` with documented semantics, so the registry
shape is settled before the algorithms arrive.

**No request-based algorithm is implemented now.** No sectioning, no line generation, no
request optimisation.

## Consequences

- Milestone 1 ships a timetable a group-based school can actually use, rather than half
  of two products.
- Sectioning becomes an *additional phase* on top of a working engine, which is a far
  smaller change than retro-fitting the data model would have been. Adding
  `course_request` later would have forced a migration through `enrolment`,
  `class_section` and every constraint that reads an audience.
- Some tables and columns exist in Phase 1–3 with no reader. That is deliberate: the cost
  is a few unused columns, and the alternative is a breaking schema change in Milestone 2.
- Fixtures should include request data from Phase 3 even though nothing consumes it, so
  the Milestone 2 work starts with realistic input rather than inventing it.
- Student records stay optional throughout Milestone 1. A school must be able to build a
  full timetable using only `student_group.size`, without entering a single student.

## Alternatives considered

**Request-based first.** Correct if the US market is the primary target. Rejected for now
because it is the larger problem, it requires the group-based machinery underneath it
anyway, and it would push the first usable release out considerably.

**Group-based only, model requests later.** Simpler now, and the reason this ADR exists:
the schema cost of carrying the request structures today is small and well understood,
whereas the migration cost of adding them to a populated database later is not.

**Build both at once.** Rejected under the project's "do not build everything in one
pass" rule. Two optimisation problems in one milestone would leave neither finished.
