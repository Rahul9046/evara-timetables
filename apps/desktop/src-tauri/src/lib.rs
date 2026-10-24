//! The Evara Timetables desktop application.
//!
//! # Layering contract
//!
//! This crate **orchestrates**; it does not decide. Command handlers translate an IPC
//! request into a call on one of the `evara-*` crates and translate the result back.
//! Business logic in a command handler is an architecture bug — it would be unreachable
//! from the headless CLI and from tests.
//!
//! Specifically, this crate must not:
//!
//! - contain a scheduling rule (those live in `evara-constraints`)
//! - contain SQL (that lives in `evara-db`)
//! - contain a solver heuristic (that lives in `evara-solver`)

pub mod commands;

use std::sync::Mutex;

use commands::project::AppState;

/// Builds and runs the application.
///
/// # Panics
///
/// Panics if the Tauri context cannot be initialised, which indicates a broken build
/// rather than a runtime condition worth recovering from.
#[expect(
    clippy::too_many_lines,
    reason = "almost all of it is the command manifest, which `tauri::generate_handler!` \
              requires in one place; splitting it would hide what the application exposes \
              without making any part of it shorter"
)]
pub fn run() {
    // The workspace owns the installation settings and the one open project. Built once,
    // here, because a failure to reach the settings database means Evara cannot function
    // and the user needs to know immediately rather than on first use.
    let workspace = evara_db::Workspace::new(env!("CARGO_PKG_VERSION"))
        .expect("failed to open the Evara settings database");

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(AppState {
            workspace: Mutex::new(workspace),
            imports: commands::import::ImportSessions::default(),
            generations: commands::session::GenerationSessions::default(),
            edits: commands::edit_session::EditSessions::default(),
            epoch: commands::session::ProjectEpoch::default(),
        })
        .invoke_handler(tauri::generate_handler![
            commands::app_info,
            // Project lifecycle — Phase 1B.
            commands::project::project_create,
            commands::project::project_open,
            commands::project::project_close,
            commands::project::project_current,
            commands::project::project_recent,
            commands::project::project_forget_recent,
            commands::project::project_extension,
            commands::project::project_default_folder,
            // Spreadsheet roster import — Stage D.
            commands::import::import_kinds,
            commands::import::import_template,
            commands::import::import_begin,
            commands::import::import_sheet,
            commands::import::import_current,
            commands::import::import_preview,
            commands::import::import_apply,
            commands::import::import_report,
            commands::import::import_save,
            commands::import::import_cancel,
            // School Setup, availability — Phase 2.
            commands::availability::setup_availability_set,
            commands::availability::setup_availability_clear,
            commands::availability::setup_availability_list,
            commands::availability::setup_availability_count,
            commands::availability::setup_availability_orphans,
            // School Setup, people and groups — Phase 2.
            commands::people::setup_year_level_create,
            commands::people::setup_year_level_get,
            commands::people::setup_year_level_list,
            commands::people::setup_year_level_update,
            commands::people::setup_year_level_delete,
            commands::people::setup_teacher_create,
            commands::people::setup_teacher_get,
            commands::people::setup_teacher_list,
            commands::people::setup_teacher_update,
            commands::people::setup_teacher_delete,
            commands::people::setup_student_create,
            commands::people::setup_student_get,
            commands::people::setup_student_list,
            commands::people::setup_student_update,
            commands::people::setup_student_delete,
            commands::people::setup_student_group_create,
            commands::people::setup_student_group_get,
            commands::people::setup_student_group_list,
            commands::people::setup_student_group_update,
            commands::people::setup_student_group_delete,
            commands::people::setup_student_group_member_add,
            commands::people::setup_student_group_member_remove,
            commands::people::setup_student_group_member_list,
            commands::people::setup_student_group_member_count,
            commands::people::setup_student_groups_for_student,
            // School Setup, shared identities — Stage 0B.
            commands::membership::setup_teacher_membership_add,
            commands::membership::setup_teacher_membership_list,
            commands::membership::setup_teacher_membership_set_active,
            commands::membership::setup_teacher_membership_remove,
            commands::membership::setup_school_teacher_list,
            commands::membership::setup_teacher_campus_list,
            commands::membership::setup_teacher_campus_assign,
            commands::membership::setup_teacher_campus_remove,
            commands::membership::setup_enrolment_create,
            commands::membership::setup_enrolment_update,
            commands::membership::setup_enrolment_current,
            commands::membership::setup_enrolment_history,
            commands::membership::setup_enrolment_leave,
            commands::membership::setup_school_student_list,
            commands::membership::setup_student_transfer,
            commands::membership::setup_resource_campus_list,
            commands::membership::setup_resource_share,
            commands::membership::setup_resource_unshare,
            // School Setup, static structure — Phase 1E.
            // Curriculum prerequisites — Stage B (V14).
            commands::curriculum::setup_department_create,
            commands::curriculum::setup_department_get,
            commands::curriculum::setup_department_list,
            commands::curriculum::setup_department_update,
            commands::curriculum::setup_department_dependents,
            commands::curriculum::setup_department_teachers,
            commands::curriculum::setup_department_delete,
            commands::curriculum::setup_subject_create,
            commands::curriculum::setup_subject_get,
            commands::curriculum::setup_subject_list,
            commands::curriculum::setup_subject_update,
            commands::curriculum::setup_subject_dependents,
            commands::curriculum::setup_subject_teachers,
            commands::curriculum::setup_subject_delete,
            commands::curriculum::setup_qualification_add,
            commands::curriculum::setup_qualification_list,
            commands::curriculum::setup_qualification_remove,
            commands::curriculum::setup_teacher_department_add,
            commands::curriculum::setup_teacher_department_list,
            commands::curriculum::setup_teacher_department_remove,
            // Stage C — lessons: courses, class sections, staffing, audience, occurrences.
            commands::lessons::setup_course_create,
            commands::lessons::setup_course_get,
            commands::lessons::setup_course_list,
            commands::lessons::setup_course_update,
            commands::lessons::setup_course_dependents,
            commands::lessons::setup_course_delete,
            commands::lessons::setup_class_section_create,
            commands::lessons::setup_class_section_get,
            commands::lessons::setup_class_section_list,
            commands::lessons::setup_class_section_update,
            commands::lessons::setup_class_section_dependents,
            commands::lessons::setup_class_section_delete,
            commands::lessons::setup_class_teacher_list,
            commands::lessons::setup_class_teacher_candidates,
            commands::lessons::setup_class_teacher_add,
            commands::lessons::setup_class_teacher_remove,
            commands::lessons::setup_teacher_sections,
            commands::lessons::setup_class_group_list,
            commands::lessons::setup_class_group_add,
            commands::lessons::setup_class_group_remove,
            commands::lessons::setup_class_enrolment_list,
            commands::lessons::setup_class_enrolment_add,
            commands::lessons::setup_class_enrolment_remove,
            commands::lessons::setup_activity_list,
            commands::lessons::setup_section_demand,
            commands::lessons::setup_activity_add,
            commands::lessons::setup_activity_generate,
            commands::lessons::setup_activity_update,
            commands::lessons::setup_activity_delete,
            commands::lessons::setup_student_class_count,
            commands::lessons::setup_group_class_count,
            commands::structure::setup_school_create,
            commands::structure::setup_school_list,
            commands::structure::setup_school_get,
            commands::structure::setup_school_dependents,
            commands::structure::setup_school_delete,
            commands::structure::setup_school_update,
            commands::structure::setup_school_logo_get,
            commands::structure::setup_school_logo_set,
            commands::structure::setup_school_logo_clear,
            commands::structure::setup_school_logo_limits,
            commands::structure::setup_campus_create,
            commands::structure::setup_campus_get,
            commands::structure::setup_campus_list,
            commands::structure::setup_campus_update,
            commands::structure::setup_campus_delete,
            commands::structure::setup_building_create,
            commands::structure::setup_building_get,
            commands::structure::setup_building_list,
            commands::structure::setup_building_campuses,
            commands::structure::setup_building_update,
            commands::structure::setup_building_share,
            commands::structure::setup_building_unshare,
            commands::structure::setup_building_delete,
            commands::structure::setup_room_type_create,
            commands::structure::setup_room_type_get,
            commands::structure::setup_room_type_list,
            commands::structure::setup_room_type_update,
            commands::structure::setup_room_type_delete,
            commands::structure::setup_room_create,
            commands::structure::setup_room_get,
            commands::structure::setup_room_list,
            commands::structure::setup_room_campuses,
            commands::structure::setup_room_update,
            commands::structure::setup_room_share,
            commands::structure::setup_room_unshare,
            commands::structure::setup_room_delete,
            commands::structure::setup_resource_create,
            commands::structure::setup_resource_get,
            commands::structure::setup_resource_list,
            commands::structure::setup_resource_update,
            commands::structure::setup_resource_delete,
            commands::structure::setup_academic_year_create,
            commands::structure::setup_academic_year_get,
            commands::structure::setup_academic_year_list,
            commands::structure::setup_academic_year_update,
            commands::structure::setup_academic_year_delete,
            commands::structure::setup_term_create,
            commands::structure::setup_term_get,
            commands::structure::setup_term_list,
            commands::structure::setup_term_update,
            commands::structure::setup_term_delete,
            // School Setup, time model — Phase 1E.
            commands::time_model::setup_cycle_create,
            commands::time_model::setup_cycle_get,
            commands::time_model::setup_cycle_list,
            commands::time_model::setup_cycle_update,
            commands::time_model::setup_cycle_delete,
            commands::time_model::setup_cycle_coverage,
            commands::time_model::setup_cycle_shape,
            commands::time_model::setup_cycle_day_create,
            commands::time_model::setup_cycle_day_get,
            commands::time_model::setup_cycle_day_list,
            commands::time_model::setup_cycle_day_update,
            commands::time_model::setup_cycle_day_delete,
            commands::time_model::setup_period_structure_create,
            commands::time_model::setup_period_structure_get,
            commands::time_model::setup_period_structure_list,
            commands::time_model::setup_period_structure_update,
            commands::time_model::setup_period_structure_delete,
            commands::time_model::setup_period_create,
            commands::time_model::setup_period_get,
            commands::time_model::setup_period_list,
            commands::time_model::setup_period_update,
            commands::time_model::setup_period_delete,
            commands::time_model::setup_calendar_day_create,
            commands::time_model::setup_calendar_day_get,
            commands::time_model::setup_calendar_day_list,
            commands::time_model::setup_calendar_non_working_days,
            commands::time_model::setup_calendar_day_update,
            commands::time_model::setup_calendar_day_delete,
            // Timetable generation — M8 stage 8A. `timetable_save` is the only one that writes.
            commands::solve::timetable_validate,
            commands::solve::timetable_generate_start,
            commands::solve::timetable_generate_status,
            commands::solve::timetable_generate_result,
            commands::solve::timetable_generate_cancel,
            commands::solve::timetable_generate_discard,
            commands::solve::timetable_generate_scope,
            commands::solve::timetable_save,
            // Saved timetables — M8 stage 8B. Rename, delete and revalidate write; the rest read.
            commands::views::timetable_list,
            commands::views::timetable_list_for_cycle,
            commands::views::timetable_open,
            commands::views::timetable_scope,
            commands::views::timetable_rename,
            commands::views::timetable_delete,
            commands::views::timetable_revalidate,
            // Manual editing — M9A. Four of these write; the rest ask what is legal.
            commands::edit::edit_open,
            commands::edit::edit_close,
            commands::edit::edit_state,
            commands::edit::edit_scope,
            commands::edit::edit_candidates,
            commands::edit::edit_validate,
            commands::edit::edit_apply,
            commands::edit::edit_unschedule,
            commands::edit::edit_set_lock,
            commands::edit::edit_undo,
            commands::edit::edit_redo,
            commands::edit::edit_history,
            commands::edit::timetable_duplicate,
            // Timetable grid — Phase 1E. `release_orphans` is the only destructive one.
            commands::grid::setup_grid_preview,
            commands::grid::setup_grid_rebuild,
            commands::grid::setup_grid_release_orphans,
            commands::grid::setup_grid_timeslots,
        ])
        .run(tauri::generate_context!())
        .expect("failed to start the Evara Timetables window");
}
