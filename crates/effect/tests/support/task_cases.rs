use fx::{effect, effect_map, effect_scope, effect_sync, effect_try_sync};
use fx::{
    BlockingExecutor, Cancellation, CpuExecutor, Defect, Effect, Exit, HasCancellation,
    HasTaskSpawner, ScheduledJob, ScheduledTask, TaskSpawner,
};
use std::cell::Cell;
use std::sync::{Arc, Mutex};

/// External conformance fixture: each finite child gets its own independent OS
/// worker. This is test machinery, not an unbounded production pool/default.
#[derive(Default)]
pub struct Threads {
    workers: Mutex<Vec<std::thread::JoinHandle<()>>>,
}

impl TaskSpawner for Threads {
    fn spawn(&self, task: ScheduledTask) -> Result<(), Defect> {
        let worker = std::thread::Builder::new()
            .spawn(move || futures_executor::block_on(task))
            .map_err(|error| Defect::new(error.to_string()))?;
        self.workers
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(worker);
        Ok(())
    }
}

impl Threads {
    fn submit_job(&self, job: ScheduledJob) -> Result<(), Defect> {
        let worker = std::thread::Builder::new()
            .spawn(move || assert!(job.run().is_ok()))
            .map_err(|error| Defect::new(error.to_string()))?;
        self.workers
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(worker);
        Ok(())
    }
}
impl CpuExecutor for Threads {
    fn submit(&self, job: ScheduledJob) -> Result<(), Defect> {
        self.submit_job(job)
    }
}
impl BlockingExecutor for Threads {
    fn submit(&self, job: ScheduledJob) -> Result<(), Defect> {
        self.submit_job(job)
    }
}

impl Threads {
    pub fn submitted(&self) -> usize {
        self.workers
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .len()
    }

    pub fn settled(&self) -> usize {
        self.workers
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .iter()
            .filter(|worker| worker.is_finished())
            .count()
    }

    pub fn finish(&self) {
        let workers = std::mem::take(
            &mut *self
                .workers
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        );
        for worker in workers {
            assert!(
                worker.join().is_ok(),
                "fixture worker escaped task panic capture"
            );
        }
    }
}

#[derive(Default)]
pub struct Environment {
    pub tasks: Threads,
    pub cancellation: Cancellation,
}
impl HasTaskSpawner for Environment {
    type Tasks = Threads;
    fn task_spawner(&self) -> &Threads {
        &self.tasks
    }
}
impl HasCancellation for Environment {
    fn cancellation(&self) -> &Cancellation {
        &self.cancellation
    }
}

/// Neither Clone nor Sync: these bounds must not be smuggled into observations.
#[derive(Debug, PartialEq, Eq)]
pub struct Payload(pub Cell<Marker>);
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Marker {
    Value,
    Error,
}

pub async fn task_cases() {
    let env = Arc::new(Environment::default());
    let owned: Effect<Payload, Payload, Environment> = effect_scope! {
    body: |scope| {
            effect! {
     bind;
     program: scope
                .fork(effect_sync! { body: || Payload(Cell::new(Marker::Value)), }),
     body: |fiber| fiber.join(),
     }
        },
    };
    for _ in 0..2 {
        // Two executions verify fresh outcomes, not copies of a stored payload.
        assert_eq!(
            owned.run(Arc::clone(&env)).await,
            Exit::Success(Payload(Cell::new(Marker::Value)))
        );
    }
    let shared: Effect<(), Payload, Environment> = effect_scope! {
    body: |scope| {
            effect! {
     bind;
     program: scope
                .fork({ let __dsl_program: Effect < () , Payload , Environment > = effect_try_sync! { body: { let __dsl_error = || {
                    Payload(Cell::new(Marker::Error))
                }; move || Err(__dsl_error()) }, on_error: ::core::convert::identity, }; __dsl_program }),
     body: |fiber| {
                    let repeated = fiber.clone();
                    effect! {
     bind;
     program: fiber.await_exit_shared(),
     body: move |first| {
                        effect_map! { program: repeated.await_exit_shared(), map: move |second| {
                            let first_address = first.with(|exit| match exit {
                                Exit::Fail(error) => Some(std::ptr::from_ref(error)),
                                Exit::Success(())
                                | Exit::Die(_)
                                | Exit::Interrupted
                                | Exit::FinalizationFailed(_) => None,
                            });
                            let second_address = second.with(|exit| match exit {
                                Exit::Fail(error) => Some(std::ptr::from_ref(error)),
                                Exit::Success(())
                                | Exit::Die(_)
                                | Exit::Interrupted
                                | Exit::FinalizationFailed(_) => None,
                            });
                            assert!(matches!(first_address, Ok(Some(_))));
                            assert_eq!(first_address, second_address);
                        }, }
                    },
     }
                },
     }
        },
    };
    // Observed expected failure is handled here, not reported again at close.
    assert_eq!(shared.run(Arc::clone(&env)).await, Exit::Success(()));
    // Two owned runs and one shared workflow each submit exactly one child.
    assert_eq!(env.tasks.submitted(), 3);
    env.tasks.finish();
    assert_eq!(env.tasks.settled(), 0); // The explicit drain retains no worker handles.
}
