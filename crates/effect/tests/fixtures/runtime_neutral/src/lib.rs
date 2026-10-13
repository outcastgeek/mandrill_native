#[cfg(test)]
#[path = "../../../support/portable_cases.rs"]
mod portable;

#[cfg(test)]
#[path = "../../../support/task_cases.rs"]
mod tasks;

#[cfg(test)]
#[path = "../../../support/parallel_cases.rs"]
mod parallel;

#[cfg(test)]
#[path = "../../../support/synchronization_cases.rs"]
mod synchronization;

#[cfg(test)]
#[path = "../../../support/queue_cases.rs"]
mod queue;

#[cfg(test)]
#[path = "../../../support/job_cases.rs"]
mod jobs;

#[test]
fn external_task_provider_owns_non_clone_outcomes() {
    futures_executor::block_on(tasks::task_cases());
}

#[test]
fn same_workflows_on_independent_executor() {
    portable::assert_send_future(portable::workflow().run(std::sync::Arc::new(
        portable::Environment {
            offset: 1,
            ..portable::Environment::default()
        },
    )));
    futures_executor::block_on(portable::portable_cases());
}

#[test]
fn parallel_operations_share_inputs_and_move_outcomes() {
    futures_executor::block_on(parallel::parallel_cases());
}

#[test]
fn synchronization_uses_non_clone_state_and_shared_outcomes() {
    futures_executor::block_on(synchronization::synchronization_cases());
}

#[test]
fn queues_move_payloads_through_scoped_rendezvous_and_buffer_drain() {
    futures_executor::block_on(queue::queue_cases());
}

#[test]
fn cpu_blocking_and_async_operations_share_one_environment() {
    futures_executor::block_on(jobs::job_cases());
}

#[cfg(test)]
#[path = "../../shared/reference_update.rs"]
mod reference_update;
#[test]
fn reference_update_with_renamed_dependency() {
    struct Env {
        tasks: tasks::Threads,
        clock: fx::TestClock,
    }
    impl fx::HasTaskSpawner for Env {
        type Tasks = tasks::Threads;
        fn task_spawner(&self) -> &Self::Tasks {
            &self.tasks
        }
    }
    impl fx::HasClock for Env {
        type Clock = fx::TestClock;
        fn clock(&self) -> &Self::Clock {
            &self.clock
        }
    }
    let env = std::sync::Arc::new(Env {
        tasks: tasks::Threads::default(),
        clock: fx::TestClock::default(),
    });
    let outcome =
        futures_executor::block_on(reference_update::exercise(std::sync::Arc::clone(&env)));
    env.tasks.finish();
    assert!(outcome.is_ok(), "{outcome:?}");
}

/// Shared composition scenario consumes the renamed dependency and independent providers.
#[cfg(test)]
#[path = "../../shared/macro_composition.rs"]
mod composition;

/// Run the same macros with independent async, CPU and blocking thread providers.
#[test]
fn macro_composition() {
    /// One explicit environment selects all fixture runtime capabilities.
    struct Env {
        /// Existing thread provider, shared as the async/CPU/blocking authority.
        tasks: tasks::Threads,
        /// Existing controllable monotonic clock.
        clock: fx::TestClock,
    }
    impl fx::HasTaskSpawner for Env {
        /// Existing independent task provider type.
        type Tasks = tasks::Threads;
        /// Borrow the same selected provider.
        fn task_spawner(&self) -> &Self::Tasks {
            &self.tasks
        }
    }
    impl fx::HasCpuExecutor for Env {
        /// Existing independent job provider type.
        type Executor = tasks::Threads;
        /// Borrow the same selected provider.
        fn cpu_executor(&self) -> &Self::Executor {
            &self.tasks
        }
    }
    impl fx::HasBlockingExecutor for Env {
        /// Existing independent blocking job provider type.
        type Executor = tasks::Threads;
        /// Borrow the same selected provider.
        fn blocking_executor(&self) -> &Self::Executor {
            &self.tasks
        }
    }
    impl fx::HasClock for Env {
        /// Existing test clock type.
        type Clock = fx::TestClock;
        /// Borrow the explicit clock capability.
        fn clock(&self) -> &Self::Clock {
            &self.clock
        }
    }
    let env = std::sync::Arc::new(Env {
        tasks: tasks::Threads::default(),
        clock: fx::TestClock::default(),
    });
    let outcome = futures_executor::block_on(composition::exercise(std::sync::Arc::clone(&env)));
    env.tasks.finish();
    assert!(outcome.is_ok(), "{outcome:?}");
}
