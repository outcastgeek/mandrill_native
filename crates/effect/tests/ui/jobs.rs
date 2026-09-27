use mandrill_effect::{BlockingExecutor, CpuExecutor, Defect, Effect, HasBlockingExecutor, HasCpuExecutor, ScheduledJob};
use std::cell::Cell;

struct Workers;
impl CpuExecutor for Workers {
    fn submit(&self, job: ScheduledJob) -> Result<(), Defect> {
        drop(std::thread::spawn(move || job.run()));
        Ok(())
    }
}
impl BlockingExecutor for Workers {
    fn submit(&self, job: ScheduledJob) -> Result<(), Defect> {
        <Self as CpuExecutor>::submit(self, job)
    }
}
struct Environment { cpu: Workers, blocking: Workers }
impl HasCpuExecutor for Environment {
    type Executor = Workers;
    fn cpu_executor(&self) -> &Workers { &self.cpu }
}
impl HasBlockingExecutor for Environment {
    type Executor = Workers;
    fn blocking_executor(&self) -> &Workers { &self.blocking }
}
struct Data(Cell<usize>); // Neither Clone nor Sync.

fn main() {
    let _: Effect<Data, Data, Environment> = Effect::cpu(|_| Data(Cell::new(0)));
    let _: Effect<Data, Data, Environment> = Effect::try_cpu(|_| Err(Data(Cell::new(0))), |error| error);
    let data = Data(Cell::new(0));
    let _: Effect<Data, Data, Environment> = Effect::cpu_once(move |_| data);
    let error = Data(Cell::new(0));
    let extra = Cell::new(1);
    let _: Effect<Data, Data, Environment> = Effect::try_cpu_once(move |_| Err(error), move |error: Data| Data(Cell::new(error.0.get() + extra.get())));
    let _: Effect<Data, Data, Environment> = Effect::blocking(|_| Data(Cell::new(0)));
    let _: Effect<Data, Data, Environment> = Effect::try_blocking(|_| Err(Data(Cell::new(0))), |error| error);
    let data = Data(Cell::new(0));
    let _: Effect<Data, Data, Environment> = Effect::blocking_once(move |_| data);
    let error = Data(Cell::new(0));
    let _: Effect<Data, Data, Environment> = Effect::try_blocking_once(move |_| Err(error), |error| error);
    // Public object-safe providers do not impose a concrete backend choice.
    fn objects(_: &dyn CpuExecutor, _: &dyn BlockingExecutor) {}
    objects(&Workers, &Workers);
}
