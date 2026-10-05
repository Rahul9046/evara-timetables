package app.evara.timetable.domain;

import java.util.List;
import ai.timefold.solver.core.api.domain.solution.*;
import ai.timefold.solver.core.api.domain.valuerange.ValueRangeProvider;
import ai.timefold.solver.core.api.score.buildin.hardsoft.HardSoftScore;

@PlanningSolution
public class Timetable {
    @ProblemFactCollectionProperty @ValueRangeProvider(id="timeslotRange") private List<Timeslot> timeslots;
    @ProblemFactCollectionProperty @ValueRangeProvider(id="roomRange") private List<Room> rooms;
    @PlanningEntityCollectionProperty private List<Lesson> lessons;
    @PlanningScore private HardSoftScore score;

    public Timetable() {}
    public Timetable(List<Timeslot> timeslots,List<Room> rooms,List<Lesson> lessons){this.timeslots=timeslots;this.rooms=rooms;this.lessons=lessons;}
    public List<Timeslot> getTimeslots(){return timeslots;} public List<Room> getRooms(){return rooms;}
    public List<Lesson> getLessons(){return lessons;} public HardSoftScore getScore(){return score;}
    public void setScore(HardSoftScore score){this.score=score;}
}