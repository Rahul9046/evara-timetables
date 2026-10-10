# Fixtures

Synthetic school scenarios used to prove constraint coverage, benchmark the solver and
test the `*.evara` format.

## Rule

**Everything here is synthetic.** No real student, teacher, parent or school data, ever —
not in a fixture, not in a test, not in a screenshot, not in an attachment to a bug
report. Names are generated.

## Status

`basic-school/problem.json` is from the pre-Tauri prototype and matches the old flat
model (`timeslots`, `teachers`, `studentGroups`, `rooms`, `lessons`). It is **not** the
current shape and nothing reads it today. It is kept as a reference for the minimum
viable scenario until Phase 3 replaces it.

## Planned (Phase 3)

A parameterised generator in `evara-cli` plus three checked-in fixtures:

| Fixture | Purpose |
| --- | --- |
| `tiny` | Small enough to verify by hand; every constraint has a known outcome |
| `medium` | ~400 activities; the realistic regression and benchmark target |
| `infeasible` | Deliberately impossible; proves diagnostics name the real cause |

Naive fixtures are a trap: they are too easy and hide the pathologies that break real
timetables. The generator takes knobs for schedule tightness, specialist-room scarcity
and part-time staff fraction, and the suite includes hand-built adversarial cases.

Solver score baselines are committed alongside the fixtures from Phase 6 so regressions
are caught by CI rather than noticed later.
