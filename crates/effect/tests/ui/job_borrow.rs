use mandrill_effect::{CpuExecutor, Defect, Effect, HasCpuExecutor, ScheduledJob};
struct Worker;
impl CpuExecutor for Worker {
    fn submit(&self, job: ScheduledJob) -> Result<(), Defect> { drop(job); Ok(()) }
}
impl HasCpuExecutor for Worker {
    type Executor = Self;
    fn cpu_executor(&self) -> &Self { self }
}
fn borrow(input: &str) -> Effect<usize, (), Worker> {
    Effect::cpu_once(move |_| input.len())
}
fn main() {}
