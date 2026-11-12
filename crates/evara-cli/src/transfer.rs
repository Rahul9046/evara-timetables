//! `evara export`, `evara inspect` and `evara import` — portable project transfer, headless.
//!
//! # This module is a command surface and nothing else
//!
//! Every byte of the protocol lives in [`evara_project`]: the archive writer, the reader and
//! its verification order, the staging lifecycle, the replacement protocol and its recovery.
//! What is here is argument parsing, a progress line, an exit code and a summary a person can
//! read. **There is no second archive writer and no second import path** — M10 Stage 3's whole
//! point is that the CLI is the first caller of Stage 2's API rather than a reimplementation of
//! it, which is also why the desktop layer in Stage 4 can be a second caller with no shared
//! code to keep in step.
//!
//! # The three commands, as the proposal specifies them
//!
//! ```text
//! evara export --project <path.evaraproj> --to <archive.evara>
//! evara inspect <archive.evara>
//! evara import <archive.evara> --to <project.evaraproj> [--replace]
//! ```
//!
//! `--to` rather than `--output`, and `export` naming its source with `--project`, because
//! that is [§4.4 of the proposal](../../../docs/M10-PROJECT-TRANSFER-PROPOSAL.md) and because
//! `--project` is already how `validate`, `generate` and `save` name a project file.
//!
//! # Exit codes
//!
//! Three, as §4.4 fixes them, and the middle one is the one that matters:
//!
//! | Code | Means |
//! | --- | --- |
//! | `0` | done |
//! | `1` | **refused for a reason the command correctly produced** — a tampered archive, a schema this build cannot read, a capability it does not implement, a destination already occupied |
//! | `2` | the command itself failed — the disk, a path that cannot be resolved, a bug |
//!
//! A refusal is not a failure. An archive from a newer Evara is a complete, correct answer to
//! the question asked, and a script that could not tell it from a crashed command would retry
//! the one and give up on the other exactly backwards. [`is_refusal`] is the single place that
//! judgement is made.
//!
//! # Cancellation
//!
//! Every function here threads a `&dyn Cancel` through to the library, so the cancellation
//! path is real and is exercised by `transfer::tests`. The **binary** passes
//! [`NeverCancel`](evara_core::scheduling::NeverCancel): nothing in `evara-cli` installs a
//! signal handler — `generate.rs` records the same thing about solving — and adding a
//! `ctrlc` dependency to deliver one is not in Stage 3's scope. So the honest summary is that
//! the seam is implemented and tested while the shipped command has nothing to trigger it.
//! Ctrl-C ends the process; a partial export leaves no archive, because the writer stages to a
//! `.part` and renames, and an interrupted import leaves a journal row that the next
//! `evara import` recovers.
//!
//! # Progress goes to stderr
//!
//! So `stdout` stays the summary and only the summary. A caller redirecting one does not get
//! the other mixed into it.

use std::path::PathBuf;

use evara_core::scheduling::{Cancel, NeverCancel};
use evara_db::{Database, Settings};
use evara_project::{
    ARCHIVE_EXTENSION, ImportReport, Importer, OnConflict, Phase, Readability, TransferError,
    TransferProgress, inspect as inspect_archive, recover_pending, seal,
};

/// This build's version, recorded in a manifest's `generator` and used as the importing
/// application version.
const VERSION: &str = env!("CARGO_PKG_VERSION");

/// What the command decided, so `main` chooses an exit code without re-deriving it.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Outcome {
    /// It did what was asked.
    Done,
    /// It refused, for a reason it has already explained on stderr.
    Refused,
}

/// Whether an error is the command *working* and the answer being no.
///
/// Everything about the archive or the destination is a refusal: the user can act on it, and a
/// script should treat it as an answer. Everything about the machine is a failure.
///
/// `Cancelled` is a failure rather than a refusal, and deliberately: the command did not do
/// what was asked and did not decide not to. It is unreachable from the shipped binary, which
/// passes `NeverCancel`.
fn is_refusal(error: &TransferError) -> bool {
    match error {
        // Judgements about the archive.
        TransferError::NotAnArchive { .. }
        | TransferError::ManifestMissing { .. }
        | TransferError::ManifestUnreadable { .. }
        | TransferError::UnknownFormat { .. }
        | TransferError::FormatTooNew { .. }
        | TransferError::ManifestDisagreesWithPayload { .. }
        | TransferError::UnsupportedCapability { .. }
        | TransferError::UnsupportedDigest { .. }
        | TransferError::EntryMissing { .. }
        | TransferError::UnsafeEntryName { .. }
        | TransferError::UnsupportedCompression { .. }
        | TransferError::EntryEncrypted { .. }
        | TransferError::EntryTooLarge { .. }
        | TransferError::EntryExpandsTooFar { .. }
        | TransferError::TooManyEntries { .. }
        | TransferError::DuplicateEntryName { .. }
        | TransferError::EntryNotAFile { .. }
        | TransferError::PayloadLengthMismatch { .. }
        | TransferError::ChecksumMismatch { .. }
        | TransferError::Truncated
        // Judgements about the destination.
        | TransferError::DestinationExists { .. }
        | TransferError::WrongExtension { .. }
        | TransferError::DestinationUnsafe { .. }
        | TransferError::ReplacementInFlight { .. } => true,

        // The payload was opened and judged by `evara-db`. Two of those are refusals a user
        // acts on; the rest are the database layer failing.
        TransferError::Database(error) => {
            error.is_schema_too_new() || matches!(error, evara_db::DbError::NotAnEvaraProject { .. })
        }

        // Everything else is a failure, and the list is worth knowing: `Cancelled`, `Io`,
        // `RenameBlocked` and `RecoveryAmbiguous` are the machine or us, not a judgement
        // about the archive.
        //
        // One wildcard rather than those four spelled out plus a wildcard, because
        // `TransferError` is `#[non_exhaustive]`: the arm is required whatever is listed
        // above it, and two arms with the same body is a lint rather than a safeguard. What
        // it means for a **new** variant is deliberate — it defaults to a failure rather
        // than to a refusal, which is the safer mistake. A refusal claims Evara judged the
        // archive and decided against it, and saying that about an error nobody has
        // classified yet would put words in the product's mouth; exit 2 says only "this did
        // not work", which is true of anything unclassified.
        _ => false,
    }
}

/// Turns a library error into an outcome, printing it either way.
///
/// One place, so a refusal is never reported as a crash and a crash is never reported as a
/// reasoned no.
fn settle(command: &str, error: &TransferError) -> Result<Outcome, String> {
    if is_refusal(error) {
        eprintln!("evara {command}: {error}");
        Ok(Outcome::Refused)
    } else {
        Err(error.to_string())
    }
}

/// A progress line on stderr, one per phase change.
///
/// Phase changes rather than percentages: the phases take wildly different times, so a single
/// bar would sit at ninety per cent through the slowest one. `Phase::as_str` is the label, so
/// the words a user sees are the library's own rather than a second vocabulary here.
struct Lines {
    quiet: bool,
    last: std::sync::Mutex<Option<Phase>>,
}

impl Lines {
    fn new(quiet: bool) -> Self {
        Self {
            quiet,
            last: std::sync::Mutex::new(None),
        }
    }
}

impl TransferProgress for Lines {
    fn progress(&self, phase: Phase, done: u64, total: u64) {
        if self.quiet {
            return;
        }
        // Only on a change, so a byte-by-byte stream does not produce a line per buffer.
        let mut last = match self.last.lock() {
            Ok(last) => last,
            // A poisoned lock means another thread panicked while reporting progress. Losing
            // a progress line is not worth propagating a panic into the transfer.
            Err(poisoned) => poisoned.into_inner(),
        };
        if *last == Some(phase) {
            return;
        }
        *last = Some(phase);
        drop(last);

        if total == 0 {
            eprintln!("  {}…", phase.as_str());
        } else {
            eprintln!("  {}… ({} bytes)", phase.as_str(), total.max(done));
        }
    }
}

// ---------------------------------------------------------------------------------------
// export
// ---------------------------------------------------------------------------------------

/// `evara export --project <path> --to <archive.evara>`
pub(crate) fn export(args: &[String]) -> Result<Outcome, String> {
    let Some(request) = ExportRequest::parse(args)? else {
        print!("{EXPORT_USAGE}");
        return Ok(Outcome::Done);
    };
    export_with(&request, &NeverCancel)
}

/// The body, with cancellation injectable so the path is tested rather than assumed.
fn export_with(request: &ExportRequest, cancel: &dyn Cancel) -> Result<Outcome, String> {
    if !request.project.is_file() {
        return Err(format!("no project at {}", request.project.display()));
    }

    // Refused here rather than in `seal`, which renames its `.part` over whatever is at the
    // destination. Nothing in Stage 2 refuses an occupied *archive* path — a destination the
    // user typed is theirs, and the library leaves the policy to the caller — so this is the
    // guard, and Stage 4's desktop export needs the same one before it offers an overwrite.
    if request.to.exists() {
        eprintln!(
            "evara export: {} already exists. Choose another name, or remove it first — \
             Evara will not overwrite an archive you may still need.",
            request.to.display()
        );
        return Ok(Outcome::Refused);
    }

    let progress = Lines::new(request.quiet);

    // `open_for_inspection`, never `open_existing`: an export reads somebody's live file and
    // must not migrate it, must not recreate a trigger and must not stamp
    // `last_opened_with_app_version`. E-01 is why the snapshot on this handle cannot be a
    // `VACUUM INTO` — it is refused under `query_only` — and E-02 is why the stepped backup
    // works there.
    let db = Database::open_for_inspection(&request.project).map_err(|error| error.to_string())?;

    // Staging beside the archive: the same volume the archive is going to, so the snapshot is
    // written where the user has already decided there is room. A `TempDir` so the directory
    // and the copy inside it go away on **every** path out of here, including a `?`.
    let parent = request.to.parent().filter(|p| !p.as_os_str().is_empty());
    let staging = match parent {
        Some(parent) => tempfile::Builder::new()
            .prefix(".evara-export-")
            .tempdir_in(parent),
        None => tempfile::Builder::new().prefix(".evara-export-").tempdir(),
    }
    .map_err(|error| format!("could not prepare a staging folder: {error}"))?;

    let snapshot = db
        .snapshot_into(staging.path(), cancel, &mut |page| {
            progress.progress(
                Phase::Snapshotting,
                u64::from(page.copied_pages),
                u64::from(page.total_pages),
            );
        })
        .map_err(|error| error.to_string())?;

    // **The lock is released before the archive is written.** D8 holds the workspace lock for
    // the snapshot and nothing more, and `seal` takes the snapshot by value precisely so a
    // caller can do this. Compressing a multi-hundred-megabyte payload with the project still
    // held open would block every other command for the whole of it.
    drop(db);

    let report = match seal(snapshot, &request.to, &progress, cancel) {
        Ok(report) => report,
        Err(error) => return settle("export", &error),
    };

    let manifest = &report.manifest;
    println!("Exported {}", report.path.display());
    println!("  project        {}", manifest.project_id);
    println!("  schema         V{}", manifest.payload.schema_version);
    println!(
        "  schools        {}",
        describe_schools(manifest.schools.as_slice())
    );
    println!(
        "  change feed    {} entries (watermark {})",
        manifest.change_log.entries, manifest.change_log.watermark
    );
    println!(
        "  payload        {} bytes -> archive {} bytes ({:.1}% smaller)",
        report.payload_bytes,
        report.archive_bytes,
        report.compression_saving_percent()
    );
    println!("  sha256         {}", report.payload_sha256);
    if !manifest.capabilities.is_empty() {
        println!("  capabilities   {}", manifest.capabilities.join(", "));
    }
    println!("\nThe project was not modified.");
    Ok(Outcome::Done)
}

/// A one-line description of the schools a manifest names.
fn describe_schools(schools: &[evara_project::manifest::SchoolSummary]) -> String {
    if schools.is_empty() {
        return String::from("none");
    }
    schools
        .iter()
        .map(|school| format!("{} ({} campuses)", school.name, school.campuses))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Parsed `export` arguments.
#[derive(Debug, PartialEq, Eq)]
struct ExportRequest {
    project: PathBuf,
    to: PathBuf,
    quiet: bool,
}

impl ExportRequest {
    /// `None` means help was asked for.
    fn parse(args: &[String]) -> Result<Option<Self>, String> {
        let mut project = None;
        let mut to = None;
        let mut quiet = false;
        let mut rest = args.iter();

        while let Some(argument) = rest.next() {
            match argument.as_str() {
                "--help" | "-h" => return Ok(None),
                "--project" | "-p" => {
                    project = Some(PathBuf::from(rest.next().ok_or("--project needs a path")?));
                }
                "--to" | "-o" => {
                    to = Some(PathBuf::from(rest.next().ok_or("--to needs a path")?));
                }
                "--quiet" | "-q" => quiet = true,
                other => return Err(format!("unknown option {other:?}\n\n{EXPORT_USAGE}")),
            }
        }

        let project =
            project.ok_or_else(|| format!("export needs --project <path>\n\n{EXPORT_USAGE}"))?;
        let to = to.ok_or_else(|| {
            format!("export needs --to <archive.{ARCHIVE_EXTENSION}>\n\n{EXPORT_USAGE}")
        })?;
        Ok(Some(Self { project, to, quiet }))
    }
}

const EXPORT_USAGE: &str = "\
Write a portable *.evara archive from a project.

Usage: evara export --project <path.evaraproj> --to <archive.evara>

The project is opened read-only: nothing is migrated, no trigger is recreated and
the file is not stamped. The archive carries a verified snapshot, so every school,
timetable version, manual edit, lock, repair lineage edge and change-feed row
travels with it.

An existing archive is refused rather than overwritten.

Options:
  --project, -p  The .evaraproj to export. Required.
  --to, -o       Where to write the .evara archive. Required.
  --quiet, -q    No progress lines on stderr.
  --help, -h     This text.

Exit codes: 0 written, 1 refused (destination exists, unsafe path), 2 failed.
";

// ---------------------------------------------------------------------------------------
// inspect
// ---------------------------------------------------------------------------------------

/// `evara inspect <archive.evara>`
pub(crate) fn inspect(args: &[String]) -> Result<Outcome, String> {
    let Some(archive) = parse_single_path(args, "inspect", INSPECT_USAGE)? else {
        print!("{INSPECT_USAGE}");
        return Ok(Outcome::Done);
    };

    // The Stage 2 reader, which writes nothing and stages nothing. There is no second parse of
    // a manifest here: an `inspect` that read the file its own way could tell a user an archive
    // was fine and then have `import` refuse it.
    let summary = match inspect_archive(&archive) {
        Ok(summary) => summary,
        Err(error) => return settle("inspect", &error),
    };

    let manifest = &summary.manifest;
    println!("{}", archive.display());
    println!(
        "  format         {} v{}",
        manifest.format, manifest.format_version
    );
    println!(
        "  written by     {} {}",
        manifest.generator.app, manifest.generator.version
    );
    println!("  exported at    {}", manifest.exported_at);
    println!("  project        {}", manifest.project_id);
    if let Some(created_by) = manifest.created_by_site_id {
        println!("  created by     installation {created_by}");
    }
    if let Some(exported_by) = manifest.exported_by_site_id {
        println!("  exported by    installation {exported_by}");
    }
    println!("  schema         V{}", manifest.payload.schema_version);
    println!(
        "  payload        {} ({} bytes)",
        manifest.payload.entry, manifest.payload.byte_length
    );
    println!("  archive        {} bytes", summary.archive_bytes);
    println!(
        "  schools        {}",
        describe_schools(manifest.schools.as_slice())
    );
    for school in &manifest.schools {
        println!("                 {} — {}", school.id, school.name);
    }
    println!(
        "  change feed    {} entries (watermark {})",
        manifest.change_log.entries, manifest.change_log.watermark
    );
    println!(
        "  tables         {} with rows",
        manifest.counts.values().filter(|n| **n > 0).count()
    );
    if !manifest.capabilities.is_empty() {
        println!("  capabilities   {}", manifest.capabilities.join(", "));
    }
    if !summary.unknown_entries.is_empty() {
        println!(
            "  also carries   {} (ignored by this build)",
            summary.unknown_entries.join(", ")
        );
    }

    // Said every time, at the point somebody is deciding whether to trust the file. The
    // digests detect corruption; they establish nothing about who made it.
    println!(
        "\nIntegrity: the digests in this archive detect corruption, not authenticity.\n\
         Anyone can rewrite a manifest and recompute every digest below it, so a .evara\n\
         carries exactly the authority of the person who sent it to you."
    );

    match &summary.readable {
        Readability::Readable => {
            // Precisely worded, because the obvious reading is wrong. `inspect` verifies the
            // manifest and `checksums.json`; **it does not read the payload** — streaming and
            // hashing that is step 9 of the verification order and belongs to `import`, which
            // is what keeps `inspect` free of staging and of a full read. So this says the
            // container is one this build understands, and says where the payload is checked.
            println!(
                "\nThis build can read the archive: the container, its manifest and its \
                 digest\nrecord are all ones it understands. The payload itself is verified \
                 against\nthat record during `evara import`, not here — so a damaged payload \
                 is\ncaught then, and refused before anything is written."
            );
            Ok(Outcome::Done)
        }
        other => {
            // Reported on stderr and as exit 1: the command worked and the answer is no.
            if let Some(refusal) = other.as_refusal() {
                eprintln!("\nevara inspect: {refusal}");
            }
            Ok(Outcome::Refused)
        }
    }
}

const INSPECT_USAGE: &str = "\
Describe a portable *.evara archive. Writes nothing and imports nothing.

Usage: evara inspect <archive.evara>

Reports the container version, the project identity, the payload's schema version,
the schools, the change feed, the declared capabilities and whether this build can
import it.

The digests in an archive detect CORRUPTION, not AUTHENTICITY: anyone can rewrite a
manifest and recompute every digest below it. A .evara carries exactly the authority
of the person who sent it.

Options:
  --help, -h     This text.

Exit codes: 0 readable, 1 this build cannot read it (or it is damaged), 2 failed.
";

// ---------------------------------------------------------------------------------------
// import
// ---------------------------------------------------------------------------------------

/// `evara import <archive.evara> --to <project.evaraproj> [--replace]`
pub(crate) fn import(args: &[String]) -> Result<Outcome, String> {
    let Some(request) = ImportRequest::parse(args)? else {
        print!("{IMPORT_USAGE}");
        return Ok(Outcome::Done);
    };
    import_with(&request, &NeverCancel)
}

/// The body, with cancellation injectable.
fn import_with(request: &ImportRequest, cancel: &dyn Cancel) -> Result<Outcome, String> {
    let progress = Lines::new(request.quiet);

    // The installation's own settings database: the journal a replacement is recorded in and
    // the source of the `site_id` V1's change-log triggers read. See `open_settings` for why a
    // test may not reach the real one.
    let mut settings = open_settings().map_err(|error| error.to_string())?;
    let site_id = settings.site_id();

    // **Recovery first.** A replacement interrupted by a crash or a Ctrl-C left a journal row,
    // and S2 says recovery reconciles it against what is actually on disk rather than trusting
    // the recorded phase. Running it before this import means a destination left half-replaced
    // is resolved before anything new is aimed at it.
    let recovered = recover_pending(&mut settings).map_err(|error| error.to_string())?;
    for entry in &recovered {
        eprintln!(
            "evara import: recovered an interrupted replacement of {} — {}{}",
            entry.destination.display(),
            entry.outcome,
            if entry.resolved {
                ""
            } else {
                " (still needs attention)"
            }
        );
    }

    let on_conflict = if request.replace {
        OnConflict::Replace
    } else {
        OnConflict::Refuse
    };

    let report = match evara_project::import(
        &request.archive,
        &request.to,
        on_conflict,
        Importer {
            app_version: VERSION,
            site_id,
        },
        &mut settings,
        &progress,
        cancel,
    ) {
        Ok(report) => report,
        Err(error) => return settle("import", &error),
    };

    print_import(&report);
    Ok(Outcome::Done)
}

/// What an import did, for somebody reading a terminal.
fn print_import(report: &ImportReport) {
    println!("Imported {}", report.path.display());
    println!("  project        {}", report.project_id);
    if report.migrated {
        println!(
            "  schema         V{} -> V{} (migrated)",
            report.schema_version_found, report.schema_version_now
        );
    } else {
        println!(
            "  schema         V{} (already current)",
            report.schema_version_now
        );
    }
    let rows: i64 = report.counts.values().sum();
    println!(
        "  carried        {rows} rows across {} tables",
        report.counts.values().filter(|n| **n > 0).count()
    );

    if let Some(kept) = &report.replaced_copy_kept_at {
        println!(
            "  replaced       the previous project is kept at {}",
            kept.display()
        );
    }
    if !report.unknown_entries.is_empty() {
        println!(
            "  ignored        {} (written by a newer Evara)",
            report.unknown_entries.join(", ")
        );
    }

    // Advisory, and said so: the payload is the authority and a manifest count is a
    // convenience, so a disagreement is reported rather than refused.
    if !report.count_disagreements.is_empty() {
        println!("\nThe manifest's row counts disagree with the payload in these tables:");
        for disagreement in &report.count_disagreements {
            println!(
                "  {} — manifest said {}, the project holds {}",
                disagreement.table, disagreement.declared, disagreement.found
            );
        }
        println!("The project itself is the authority; these counts are advisory.");
    }

    println!("\nEvara did not open the imported project.");
}

/// Parsed `import` arguments.
#[derive(Debug, PartialEq, Eq)]
struct ImportRequest {
    archive: PathBuf,
    to: PathBuf,
    replace: bool,
    quiet: bool,
}

impl ImportRequest {
    /// `None` means help was asked for.
    fn parse(args: &[String]) -> Result<Option<Self>, String> {
        let mut archive = None;
        let mut to = None;
        let mut replace = false;
        let mut quiet = false;
        let mut rest = args.iter();

        while let Some(argument) = rest.next() {
            match argument.as_str() {
                "--help" | "-h" => return Ok(None),
                "--to" | "-o" => {
                    to = Some(PathBuf::from(rest.next().ok_or("--to needs a path")?));
                }
                "--replace" => replace = true,
                "--quiet" | "-q" => quiet = true,
                other if other.starts_with('-') => {
                    return Err(format!("unknown option {other:?}\n\n{IMPORT_USAGE}"));
                }
                positional => {
                    if archive.is_some() {
                        return Err(format!(
                            "import takes one archive; {positional:?} is a second\n\n{IMPORT_USAGE}"
                        ));
                    }
                    archive = Some(PathBuf::from(positional));
                }
            }
        }

        let archive = archive.ok_or_else(|| {
            format!("import needs an archive\n\nUsage: evara import <archive.{ARCHIVE_EXTENSION}> --to <project.evaraproj>\n")
        })?;
        let to = to.ok_or_else(|| format!("import needs --to <path>\n\n{IMPORT_USAGE}"))?;
        Ok(Some(Self {
            archive,
            to,
            replace,
            quiet,
        }))
    }
}

const IMPORT_USAGE: &str = "\
Read a portable *.evara archive into a project file.

Usage: evara import <archive.evara> --to <project.evaraproj> [--replace]

The archive is verified before anything is written: entry names and sizes, the
SHA-256 chain, then the payload is staged and audited read-only, migrated forward
and verified again. An archive this build cannot read is refused before a single
byte of payload reaches the disk.

The destination is only ever written by a rename. An existing project is REFUSED
unless --replace is given, and a replaced project is never deleted: it is kept
beside the new one as <stem>.replaced-<stamp>.evaraproj.

An interrupted replacement from a previous run is recovered first.

Evara does not open what it imported.

Options:
  --to, -o       Where to write the .evaraproj. Required.
  --replace      Replace an existing project, keeping the old one beside it.
  --quiet, -q    No progress lines on stderr.
  --help, -h     This text.

Exit codes: 0 imported, 1 refused (damaged or tampered archive, schema too new,
unsupported capability, destination exists without --replace, unsafe path), 2 failed.
";

// ---------------------------------------------------------------------------------------
// shared
// ---------------------------------------------------------------------------------------

/// Parses a lone positional path, or `None` for help.
fn parse_single_path(
    args: &[String],
    command: &str,
    usage: &'static str,
) -> Result<Option<PathBuf>, String> {
    let mut found: Option<PathBuf> = None;
    for argument in args {
        match argument.as_str() {
            "--help" | "-h" => return Ok(None),
            other if other.starts_with('-') => {
                return Err(format!("unknown option {other:?}\n\n{usage}"));
            }
            positional => {
                if found.is_some() {
                    return Err(format!(
                        "{command} takes one path; {positional:?} is a second\n\n{usage}"
                    ));
                }
                found = Some(PathBuf::from(positional));
            }
        }
    }
    found
        .map(Some)
        .ok_or_else(|| format!("{command} needs a path\n\n{usage}"))
}

/// This installation's settings database — the only answer in a shipped binary.
#[cfg(not(test))]
fn open_settings() -> evara_db::Result<Settings> {
    Settings::open_default()
}

/// The settings database a test named, and **never** the installation's own.
///
/// The same seam `save.rs` carries, for the same reason and with the same refusal to fall
/// back. `Settings::open_at` runs the settings refinery chain with `set_abort_missing(true)`,
/// so the chain is **forward only**: once a newer build has migrated that file, every older
/// build refuses to open its own settings until somebody deletes it. A test that reached the
/// real file would migrate the developer's installation as a side effect of `cargo test`, and
/// the damage would surface later, on another branch, as an unrelated-looking failure. CI
/// never sees it, because its runners start with no settings database at all.
///
/// An unset override **panics** rather than falling back, because falling back would restore
/// exactly that defect and restore it silently — a test added without the fixture would pass
/// while quietly migrating the developer's installation.
#[cfg(test)]
fn open_settings() -> evara_db::Result<Settings> {
    let path = tests::settings_override().expect(
        "an `import` test must set the settings path with `tests::use_settings_at` -- opening \
         the installation's own settings database from a test migrates it forward and leaves \
         every older build unable to open it",
    );
    Settings::open_at(&path)
}

#[cfg(test)]
mod tests;
