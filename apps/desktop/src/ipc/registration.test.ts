/**
 * Every command the frontend calls is actually registered, and vice versa.
 *
 * ## The one gap generated types cannot close
 *
 * ADR 0009 named the cost of choosing `ts-rs` over `tauri-specta`: the command *name*
 * stays unchecked across the boundary. `invoke("setup_campus_lst")` compiles, type-checks,
 * passes every other test in this suite, and fails only when a human clicks the button —
 * with "Command setup_campus_lst not found", at which point the screen is stuck.
 *
 * The same hole exists in the other direction: a handler written and never added to
 * `generate_handler!` is dead code that looks alive, and a handler added to the macro but
 * never wrapped is a command nothing can reach.
 *
 * This test reads both sides as text and compares them. Parsing your own source in a test
 * is unusual, and it is justified here because this is the only part of the IPC contract
 * with no compiler and no type behind it.
 *
 * ## Read through Vite, not through Node
 *
 * `?raw` and `import.meta.glob` rather than `node:fs`. The desktop app deliberately has
 * no `@types/node`: it is a local-first application whose view layer must not do I/O, and
 * putting Node's globals in its type surface is an invitation to use them. Vite's own
 * primitives read the files at transform time and keep that surface closed.
 */
import { describe, expect, it } from "vitest";

import libRs from "../../src-tauri/src/lib.rs?raw";

/** Every non-test module in this directory, as source text keyed by path. */
const IPC_SOURCES = import.meta.glob<string>("./*.ts", {
  query: "?raw",
  import: "default",
  eager: true,
});

/**
 * Commands registered but deliberately not wrapped.
 *
 * Empty, and an entry added here should come with a reason. `app_info` is wrapped in
 * `./index.ts`, which this scan reads.
 */
const UNWRAPPED_BY_DESIGN = new Set<string>([]);

/** The command names listed in `generate_handler!`. */
function registeredCommands(): Set<string> {
  const block = /invoke_handler\(tauri::generate_handler!\[([\s\S]*?)\]\)/.exec(libRs);
  if (!block?.[1]) {
    throw new Error("could not find generate_handler! in src-tauri/src/lib.rs");
  }

  const names = new Set<string>();
  for (const line of block[1].split("\n")) {
    // `commands::structure::setup_campus_create,` — take the last path segment.
    const match = /^\s*(?:commands::)?([\w:]+?)\s*,\s*$/.exec(line);
    if (!match?.[1]) continue;
    const segments = match[1].split("::");
    const name = segments[segments.length - 1];
    if (name) names.add(name);
  }
  return names;
}

/** The command names the frontend actually invokes, and which module sends each. */
function invokedCommands(): Map<string, string> {
  const found = new Map<string, string>();
  for (const [path, source] of Object.entries(IPC_SOURCES)) {
    if (path.endsWith(".test.ts")) continue;
    for (const match of source.matchAll(/invoke(?:<[^>]*>)?\(\s*"(\w+)"/g)) {
      const name = match[1];
      if (name) found.set(name, path);
    }
  }
  return found;
}

describe("the IPC command surface", () => {
  const registered = registeredCommands();
  const invoked = invokedCommands();

  it("finds both sides, so a silent zero does not make this test vacuous", () => {
    expect(registered.size).toBeGreaterThan(50);
    expect(invoked.size).toBeGreaterThan(50);
  });

  it("registers every command the frontend invokes", () => {
    const missing = [...invoked.entries()]
      .filter(([name]) => !registered.has(name))
      .map(([name, path]) => `${name} (invoked from ${path})`);

    expect(
      missing,
      "these commands are called but not in generate_handler!, so they would fail at runtime",
    ).toEqual([]);
  });

  it("wraps every command it registers", () => {
    const unreachable = [...registered]
      .filter((name) => !invoked.has(name) && !UNWRAPPED_BY_DESIGN.has(name))
      .sort();

    expect(
      unreachable,
      "these commands are registered but nothing calls them; wrap them or remove them",
    ).toEqual([]);
  });

  it("covers every School Setup entity", () => {
    // A whole entity quietly missing from the registration is the failure this catches:
    // the eight others would work, and one screen would be inert.
    const entities = [
      "school",
      "campus",
      "building",
      "room_type",
      "room",
      "resource",
      "academic_year",
      "term",
      "cycle",
      "cycle_day",
      "period_structure",
      "period",
      "calendar_day",
    ];

    for (const entity of entities) {
      const commands = [...registered].filter((name) =>
        new RegExp(`^setup_${entity}_[a-z]+$`).test(name),
      );
      expect(commands.length, `no commands registered for ${entity}`).toBeGreaterThan(0);
    }
  });

  it("registers the grid's three operations as three separate commands", () => {
    // The safety property, asserted at the registration level: if rebuild and release
    // were ever one command, destruction would be reachable from the safe path.
    expect(registered.has("setup_grid_preview")).toBe(true);
    expect(registered.has("setup_grid_rebuild")).toBe(true);
    expect(registered.has("setup_grid_release_orphans")).toBe(true);
  });

  it("keeps the Phase 1B project lifecycle registered", () => {
    for (const name of [
      "project_create",
      "project_open",
      "project_close",
      "project_current",
      "project_recent",
      "project_forget_recent",
      "project_extension",
      "project_default_folder",
    ]) {
      expect(registered.has(name), `${name} must stay registered`).toBe(true);
    }
  });
});
