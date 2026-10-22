//! Teachers and student groups: who teaches, and who is taught.
//!
//! # A teacher is one identity, project-wide
//!
//! [ADR 0021](../../../../docs/adr/0021-project-level-people-and-school-membership.md): a
//! person belongs to the project and a membership belongs to the school, so somebody
//! employed by two schools is **one** `teacher.id` in both. A loader that minted a
//! school-specific identity would make
//! [ADR 0032](../../../../docs/adr/0032-timetable-generation-is-scoped-to-one-school.md)
//! rule 3's shared-teacher warning impossible to write — it needs both school names — and
//! would hide the very sharing that warning exists to report.
//!
//! # Who is in scope, and the case the obvious answer gets wrong
//!
//! `TeacherFact` is documented as *"every teacher with an active membership at this
//! school"*, and loading exactly that set is wrong.
//!
//! `V15`'s `class_teacher_requires_active_membership` trigger pair fires on an INSERT or
//! UPDATE of `class_teacher` and **never on a change to `teacher_school`**, which its own
//! comment states is deliberate: ADR 0018 requires a deactivated teacher to keep *"every
//! assignment they ever had"* and simply stop being offered for new ones.
//! `Membership::set_teacher_membership_active(…, false)` therefore succeeds while the
//! teacher still teaches, and is the supported way to record somebody leaving mid-year.
//!
//! So a section legitimately in scope may be taught by a teacher with no active membership.
//! Loading only the employed set would leave `SectionFact::teachers` pointing past the end
//! of the table — `IntegrityError::DanglingIndex`, reported as a loader bug, which is what
//! it would be. The set is therefore **the active memberships together with every teacher an
//! in-scope section names**, and offering the same identifier twice returns one index.
//!
//! # Groups are the scheduling population, and the relation is not computed here
//!
//! [ADR 0035](../../../../docs/adr/0035-student-group-overlap-is-a-three-state-answer.md)
//! §5: a set of **groups**, never an expanded set of students. This module reads the inputs
//! §3's seven rules consume — the parent edge, the declared size, the effective membership
//! count and the parent's disjointness declaration. It does not evaluate them; the
//! `overlap` function in this module's parent records why that is M5's and what this
//! milestone leaves behind.

use evara_core::scheduling::{
    DisjointnessDeclaration, GroupFact, GroupKind, GroupMembers, OtherSchool, OverlapOutcome,
    StudentIx, TeacherFact, evaluate_overlap,
};
use rusqlite::Connection;
use uuid::Uuid;

use super::index::ProblemIndex;
use super::{IN_SCOPE_TERM, SchedulingSelection, corrupt, require_index};
use crate::error::Result;
use crate::repo::row::{opt_text, parse_id_text, text};

/// The teachers in scope, and how many of them are no longer employed here.
///
/// A named pair rather than a tuple, because the count is easy to mistake for a length.
pub(super) struct Teachers {
    /// The facts, in index order.
    pub(super) facts: Vec<TeacherFact>,
    /// How many have no active membership at this school — the ADR 0018 case above.
    pub(super) without_active_membership: usize,
}

/// Reads the teachers in scope.
///
/// Two queries and a union, in the order this module's header explains: the school's active
/// memberships first, so an ordinary school's indices are exactly its staff list, then any
/// teacher an in-scope section names who is not among them.
pub(super) fn teachers(
    conn: &Connection,
    selection: &SchedulingSelection,
    index: &mut ProblemIndex,
) -> Result<Teachers> {
    let mut facts = Vec::new();

    let mut employed = conn.prepare(
        "SELECT t.id, t.given_name, t.family_name
         FROM teacher        t
         JOIN teacher_school ts ON ts.teacher_id = t.id
         WHERE ts.school_id = ?1 AND ts.is_active = 1
         ORDER BY t.family_name, t.given_name, t.id",
    )?;
    let rows = employed.query_map([text(selection.school_id)], read_teacher)?;
    for row in rows {
        append_teacher(row?, index, &mut facts)?;
    }
    drop(employed);
    let active = facts.len();

    let mut assigned = conn.prepare(&format!(
        "SELECT DISTINCT t.id, t.given_name, t.family_name
         FROM class_teacher ct
         JOIN class_section cs ON cs.id = ct.class_section_id
         JOIN course        co ON co.id = cs.course_id
         JOIN subject       su ON su.id = co.subject_id
         JOIN teacher       t  ON t.id  = ct.teacher_id
         WHERE su.school_id = ?1 AND {IN_SCOPE_TERM}
         ORDER BY t.family_name, t.given_name, t.id"
    ))?;
    let rows = assigned.query_map(
        rusqlite::params![text(selection.school_id), opt_text(selection.term_id)],
        read_teacher,
    )?;
    for row in rows {
        append_teacher(row?, index, &mut facts)?;
    }
    drop(assigned);

    eligibility(conn, selection, index, &mut facts)?;
    sharing(conn, selection, index, &mut facts)?;
    Ok(Teachers {
        without_active_membership: facts.len() - active,
        facts,
    })
}

/// Appends one teacher, or leaves the table alone if it already holds them.
fn append_teacher(
    row: (String, String, String),
    index: &mut ProblemIndex,
    facts: &mut Vec<TeacherFact>,
) -> Result<()> {
    let (raw, given_name, family_name) = row;
    let id = parse_id_text("teacher.id", &raw)?;
    if index.teachers.index_of(id).is_some() {
        return Ok(());
    }
    index.teachers.intern(id);
    facts.push(TeacherFact {
        id,
        name: format!("{given_name} {family_name}"),
        eligible_campuses: None,
        also_at: Vec::new(),
    });
    Ok(())
}

/// Fills in each teacher's campus eligibility at **this** school.
///
/// `None` and an empty list are different answers and both are reachable. `teacher_campus`
/// is a *narrowing* (ADR 0021 §3): no row means "not narrowed", which is the state every
/// teacher is in until somebody says otherwise, and `V15`'s campus trigger reads it exactly
/// that way. Reading an empty set as "eligible nowhere" would make every teacher at a
/// multi-campus school unassignable, which is the mistake that trigger's comment warns
/// against in as many words.
///
/// Eligibility recorded at another school's campuses is not read: *"a teacher narrowed at
/// Southmoor is not thereby narrowed at Northgate"*.
fn eligibility(
    conn: &Connection,
    selection: &SchedulingSelection,
    index: &ProblemIndex,
    facts: &mut [TeacherFact],
) -> Result<()> {
    let mut stmt = conn.prepare(
        "SELECT tc.teacher_id, tc.campus_id
         FROM teacher_campus tc
         JOIN campus         c ON c.id = tc.campus_id
         WHERE c.school_id = ?
         ORDER BY c.name, c.id",
    )?;
    let rows = stmt.query_map([text(selection.school_id)], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;

    for row in rows {
        let (raw_teacher, raw_campus) = row?;
        let teacher = parse_id_text("teacher_campus.teacher_id", &raw_teacher)?;
        let Some(at) = index.teachers.index_of(teacher) else {
            continue;
        };
        let campus = parse_id_text("teacher_campus.campus_id", &raw_campus)?;
        let campus = require_index(&index.campuses, campus, "teacher_campus.campus_id")?;
        facts[at.get()]
            .eligible_campuses
            .get_or_insert_with(Vec::new)
            .push(campus);
    }
    Ok(())
}

/// Names the other schools that also actively employ each in-scope teacher.
///
/// The condition of ADR 0032 rule 3's first bullet. **No row of the other school is read**
/// beyond its name: the sharing is a property of the second membership, not of the other
/// school's timetable, which may be absent, a draft, or several.
fn sharing(
    conn: &Connection,
    selection: &SchedulingSelection,
    index: &ProblemIndex,
    facts: &mut [TeacherFact],
) -> Result<()> {
    let mut stmt = conn.prepare(
        "SELECT ts.teacher_id, s.id, s.name
         FROM teacher_school ts
         JOIN school         s ON s.id = ts.school_id
         WHERE ts.is_active = 1 AND ts.school_id <> ?
         ORDER BY s.name, s.id",
    )?;
    let rows = stmt.query_map([text(selection.school_id)], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
        ))
    })?;

    for row in rows {
        let (raw_teacher, raw_school, name) = row?;
        let teacher = parse_id_text("teacher_school.teacher_id", &raw_teacher)?;
        let Some(at) = index.teachers.index_of(teacher) else {
            continue;
        };
        let id = parse_id_text("school.id", &raw_school)?;
        facts[at.get()].also_at.push(OtherSchool { id, name });
    }
    Ok(())
}

/// Reads every student group of the school, and who is in each one.
///
/// All of them, not only the ones a section is taught to: `GroupFact::parent` indexes into
/// this table, and an ancestor that attends nothing is still the group whose
/// `children_are_disjoint` answer rule 6 reads. `V12`'s trigger pair holds a parent to its
/// child's school, so loading the school's groups yields a closed parent set and no parent
/// edge can dangle — which is also why ADR 0035 truth table row 20 needs no check here: a
/// subtree cannot cross a school boundary.
///
/// Returns the facts with `effective_members` still zero, because the count comes from the
/// member sets and the sets are unioned by the evaluator. [`overlap`] fills both in.
pub(super) fn groups(
    conn: &Connection,
    selection: &SchedulingSelection,
    index: &mut ProblemIndex,
) -> Result<(Vec<GroupFact>, GroupMembers, Vec<Student>)> {
    // Interned in one pass before any parent edge is resolved, because a group's parent may
    // sort after it and a forward reference must resolve to the same index either way.
    let mut rows = Vec::new();
    {
        let mut stmt = conn.prepare(
            "SELECT id, name, kind, parent_group_id, size
             FROM student_group WHERE school_id = ? ORDER BY name, id",
        )?;
        let read = stmt.query_map([text(selection.school_id)], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, Option<i64>>(4)?,
            ))
        })?;
        for row in read {
            let row = row?;
            let id = parse_id_text("student_group.id", &row.0)?;
            index.groups.intern(id);
            rows.push((id, row.1, row.2, row.3, row.4));
        }
    }

    let mut facts = Vec::with_capacity(rows.len());
    for (id, name, kind, raw_parent, declared_size) in rows {
        let parent = raw_parent
            .map(|raw| parse_id_text("student_group.parent_group_id", &raw))
            .transpose()?
            .map(|parent| require_index(&index.groups, parent, "student_group.parent_group_id"))
            .transpose()?;
        facts.push(GroupFact {
            id,
            name,
            kind: group_kind(&kind)?,
            parent,
            declared_size,
            // Filled in by `overlap`, from the union of the subtree's member sets.
            effective_members: 0,
            // `student_group.children_are_disjoint` is a FUTURE column: ADR 0035 §4 defines
            // it and `SCHEDULING-ARCHITECTURE-DECISIONS.md` §4.1 schedules it for M5's
            // migration. Every existing project reads `NULL`, which is exactly what
            // `NotStated` means — and `DisjointnessDeclaration::from_column(None)` is the
            // one conversion, so when the column lands this line becomes a read rather
            // than a constant. Defaulting to either answer would be inventing a statement
            // the school has not made.
            children_are_disjoint: DisjointnessDeclaration::from_column(None),
        });
    }

    let (members, students) = membership(conn, selection, index, facts.len())?;
    Ok((facts, members, students))
}

/// A student whose membership the evaluator read, for a report that has to name them.
///
/// Kept beside the problem rather than in it. `Problem` holds groups and never an expanded
/// student set — ADR 0035 §5, which ADR 0006 depends on — so these exist only so a
/// `group-overlap-contradiction` can say *which* child is in two groups that were declared
/// disjoint.
pub(super) struct Student {
    /// Stable identity, as stored.
    pub(super) id: Uuid,
    /// What to call them in a finding.
    pub(super) name: String,
}

/// Reads the direct `student_group_member` rows, filtered to who could be in the room.
///
/// # ADR 0035 §2's conjunction, in SQL
///
/// A student counts when they are `is_active` **and** hold a `student_school_enrolment` spell
/// at this school whose status is not `LEFT`. Both halves are load-bearing:
/// `student_group_member` deliberately keeps history (`V13:468-470`), which is right for the
/// record and wrong for a room — counting a child who has left would manufacture clashes
/// between groups that no longer share anybody.
///
/// **Direct rows only.** The subtree union is the evaluator's, which is what makes
/// `effective_members` and rule 3 read one set rather than two that have to agree. The
/// recursive query this replaces computed only a count, and a count cannot answer rule 3.
fn membership(
    conn: &Connection,
    selection: &SchedulingSelection,
    index: &mut ProblemIndex,
    groups: usize,
) -> Result<(GroupMembers, Vec<Student>)> {
    let mut pairs = Vec::new();
    let mut students = Vec::new();
    {
        let mut stmt = conn.prepare(
            "SELECT m.student_group_id, m.student_id, s.given_name, s.family_name
             FROM student_group_member m
             JOIN student_group       g ON g.id = m.student_group_id
             JOIN student             s ON s.id = m.student_id
             WHERE g.school_id = ?1
               AND s.is_active = 1
               AND EXISTS (
                   SELECT 1 FROM student_school_enrolment e
                   WHERE e.student_id = m.student_id
                     AND e.school_id  = ?1
                     AND e.status    <> 'LEFT'
               )
             ORDER BY s.family_name, s.given_name, s.id, g.name, g.id",
        )?;
        let read = stmt.query_map([text(selection.school_id)], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
            ))
        })?;
        for row in read {
            let (raw_group, raw_student, given_name, family_name) = row?;
            let group = parse_id_text("student_group_member.student_group_id", &raw_group)?;
            let student = parse_id_text("student_group_member.student_id", &raw_student)?;
            // A group of another school cannot appear — the join restricts it — so a miss
            // here would be corruption rather than an out-of-scope row.
            let group = require_index(
                &index.groups,
                group,
                "student_group_member.student_group_id",
            )?;
            if index.students.index_of(student).is_none() {
                index.students.intern(student);
                students.push(Student {
                    id: student,
                    name: format!("{given_name} {family_name}"),
                });
            }
            let at = require_index(&index.students, student, "student_group_member.student_id")?;
            pairs.push((group, at));
        }
    }

    let mut members = GroupMembers::new(groups, index.students.len());
    for (group, student) in pairs {
        members.add(group, student);
    }
    Ok((members, students))
}

/// Compiles the three-state overlap answer for every pair, and the counts beside it.
///
/// # The evaluator is called, not reimplemented
///
/// `evara_core::scheduling::overlap` is ADR 0035 §3's only implementation, and this is the
/// call site the record means by *"computed once per solve and never inside the search
/// loop"*. There is no overlap logic in this crate; a second copy would be the duplication
/// `CLAUDE.md` §2 forbids, with rule 3's precedence over rule 6 the thing most expensive to
/// get differently wrong in two places.
///
/// `effective_members` is written from the same sets the evaluator intersected, so the count
/// a report shows and the set rule 3 read cannot disagree.
pub(super) fn overlap(facts: &mut [GroupFact], members: &mut GroupMembers) -> OverlapOutcome {
    let outcome = evaluate_overlap(facts, members);
    for (at, fact) in facts.iter_mut().enumerate() {
        let group = evara_core::scheduling::GroupIx::new(u32::try_from(at).unwrap_or(u32::MAX));
        fact.effective_members = members.effective_count(group);
    }
    outcome
}

/// The student a contradiction names, by index.
pub(super) fn student_at(students: &[Student], at: StudentIx) -> Option<&Student> {
    students.get(at.get())
}

/// Reads `student_group.kind`.
///
/// Carries no scheduling meaning and must not acquire one — ADR 0035 records why it was the
/// tempting shortcut for sibling disjointness and cannot work: `9-Mat-1` and `9-Fre-1` are
/// both `TEACHING_SET` and they overlap heavily.
fn group_kind(raw: &str) -> Result<GroupKind> {
    match raw {
        "CLASS" => Ok(GroupKind::Class),
        "COHORT" => Ok(GroupKind::Cohort),
        "HOMEROOM" => Ok(GroupKind::Homeroom),
        "ELECTIVE_SET" => Ok(GroupKind::ElectiveSet),
        "TEACHING_SET" => Ok(GroupKind::TeachingSet),
        other => Err(corrupt("student_group.kind", other.into())),
    }
}

/// Reads one teacher row.
fn read_teacher(row: &rusqlite::Row<'_>) -> rusqlite::Result<(String, String, String)> {
    Ok((row.get(0)?, row.get(1)?, row.get(2)?))
}
