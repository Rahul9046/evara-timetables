# ADR 0001 — Local-first Tauri desktop application, no server, no authentication

- Status: **Accepted**
- Date: 2026-10-06
- Supersedes: the original `docs/ARCHITECTURE.md` module split

## Context

The initial repository was shaped as a web application plus a separate optimisation
service communicating over REST. The product requirement is the opposite: a desktop
application for Windows and macOS where all school data stays on the machine, and where
timetable creation, editing, generation, viewing, import and export all work with no
internet connection. There is no authentication and no cloud backend.

## Decision

Build a single desktop application on **Tauri 2** with a **React + TypeScript** frontend
and a **Rust** core. No HTTP server, no localhost port, no bundled runtime beyond the
application binary itself.

- No login, logout, accounts, sessions, roles or tokens anywhere in the codebase.
- No code path in normal operation performs a network request.
- Synchronisation is explicitly out of scope, but the schema carries stable UUIDs, row
  revisions and a change log so it remains possible later (see
  [ADR 0003](0003-sqlite-document-model.md)).

## Consequences

- Tauri requires a Rust toolchain for all contributors. This is a prerequisite anyway and
  informs [ADR 0002](0002-rust-in-process-solver.md).
- Packaging and signing become real work: MSI/NSIS plus DMG, and macOS notarisation has a
  recurring cost (see OPEN-DECISIONS D11).
- A browser-accessible version is not available for free. `evara-solver` is kept free of
  Tauri dependencies so a WASM build remains possible if a viewer is ever wanted.
- Features that assume a server — multi-user editing, central reporting, live sharing —
  are deferred, not designed around.

## Alternatives considered

**Electron.** Larger bundles, a Node runtime to ship and secure, and no advantage here
since the heavy work belongs in a compiled language.

**A web app with a local server.** Reintroduces a port, a process to supervise, firewall
prompts and a security surface, for no gain on a single-user desktop product.

**Progressive web app with browser storage.** Cannot offer real file-based projects,
native print, or a credible "your data never leaves the machine" guarantee.
