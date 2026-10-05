# Architecture

Evara Timetables is a vendor-neutral, open-source school timetabling platform.

## Principles

1. Model school concepts independently from any SIS or timetable vendor.
2. Express scheduling policy as reusable hard and soft constraints.
3. Keep optimization separate from the web application.
4. Preserve human control: generated schedules must remain editable.
5. Explain infeasibility and constraint violations instead of returning a generic failure.
6. Keep integrations at the boundary of the system.

## Modules

- **Web** — timetable setup, generation, review, editing and diagnostics.
- **Domain** — canonical types shared by UI, imports and API clients.
- **Solver service** — optimization using Timefold Solver.
- **Import/export** — adapters for CSV and future SIS/LISS integrations.
- **Fixtures** — reproducible school scenarios used to prove constraint coverage.

## Solver model

Planning entities are lessons/activities. Planning variables initially include timeslot and room. Teachers, student groups, rooms and calendar periods are problem facts.

Constraints are classified as:

- **Hard** — must never be violated in a feasible timetable.
- **Medium** — important allocation objectives.
- **Soft** — preferences optimized after feasibility.

Advanced capabilities such as electives, multi-week cycles, travel time, linked activities, student sectioning and daily operations extend this model without changing its core vocabulary.
