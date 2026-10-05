package app.evara.timetable.domain;

import ai.timefold.solver.core.api.domain.entity.PlanningEntity;
import ai.timefold.solver.core.api.domain.lookup.PlanningId;
import ai.timefold.solver.core.api.domain.variable.PlanningVariable;

@PlanningEntity
public class Lesson {
    @PlanningId private String id;
    private String subject;
    private String teacherId;
    private String studentGroupId;
    private int requiredCapacity;
    @PlanningVariable(valueRangeProviderRefs = "timeslotRange") private Timeslot timeslot;
    @PlanningVariable(valueRangeProviderRefs = "roomRange") private Room room;

    public Lesson() {}
    public Lesson(String id,String subject,String teacherId,String studentGroupId,int requiredCapacity){
        this.id=id; this.subject=subject; this.teacherId=teacherId; this.studentGroupId=studentGroupId; this.requiredCapacity=requiredCapacity;
    }
    public String getId(){return id;} public String getSubject(){return subject;}
    public String getTeacherId(){return teacherId;} public String getStudentGroupId(){return studentGroupId;}
    public int getRequiredCapacity(){return requiredCapacity;}
    public Timeslot getTimeslot(){return timeslot;} public void setTimeslot(Timeslot v){timeslot=v;}
    public Room getRoom(){return room;} public void setRoom(Room v){room=v;}
}