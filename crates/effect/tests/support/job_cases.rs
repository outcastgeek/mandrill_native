use super::tasks::Threads;
use fx::{
    Cancellation, Effect, Exit, HasBlockingExecutor, HasCancellation, HasCpuExecutor,
    HasTaskSpawner, Scope,
};
use std::{cell::Cell, sync::Arc};

#[derive(Default)]
struct Environment {
    tasks: Threads,
    cpu: Threads,
    blocking: Threads,
    cancellation: Cancellation,
}
impl HasTaskSpawner for Environment {
    type Tasks = Threads;
    fn task_spawner(&self) -> &Threads {
        &self.tasks
    }
}
impl HasCpuExecutor for Environment {
    type Executor = Threads;
    fn cpu_executor(&self) -> &Threads {
        &self.cpu
    }
}
impl HasBlockingExecutor for Environment {
    type Executor = Threads;
    fn blocking_executor(&self) -> &Threads {
        &self.blocking
    }
}
impl HasCancellation for Environment {
    fn cancellation(&self) -> &Cancellation {
        &self.cancellation
    }
}
#[derive(Debug, PartialEq, Eq)]
struct Data(Cell<usize>); // Neither Clone nor Sync.

pub async fn job_cases() {
    let env = Arc::new(Environment::default());
    let workflow: Effect<Data, Data, Environment> = Scope::scoped(|scope| {
        fx::effect! {
            let scope = scope.clone();
            let fetched = Effect::from_future(|_| async { Data(Cell::new(3)) })?;
            let parsed = Effect::cpu_once(move |cancel| {
                assert!(!cancel.is_requested());
                fetched
            })?;
            let local = scope.fork(Effect::cpu_once(move |_| Data(Cell::new(parsed.0.get()))))?;
            let remote = Effect::from_future(|_| async { Data(Cell::new(2)) })?;
            let scored = local.join()?;
            let saved = Effect::blocking_once(move |_| Data(Cell::new(scored.0.get() + remote.0.get())))?;
            let result = saved.0.get();
            Effect::from_future(move |_| async move { Data(Cell::new(result)) })?
        }
    });
    for _ in 0..2 {
        assert_eq!(
            workflow.run_cancellable(Arc::clone(&env)).await,
            Exit::Success(Data(Cell::new(5)))
        );
    }
    let failure: Effect<Data, Data, Environment> =
        Effect::try_cpu_once(|_| Err(Data(Cell::new(7))), |error| error);
    assert_eq!(
        failure.run(Arc::clone(&env)).await,
        Exit::Fail(Data(Cell::new(7)))
    );
    // Two complete workflows: two CPU jobs, one blocking job and one child
    // each. The final expected-error operation adds one more CPU job.
    assert_eq!(env.cpu.submitted(), 5);
    assert_eq!(env.blocking.submitted(), 2);
    assert_eq!(env.tasks.submitted(), 2);
    env.tasks.finish();
    env.cpu.finish();
    env.blocking.finish();
}
