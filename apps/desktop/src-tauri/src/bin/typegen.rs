//! Writes the generated TypeScript view of the Evara IPC contract.
//!
//! ```text
//! cargo run -p evara-desktop --features ts --bin evara-typegen
//! cargo run -p evara-desktop --features ts --bin evara-typegen -- --check
//! ```
//!
//! or, from the repository root, `npm run types:generate` and `npm run types:check`.
//!
//! # Why a binary rather than `cargo test`
//!
//! `ts-rs` can export as a side effect of `#[ts(export)]` during `cargo test`. That was
//! rejected: it would mean `npm run check` silently writing files into the working tree,
//! and it gives no way to ask "is the committed output current?" without writing first.
//! An explicit binary makes generation a command and staleness a question, which is what
//! [ADR 0009](../../../../../docs/adr/0009-typescript-type-generation.md) asks for when it says
//! a CI check must fail on drift — "otherwise *generated* degrades into *hand-maintained
//! with extra steps*".
//!
//! # The list below is the contract
//!
//! Every type the frontend may see is named in [`write_all`]. Nothing is discovered by
//! reflection, so adding a DTO is a deliberate edit — and
//! [`check_dependencies_are_listed`] fails the build if a listed type references one that
//! is not listed, which is the mistake that list would otherwise permit.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use ts_rs::{Config, TS};

/// Where the generated files go, relative to the repository root.
const OUTPUT_DIR: &str = "packages/domain/src/generated";

/// Name of the barrel file this tool writes alongside the per-type files.
const INDEX: &str = "index.ts";

/// One generated file.
struct Generated {
    /// File name within the output directory, e.g. `Campus.ts`.
    path: PathBuf,
    /// Exported TypeScript identifier.
    name: String,
    /// Complete file contents, `ts-rs` header and imports included.
    contents: String,
}

/// Declares the IPC contract: every type that may cross the boundary.
///
/// Expands to the generated files, any generation errors, and the set of
/// `(path, name)` pairs the listed types depend on — so the barrel and the completeness
/// check work from the same list rather than from two that could disagree.
macro_rules! contract {
    ($($ty:ty),* $(,)?) => {{
        let cfg = config();
        let mut files: Vec<Generated> = Vec::new();
        let mut errors: Vec<String> = Vec::new();
        let mut required: BTreeSet<(PathBuf, String)> = BTreeSet::new();

        $(
            match (
                <$ty as TS>::output_path(),
                <$ty as TS>::export_to_string(&cfg),
            ) {
                (Some(path), Ok(contents)) => files.push(Generated {
                    path,
                    name: <$ty as TS>::ident(&cfg),
                    contents,
                }),
                (None, _) => errors.push(format!(
                    "{} has no output path, so it cannot be exported",
                    std::any::type_name::<$ty>()
                )),
                (_, Err(error)) => errors.push(format!(
                    "{} could not be generated: {error}",
                    std::any::type_name::<$ty>()
                )),
            }

            // Every dependency must itself be in the list, or the generated file would
            // import a module this tool never writes.
            for dependency in <$ty as TS>::dependencies(&cfg) {
                required.insert((dependency.output_path.clone(), dependency.ts_name.clone()));
            }
        )*

        (files, errors, required)
    }};
}

/// Generation settings.
///
/// `large_int = "number"` is the one that matters. `ts-rs` maps `i64` to `bigint` by
/// default, which would be wrong here: Tauri carries the payload as JSON, so an `i64`
/// arrives in the frontend as a plain `number`. Declaring `bigint` would produce
/// TypeScript that disagrees with the value at runtime — every ordinal, capacity and
/// revision in the model. The values involved are positions and counts, far inside the
/// 2^53 a JSON number represents exactly.
fn config() -> Config {
    Config::new()
        .with_out_dir(OUTPUT_DIR)
        .with_large_int("number")
}

fn main() -> ExitCode {
    let check_only = std::env::args().any(|arg| arg == "--check");
    let out_dir = output_dir();

    let (files, errors) = write_all();
    if !errors.is_empty() {
        for error in &errors {
            eprintln!("evara-typegen: {error}");
        }
        return ExitCode::FAILURE;
    }

    if check_only {
        report(&check(&out_dir, &files), &out_dir)
    } else {
        match write(&out_dir, &files) {
            Ok(()) => {
                println!(
                    "evara-typegen: wrote {} types + {INDEX} to {}",
                    files.len(),
                    out_dir.display()
                );
                ExitCode::SUCCESS
            }
            Err(error) => {
                eprintln!("evara-typegen: {error}");
                ExitCode::FAILURE
            }
        }
    }
}

/// The IPC contract, as generated TypeScript.
///
/// Grouped the way the repositories are, so a reader can check the list against
/// `evara-db` rather than against nothing.
fn write_all() -> (Vec<Generated>, Vec<String>) {
    use evara_core::time::CycleCoverage;
    use evara_db::{
        AcademicYear, AcademicYearInput, Building, BuildingInput, CalendarDay, CalendarDayInput,
        CalendarDayKind, Campus, CampusInput, Cycle, CycleDay, CycleDayInput, CycleInput, GridCell,
        GridPreview, MaterialisationCounts, MaterialisationPlan, MaterialisationRequest, Period,
        PeriodInput, PeriodKind, PeriodStructure, PeriodStructureInput, PlanFingerprint,
        PlannedSlot, Resource, ResourceInput, Room, RoomInput, RoomType, RoomTypeInput, School,
        SchoolInput, Term, TermInput, Timeslot, TimeslotInput,
    };
    use evara_desktop_lib::commands::AppInfo;
    use evara_desktop_lib::commands::error::SetupError;

    let (files, mut errors, required) = contract![
        // Static structure.
        School,
        SchoolInput,
        Campus,
        CampusInput,
        Building,
        BuildingInput,
        RoomType,
        RoomTypeInput,
        Room,
        RoomInput,
        Resource,
        ResourceInput,
        AcademicYear,
        AcademicYearInput,
        Term,
        TermInput,
        // Time model.
        Cycle,
        CycleInput,
        CycleDay,
        CycleDayInput,
        PeriodStructure,
        PeriodStructureInput,
        Period,
        PeriodInput,
        PeriodKind,
        CalendarDay,
        CalendarDayInput,
        CalendarDayKind,
        CycleCoverage,
        // The materialised grid.
        Timeslot,
        TimeslotInput,
        MaterialisationRequest,
        MaterialisationPlan,
        MaterialisationCounts,
        PlannedSlot,
        PlanFingerprint,
        GridCell,
        GridPreview,
        // Interface contracts defined with the command handlers.
        AppInfo,
        SetupError,
    ];

    errors.extend(check_dependencies_are_listed(&files, &required));
    (files, errors)
}

/// Fails when a listed type depends on one that is not listed.
///
/// Without this, forgetting an entry produces a generated file importing a module that
/// was never written — a TypeScript error at the far end of the build, blamed on the
/// frontend rather than on this list.
fn check_dependencies_are_listed(
    files: &[Generated],
    required: &BTreeSet<(PathBuf, String)>,
) -> Vec<String> {
    let written: BTreeSet<&Path> = files.iter().map(|f| f.path.as_path()).collect();
    required
        .iter()
        .filter(|(path, _)| !written.contains(path.as_path()))
        .map(|(path, name)| {
            format!(
                "`{name}` is referenced by the contract but not listed in `write_all` \
                 (expected to generate {})",
                path.display()
            )
        })
        .collect()
}

/// The barrel file, so the frontend imports one module rather than forty.
///
/// Written here rather than by hand because its contents are entirely determined by the
/// list above, and a hand-written barrel is one more thing to forget.
fn index_of(files: &[Generated]) -> String {
    let mut by_path: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for file in files {
        let module = file
            .path
            .file_stem()
            .and_then(std::ffi::OsStr::to_str)
            .unwrap_or_default();
        by_path.entry(module).or_default().push(&file.name);
    }

    let mut out = String::from(
        "// This file was generated by `evara-typegen`. Do not edit this file manually.\n\
         //\n\
         // The Evara IPC contract, as TypeScript. Every type here is generated from a Rust\n\
         // definition; see docs/adr/0009-typescript-type-generation.md and\n\
         // docs/adr/0012-records-are-the-ipc-contract.md.\n\n",
    );
    for (module, mut names) in by_path {
        names.sort_unstable();
        // `writeln!` into the buffer rather than `push_str(&format!(..))`: one
        // allocation instead of two. Writing to a `String` cannot fail, so the
        // result is discarded deliberately.
        let _ = writeln!(
            out,
            "export type {{ {} }} from \"./{module}\";",
            names.join(", ")
        );
    }
    out
}

/// Writes every file, replacing what is there.
fn write(out_dir: &Path, files: &[Generated]) -> std::io::Result<()> {
    std::fs::create_dir_all(out_dir)?;

    // Anything left over from a type that has since been removed or renamed would keep
    // type-checking and keep lying, so the directory is cleared rather than merged into.
    for entry in std::fs::read_dir(out_dir)? {
        let path = entry?.path();
        if path.extension().is_some_and(|ext| ext == "ts") {
            std::fs::remove_file(path)?;
        }
    }

    for file in files {
        std::fs::write(out_dir.join(&file.path), &file.contents)?;
    }
    std::fs::write(out_dir.join(INDEX), index_of(files))
}

/// How the committed output differs from what the Rust types say it should be.
fn check(out_dir: &Path, files: &[Generated]) -> Vec<String> {
    let mut drift = Vec::new();
    let mut expected: BTreeSet<PathBuf> = BTreeSet::new();

    for file in files {
        expected.insert(file.path.clone());
        match std::fs::read_to_string(out_dir.join(&file.path)) {
            Err(_) => drift.push(format!("{} is missing", file.path.display())),
            Ok(found) if normalise(&found) != normalise(&file.contents) => {
                drift.push(format!("{} is out of date", file.path.display()));
            }
            Ok(_) => {}
        }
    }

    expected.insert(PathBuf::from(INDEX));
    match std::fs::read_to_string(out_dir.join(INDEX)) {
        Err(_) => drift.push(format!("{INDEX} is missing")),
        Ok(found) if normalise(&found) != normalise(&index_of(files)) => {
            drift.push(format!("{INDEX} is out of date"));
        }
        Ok(_) => {}
    }

    // A file for a type that no longer exists is drift too: it still type-checks, so
    // nothing else would ever notice it.
    if let Ok(entries) = std::fs::read_dir(out_dir) {
        for entry in entries.flatten() {
            let name = PathBuf::from(entry.file_name());
            if name.extension().is_some_and(|ext| ext == "ts") && !expected.contains(&name) {
                drift.push(format!("{} is no longer generated", name.display()));
            }
        }
    }

    drift
}

/// Ignores line-ending differences when comparing.
///
/// Git may check these files out with CRLF on Windows while the generator emits LF, and a
/// drift report caused purely by that would be noise rather than a signal. `.gitattributes`
/// pins the committed form; this keeps the check honest either way.
fn normalise(text: &str) -> String {
    text.replace("\r\n", "\n")
}

fn report(drift: &[String], out_dir: &Path) -> ExitCode {
    if drift.is_empty() {
        println!("evara-typegen: generated types are up to date");
        return ExitCode::SUCCESS;
    }

    eprintln!(
        "evara-typegen: {} is stale — the Rust types and the committed TypeScript disagree:",
        out_dir.display()
    );
    for item in drift {
        eprintln!("  - {item}");
    }
    eprintln!("\nRun `npm run types:generate` and commit the result.");
    ExitCode::FAILURE
}

/// Resolves the output directory from this crate's location.
///
/// Derived from `CARGO_MANIFEST_DIR` rather than the current working directory so that
/// `cargo run -p evara-typegen` writes to the right place from anywhere in the workspace.
/// `--out` overrides it, which is how a test points it at a temporary directory.
fn output_dir() -> PathBuf {
    let mut args = std::env::args().skip_while(|arg| arg != "--out");
    if let (Some(_), Some(dir)) = (args.next(), args.next()) {
        return PathBuf::from(dir);
    }

    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("..")
        .join(OUTPUT_DIR)
}
