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
    let (mut files, mut errors, mut required) = entities();
    for (more_files, more_errors, more_required) in [
        lessons(),
        time_model_and_interface(),
        spreadsheet_import(),
        timetables(),
        repair(),
    ] {
        files.extend(more_files);
        errors.extend(more_errors);
        required.extend(more_required);
    }

    errors.extend(check_dependencies_are_listed(&files, &required));
    (files, errors)
}

/// What a school teaches, to whom, and how often: the Lessons group and its arithmetic.
///
/// Split out of [`entities`] when Stage C made that list outgrow one screen again, along
/// the same seam — the groups are `DATA-MODEL.md`'s, and this is §5's second half plus §6.
///
/// The four `evara-core` types at the foot are the demand arithmetic the views carry. They
/// are the interface's *answer*, not its calculation: working any of it out in TypeScript
/// would be a scheduling rule in the view layer.
fn lessons() -> (Vec<Generated>, Vec<String>, BTreeSet<(PathBuf, String)>) {
    use evara_core::lessons::{CycleCapacity, DeclaredComparison, DemandFinding, LessonDemand};
    use evara_db::{
        Activity, AssignedTeacher, AssignmentEligibility, AttendingGroup, ClassEnrolment,
        ClassEnrolmentInput, ClassGroup, ClassGroupInput, ClassSection, ClassSectionDependents,
        ClassSectionInput, ClassSectionView, ClassTeacher, ClassTeacherInput, Course,
        CourseDependents, CourseInput, CourseView, RosteredStudent, SectionDemand, TaughtSection,
        TeacherCandidate,
    };

    contract![
        // `subject -> course -> class_section -> activity`, with no `school_id` anywhere in
        // the chain; see ADR 0029 §1.
        Course,
        CourseInput,
        CourseView,
        CourseDependents,
        ClassSection,
        ClassSectionInput,
        ClassSectionView,
        ClassSectionDependents,
        // Staffing. `AssignmentEligibility` keeps ADR 0028's three answers on a named type,
        // so a screen cannot treat the warning as a refusal.
        ClassTeacher,
        ClassTeacherInput,
        AssignedTeacher,
        TeacherCandidate,
        AssignmentEligibility,
        TaughtSection,
        // The audience, and the optional roster that is never added to it.
        ClassGroup,
        ClassGroupInput,
        AttendingGroup,
        ClassEnrolment,
        ClassEnrolmentInput,
        RosteredStudent,
        // One placeable occurrence, and what a section's worth of them demands.
        Activity,
        SectionDemand,
        LessonDemand,
        DeclaredComparison,
        CycleCapacity,
        DemandFinding,
    ]
}

/// Stage D: the vocabulary a spreadsheet import is reviewed and applied in.
///
/// Its own group rather than an addition to [`time_model_and_interface`] because the whole
/// of it is one feature's contract, and because a reader looking for "what does the import
/// screen receive" should find one list rather than a dozen entries interleaved with the
/// grid's.
///
/// All but the four view types come from `evara_core::import`, which is where they have to
/// live: `evara-import` parses into them and `evara-db` plans with them, and a vocabulary
/// two crates share cannot sit in either. Note that `SourceCell` and `SourceRow` are
/// listed and `SourceTable` is not — the table is held in the backend session and never
/// crosses, but a cell is what a sample row is made of.
fn spreadsheet_import() -> (Vec<Generated>, Vec<String>, BTreeSet<(PathBuf, String)>) {
    use evara_core::import::{
        Choice, ColumnMapping, FieldSpec, ImportFingerprint, ImportKind, ImportPlan, ImportSummary,
        PlanCounts, PlannedRow, RowAction, RowDecision, RowFinding, Severity, SourceCell,
        SourceRow,
    };
    use evara_desktop_lib::commands::import::{
        ImportKindView, ImportSourceView, SampleRow, TemplateKindFormat,
    };

    contract![
        // What is being imported, and the columns it has.
        ImportKind,
        FieldSpec,
        ColumnMapping,
        // What a file turned out to hold.
        SourceCell,
        SourceRow,
        // The plan, which is the whole of what a user reviews.
        ImportPlan,
        PlannedRow,
        PlanCounts,
        RowAction,
        RowFinding,
        Severity,
        ImportFingerprint,
        // What the user decides about a row the planner would not decide alone.
        RowDecision,
        Choice,
        // What happened, once it had.
        ImportSummary,
        // Interface contracts defined with the command handlers.
        ImportKindView,
        ImportSourceView,
        SampleRow,
        TemplateKindFormat,
    ]
}

/// What a project holds: Structure, People, shared identities, Curriculum, availability.
///
/// Split from [`time_model_and_interface`] because the list had outgrown one screen, and
/// the seam is the one a reader already uses — the groups are `DATA-MODEL.md`'s.
fn entities() -> (Vec<Generated>, Vec<String>, BTreeSet<(PathBuf, String)>) {
    use evara_db::{
        AcademicYear, AcademicYearInput, AvailabilityEntity, AvailabilityRule,
        AvailabilityRuleInput, AvailabilityStatus, Building, BuildingInput, BuildingWithCampuses,
        Campus, CampusInput, Department, DepartmentDependents, DepartmentInput, EnrolmentStatus,
        GroupKind, QualifiedSubject, QualifiedTeacher, Resource, ResourceInput, Room, RoomInput,
        RoomType, RoomTypeInput, RoomWithCampuses, School, SchoolDependents, SchoolInput, Student,
        StudentEnrolment, StudentEnrolmentInput, StudentGroup, StudentGroupInput,
        StudentGroupMember, StudentGroupMemberInput, StudentInSchool, StudentInput, Subject,
        SubjectDependents, SubjectInput, Teacher, TeacherCampus, TeacherCampusInput,
        TeacherDepartment, TeacherDepartmentInput, TeacherDepartmentView, TeacherInSchool,
        TeacherInput, TeacherQualification, TeacherQualificationInput, TeacherSchool,
        TeacherSchoolInput, Term, TermInput, YearLevel, YearLevelInput,
    };

    contract![
        // Static structure.
        School,
        SchoolInput,
        SchoolDependents,
        Campus,
        CampusInput,
        Building,
        BuildingInput,
        BuildingWithCampuses,
        RoomType,
        RoomTypeInput,
        Room,
        RoomInput,
        RoomWithCampuses,
        Resource,
        ResourceInput,
        AcademicYear,
        AcademicYearInput,
        Term,
        TermInput,
        // People.
        YearLevel,
        YearLevelInput,
        Teacher,
        TeacherInput,
        Student,
        StudentInput,
        StudentGroup,
        StudentGroupInput,
        GroupKind,
        StudentGroupMember,
        StudentGroupMemberInput,
        // Shared identities — Stage 0B.
        TeacherSchool,
        TeacherSchoolInput,
        TeacherInSchool,
        TeacherCampus,
        TeacherCampusInput,
        StudentEnrolment,
        StudentEnrolmentInput,
        StudentInSchool,
        EnrolmentStatus,
        // Curriculum prerequisites — Stage B.
        Department,
        DepartmentInput,
        DepartmentDependents,
        Subject,
        SubjectInput,
        SubjectDependents,
        TeacherQualification,
        TeacherQualificationInput,
        QualifiedSubject,
        QualifiedTeacher,
        TeacherDepartment,
        TeacherDepartmentInput,
        TeacherDepartmentView,
        // Availability.
        AvailabilityRule,
        AvailabilityRuleInput,
        AvailabilityEntity,
        AvailabilityStatus,
    ]
}

/// When it happens, and the few types the command handlers define themselves.
fn time_model_and_interface() -> (Vec<Generated>, Vec<String>, BTreeSet<(PathBuf, String)>) {
    use evara_core::time::{CycleCoverage, CycleRotation};
    use evara_db::{
        CalendarDay, CalendarDayInput, CalendarDayKind, Cycle, CycleDay, CycleDayInput, CycleInput,
        CycleKind, CycleShape, GridCell, GridPreview, MaterialisationCounts, MaterialisationPlan,
        MaterialisationRequest, NonWorkingInstructionalDay, Period, PeriodInput, PeriodKind,
        PeriodStructure, PeriodStructureInput, PlanFingerprint, PlannedSlot, Timeslot,
        TimeslotInput, WeeklyCycleDay,
    };
    use evara_desktop_lib::commands::AppInfo;
    use evara_desktop_lib::commands::error::SetupError;
    use evara_desktop_lib::commands::structure::{LogoLimits, SchoolLogoView};

    contract![
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
        CycleKind,
        CycleCoverage,
        CycleRotation,
        CycleShape,
        WeeklyCycleDay,
        NonWorkingInstructionalDay,
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
        // Binary is the one exception to "records are the IPC contract" — see
        // `SchoolLogoView`. `SchoolLogo` itself is deliberately absent: its `Vec<u8>` has
        // no business crossing as JSON.
        SchoolLogoView,
        LogoLimits,
    ]
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

/// Generating, viewing and reporting a timetable: M8's surface.
///
/// # What is deliberately absent, and why
///
/// `Problem`, `Assignment`, `Placement` and every `*Ix` dense index. An index has meaning
/// only inside the one problem that assigned it, so a TypeScript value holding one would
/// invite arithmetic that is correct for one load and wrong for the next. Everything here is
/// addressed by **stored identifier**, and every span is resolved in Rust before it crosses
/// — which is also what keeps `CLAUDE.md` §2's "no scheduling rule in the view layer" true
/// for a grid that has to render multi-period lessons.
///
/// `TimetableProjection` is the largest type here and is sent **once** per opened timetable;
/// the scope grids address its `lessons` by index, so no occurrence is serialised twice
/// however many grids it appears in.
fn timetables() -> (Vec<Generated>, Vec<String>, BTreeSet<(PathBuf, String)>) {
    use evara_core::scheduling::UnverifiedCounts;
    use evara_core::scheduling::display::{LabelSet, RoomLabel, SectionLabel, StudentLabel};
    use evara_core::scheduling::span::SpanDefect;
    use evara_desktop_lib::commands::edit::{
        CandidateCell, CandidateGrid, CandidateRoom, EditHistoryView, EditLesson, EditStateView,
        EditUnplaced, MoveVerdictView, ProposedMove, ScoreView,
    };
    use evara_desktop_lib::commands::session::SessionPhase;
    use evara_desktop_lib::commands::solve::{
        FindingLine, FinishedEvent, GenerateResult, GenerateSettings, GenerateStarted,
        GenerateStatus, ProgressEvent, SavedView, ValidationView,
    };
    use evara_desktop_lib::commands::views::{
        DeletedView, OutcomeView, RunView, ScoringView, SharedEntity, TimetableSummary,
        TimetableView, UnscheduledView,
    };
    use evara_report::findings::{FindingCounts, FindingEntity, FindingSource, FindingView};
    use evara_report::projection::{
        CellContent, GridConflict, GridPanel, LessonFlags, PinReason, ProjectedCellRef,
        ProjectedDay, ProjectedLesson, ProjectedPeriod, ProjectionScope, ScheduleAxis, ScopeGrid,
        ScopeKind, TimetableCell, TimetableProjection,
    };

    contract![
        // The display vocabulary. `evara-db` populates it, `evara-report` consumes it.
        LabelSet,
        SectionLabel,
        RoomLabel,
        StudentLabel,
        // The projection: axes, lessons, scopes and the laid-out cells.
        TimetableProjection,
        ScheduleAxis,
        ProjectedDay,
        ProjectedPeriod,
        ProjectedLesson,
        ProjectedCellRef,
        LessonFlags,
        SpanDefect,
        ProjectionScope,
        ScopeKind,
        ScopeGrid,
        GridPanel,
        TimetableCell,
        CellContent,
        PinReason,
        GridConflict,
        // Findings, at either fidelity.
        FindingView,
        FindingEntity,
        FindingSource,
        FindingCounts,
        // Generation orchestration.
        ValidationView,
        FindingLine,
        GenerateSettings,
        GenerateStarted,
        GenerateStatus,
        GenerateResult,
        SavedView,
        SessionPhase,
        ProgressEvent,
        FinishedEvent,
        // Saved timetables.
        TimetableSummary,
        TimetableView,
        ScoringView,
        UnscheduledView,
        SharedEntity,
        OutcomeView,
        RunView,
        DeletedView,
        // Manual editing — M9A. Every verdict in here is the registry's; the interface
        // renders them and computes none.
        EditStateView,
        EditLesson,
        EditUnplaced,
        EditHistoryView,
        ScoreView,
        ProposedMove,
        MoveVerdictView,
        CandidateGrid,
        CandidateCell,
        CandidateRoom,
        // Shared with the solver's own vocabulary: never omitted from a timetable's
        // description, so never omitted from its type.
        UnverifiedCounts,
    ]
}

/// Automated repair and partial regeneration — Phase 8B.
///
/// Its own group because the whole of M9B crosses the boundary together: the scope the user
/// chose, the pre-flight that may refuse it, the session's phase, and the diff they approve
/// before anything is written.
///
/// Note what is **not** here. `MovableSet` stays in Rust: it is dense activity indices, which
/// are meaningful only inside the one problem that assigned them — the same reason the grid
/// projection addresses everything by stored identifier. And `Score` does not cross either;
/// [`ScoreLine`] is the flat three-integer view, so the interface never has to reimplement a
/// lexicographic comparison to tell whether a repair improved anything.
fn repair() -> (Vec<Generated>, Vec<String>, BTreeSet<(PathBuf, String)>) {
    // Fully qualified rather than imported: `contract!` names each type inside a macro
    // expansion, which clippy's unused-import analysis cannot see through.
    use evara_desktop_lib::commands::repair as cmd;
    use evara_report::repair as view;

    contract![
        evara_core::scheduling::RepairScope,
        cmd::RepairScopes,
        cmd::ScopeOption,
        cmd::RepairPreflight,
        cmd::PreflightFinding,
        cmd::RepairStarted,
        cmd::RepairStatus,
        evara_desktop_lib::commands::repair_session::RepairPhase,
        cmd::RepairResult,
        cmd::RepairApplied,
        cmd::RepairProgressEvent,
        cmd::RepairFinishedEvent,
        // The diff, which is a projection in the same sense the grid is.
        view::RepairDiff,
        view::ScoreLine,
        view::OccurrenceChange,
        view::ChangeKind,
        view::ChangeCounts,
        view::MovedParts,
        view::PlacementView,
        view::Attribution,
    ]
}
