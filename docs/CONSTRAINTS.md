# Constraint Registry

Every supported scheduling rule must have an identifier, documented semantics and an automated fixture/test.

## v0.1 baseline

| ID | Constraint | Level |
| --- | --- | --- |
| CORE-001 | A teacher cannot teach two lessons simultaneously | Hard |
| CORE-002 | A student group cannot attend two lessons simultaneously | Hard |
| CORE-003 | A room cannot host two lessons simultaneously | Hard |
| CORE-004 | Teacher unavailability must be respected | Hard |
| CORE-005 | Room capacity must satisfy the activity requirement | Hard |
| CORE-006 | Required room/resource type must be available | Hard |
| CORE-007 | Fixed/locked activities keep their assigned time | Hard |
| CORE-008 | Required lesson count per cycle must be scheduled | Hard |
| CORE-009 | Prefer spreading repeated subject lessons across days | Soft |
| CORE-010 | Prefer minimizing teacher gaps | Soft |
| CORE-011 | Prefer minimizing unnecessary room changes | Soft |
| CORE-012 | Teacher preferred/unpreferred periods affect score | Soft |

## Coverage policy

Competitor features discovered during research are mapped to generic Evara constraints rather than implemented as vendor-specific behavior. New constraints require a reproducible test case before being considered supported.
