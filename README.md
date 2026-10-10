# Evara Timetables

Open-source, **local-first desktop** school timetabling. Build complex, constraint-based
school timetables on your own machine — generate them automatically, edit them by hand,
and understand exactly why a change is or is not allowed.

- **Local-first.** All school data lives in a file on your computer.
- **Offline.** No internet connection is needed to create, edit, generate, view, import
  or export a timetable.
- **No accounts.** No login, no cloud backend, no telemetry.
- **Vendor-neutral.** Features from existing timetabling products are modelled as generic
  scheduling concepts and constraints, not vendor-specific behaviour.
- **Apache-2.0.** No paid services, no mandatory external dependencies.

> **Status: design phase.** The architecture is documented and under review; feature
> implementation has not started. See [docs/ROADMAP.md](docs/ROADMAP.md) for the plan and
> [docs/OPEN-DECISIONS.md](docs/OPEN-DECISIONS.md) for what is still undecided.

## Planned stack

React + TypeScript · Tauri 2 · Rust · SQLite — one signed desktop binary per platform,
with the solver and constraint engine compiled in rather than running as a service.

## Documentation

| Document | Contents |
| --- | --- |
| [ARCHITECTURE.md](docs/ARCHITECTURE.md) | System shape, layering rules, what the current repository gets wrong |
| [DATA-MODEL.md](docs/DATA-MODEL.md) | SQLite schema, domain boundaries, calendar/cycle model |
| [SOLVER.md](docs/SOLVER.md) | Solver architecture, scoring, manual-move validation, repair, infeasibility explanation |
| [CONSTRAINTS.md](docs/CONSTRAINTS.md) | Constraint registry, levels, vendor mapping |
| [PROJECT-FORMAT.md](docs/PROJECT-FORMAT.md) | The portable `*.evara` project format |
| [ROADMAP.md](docs/ROADMAP.md) | Milestone 1, phase by phase |
| [OPEN-DECISIONS.md](docs/OPEN-DECISIONS.md) | Open decisions and technical risks |
| [adr/](docs/adr/) | Architecture decision records |

## What Milestone 1 delivers

Create a local school project; define a timetable structure (including rotating and
multi-week cycles); add teachers, subjects, rooms, student groups and lessons; configure
hard, medium and soft constraints; generate a timetable locally; view master, teacher and
class timetables; move a lesson by hand and be told precisely why an invalid move is
invalid; save, close and reopen without loss; export the whole project and import it into
another installation; print and export timetable views.

## Platforms

Windows and macOS.

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md). Two rules matter most: keep vendor-specific
integration code out of the core domain and solver, and **never commit real school,
teacher or student data** — synthetic fixtures only.

## Licence

[Apache-2.0](LICENSE).
