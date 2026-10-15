/**
 * Section 7 — Year levels.
 *
 * ## Why this left the Students screen
 *
 * It used to sit above the students it classifies, on the reasoning that School Setup
 * already pairs an entity with the small table that classifies it — rooms with room types,
 * cycles with cycle days. That reading was wrong about what a year level *is*.
 *
 * A room type is an attribute of rooms: it exists because somebody wanted to say "this
 * lesson needs a lab", and it has no life outside the rooms it classifies. A year level is
 * part of **the structure of a school year**. It exists whether or not a single student is
 * entered — ADR 0006's whole promise is that a school can timetable with group sizes and no
 * student records at all, and such a school still has a Grade 8. Stage C's lessons are
 * taught *to* year levels, and Stage D's Excel templates reference them as a sheet of their
 * own.
 *
 * So it belongs with Academic years and terms, which is where Stage A put it: the shape of
 * the year, then the stages within it, then the people.
 *
 * ## School-owned, and never shared
 *
 * [ADR 0019](../../../../../docs/adr/0019-multiple-schools-in-one-project.md) is explicit:
 * *"A year level is never shared."* Two schools in one campus file may each hold their own
 * "Grade 1" at their own ordinal 1, as two records with two identifiers, and
 * `UNIQUE (school_id, name)` keeps a duplicate *within* one school refused. That was the
 * requirement the multi-school work existed for and it needed no schema change — but it
 * does need a screen that never shows one school's stages under another's name, which is
 * what the explicit `school.id` on every call here is for.
 *
 * ## Position is the school's own order, and is not an age
 *
 * `ordinal` is the only progression identity a year level has. Nothing derives it from a
 * birthday, for the same reason nothing derives a cycle day from a weekday: a school that
 * numbers its stages 0–13 and a school that numbers them 7–12 are both describing
 * themselves correctly, and an inferred age would be a claim about children rather than
 * about the school.
 *
 * ## Deleting one unclassifies rather than destroys
 *
 * Students and groups in a deleted year level are kept and simply stop naming a year. The
 * repository does that through a real update, so each row's revision stays truthful — not
 * through a database-level `SET NULL`, which would bypass the revision contract (see R10 in
 * [OPEN-DECISIONS](../../../../../docs/OPEN-DECISIONS.md)).
 */
import type { School, YearLevel, YearLevelInput } from "@evara/domain";
import {
  createYearLevel,
  deleteYearLevel,
  listYearLevels,
  updateYearLevel,
} from "../../ipc/setup";
import { CrudPanel, type CrudSpec } from "../ui/CrudPanel";
import { number, text, validateYearLevel } from "../validation";

export function YearLevels(props: { school: School }) {
  const { school } = props;

  const spec: CrudSpec<YearLevel, YearLevelInput> = {
    noun: "year level",
    plural: "year levels",
    title: "Year levels",
    columns: [
      { header: "Name", render: (row) => row.name },
      { header: "Position", render: (row) => String(row.ordinal), numeric: true },
    ],
    fields: [
      { kind: "text", key: "name", label: "Name", required: true, placeholder: "Year 9" },
      {
        kind: "number",
        key: "ordinal",
        label: "Position",
        required: true,
        min: 0,
        hint: "The school's own order, lowest first. Not an age — nothing in Evara derives a year level from a date of birth.",
      },
    ],
    describe: (row) => row.name,
    validate: validateYearLevel,
    toDraft: (row) => ({
      name: row?.name ?? "",
      ordinal: row === null ? "" : String(row.ordinal),
    }),
    toInput: (draft) => ({
      schoolId: school.id,
      name: text(draft, "name"),
      ordinal: number(draft, "ordinal"),
    }),
    list: () => listYearLevels(school.id),
    create: (input) => createYearLevel(input),
    update: (row, input) => updateYearLevel(row.id, row.rev, input),
    remove: (row) => deleteYearLevel(school.id, row.id, row.rev),
    emptyHint:
      "No year levels yet. Add the stages this school teaches — Year 7 to Year 13, Grade 1 to Grade 6, whatever this school calls them. They belong to this school alone, so another school in this campus file can hold its own Grade 1 without colliding.",
    deleteNote: () =>
      "Students and groups in this year level are kept. They simply stop naming a year, and can be given another one.",
  };

  return (
    <div className="section">
      <header className="sectionIntro">
        <h2>Year levels</h2>
        <p>
          The stages a student progresses through: &ldquo;Year 9&rdquo;, &ldquo;Grade
          8&rdquo;, &ldquo;Sixth Form&rdquo;. They are <strong>this school&rsquo;s own</strong>
          — a campus file with two schools holds two sets, and neither sees the other&rsquo;s.
        </p>
        <p className="muted">
          A student&rsquo;s year level is recorded on their enrolment, so the same person can
          sit in Grade 8 at one school and Year 9 at another. Student groups and, from Stage
          C, lessons are classified by year level too — which is why it is set up here, with
          the rest of the school year, rather than alongside the children in it.
        </p>
      </header>

      <CrudPanel spec={spec} reloadKey={school.id} />
    </div>
  );
}
