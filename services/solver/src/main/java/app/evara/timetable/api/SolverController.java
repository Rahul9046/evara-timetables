package app.evara.timetable.api;

import java.time.Duration;
import app.evara.timetable.domain.Timetable;
import ai.timefold.solver.core.api.solver.SolverManager;
import org.springframework.web.bind.annotation.*;

@RestController
@RequestMapping("/api/timetables")
@CrossOrigin(origins="*")
public class SolverController {
    private final SolverManager<Timetable,String> solverManager;
    public SolverController(SolverManager<Timetable,String> solverManager){this.solverManager=solverManager;}

    @PostMapping("/solve")
    public Timetable solve(@RequestBody Timetable problem) throws Exception {
        String id=java.util.UUID.randomUUID().toString();
        return solverManager.solve(id, problem).getFinalBestSolution();
    }
}