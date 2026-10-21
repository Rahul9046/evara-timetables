# Fixtures

Synthetic school scenarios used to prove constraint coverage, benchmark the solver and
test the `*.evara` format.

## Rule

**Everything here is synthetic.** No real student, teacher, parent or school data, ever —
not in a fixture, not in a test, not in a screenshot, not in an attachment to a bug
report. Names are generated.

## Status — the fixtures are code, not files in this directory

Since **M2** the scheduling fixtures live in `crates/evara-core/src/scheduling/fixtures/`,
behind the `fixtures` feature. There are 24 of them, including `tiny`, `medium` and two
deliberately impossible ones, and they are documented in
[docs/SCHEDULING-FIXTURES.md](../docs/SCHEDULING-FIXTURES.md).

Nothing generated is checked in here, and that is deliberate. A generator that is a pure
function of a seed already **is** a fixture: `fixture("tiny", 2026)` returns the same school
on every platform and every toolchain version, so a committed copy of its output could only
drift from the thing that produced it. ADR 0009 names that exact failure for generated
TypeScript and pays for a CI staleness check because that output is an IPC contract a human
reads; a fixture's output is read by tests, which can call the generator.

To look at one:

```sh
cargo run -p evara-cli -- fixture --list
cargo run -p evara-cli -- fixture tiny
cargo run -p evara-cli -- fixture medium --seed 7 --out build/fixtures
```

That writes **synthetic scheduling data**, not a timetable. Nothing in Evara generates a
timetable yet.

### `basic-school/problem.json`

From the pre-Tauri prototype, matching the old flat model (`timeslots`, `teachers`,
`studentGroups`, `rooms`, `lessons`). It is **not** the current shape and nothing reads it.
Kept only as a historical reference for the minimum viable scenario; the `tiny` fixture is
its replacement.

## What arrives later

Solver score baselines are committed alongside the fixtures from **Phase 6**, so solver
regressions are caught by CI rather than noticed later. The generator's parameterised knobs
the earlier draft of this file promised — schedule tightness, specialist-room scarcity,
part-time staff fraction — were not built in M2: each focused fixture states the pathology it
carries instead, which is more diagnosable than a dial. See
[docs/SCHEDULING-FIXTURES.md](../docs/SCHEDULING-FIXTURES.md) §10 for what is and is not
there.
