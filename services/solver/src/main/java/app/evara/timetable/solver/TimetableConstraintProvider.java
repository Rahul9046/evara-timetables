package app.evara.timetable.solver;

import app.evara.timetable.domain.Lesson;
import ai.timefold.solver.core.api.score.buildin.hardsoft.HardSoftScore;
import ai.timefold.solver.core.api.score.stream.*;

public class TimetableConstraintProvider implements ConstraintProvider {
    @Override public Constraint[] defineConstraints(ConstraintFactory f) {
        return new Constraint[]{teacherConflict(f), studentGroupConflict(f), roomConflict(f), roomCapacity(f), spreadSubjects(f)};
    }

    Constraint teacherConflict(ConstraintFactory f){return f.forEachUniquePair(Lesson.class,
        Joiners.equal(Lesson::getTimeslot),Joiners.equal(Lesson::getTeacherId))
        .penalize(HardSoftScore.ONE_HARD).asConstraint("Teacher conflict");}

    Constraint studentGroupConflict(ConstraintFactory f){return f.forEachUniquePair(Lesson.class,
        Joiners.equal(Lesson::getTimeslot),Joiners.equal(Lesson::getStudentGroupId))
        .penalize(HardSoftScore.ONE_HARD).asConstraint("Student group conflict");}

    Constraint roomConflict(ConstraintFactory f){return f.forEachUniquePair(Lesson.class,
        Joiners.equal(Lesson::getTimeslot),Joiners.equal(Lesson::getRoom))
        .penalize(HardSoftScore.ONE_HARD).asConstraint("Room conflict");}

    Constraint roomCapacity(ConstraintFactory f){return f.forEach(Lesson.class)
        .filter(l -> l.getRoom()!=null && l.getRoom().capacity()<l.getRequiredCapacity())
        .penalize(HardSoftScore.ONE_HARD, l -> l.getRequiredCapacity()-l.getRoom().capacity())
        .asConstraint("Room capacity");}

    Constraint spreadSubjects(ConstraintFactory f){return f.forEachUniquePair(Lesson.class,
        Joiners.equal(Lesson::getStudentGroupId),Joiners.equal(Lesson::getSubject))
        .filter((a,b)->a.getTimeslot()!=null&&b.getTimeslot()!=null&&a.getTimeslot().cycleDay()==b.getTimeslot().cycleDay())
        .penalize(HardSoftScore.ONE_SOFT).asConstraint("Spread repeated subjects");}
}