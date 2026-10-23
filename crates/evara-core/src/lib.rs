//! Domain types, identifiers, the cycle/calendar time model and scoring primitives.
//!
//! # Layering contract
//!
//! `evara-core` is the bottom of the stack. It **must not**:
//!
//! - perform any I/O (no files, no sockets, no database)
//! - depend on any other `evara-*` crate
//! - mention a vendor's name or a vendor-specific concept
//!
//! Everything here is a plain data type or a pure function, so it is trivially
//! testable and can compile to any target including `wasm32`.
//!
//! # What lives here (added in later phases)
//!
//! - Identifier newtypes over UUIDv7
//! - The structure model: school, campus, term, cycle, cycle day, period, timeslot
//! - People, curriculum and activity types
//! - `Score` (hard/medium/soft, lexicographic) and `Violation`
//! - `Problem` / `Assignment` / `Placement`, the solver's in-memory contract
//!
//! Phase 1E adds the first of them: [`time::CycleCoverage`], the rule that decides
//! whether a repeating cycle is finished. It is here rather than in `evara-db` or the
//! interface because it is a question about the model, and a scheduling rule in the view
//! layer is an architecture bug.
//!
//! [`time::CycleRotation`] joins it with the other such rule: the order a cycle's positions
//! run in and where the rotation comes back round. Day 5 of a five-day cycle is followed by
//! Day 1, and "what follows the last day" is stated once, here, rather than worked out again
//! by whichever screen draws the sequence.
//!
//! [`branding::LogoImage`] decides whether an uploaded school logo is one the project
//! will carry, reading the bytes' own header rather than a filename or an extension.
//!
//! [`calendar::WorkingDays`] joins it: which weekdays a school normally teaches on. The
//! two modules are deliberately separate, because a cycle position and a weekday are
//! different kinds of thing and only one of them schedules anything — see the
//! [`calendar`] header.
//!
//! [`calendar::WeeklyPosition`] is the one place the two meet, and the header there sets out
//! why that is not a contradiction: a weekly cycle's positions are *named* by the calendar —
//! position 1 is Monday — while `ordinal` remains the whole of a day's scheduling identity.
//! All seven exist, always, so that one cycle definition can be shared by schools keeping
//! different weeks; [`calendar::WeeklyPosition::applies_to`] answers "and which of those does
//! this school teach on?" without storing the answer anywhere. See
//! [ADR 0017](../../../docs/adr/0017-weekly-and-custom-cycles.md).

//! [`lessons::LessonDemand`] is Stage C's addition, and the same kind of thing: how much of
//! a cycle one class section asks for, and whether the school's time model could hold one
//! occurrence of it at all. It is arithmetic over counts, so it belongs here rather than in
//! SQL or in a React component — and deliberately not in `evara-constraints`, which is
//! Phase 4's registry of timetable-scored evaluators. See that module's header.

//! [`import`] is Stage D's, and is here for a different reason: four crates have to agree
//! on what a source table and an import plan *are*. The parser in `evara-import` produces
//! one, `evara-db` turns it into a plan, the command layer carries both, and the interface
//! renders it — so the vocabulary sits under all of them. The per-cell rules come with it,
//! because "is this a code or a number the spreadsheet mangled?" is a question about a cell
//! and not about a school.

//! [`scheduling`] is M2's, and is here for the same reason as [`import`], one layer further
//! down: `evara-db` compiles a school into a [`scheduling::Problem`], `evara-constraints`
//! evaluates rules against one, `evara-solver` searches over one, and the command layer
//! carries the result — so the vocabulary sits under all four.
//! `docs/SCHEDULING-ARCHITECTURE-DECISIONS.md` §4.2 says so in one line, and that module's
//! header sets out what M2 declares and what each later milestone supplies. **No solver, no
//! constraint evaluator and no loader lives there**; it is a data contract plus the
//! deterministic fixtures the milestones after it are tested against.

pub mod branding;
pub mod calendar;
pub mod import;
pub mod lessons;
pub mod scheduling;
pub mod time;

#[cfg(test)]
mod layering_tests {
    /// This crate's own manifest, read at compile time.
    ///
    /// `include_str!` rather than `std::fs`: this crate performs no I/O, and a test that read
    /// a file at run time would be the first thing in it to do so. The path is relative to
    /// this source file, so it cannot drift.
    const MANIFEST: &str = include_str!("../Cargo.toml");

    /// Crates whose presence here would be an architecture bug rather than a dependency
    /// choice.
    ///
    /// `rusqlite` and `refinery` would make the bottom of the stack know SQL;
    /// `tauri` would tie it to a window and stop it compiling to `wasm32`; `calamine`,
    /// `csv` and `rust_xlsxwriter` belong to `evara-import` alone and
    /// `Cargo.toml`'s own comment says `chrono` "never leaves" it.
    const FORBIDDEN: &[&str] = &[
        "rusqlite",
        "refinery",
        "tauri",
        "calamine",
        "csv",
        "rust_xlsxwriter",
        "chrono",
        "reqwest",
        "tokio",
    ];

    #[test]
    fn the_manifest_search_can_actually_fail() {
        // A positive control, because a string search that matched nothing would make the two
        // tests below pass for the wrong reason — and a test that cannot fail is worse than no
        // test. `serde` *is* a dependency, so the same expression must find it.
        assert!(
            MANIFEST.contains("\nserde ="),
            "the manifest search no longer finds a dependency that is present; the two \
             layering tests below are vacuous until this is fixed"
        );
    }

    #[test]
    fn no_database_driver_or_window_reaches_the_bottom_of_the_stack() {
        // `CLAUDE.md` §2: `evara-core` depends on *nothing* — no other `evara-*` crate, no
        // I/O, no driver. The layering diagram asserts it and nothing enforced it; this does.
        // It is a string search over the manifest rather than a graph query, which is crude
        // and catches the case that actually happens: somebody adds a line.
        for crate_name in FORBIDDEN {
            assert!(
                !MANIFEST.contains(&format!("\n{crate_name} =")),
                "{crate_name} must not be a dependency of evara-core"
            );
        }
    }

    #[test]
    fn no_sibling_evara_crate_reaches_the_bottom_of_the_stack() {
        // The one rule that keeps the dependency graph acyclic: `evara-core` is the bottom,
        // so a dependency on any sibling would make `evara-db` → `evara-core` → `evara-db`
        // expressible. The scheduling vocabulary lives here precisely because four crates
        // need it and this crate needs none of them.
        assert!(
            !MANIFEST.contains("\nevara-"),
            "evara-core must not depend on another evara-* crate"
        );
    }

    #[test]
    fn the_fixtures_feature_adds_no_dependency() {
        // The generator is twenty lines of SplitMix64 and a UUID builder, deliberately: a
        // fixture whose bytes changed under a dependency bump would invalidate every solver
        // baseline committed against it. See `scheduling::rng`.
        assert!(
            MANIFEST.contains("fixtures = []"),
            "the fixtures feature must enable no optional dependency"
        );
    }
}
