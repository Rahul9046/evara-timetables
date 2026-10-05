# Solver service

This service will contain the optimization engine for Evara Timetables.

## Direction

- Java 21
- Spring Boot
- Timefold Solver
- REST API between the web application and optimizer

The first solver milestone implements the baseline constraints in `docs/CONSTRAINTS.md` against the synthetic fixtures in `fixtures/`.

Keeping optimization isolated allows the domain and UI to evolve independently and makes the solver usable by other open-source SIS projects.
