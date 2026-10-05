export type ID = string;

export interface Timeslot {
  id: ID;
  cycleDay: number;
  period: number;
  label: string;
}

export interface Teacher {
  id: ID;
  name: string;
  unavailableTimeslotIds?: ID[];
}

export interface StudentGroup {
  id: ID;
  name: string;
  size: number;
}

export interface Room {
  id: ID;
  name: string;
  capacity: number;
  resourceTypes?: string[];
}

export interface Lesson {
  id: ID;
  subjectId: ID;
  teacherIds: ID[];
  studentGroupIds: ID[];
  requiredCapacity?: number;
  requiredResourceTypes?: string[];
  durationPeriods?: number;
  lockedTimeslotId?: ID;
  lockedRoomId?: ID;
}

export type ConstraintLevel = "HARD" | "MEDIUM" | "SOFT";

export interface ConstraintDefinition<T = Record<string, unknown>> {
  id: ID;
  type: string;
  level: ConstraintLevel;
  weight: number;
  enabled: boolean;
  parameters: T;
}

export interface SchoolTimetableProblem {
  timeslots: Timeslot[];
  teachers: Teacher[];
  studentGroups: StudentGroup[];
  rooms: Room[];
  lessons: Lesson[];
  constraints: ConstraintDefinition[];
}
