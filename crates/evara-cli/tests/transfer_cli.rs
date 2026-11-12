//! The real `evara` executable, driven as a user drives it.
//!
//! # Why these are here and the `import` tests are not
//!
//! A spawned binary is the only honest test of a command's *contract*: the arguments it
//! accepts, the exit code it returns and the text it prints. `cargo` hands an integration
//! test the built binary's path through `CARGO_BIN_EXE_evara`, so these run the same bytes a
//! user would.
//!
//! `export` and `inspect` touch no settings database, so spawning them is safe anywhere.
//! **`import` is deliberately absent**: the spawned binary is compiled without `cfg(test)`, so
//! its settings seam is the production one and it would open — and migrate — the developer's
//! own installation settings. That is the defect PR #39 fixed for `save`, and there is no
//! portable way to redirect it, because `directories` reads `$HOME` on macOS but asks the
//! Windows known-folder API on Windows. `import` is covered in-crate instead, through the real
//! protocol with a settings database pointed at a `TempDir`; `src/transfer/tests.rs` says so
//! where a reader will find it.
//!
//! # Exit codes are the subject, not a detail
//!
//! `0` done, `1` refused for a reason the command correctly produced, `2` the command failed.
//! A script that could not tell a tampered archive from a crashed command would retry one and
//! give up on the other exactly backwards, so each code is asserted rather than assumed.

use std::path::{Path, PathBuf};
use std::process::Command;

use evara_db::fixtures::transfer::Shape;

/// The binary under test — the one `cargo` just built.
const EVARA: &str = env!("CARGO_BIN_EXE_evara");

/// What a run produced.
struct Run {
    code: i32,
    stdout: String,
    stderr: String,
}

impl Run {
    /// Runs `evara` with `args`.
    fn of(args: &[&str]) -> Self {
        let output = Command::new(EVARA)
            .args(args)
            .output()
            .expect("run the evara binary");
        Self {
            code: output.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        }
    }

    /// Everything the command said, for an assertion message.
    fn said(&self) -> String {
        format!("stdout:\n{}\nstderr:\n{}", self.stdout, self.stderr)
    }
}

/// A realistic project in a directory the test owns.
struct Fixture {
    dir: tempfile::TempDir,
    project: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let (mut built, _) = evara_db::fixtures::transfer::build(Shape::Varied);
        let source = built.path().to_path_buf();
        built.close();

        let dir = tempfile::TempDir::new().expect("temp dir");
        let project = dir.path().join("northgate.evaraproj");
        std::fs::copy(&source, &project).expect("copy the project somewhere we own");
        drop(built);

        Self { dir, project }
    }

    fn at(&self, name: &str) -> PathBuf {
        self.dir.path().join(name)
    }

    /// Exports and returns the archive path, asserting success.
    fn export_to(&self, name: &str) -> PathBuf {
        let archive = self.at(name);
        let run = Run::of(&[
            "export",
            "--project",
            &self.project.to_string_lossy(),
            "--to",
            &archive.to_string_lossy(),
        ]);
        assert_eq!(run.code, 0, "export should succeed: {}", run.said());
        assert!(archive.is_file(), "the archive must exist: {}", run.said());
        archive
    }
}

/// A path as the command line wants it.
fn text(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

// ---------------------------------------------------------------------------------------
// the commands exist and describe themselves
// ---------------------------------------------------------------------------------------

#[test]
fn the_three_commands_are_listed_as_implemented_rather_than_planned() {
    let run = Run::of(&["--help"]);
    assert_eq!(run.code, 0);
    for command in ["export", "inspect", "import"] {
        assert!(
            run.stdout.contains(command),
            "`{command}` must appear in the help text: {}",
            run.said()
        );
    }
    // They moved out of the planned list, so neither may still be announced as a future
    // phase. "phase 9" was export and import's entry.
    assert!(
        !run.stdout.contains("phase 9"),
        "export and import are implemented and must not still be listed as planned: {}",
        run.said()
    );
}

#[test]
fn each_command_prints_its_own_usage() {
    for command in ["export", "inspect", "import"] {
        let run = Run::of(&[command, "--help"]);
        assert_eq!(
            run.code,
            0,
            "{command} --help should succeed: {}",
            run.said()
        );
        assert!(
            run.stdout.contains("Usage: evara"),
            "{command} --help must print usage: {}",
            run.said()
        );
    }
}

#[test]
fn a_missing_required_argument_is_a_command_failure() {
    // Exit 2: the command could not run at all. Distinct from a refusal, which means it ran
    // and the answer was no.
    assert_eq!(Run::of(&["export"]).code, 2);
    assert_eq!(Run::of(&["inspect"]).code, 2);
    assert_eq!(Run::of(&["import"]).code, 2);
    assert_eq!(Run::of(&["export", "--project", "x.evaraproj"]).code, 2);
}

// ---------------------------------------------------------------------------------------
// export
// ---------------------------------------------------------------------------------------

#[test]
fn exporting_a_realistic_project_writes_an_archive_and_says_what_it_carried() {
    let fixture = Fixture::new();
    let archive = fixture.at("northgate.evara");

    let run = Run::of(&[
        "export",
        "--project",
        &text(&fixture.project),
        "--to",
        &text(&archive),
    ]);

    assert_eq!(run.code, 0, "{}", run.said());
    assert!(archive.is_file());
    assert!(run.stdout.contains("Exported"), "{}", run.said());
    assert!(run.stdout.contains("project"), "{}", run.said());
    assert!(run.stdout.contains("schema"), "{}", run.said());
    assert!(
        run.stdout.contains("The project was not modified."),
        "an export must say it left the source alone: {}",
        run.said()
    );
    // Progress went to stderr, so stdout is the summary and only the summary.
    assert!(
        run.stderr.contains("snapshotting") || run.stderr.contains("compressing"),
        "progress belongs on stderr: {}",
        run.said()
    );
}

#[test]
fn quiet_suppresses_progress_but_not_the_summary() {
    let fixture = Fixture::new();
    let archive = fixture.at("quiet.evara");
    let run = Run::of(&[
        "export",
        "--project",
        &text(&fixture.project),
        "--to",
        &text(&archive),
        "--quiet",
    ]);
    assert_eq!(run.code, 0, "{}", run.said());
    assert!(run.stdout.contains("Exported"));
    assert!(
        !run.stderr.contains("snapshotting"),
        "--quiet must silence progress: {}",
        run.said()
    );
}

#[test]
fn exporting_over_an_existing_archive_is_refused_and_leaves_it_alone() {
    let fixture = Fixture::new();
    let archive = fixture.export_to("once.evara");
    let before = std::fs::read(&archive).expect("read the archive");

    let run = Run::of(&[
        "export",
        "--project",
        &text(&fixture.project),
        "--to",
        &text(&archive),
    ]);

    assert_eq!(
        run.code,
        1,
        "an occupied destination is a refusal: {}",
        run.said()
    );
    assert!(
        run.stderr.contains("already exists"),
        "and says so: {}",
        run.said()
    );
    assert_eq!(
        std::fs::read(&archive).expect("read again"),
        before,
        "the existing archive must be untouched"
    );
}

#[test]
fn exporting_a_project_that_is_not_there_fails() {
    let fixture = Fixture::new();
    let run = Run::of(&[
        "export",
        "--project",
        &text(&fixture.at("absent.evaraproj")),
        "--to",
        &text(&fixture.at("out.evara")),
    ]);
    assert_eq!(run.code, 2, "{}", run.said());
    assert!(!fixture.at("out.evara").exists());
}

#[test]
fn an_export_leaves_the_source_and_its_directory_clean() {
    let fixture = Fixture::new();
    let before = std::fs::read(&fixture.project).expect("read the source");
    fixture.export_to("clean.evara");

    assert_eq!(
        std::fs::read(&fixture.project).expect("read again"),
        before,
        "the source project must be byte-for-byte unchanged"
    );
    for suffix in ["-wal", "-shm"] {
        assert!(
            !PathBuf::from(format!("{}{suffix}", fixture.project.display())).exists(),
            "an export must not leave a {suffix} beside the project"
        );
    }
    let strays: Vec<String> = std::fs::read_dir(fixture.dir.path())
        .expect("list")
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name.starts_with(".evara-export-"))
        .collect();
    assert!(strays.is_empty(), "staging left behind: {strays:?}");
}

// ---------------------------------------------------------------------------------------
// inspect
// ---------------------------------------------------------------------------------------

#[test]
fn inspecting_an_archive_reports_its_manifest_and_writes_nothing() {
    let fixture = Fixture::new();
    let archive = fixture.export_to("to-inspect.evara");

    let before: Vec<_> = std::fs::read_dir(fixture.dir.path())
        .expect("list")
        .filter_map(Result::ok)
        .map(|entry| entry.file_name())
        .collect();

    let run = Run::of(&["inspect", &text(&archive)]);

    assert_eq!(run.code, 0, "{}", run.said());
    assert!(run.stdout.contains("evara-project v1"), "{}", run.said());
    assert!(run.stdout.contains("schema"), "{}", run.said());
    assert!(run.stdout.contains("change feed"), "{}", run.said());
    assert!(
        run.stdout.contains("This build can read the archive"),
        "{}",
        run.said()
    );
    // The claim a user cannot check for themselves, at the point they are deciding to trust
    // the file.
    assert!(
        run.stdout.contains("corruption, not authenticity"),
        "inspect must not let a digest be read as a signature: {}",
        run.said()
    );

    let after: Vec<_> = std::fs::read_dir(fixture.dir.path())
        .expect("list")
        .filter_map(Result::ok)
        .map(|entry| entry.file_name())
        .collect();
    assert_eq!(
        before.len(),
        after.len(),
        "inspect must create nothing: {before:?} became {after:?}"
    );
}

#[test]
fn inspecting_bytes_that_are_not_an_archive_is_refused() {
    let fixture = Fixture::new();
    let nonsense = fixture.at("nonsense.evara");
    std::fs::write(&nonsense, b"not a zip").expect("write");

    let run = Run::of(&["inspect", &text(&nonsense)]);
    assert_eq!(
        run.code,
        1,
        "a damaged archive is a reasoned no, not a crash: {}",
        run.said()
    );
}

#[test]
fn inspecting_does_not_read_the_payload_and_says_so() {
    // A property worth pinning, because the obvious expectation is the wrong one. `inspect`
    // verifies the manifest and the digest record; **it does not stream the payload** — that
    // is step 9 of the verification order and belongs to `import`, which is what lets
    // `inspect` stage nothing and read almost nothing.
    //
    // So a payload-only tamper is *invisible* here and is caught at import. That is correct,
    // and it is also exactly the kind of thing a user could misread as "this archive is
    // fine", so the command's own output has to say what its answer covers. If this test ever
    // fails because the code became 1, the fix is **not** to update the number: it would mean
    // `inspect` had started reading payloads, which is a different command with a different
    // cost.
    let fixture = Fixture::new();
    let archive = fixture.export_to("tamper.evara");

    let mut bytes = std::fs::read(&archive).expect("read");
    let midpoint = bytes.len() / 2;
    bytes[midpoint] ^= 0xFF;
    let tampered = fixture.at("tampered.evara");
    std::fs::write(&tampered, &bytes).expect("write");

    let run = Run::of(&["inspect", &text(&tampered)]);
    assert_eq!(
        run.code,
        0,
        "inspect reads the container, not the payload: {}",
        run.said()
    );
    assert!(
        run.stdout.contains("verified") && run.stdout.contains("evara import"),
        "and it must say where the payload actually gets checked: {}",
        run.said()
    );
}

#[test]
fn inspecting_a_file_that_is_not_there_is_refused_or_fails_but_never_panics() {
    let fixture = Fixture::new();
    let run = Run::of(&["inspect", &text(&fixture.at("absent.evara"))]);
    assert!(
        run.code == 1 || run.code == 2,
        "a missing archive must be reported, not panicked: {}",
        run.said()
    );
    assert!(
        !run.stderr.contains("panicked"),
        "nothing here may panic: {}",
        run.said()
    );
}

// ---------------------------------------------------------------------------------------
// the round trip, as far as a spawned binary can take it
// ---------------------------------------------------------------------------------------

#[test]
fn an_exported_archive_is_readable_by_the_library_the_importer_uses() {
    // The last step a subprocess test can take without opening the installation's settings
    // database: the archive the real binary wrote is accepted by the real reader, by name,
    // with the identity preserved. The import itself is `src/transfer/tests.rs`.
    let fixture = Fixture::new();
    let source = evara_db::transfer::facts_of(&fixture.project).expect("read the source facts");
    let archive = fixture.export_to("round-trip.evara");

    let summary = evara_project::inspect(&archive).expect("the reader accepts it");
    assert!(summary.readable.is_readable());
    assert_eq!(
        summary.manifest.project_id, source.identity.project_id,
        "the archive carries the project's own identity"
    );
    assert_eq!(
        summary.manifest.payload.schema_version, source.schema_version,
        "and the schema it was written at"
    );
    assert_eq!(
        summary.manifest.counts, source.census,
        "and the manifest describes every table the project holds"
    );
}
