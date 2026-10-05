package app.evara.timetable.domain;
public record Timeslot(String id, int cycleDay, int period, String label) {}