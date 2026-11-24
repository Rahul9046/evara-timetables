# ADR 0056 — A payload is audited against a generated reference before anything writes to it

- Status: **Accepted** (2026-11-22)
- Date: 2026-11-22
- Decides: M10 proposal D10, and approved decision A2
- Builds on: [ADR 0053](0053-the-payload-is-a-verified-sqlite-snapshot.md) (the payload is a
  database from somewhere else), [ADR 0007](0007-rusqlite-refinery.md) (one connection factory,
  embedded migrations), [ADR 0008](0008-installation-identity-and-project-lifecycle.md)
  (`app_meta` holds the project's identity)
- Binds: `evara_db::transfer::{boundary, conformance, stage}`, `evara_db::change_log::install_all`

## Context

A staged payload is a SQLite database that arrived from outside this installation. Opening it
means handing an untrusted file to a C parser, and then — if the import is to work at all —
running migrations and triggers inside it.

Two things have to be decided: **what is checked**, and **in what order**. The order turns out
to matter more than the list.

Revision 1 of the design read the project's identity out of `app_meta` first, to decide whether
the file was an Evara project worth continuing with. That is backwards: it trusts a row out of
the file in order to decide whether to trust the file. A hostile payload would be asked to
introduce itself before anything had established that its schema was even the shape Evara
expects.

There is also a less obvious hazard. Evara's `change_log` triggers call `evara_site_id()`. If
migrations ran with `trusted_schema = ON`, a payload could carry a view or trigger of its own
that executes during an ordinary statement; if they ran with it **off** and `evara_site_id()`
were not marked `SQLITE_INNOCUOUS`, SQLite would refuse to call it from inside a trigger and
*every* change-log trigger would fail during migration.

## Decision

**Three phases, in this order, first failure stopping everything.** V0 before V1, V1 before V2,
and the identity read last within V0.

### V0 — audit, before anything writes

The payload is opened through a constructor that exists only for this purpose, with SQLite's own
capabilities turned down:

| Turned off | Why it matters |
| --- | --- |
| `trusted_schema` | a view or trigger running during an ordinary `SELECT` is the main way a hostile schema executes anything at all |
| `writable_schema` | direct `sqlite_master` rewriting |
| views | the census and the conformance comparison read tables; a view is a way for the file to interpose on that |
| triggers | needed in V1 and not before |
| `ATTACH` create, `ATTACH` write | a filesystem capability a payload must not have. `SQLITE_LIMIT_ATTACHED = 0` makes it unreachable as well as unprivileged |

Then, in order: `PRAGMA integrity_check`, `PRAGMA foreign_key_check`, and a comparison of the
payload's schema against a **reference this binary generates from its own migrations**. Only
after all three is `app_meta` read.

The open is file-level read-only and is **not** `open_query_only`. That factory calls
`open_plain`, which runs `PRAGMA journal_mode = WAL` — a statement that can write the file
header. Acceptable on a file Evara created; not acceptable as the first thing done to a file
from elsewhere. The two constructors are deliberately separate, and a later refactor that
"shares the factory" would reintroduce a header write on an untrusted file.

### V1 — migrate, only once V0 has passed

A writable connection on the staged copy, `migrations::run` in its grouped transaction, then
`change_log::install_all`. `trusted_schema` stays **off**, which is what makes
`evara_site_id()`'s `SQLITE_INNOCUOUS` flag necessary rather than decorative.

### V2 — verify what migration produced

`foreign_key_check` empty again, conformance at the **current** version, and a table census the
caller can compare a round trip against. A migration is code; asserting its output on a file
nobody has seen before is cheap.

### Conformance is compared against a generated reference

The reference schema is produced by running this binary's own migration chain into a scratch
database and reading the result. So it cannot go stale: a `V19` migration updates the reference
by existing.

A hand-written allow-list was the alternative, and the numbers are why it was rejected — the
schema carries on the order of 74 migration-created triggers and ~114 change-log triggers, and
a list of those maintained by hand is wrong on the first migration somebody forgets to update
it for. Worse, it is wrong in the permissive direction.

`integrity_check` alone is **not** sufficient and this is proven rather than assumed: a payload
with a dangling foreign key leaves `integrity_check` reporting `ok`, and a payload carrying a
hostile extra trigger also leaves it reporting `ok`. Structure, references and shape are three
questions.

## Alternatives considered

| Alternative | Why not |
| --- | --- |
| Read `app_meta` first to decide whether to continue | Trusts a row out of the file to decide whether to trust the file. This is the ordering revision 1 got wrong |
| A hand-maintained list of expected tables, indexes and triggers | Stale on every migration, and permissively stale. ~188 triggers is not a list anybody maintains correctly |
| `integrity_check` only | Proven insufficient: it reports `ok` for a dangling reference and for a hostile trigger |
| `trusted_schema = OFF` without `SQLITE_INNOCUOUS` on `evara_site_id` | Every change-log trigger fails during migration, so V1 cannot complete |
| `trusted_schema = ON` during migration | Hands execution to a schema that arrived from elsewhere, at the exact moment a writable connection is open on it |
| Reuse `open_query_only` for the audit | It runs `PRAGMA journal_mode = WAL`, which can write the header of a file we have not yet decided to trust |
| Sandbox the engine (separate process, seccomp, WASM) | A real option and a large one. Out of M10's scope, and it would not remove the need for any check above — it bounds the blast radius of a parser defect rather than detecting a bad file |

## Consequences

- **This is not a sandbox, and the record says so.** No arrangement of pragmas makes a C parser
  invulnerable. What the controls remove is the capabilities an attacker would otherwise simply
  ask for. Corruption and hostility are **detected**; invulnerability is **not claimed** —
  OPEN-DECISIONS lists it as the highest-ranked residual risk of the whole feature.
- **A refusal here is a `DbError`, not a format error.** A corrupt, non-conforming or too-new
  payload is reported by the layer that opened it.
- **The audit's findings are carried, not just a boolean.** The view and trigger counts are
  reported so a refusal can say what was unexpected.
- **`evara_site_id` is marked `SQLITE_INNOCUOUS` permanently**, and the test that proves it
  matters turns `trusted_schema` **off** first — with it on (the bundled default) the flag
  changes nothing observable and a test would pass either way.
- **Nothing derived is rebuilt during staging**, which keeps V1 and V2 cheap and is
  [ADR 0057](0057-an-import-is-staged-verified-and-renamed.md)'s decision rather than this one's.
