# Contributing to Evara Timetables

Thanks for helping build an open-source school timetabling platform.

## Ground rules

- Open an issue for substantial features or new constraint families.
- Keep vendor-specific integration code outside the core domain and solver.
- Every scheduling constraint must include automated tests and a documented scenario.
- Prefer small pull requests with a single clear purpose.
- Do not commit real student, teacher or school data. Use synthetic fixtures only.

## Development direction

The project is being bootstrapped. The first milestone is a complete vertical slice: define a school, generate a feasible timetable, inspect its score, edit it and understand violations.
