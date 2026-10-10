//! Phase 1E tests: what School Setup needs from the repository that Phase 1D did not
//! provide.
//!
//! Three things, and the middle one is the reason this file exists:
//!
//! 1. **Cycle coverage** — whether a cycle is finished, and which positions are missing.
//!    The rule is in `evara-core`; these tests pin down the query that feeds it and the
//!    fact that an unknown cycle is reported rather than returned as an empty coverage.
//! 2. **The preview/confirmation contract** — a confirmation is only honoured if the plan
//!    it was issued against is still the current plan. `apply_materialisation` was already
//!    safe in the sense that it cannot act on tuples that no longer exist; it was not safe
//!    in the sense of applying *what the user agreed to*, and for the destructive path
//!    that difference can delete a slot which has come back into use.
//! 3. **The grid preview projection** — the axes, the existing and missing cells, and the
//!    scoping that keeps two grids over one cycle from reporting each other's slots.
//!
//! Real temporary project files throughout, never `:memory:`. Synthetic school data only.

use evara_core::time::CycleCoverage;
use uuid::Uuid;

use crate::DbError;
use crate::repo::time_model::{MaterialisationRequest, PeriodKind, PlanFingerprint};
use crate::tests_time_model::Project;

// ===================================================================== cycle coverage

#[test]
fn a_full_cycle_reports_itself_complete() {
    let mut project = Project::new();
    let cycle = project.cycle("Five-day week", 5);
    for ordinal in 1..=5 {
        project.day(cycle, ordinal, &format!("Day {ordinal}"));
    }

    let coverage = project
        .db()
        .time_model()
        .cycle_coverage(cycle)
        .expect("coverage");

    assert!(coverage.is_complete());
    assert_eq!(coverage.present, 5);
    assert_eq!(coverage.declared_day_count, 5);
    assert_eq!(coverage.missing_ordinals, Vec::<i64>::new());
}

#[test]
fn a_half_built_cycle_names_the_positions_with_no_day() {
    let mut project = Project::new();
    let cycle = project.cycle("Six-day rotation", 6);
    project.day(cycle, 1, "Day A");
    project.day(cycle, 2, "Day B");
    project.day(cycle, 5, "Day E");

    let coverage = project
        .db()
        .time_model()
        .cycle_coverage(cycle)
        .expect("coverage");

    assert!(
        !coverage.is_complete(),
        "a cycle with gaps is legal but not finished"
    );
    assert_eq!(coverage.missing_ordinals, vec![3, 4, 6]);
    assert_eq!(coverage.present, 3);
}

#[test]
fn coverage_of_a_ten_day_fortnight_is_not_a_special_case() {
    let mut project = Project::new();
    let cycle = project.cycle("Ten-day fortnight", 10);
    for ordinal in 1..=10 {
        project.day(cycle, ordinal, &format!("Day {ordinal}"));
    }

    let coverage = project
        .db()
        .time_model()
        .cycle_coverage(cycle)
        .expect("coverage");

    assert!(coverage.is_complete());
    assert_eq!(coverage.declared_day_count, 10);
}

#[test]
fn coverage_of_a_one_day_cycle_is_complete_with_one_day() {
    let mut project = Project::new();
    let cycle = project.cycle("Every day the same", 1);
    project.day(cycle, 1, "Every day");

    let coverage = project
        .db()
        .time_model()
        .cycle_coverage(cycle)
        .expect("coverage");

    assert!(coverage.is_complete());
    assert_eq!(coverage.present, 1);
}

#[test]
fn coverage_of_an_unknown_cycle_is_not_found_rather_than_empty() {
    let mut project = Project::new();
    let error = project
        .db()
        .time_model()
        .cycle_coverage(Uuid::now_v7())
        .expect_err("an unknown cycle must be reported");

    assert!(
        matches!(error, DbError::NotFound { entity: "cycle" }),
        "an empty coverage would read as a legitimately empty cycle: {error:?}"
    );
}

#[test]
fn coverage_survives_closing_and_reopening_the_project() {
    let mut project = Project::new();
    let cycle = project.cycle("Six-day rotation", 6);
    project.day(cycle, 1, "Day A");
    project.day(cycle, 2, "Day B");

    let before = project
        .db()
        .time_model()
        .cycle_coverage(cycle)
        .expect("coverage");
    project.reopen();
    let after = project
        .db()
        .time_model()
        .cycle_coverage(cycle)
        .expect("coverage");

    assert_eq!(before, after);
    assert_eq!(after.missing_ordinals, vec![3, 4, 5, 6]);
}

// =========================================================== the plan fingerprint

#[test]
fn the_same_configuration_fingerprints_the_same_way_twice() {
    let mut project = Project::new();
    let (cycle, structure, _, _) = project.six_day_grid();
    let request = MaterialisationRequest::new(cycle, structure, None);

    let first = project
        .db()
        .time_model()
        .plan_materialisation(&request)
        .expect("plan")
        .fingerprint();
    let second = project
        .db()
        .time_model()
        .plan_materialisation(&request)
        .expect("plan")
        .fingerprint();

    assert_eq!(
        first, second,
        "a fingerprint that is not stable would demand a re-review on every confirmation"
    );
}

#[test]
fn adding_a_period_changes_the_fingerprint() {
    let mut project = Project::new();
    let (cycle, structure, _, _) = project.six_day_grid();
    let request = MaterialisationRequest::new(cycle, structure, None);

    let before = project
        .db()
        .time_model()
        .plan_materialisation(&request)
        .expect("plan")
        .fingerprint();

    project.period(structure, 5, "4", PeriodKind::Teaching);

    let after = project
        .db()
        .time_model()
        .plan_materialisation(&request)
        .expect("plan")
        .fingerprint();

    assert_ne!(
        before, after,
        "six more cells to create is exactly the kind of change a user must re-approve"
    );
}

#[test]
fn renaming_a_cycle_day_does_not_change_the_fingerprint() {
    let mut project = Project::new();
    let (cycle, structure, days, _) = project.six_day_grid();
    let request = MaterialisationRequest::new(cycle, structure, None);

    let before = project
        .db()
        .time_model()
        .plan_materialisation(&request)
        .expect("plan")
        .fingerprint();

    project.relabel_day(days[0], "Monday");

    let after = project
        .db()
        .time_model()
        .plan_materialisation(&request)
        .expect("plan")
        .fingerprint();

    assert_eq!(
        before, after,
        "a label is display only: no slot's fate changed, so nothing needs re-approving"
    );
}

#[test]
fn an_applied_grid_fingerprints_differently_from_an_unbuilt_one() {
    let mut project = Project::new();
    let (cycle, structure, _, _) = project.six_day_grid();
    let request = MaterialisationRequest::new(cycle, structure, None);

    let unbuilt = project
        .db()
        .time_model()
        .plan_materialisation(&request)
        .expect("plan")
        .fingerprint();

    project
        .db()
        .time_model()
        .apply_materialisation(&request)
        .expect("apply");

    let built = project
        .db()
        .time_model()
        .plan_materialisation(&request)
        .expect("plan")
        .fingerprint();

    assert_ne!(
        unbuilt, built,
        "the same cells are now preserved rather than created"
    );
}

// ================================================== reviewed apply: the staleness gate

#[test]
fn a_reviewed_rebuild_applies_when_the_plan_has_not_moved() {
    let mut project = Project::new();
    let (cycle, structure, _, _) = project.six_day_grid();
    let request = MaterialisationRequest::new(cycle, structure, None);

    let reviewed = project
        .db()
        .time_model()
        .plan_materialisation(&request)
        .expect("plan")
        .fingerprint();

    let plan = project
        .db()
        .time_model()
        .apply_materialisation_reviewed(&request, &reviewed)
        .expect("the plan is current, so it applies");

    assert_eq!(plan.created.len(), 24, "six days by four periods");
    assert_eq!(project.slots(cycle, None).len(), 24);
}

#[test]
fn a_reviewed_rebuild_is_refused_when_the_plan_has_moved_and_writes_nothing() {
    let mut project = Project::new();
    let (cycle, structure, _, _) = project.six_day_grid();
    let request = MaterialisationRequest::new(cycle, structure, None);

    let reviewed = project
        .db()
        .time_model()
        .plan_materialisation(&request)
        .expect("plan")
        .fingerprint();

    // Exactly the situation the gate exists for: someone changes the bell schedule
    // between the preview and the confirmation.
    project.period(structure, 5, "4", PeriodKind::Teaching);

    let error = project
        .db()
        .time_model()
        .apply_materialisation_reviewed(&request, &reviewed)
        .expect_err("a stale confirmation must be refused");

    assert!(
        matches!(error, DbError::ReviewRequired { .. }),
        "the interface has to tell these apart from a bad value: {error:?}"
    );
    assert!(
        project.slots(cycle, None).is_empty(),
        "a refused confirmation must leave the grid exactly as it was"
    );
}

#[test]
fn a_refused_rebuild_names_both_fingerprints_for_diagnosis() {
    let mut project = Project::new();
    let (cycle, structure, _, _) = project.six_day_grid();
    let request = MaterialisationRequest::new(cycle, structure, None);
    let reviewed = PlanFingerprint::from_str_for_test("0000000000000000");

    let error = project
        .db()
        .time_model()
        .apply_materialisation_reviewed(&request, &reviewed)
        .expect_err("a fabricated fingerprint cannot match");

    match error {
        DbError::ReviewRequired { reviewed, current } => {
            assert_eq!(reviewed, "0000000000000000");
            assert_ne!(current, reviewed);
        }
        other => panic!("expected ReviewRequired, got {other:?}"),
    }
}

#[test]
fn re_reading_the_plan_makes_a_stale_confirmation_work() {
    let mut project = Project::new();
    let (cycle, structure, _, _) = project.six_day_grid();
    let request = MaterialisationRequest::new(cycle, structure, None);

    let stale = project
        .db()
        .time_model()
        .plan_materialisation(&request)
        .expect("plan")
        .fingerprint();
    project.period(structure, 5, "4", PeriodKind::Teaching);

    assert!(
        project
            .db()
            .time_model()
            .apply_materialisation_reviewed(&request, &stale)
            .is_err()
    );

    // What the interface does on `reviewRequired`: re-read, show the new figures, confirm.
    let fresh = project
        .db()
        .time_model()
        .plan_materialisation(&request)
        .expect("plan")
        .fingerprint();
    let plan = project
        .db()
        .time_model()
        .apply_materialisation_reviewed(&request, &fresh)
        .expect("the re-read plan applies");

    assert_eq!(plan.created.len(), 30, "six days by five periods");
}

#[test]
fn a_reviewed_rebuild_preserves_every_identifier() {
    let mut project = Project::new();
    let (cycle, structure, _, _) = project.six_day_grid();
    let request = MaterialisationRequest::new(cycle, structure, None);

    project
        .db()
        .time_model()
        .apply_materialisation(&request)
        .expect("first build");
    let before = project.slots_by_tuple(cycle, None);

    project.period(structure, 5, "4", PeriodKind::Teaching);
    let reviewed = project
        .db()
        .time_model()
        .plan_materialisation(&request)
        .expect("plan")
        .fingerprint();
    project
        .db()
        .time_model()
        .apply_materialisation_reviewed(&request, &reviewed)
        .expect("rebuild");

    let after = project.slots_by_tuple(cycle, None);
    for (tuple, id) in &before {
        assert_eq!(
            after.get(tuple),
            Some(id),
            "the identifier of a surviving slot is what a future lesson points at"
        );
    }
    assert_eq!(after.len(), 30);
}

// ============================================= releasing orphans: the destructive path

#[test]
fn releasing_orphans_deletes_exactly_what_the_reviewed_plan_named() {
    let mut project = Project::new();
    let (cycle, structure, days, _) = project.six_day_grid();
    let request = MaterialisationRequest::new(cycle, structure, None);
    project
        .db()
        .time_model()
        .apply_materialisation(&request)
        .expect("build");
    assert_eq!(project.slots(cycle, None).len(), 24);

    // Orphan-hood is a property of a proposal: ask what the grid would be without Day F.
    let proposal = MaterialisationRequest::new(cycle, structure, None).without_cycle_day(days[5]);
    let plan = project
        .db()
        .time_model()
        .plan_materialisation(&proposal)
        .expect("plan");
    assert_eq!(plan.orphaned.len(), 4, "Day F has four slots");

    let removed = project
        .db()
        .time_model()
        .release_orphans(&proposal, &plan.fingerprint())
        .expect("release");

    assert_eq!(removed.len(), 4);
    assert_eq!(project.slots(cycle, None).len(), 20);
}

#[test]
fn releasing_orphans_is_refused_when_the_plan_has_moved_and_deletes_nothing() {
    let mut project = Project::new();
    let (cycle, structure, days, _) = project.six_day_grid();
    let request = MaterialisationRequest::new(cycle, structure, None);
    project
        .db()
        .time_model()
        .apply_materialisation(&request)
        .expect("build");

    let proposal = MaterialisationRequest::new(cycle, structure, None).without_cycle_day(days[5]);
    let reviewed = project
        .db()
        .time_model()
        .plan_materialisation(&proposal)
        .expect("plan")
        .fingerprint();

    // The grid moves underneath the confirmation.
    project.period(structure, 5, "4", PeriodKind::Teaching);

    let error = project
        .db()
        .time_model()
        .release_orphans(&proposal, &reviewed)
        .expect_err("a stale destructive confirmation must be refused");

    assert!(matches!(error, DbError::ReviewRequired { .. }), "{error:?}");
    assert_eq!(
        project.slots(cycle, None).len(),
        24,
        "nothing may be deleted on the refusal path"
    );
}

/// The hole this guard exists to close.
///
/// A list of orphan identifiers captured when the screen was drawn can name slots that
/// have since come back into use. `delete_timeslots` cannot tell — ADR 0011 §4 is explicit
/// that orphan-hood is a property of a proposal, not of a row — so handed that stale list
/// it deletes wanted slots. `release_orphans` refuses instead.
#[test]
fn a_stale_orphan_list_cannot_delete_a_slot_that_is_wanted_again() {
    let mut project = Project::new();
    let (cycle, structure, days, periods) = project.six_day_grid();
    let request = MaterialisationRequest::new(cycle, structure, None);
    project
        .db()
        .time_model()
        .apply_materialisation(&request)
        .expect("build");

    // The user previews "without the break period" and walks away.
    let proposal = MaterialisationRequest::new(cycle, structure, None).without_period(periods[2]);
    let plan = project
        .db()
        .time_model()
        .plan_materialisation(&proposal)
        .expect("plan");
    let stale = plan.fingerprint();
    let would_have_deleted = plan.orphaned_ids();
    assert_eq!(would_have_deleted.len(), 6, "one per cycle day");

    // Meanwhile the configuration changes, so the proposal describes something else.
    project.period(structure, 5, "4", PeriodKind::Teaching);

    let error = project
        .db()
        .time_model()
        .release_orphans(&proposal, &stale)
        .expect_err("the stale agreement must not be honoured");
    assert!(matches!(error, DbError::ReviewRequired { .. }), "{error:?}");

    // The unguarded call is what the guard protects against: it would have gone ahead.
    let still_there = project.slots_by_tuple(cycle, None);
    for id in &would_have_deleted {
        assert!(
            still_there.values().any(|existing| existing == id),
            "slot {id} must still be present after a refused release"
        );
    }
    assert!(
        days.len() == 6,
        "the six-day cycle is unchanged by this test's edits"
    );
}

#[test]
fn releasing_nothing_is_a_success_not_a_failure() {
    let mut project = Project::new();
    let (cycle, structure, _, _) = project.six_day_grid();
    let request = MaterialisationRequest::new(cycle, structure, None);
    project
        .db()
        .time_model()
        .apply_materialisation(&request)
        .expect("build");

    let plan = project
        .db()
        .time_model()
        .plan_materialisation(&request)
        .expect("plan");
    assert!(!plan.would_orphan());

    let removed = project
        .db()
        .time_model()
        .release_orphans(&request, &plan.fingerprint())
        .expect("a proposal that strands nothing is not an error");

    assert_eq!(removed, Vec::<Uuid>::new());
    assert_eq!(project.slots(cycle, None).len(), 24);
}

#[test]
fn releasing_orphans_is_what_finally_allows_the_day_to_be_deleted() {
    let mut project = Project::new();
    let (cycle, structure, days, _) = project.six_day_grid();
    let request = MaterialisationRequest::new(cycle, structure, None);
    project
        .db()
        .time_model()
        .apply_materialisation(&request)
        .expect("build");

    // The grid blocks the deletion, which is the whole reason the preview exists.
    let blocked = project
        .db()
        .time_model()
        .delete_cycle_day(days[5])
        .expect_err("a materialised grid holds the day");
    assert!(
        matches!(blocked, DbError::StillReferenced { .. }),
        "{blocked:?}"
    );

    let proposal = MaterialisationRequest::new(cycle, structure, None).without_cycle_day(days[5]);
    let plan = project
        .db()
        .time_model()
        .plan_materialisation(&proposal)
        .expect("plan");
    project
        .db()
        .time_model()
        .release_orphans(&proposal, &plan.fingerprint())
        .expect("release");

    project
        .db()
        .time_model()
        .delete_cycle_day(days[5])
        .expect("with its slots gone, the day can go");
}

#[test]
fn a_rebuild_never_deletes_even_when_it_would_strand_slots() {
    let mut project = Project::new();
    let (cycle, structure, days, _) = project.six_day_grid();
    let request = MaterialisationRequest::new(cycle, structure, None);
    project
        .db()
        .time_model()
        .apply_materialisation(&request)
        .expect("build");

    let proposal = MaterialisationRequest::new(cycle, structure, None).without_cycle_day(days[5]);
    let reviewed = project
        .db()
        .time_model()
        .plan_materialisation(&proposal)
        .expect("plan")
        .fingerprint();

    let plan = project
        .db()
        .time_model()
        .apply_materialisation_reviewed(&proposal, &reviewed)
        .expect("apply");

    assert_eq!(plan.orphaned.len(), 4, "reported");
    assert_eq!(
        project.slots(cycle, None).len(),
        24,
        "and left exactly where they were"
    );
}

// ====================================================================== grid preview

#[test]
fn a_preview_of_an_unbuilt_grid_reports_every_cell_missing() {
    let mut project = Project::new();
    let (cycle, structure, _, _) = project.six_day_grid();
    let request = MaterialisationRequest::new(cycle, structure, None);

    let preview = project
        .db()
        .time_model()
        .grid_preview(&request)
        .expect("preview");

    assert_eq!(preview.days.len(), 6);
    assert_eq!(preview.periods.len(), 4);
    assert_eq!(preview.cells.len(), 24);
    assert!(
        preview.cells.iter().all(|cell| cell.timeslot_id.is_none()),
        "nothing has been materialised yet"
    );
    assert_eq!(preview.counts.created, 24);
    assert_eq!(preview.counts.preserved, 0);
    assert!(!preview.is_complete());
}

#[test]
fn a_preview_of_a_built_grid_reports_every_cell_present() {
    let mut project = Project::new();
    let (cycle, structure, _, _) = project.six_day_grid();
    let request = MaterialisationRequest::new(cycle, structure, None);
    project
        .db()
        .time_model()
        .apply_materialisation(&request)
        .expect("build");

    let preview = project
        .db()
        .time_model()
        .grid_preview(&request)
        .expect("preview");

    assert!(preview.cells.iter().all(|cell| cell.timeslot_id.is_some()));
    assert_eq!(preview.counts.created, 0);
    assert_eq!(preview.counts.preserved, 24);
    assert!(
        preview.is_complete(),
        "a full grid over a complete cycle is finished"
    );
}

#[test]
fn a_preview_over_an_incomplete_cycle_is_never_complete() {
    let mut project = Project::new();
    let cycle = project.cycle("Six-day rotation", 6);
    let _ = project.day(cycle, 1, "Day A");
    let _ = project.day(cycle, 2, "Day B");
    let structure = project.structure("Standard bells", None);
    project.period(structure, 1, "1", PeriodKind::Teaching);

    let request = MaterialisationRequest::new(cycle, structure, None);
    project
        .db()
        .time_model()
        .apply_materialisation(&request)
        .expect("build");

    let preview = project
        .db()
        .time_model()
        .grid_preview(&request)
        .expect("preview");

    assert!(
        preview.cells.iter().all(|cell| cell.timeslot_id.is_some()),
        "every cell that exists has a slot"
    );
    assert!(
        !preview.is_complete(),
        "but the cycle is missing four of its six declared days, so the grid is not finished"
    );
    assert_eq!(preview.coverage.missing_ordinals, vec![3, 4, 5, 6]);
}

#[test]
fn a_preview_with_no_periods_is_not_complete_despite_missing_nothing() {
    let mut project = Project::new();
    let cycle = project.cycle("One-day cycle", 1);
    project.day(cycle, 1, "Every day");
    let structure = project.structure("Empty bells", None);

    let preview = project
        .db()
        .time_model()
        .grid_preview(&MaterialisationRequest::new(cycle, structure, None))
        .expect("preview");

    assert_eq!(preview.cells, Vec::new(), "no columns means no cells");
    assert!(
        !preview.is_complete(),
        "a grid with nothing in it must not report itself finished"
    );
}

#[test]
fn preview_cells_are_ordered_by_day_then_period() {
    let mut project = Project::new();
    let (cycle, structure, days, periods) = project.six_day_grid();

    let preview = project
        .db()
        .time_model()
        .grid_preview(&MaterialisationRequest::new(cycle, structure, None))
        .expect("preview");

    let expected: Vec<(Uuid, Uuid)> = days
        .iter()
        .flat_map(|day| periods.iter().map(move |period| (*day, *period)))
        .collect();
    let found: Vec<(Uuid, Uuid)> = preview
        .cells
        .iter()
        .map(|cell| (cell.cycle_day_id, cell.period_id))
        .collect();

    assert_eq!(
        found, expected,
        "the screen renders rows and columns straight from this order"
    );
}

#[test]
fn a_preview_marks_only_teaching_cells_as_teachable() {
    let mut project = Project::new();
    let (cycle, structure, _, periods) = project.six_day_grid();

    let preview = project
        .db()
        .time_model()
        .grid_preview(&MaterialisationRequest::new(cycle, structure, None))
        .expect("preview");

    let break_period = periods[2];
    for cell in &preview.cells {
        let expected = cell.period_id != break_period;
        assert_eq!(
            cell.is_teaching, expected,
            "a break must never read as schedulable, built or not"
        );
    }
}

#[test]
fn two_grids_over_one_cycle_do_not_see_each_others_slots() {
    let mut project = Project::new();
    let cycle = project.cycle("Five-day week", 5);
    for ordinal in 1..=5 {
        project.day(cycle, ordinal, &format!("Day {ordinal}"));
    }

    let campus = project.campus;
    let main = project.structure("Main bells", Some(campus));
    project.period(main, 1, "1", PeriodKind::Teaching);
    project.period(main, 2, "2", PeriodKind::Teaching);

    let annexe = project.structure("Annexe bells", None);
    project.period(annexe, 1, "A", PeriodKind::Teaching);

    project
        .db()
        .time_model()
        .apply_materialisation(&MaterialisationRequest::new(cycle, main, None))
        .expect("build main");

    let preview = project
        .db()
        .time_model()
        .grid_preview(&MaterialisationRequest::new(cycle, annexe, None))
        .expect("preview annexe");

    assert_eq!(preview.cells.len(), 5, "one period, five days");
    assert!(
        preview.cells.iter().all(|cell| cell.timeslot_id.is_none()),
        "the annexe grid is unbuilt; the main campus's ten slots are not its business"
    );
    assert_eq!(preview.counts.orphaned, 0, "and are not its orphans either");
}

#[test]
fn a_term_scoped_preview_is_a_different_grid_from_the_year_wide_one() {
    let mut project = Project::new();
    let (cycle, structure, _, _) = project.six_day_grid();
    let term = project.term;

    project
        .db()
        .time_model()
        .apply_materialisation(&MaterialisationRequest::new(cycle, structure, None))
        .expect("build year-wide");

    let scoped = project
        .db()
        .time_model()
        .grid_preview(&MaterialisationRequest::new(cycle, structure, Some(term)))
        .expect("preview term");

    assert_eq!(scoped.term_id, Some(term));
    assert!(
        scoped.cells.iter().all(|cell| cell.timeslot_id.is_none()),
        "the year-wide grid is not the autumn-term grid"
    );
    assert_eq!(scoped.counts.created, 24);
    assert_eq!(scoped.counts.orphaned, 0);
}

#[test]
fn a_preview_carries_the_fingerprint_of_the_plan_it_reports() {
    let mut project = Project::new();
    let (cycle, structure, _, _) = project.six_day_grid();
    let request = MaterialisationRequest::new(cycle, structure, None);

    let preview = project
        .db()
        .time_model()
        .grid_preview(&request)
        .expect("preview");

    // The screen confirms with the fingerprint the preview handed it, and nothing else.
    let plan = project
        .db()
        .time_model()
        .apply_materialisation_reviewed(&request, &preview.fingerprint)
        .expect("the preview's own fingerprint must be accepted");

    assert_eq!(plan.created.len(), preview.counts.created);
}

#[test]
fn a_preview_with_exclusions_reports_orphans_without_touching_anything() {
    let mut project = Project::new();
    let (cycle, structure, days, _) = project.six_day_grid();
    let request = MaterialisationRequest::new(cycle, structure, None);
    project
        .db()
        .time_model()
        .apply_materialisation(&request)
        .expect("build");

    let preview = project
        .db()
        .time_model()
        .grid_preview(
            &MaterialisationRequest::new(cycle, structure, None).without_cycle_day(days[0]),
        )
        .expect("preview");

    assert_eq!(preview.counts.orphaned, 4);
    assert_eq!(preview.orphaned.len(), 4);
    assert!(
        preview.orphaned.iter().all(|slot| slot.id.is_some()),
        "an orphan is an existing row, so it always has an identifier to show"
    );
    assert_eq!(
        project.slots(cycle, None).len(),
        24,
        "a preview writes nothing"
    );
}

#[test]
fn preview_survives_closing_and_reopening_the_project() {
    let mut project = Project::new();
    let (cycle, structure, _, _) = project.six_day_grid();
    let request = MaterialisationRequest::new(cycle, structure, None);
    project
        .db()
        .time_model()
        .apply_materialisation(&request)
        .expect("build");

    let before = project
        .db()
        .time_model()
        .grid_preview(&request)
        .expect("preview");
    project.reopen();
    let after = project
        .db()
        .time_model()
        .grid_preview(&request)
        .expect("preview");

    assert_eq!(
        before.cells, after.cells,
        "including every identifier: the grid is persisted, not in memory"
    );
    assert_eq!(before.fingerprint, after.fingerprint);
}

// ================================================= the workspace accessors the UI uses

#[test]
fn the_repositories_are_reachable_through_the_workspace() {
    let mut project = Project::new();
    let campuses = project
        .workspace
        .structure()
        .expect("a project is open")
        .campuses()
        .expect("read");
    assert_eq!(campuses.len(), 1, "the fixture's one campus");

    let cycles = project
        .workspace
        .time_model()
        .expect("a project is open")
        .cycles()
        .expect("read");
    assert_eq!(cycles, Vec::new());
}

#[test]
fn reaching_a_repository_with_nothing_open_is_reported_not_panicked() {
    let mut project = Project::new();
    project.workspace.close_project();

    let error = project
        .workspace
        .structure()
        .expect_err("no project is open");
    assert!(matches!(error, DbError::NoProjectOpen), "{error:?}");

    let error = project
        .workspace
        .time_model()
        .expect_err("no project is open");
    assert!(matches!(error, DbError::NoProjectOpen), "{error:?}");
}

// ==================================================== the wire vocabulary is one list

/// Guards the one duplicated list in the codebase.
///
/// `PeriodKind` and `CalendarDayKind` declare their stored spelling twice: once in
/// `stored_as!`, which the SQL uses, and once in `#[serde(rename = …)]`, which the JSON
/// and the generated TypeScript use. Keeping them in one macro was not possible without
/// generating the enums themselves, so drift is made a red test instead.
#[test]
fn kinds_match_their_stored_spelling() {
    use crate::repo::time_model::{CalendarDayKind, PeriodKind};

    for kind in PeriodKind::ALL {
        let json = serde_json::to_string(kind).expect("serialise");
        assert_eq!(
            json,
            format!("\"{}\"", kind.as_str()),
            "the wire spelling of {kind:?} must be the stored spelling"
        );
        let back: PeriodKind = serde_json::from_str(&json).expect("round trip");
        assert_eq!(back, *kind);
    }

    for kind in CalendarDayKind::ALL {
        let json = serde_json::to_string(kind).expect("serialise");
        assert_eq!(
            json,
            format!("\"{}\"", kind.as_str()),
            "the wire spelling of {kind:?} must be the stored spelling"
        );
        let back: CalendarDayKind = serde_json::from_str(&json).expect("round trip");
        assert_eq!(back, *kind);
    }
}

/// The records are the IPC contract (ADR 0012), so `camelCase` has to actually happen.
#[test]
fn records_serialise_as_camel_case_for_the_interface() {
    let mut project = Project::new();
    let campus = project
        .db()
        .structure()
        .campuses()
        .expect("read")
        .pop()
        .expect("the fixture's campus");

    let json = serde_json::to_string(&campus).expect("serialise");
    assert!(json.contains("\"schoolId\""), "{json}");
    assert!(json.contains("\"createdAt\""), "{json}");
    assert!(
        !json.contains("school_id"),
        "snake_case must not reach the frontend: {json}"
    );
}

/// An input arrives as JSON from the interface, so it has to deserialise from one.
#[test]
fn inputs_deserialise_from_the_camel_case_the_interface_sends() {
    use crate::repo::structure::RoomInput;

    let sent = r#"{
        "campusId": "018f3a2c-0000-7000-8000-000000000001",
        "buildingId": null,
        "roomTypeId": null,
        "name": "Lab 1",
        "code": "L1",
        "capacity": 24,
        "isBookable": true,
        "notes": null
    }"#;

    let input: RoomInput = serde_json::from_str(sent).expect("the frontend's payload must parse");
    assert_eq!(input.name, "Lab 1");
    assert_eq!(input.capacity, 24);
    assert!(input.is_bookable);
    assert_eq!(
        input.building_id, None,
        "a cleared optional reference must stay cleared"
    );
}

/// `CycleCoverage` crosses the boundary too, and its field names are read by the screen.
#[test]
fn coverage_serialises_as_camel_case() {
    let coverage = CycleCoverage::of(6, &[1, 2, 5]);
    let json = serde_json::to_string(&coverage).expect("serialise");
    assert!(json.contains("\"missingOrdinals\":[3,4,6]"), "{json}");
    assert!(json.contains("\"declaredDayCount\":6"), "{json}");
}
