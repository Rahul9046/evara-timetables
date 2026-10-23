//! The display vocabulary a timetable view needs and a solve does not.
//!
//! # Why this is here and not in `Problem`
//!
//! A compiled [`Problem`](super::Problem) carries the names a *rule* needs in order to
//! explain itself — a teacher, a room, a group, a section. It carries no subject, no course,
//! no year level, no building and no student, because none of those is an input to any
//! scheduling decision. `SectionFact.name` is `class_section.name` alone, so a grid driven by
//! the problem would render a cell reading `"B"`.
//!
//! [ADR 0037](../../../../docs/adr/0037-stored-row-diagnostics-belong-to-the-loader.md) §2
//! refused to widen `Problem` with *"the first field whose only consumer is a diagnostic,
//! which invites more of them"*. A field whose only consumer is a **report** is the same
//! field, so the vocabulary sits beside the problem rather than inside it.
//!
//! # Why it is in `evara-core`
//!
//! Two crates have to agree on it and neither can see the other:
//!
//! | Crate | Role |
//! | --- | --- |
//! | `evara-db` | **populates** it from SQL — the only crate that may |
//! | `evara-report` | **consumes** it, joining it to the problem to project a grid |
//!
//! `evara-db -> core` and `evara-report -> core, constraints`, so the shared vocabulary has
//! to sit underneath both. That is the same argument `ARCHITECTURE.md` §3.2 makes for
//! `StoredConstraint` and [ADR 0042](../../../../docs/adr/0042-a-stored-timetable-is-a-version-and-a-save-refuses-stale-inputs.md)
//! D2 makes for [`span`](super::span).
//!
//! # A label is never a scheduling input
//!
//! Nothing in this module is read by a constraint, a solver or a diagnostic. If a label is
//! missing the grid says so and renders; it never substitutes a default that could be
//! mistaken for data. That is why every lookup returns `Option` and why
//! [`SectionLabel::short`] is composed **once**, here, rather than by each screen that wants
//! a cell caption.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// What a lesson is called, for a reader.
///
/// `subject -> course -> class_section` is the chain `V15` stores and
/// [ADR 0029](../../../../docs/adr/0029-a-lesson-is-a-course-a-section-and-its-occurrences.md)
/// §1 describes; all three parts are carried because a cell caption, a row header and a
/// findings list each want a different amount of it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct SectionLabel {
    /// Which section this labels.
    pub section_id: Uuid,
    /// The subject's name, e.g. `"Mathematics"`.
    pub subject: String,
    /// The course's name, e.g. `"Mathematics Year 9"`.
    pub course: String,
    /// The section's own name, e.g. `"B"`.
    pub section: String,
    /// The year level the course belongs to, where it names one.
    pub year_level: Option<String>,
    /// The campus the section is taught at, where it names one.
    pub campus: Option<String>,
    /// A short caption for a grid cell, composed once.
    ///
    /// Composed here rather than by each screen so that a cell, a tooltip and a findings
    /// entry cannot disagree about what a lesson is called. The order is year level, subject,
    /// section — the order a timetabler says it in.
    pub short: String,
}

impl SectionLabel {
    /// Composes [`SectionLabel::short`] from the parts.
    ///
    /// The one implementation. A caller building a label by hand would be the second.
    #[must_use]
    pub fn compose_short(year_level: Option<&str>, subject: &str, section: &str) -> String {
        let mut out = String::new();
        if let Some(year) = year_level.map(str::trim).filter(|y| !y.is_empty()) {
            out.push_str(year);
            out.push(' ');
        }
        out.push_str(subject.trim());
        let section = section.trim();
        if !section.is_empty() {
            out.push(' ');
            out.push_str(section);
        }
        out
    }
}

/// What a room is called, for a reader.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct RoomLabel {
    /// Which room this labels.
    pub room_id: Uuid,
    /// The room's name.
    pub name: String,
    /// The building it is in, where it names one.
    pub building: Option<String>,
}

/// One student, and the sections their enrolment names.
///
/// # This is a roster, not a scheduling population
///
/// [ADR 0006](../../../../docs/adr/0006-group-based-scheduling-first.md) requires a full
/// timetable to be buildable from `student_group.size` with **no student records at all**, so
/// `class_enrolment` is optional everywhere and has no scheduling meaning in Milestone 1 —
/// `CLAUDE.md` §3, and `SCHEDULING-ARCHITECTURE-DECISIONS.md` §7 defers *"individually
/// enrolled students as a scheduling population"*.
///
/// So a per-student view is **derived from explicit enrolment rows and from nothing else**.
/// [`StudentLabel::sections`] is exactly what `class_enrolment` says. It is never inferred
/// from a group's size, and a student with no rows has an **empty** list rather than the
/// timetable of a group they might be in: substituting one would present a guess as a
/// child's confirmed schedule.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct StudentLabel {
    /// Which student this labels.
    pub student_id: Uuid,
    /// The student's display name.
    pub name: String,
    /// The student's own identifier at the school, where there is one.
    pub code: Option<String>,
    /// The sections the student is **individually enrolled** in.
    ///
    /// Straight from `class_enrolment`. Empty means no row names them, which is an ordinary
    /// state and is not evidence that they attend nothing.
    pub sections: Vec<Uuid>,
}

/// The display names a view needs, keyed by stored identifier.
///
/// Built by `evara-db` and read by `evara-report`. Carries no index: a `LabelSet` is about
/// stored rows and a dense index belongs to the one problem that assigned it.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct LabelSet {
    /// One per class section in scope.
    pub sections: Vec<SectionLabel>,
    /// One per room in scope.
    pub rooms: Vec<RoomLabel>,
    /// One per student with at least one enrolment row in scope.
    ///
    /// A student with no `class_enrolment` row is **absent**, not present with an empty
    /// list — there is nothing to show them and inventing a row would be the inference this
    /// type exists to prevent.
    pub students: Vec<StudentLabel>,
}

impl LabelSet {
    /// An index from section identifier to label, for repeated lookup.
    ///
    /// A `BTreeMap` rather than a `HashMap`: every projection output has to be deterministic
    /// and nothing in this workspace iterates a hash map (`CONSTRAINT-ENGINE.md` §6).
    #[must_use]
    pub fn section_index(&self) -> BTreeMap<Uuid, &SectionLabel> {
        self.sections.iter().map(|s| (s.section_id, s)).collect()
    }

    /// An index from room identifier to label.
    #[must_use]
    pub fn room_index(&self) -> BTreeMap<Uuid, &RoomLabel> {
        self.rooms.iter().map(|r| (r.room_id, r)).collect()
    }

    /// The label of one section, or `None` where the set does not hold it.
    #[must_use]
    pub fn section(&self, section_id: Uuid) -> Option<&SectionLabel> {
        self.sections.iter().find(|s| s.section_id == section_id)
    }

    /// The label of one room, or `None` where the set does not hold it.
    #[must_use]
    pub fn room(&self, room_id: Uuid) -> Option<&RoomLabel> {
        self.rooms.iter().find(|r| r.room_id == room_id)
    }

    /// Whether any student carries an enrolment row.
    ///
    /// What a screen asks before offering a per-student view at all: with no rows the view
    /// has nothing to derive from, and an empty picker with an explanation is better than a
    /// picker of every student whose timetable would be blank.
    #[must_use]
    pub fn has_enrolments(&self) -> bool {
        self.students.iter().any(|s| !s.sections.is_empty())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn an_id(n: u8) -> Uuid {
        Uuid::from_bytes([n; 16])
    }

    #[test]
    fn a_short_caption_reads_year_subject_section() {
        assert_eq!(
            SectionLabel::compose_short(Some("9"), "Mathematics", "B"),
            "9 Mathematics B"
        );
    }

    #[test]
    fn a_missing_year_level_is_omitted_rather_than_spaced() {
        assert_eq!(
            SectionLabel::compose_short(None, "Mathematics", "B"),
            "Mathematics B"
        );
    }

    #[test]
    fn a_blank_year_level_is_treated_as_absent() {
        assert_eq!(
            SectionLabel::compose_short(Some("   "), "Art", "A"),
            "Art A"
        );
    }

    #[test]
    fn a_section_with_no_name_leaves_no_trailing_space() {
        assert_eq!(
            SectionLabel::compose_short(Some("7"), "Drama", ""),
            "7 Drama"
        );
    }

    #[test]
    fn a_label_set_with_no_enrolment_rows_reports_none() {
        let set = LabelSet {
            students: vec![StudentLabel {
                student_id: an_id(1),
                name: "A. Student".to_owned(),
                code: None,
                sections: Vec::new(),
            }],
            ..LabelSet::default()
        };
        assert!(
            !set.has_enrolments(),
            "a student with no sections is not an enrolment"
        );
    }

    #[test]
    fn a_label_set_with_one_enrolment_row_reports_some() {
        let set = LabelSet {
            students: vec![StudentLabel {
                student_id: an_id(1),
                name: "A. Student".to_owned(),
                code: Some("S001".to_owned()),
                sections: vec![an_id(9)],
            }],
            ..LabelSet::default()
        };
        assert!(set.has_enrolments());
    }

    #[test]
    fn a_lookup_that_misses_returns_none_rather_than_a_default() {
        let set = LabelSet::default();
        assert!(set.section(an_id(3)).is_none());
        assert!(set.room(an_id(3)).is_none());
    }
}
