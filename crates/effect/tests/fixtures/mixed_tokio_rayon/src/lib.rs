extern crate effect as fx;
/// Shared composition scenario used unchanged by both provider consumers.
#[cfg(test)]
#[path = "../../shared/macro_composition.rs"]
mod composition;
#[cfg(test)]
#[path = "../../../support/drive.rs"]
mod drive;
#[cfg(test)]
#[path = "../../shared/reference_update.rs"]
mod reference_update;
#[cfg(test)]
mod tests {
    use futures_util::poll;

    use super::drive;
    use effect::*;
    use effect_rayon::RayonCpu;
    use effect_tokio::{TokioBlocking, TokioClock, TokioTasks};
    use std::num::NonZeroUsize;
    use std::sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicUsize, Ordering},
    };
    use std::time::Duration;

    #[derive(Clone)]
    struct Env {
        tasks: TokioTasks,
        cpu: RayonCpu,
        blocking: TokioBlocking,
        clock: TokioClock,
        cancel: Cancellation,
    }
    impl HasTaskSpawner for Env {
        type Tasks = TokioTasks;
        fn task_spawner(&self) -> &TokioTasks {
            &self.tasks
        }
    }
    impl HasCpuExecutor for Env {
        type Executor = RayonCpu;
        fn cpu_executor(&self) -> &RayonCpu {
            &self.cpu
        }
    }
    impl HasBlockingExecutor for Env {
        type Executor = TokioBlocking;
        fn blocking_executor(&self) -> &TokioBlocking {
            &self.blocking
        }
    }
    impl HasClock for Env {
        type Clock = TokioClock;
        fn clock(&self) -> &TokioClock {
            &self.clock
        }
    }
    impl HasCancellation for Env {
        fn cancellation(&self) -> &Cancellation {
            &self.cancel
        }
    }
    fn nz(value: usize) -> NonZeroUsize {
        assert!(value > 0, "positive fixture capacity");
        NonZeroUsize::MIN.saturating_add(value - 1)
    }
    fn diagnostics() -> Diagnostics {
        Diagnostics::new(
            DiagnosticCapacity::Unbounded,
            DiagnosticOverflow::DiscardNewest,
        )
    }
    fn fixture(
        cpu_workers: usize,
        cpu_capacity: usize,
    ) -> Result<(Arc<tokio::runtime::Runtime>, Arc<Env>), String> {
        let runtime = Arc::new(
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .max_blocking_threads(2)
                .enable_all()
                .build()
                .map_err(|error| format!("fixture runtime: {error}"))?,
        );
        let pool = Arc::new(
            rayon::ThreadPoolBuilder::new()
                .num_threads(cpu_workers)
                .build()
                .map_err(|error| format!("fixture pool: {error}"))?,
        );
        let env = Arc::new(Env {
            tasks: TokioTasks::new(Arc::clone(&runtime), AdmissionCapacity::Bounded(nz(8)))
                .map_err(|error| format!("independent task provider: {error:?}"))?,
            cpu: RayonCpu::new(
                pool,
                AdmissionCapacity::Bounded(nz(cpu_capacity)),
                diagnostics(),
            ),
            blocking: TokioBlocking::new(
                Arc::clone(&runtime),
                AdmissionCapacity::Bounded(nz(2)),
                diagnostics(),
            ),
            clock: TokioClock::new(Arc::clone(&runtime)),
            cancel: Cancellation::new(),
        });
        Ok((runtime, env))
    }
    #[test]
    fn reference_update_paths() -> Result<(), String> {
        let (runtime, env) = fixture(2, 4)?;
        let outcome = runtime.block_on(super::reference_update::exercise(env));
        assert!(outcome.is_ok(), "{outcome:?}");
        Ok(())
    }
    /// Execute the shared macro scenario with Tokio async/blocking and Rayon CPU providers.
    #[test]
    fn macro_composition() -> Result<(), String> {
        let (runtime, env) = fixture(2, 4)?;
        let outcome = runtime.block_on(super::composition::exercise(env));
        assert!(outcome.is_ok(), "{outcome:?}");
        Ok(())
    }
    #[derive(Default)]
    struct Gate {
        entered: AtomicUsize,
        open: Mutex<bool>,
        changed: Condvar,
    }
    impl Gate {
        fn wait(&self) {
            self.entered.fetch_add(1, Ordering::SeqCst);
            let guard = self.open.lock();
            assert!(guard.is_ok(), "gate lock poisoned");
            let mut open = guard.unwrap_or_else(std::sync::PoisonError::into_inner);
            while !*open {
                let guard = self.changed.wait(open);
                assert!(guard.is_ok(), "gate wait poisoned");
                open = guard.unwrap_or_else(std::sync::PoisonError::into_inner);
            }
        }
        fn release(&self) {
            let guard = self.open.lock();
            assert!(guard.is_ok(), "gate lock poisoned");
            *guard.unwrap_or_else(std::sync::PoisonError::into_inner) = true;
            self.changed.notify_all();
        }
    }
    #[test]
    fn async_cpu_mixed_branches_and_blocking_resume_with_subtree_pool_override()
    -> Result<(), String> {
        let (runtime, env) = fixture(2, 4)?;
        let entered = Arc::new(tokio::sync::Semaphore::new(0));
        let gate = Arc::new(Gate::default());
        let ready = Arc::clone(&entered);
        let work = Arc::clone(&gate);
        let cpu: Effect<usize, (), Env> = effect_cpu! { body: move |_| {
            ready.add_permits(1);
            work.wait();
            rayon::current_num_threads()
        }, };
        let progress = Arc::new(AtomicUsize::new(0));
        let progressed = Arc::clone(&progress);
        let signal = Arc::clone(&gate);
        let io = effect! {
 bind;
 program: effect_future! { body: move |_| {
            let entered = Arc::clone(&entered);
            let signal = Arc::clone(&signal);
            let progressed = Arc::clone(&progressed);
            async move {
                let permits = entered
                    .acquire_many(2)
                    .await
                    .map_err(|error| Defect::new(format!("workers admitted: {error}")))?;
                permits.forget();
                progressed.fetch_add(1, Ordering::SeqCst);
                signal.release();
                Ok::<usize, Defect>(10usize)
            }
        }, },
 body: |result| match result {
            Ok(value) => effect_sync! { once; body: move || value, },
            Err(defect) => effect_die! { defect: defect, },
        },
 };
        let alternate = RayonCpu::new(
            Arc::new(
                rayon::ThreadPoolBuilder::new()
                    .num_threads(3)
                    .build()
                    .map_err(|error| format!("alternate pool: {error}"))?,
            ),
            AdmissionCapacity::Bounded(nz(2)),
            diagnostics(),
        );
        let projected = effect_provide_some! { program: { let __dsl_program: Effect < usize , () , Env > = effect_cpu! { body: |_| rayon::current_num_threads(), }; __dsl_program }, project: move |root: &Env| Env {
                cpu: alternate.clone(),
                ..root.clone()
            }, };
        let program: Effect<usize, (), Env> = effect_scope! {
body: move |scope| {
            let cpu = cpu.clone();
            let io = io.clone();
            let projected = projected.clone();
            effect! {
              let scope=scope.clone(); let cpu=cpu.clone(); let io=io.clone(); let projected=projected.clone();
              Effect::sleep(Duration::ZERO)?;
              let a=scope.fork(cpu.clone())?;
              let b=scope.fork(cpu.clone())?;
              let c=scope.fork(io)?;
              let left=a.join()?; let right=b.join()?; let io=c.join()?;
              assert_eq!((left,right),(2,2));
              let override_workers=projected?; assert_eq!(override_workers,3);
              let root_workers=effect_cpu! { body: |_| rayon::current_num_threads(), }?; assert_eq!(root_workers,2);
              let result=effect_blocking! { once; body: move |_| left+right+io, }?;
              Effect::sleep(Duration::ZERO)?;
              result
            }
        },
};
        let outcome = runtime
            .block_on(async {
                tokio::time::timeout(Duration::from_secs(10), program.run(Arc::clone(&env))).await
            })
            .map_err(|error| format!("mixed workflow settles: {error}"))?;
        assert_eq!(outcome, Exit::Success(14));
        assert_eq!(gate.entered.load(Ordering::SeqCst), 2);
        assert_eq!(progress.load(Ordering::SeqCst), 1);
        runtime.block_on(async {
            env.tasks.stop().wait().await;
            env.cpu.stop().wait().await;
            env.blocking.stop().wait().await;
        });
        Ok(())
    }
    #[test]
    fn bounded_cpu_admission_queued_running_cancellation_and_shutdown_join() -> Result<(), String> {
        let (runtime, env) = fixture(1, 2)?;
        let gate = Arc::new(Gate::default());
        let running = Arc::clone(&gate);
        let first: Effect<(), (), Env> = effect_cpu! { once; body: move |_| running.wait(), };
        let mut first = Box::pin(first.run_cancellable(Arc::clone(&env)));
        drive::until(first.as_mut(), || gate.entered.load(Ordering::SeqCst) == 1);
        let second_env = Arc::new(Env {
            cancel: Cancellation::new(),
            ..env.as_ref().clone()
        });
        let called = Arc::new(AtomicUsize::new(0));
        let executed = Arc::clone(&called);
        let second: Effect<(), (), Env> = effect_cpu! { once; body: move |_| {
            executed.fetch_add(1, Ordering::SeqCst);
        }, };
        let mut second = Box::pin(second.run_cancellable(Arc::clone(&second_env)));
        drive::until(second.as_mut(), || env.cpu.active() == 2);
        let rejected: Effect<(), (), Env> = effect_cpu! { body: |_| (), };
        assert!(
            matches!(futures_executor::block_on(rejected.run(Arc::clone(&env))),Exit::Die(defect) if defect.kind()==DefectKind::Library(LibraryDefect::ProviderCapacityFull))
        );
        assert!(env.cancel.request());
        assert!(second_env.cancel.request());
        let mut drain = Box::pin(env.cpu.stop().wait());
        futures_executor::block_on(async {
            assert!(poll!(drain.as_mut()).is_pending());
        });
        gate.release();
        assert_eq!(futures_executor::block_on(first), Exit::Interrupted);
        assert_eq!(futures_executor::block_on(second), Exit::Interrupted);
        futures_executor::block_on(drain);
        assert_eq!(called.load(Ordering::SeqCst), 0);
        assert_eq!(env.cpu.active(), 0);
        assert!(
            matches!(futures_executor::block_on(rejected.run(env)),Exit::Die(defect) if defect.kind()==DefectKind::Library(LibraryDefect::ProviderClosed))
        );
        drop(runtime);
        Ok(())
    }
    #[test]
    fn adapters_validate_independence_and_native_requests_preserve_cleanup() -> Result<(), String> {
        let single = Arc::new(
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|error| format!("single runtime: {error}"))?,
        );
        assert!(
            matches!(TokioTasks::new(single,AdmissionCapacity::Unbounded),Err(defect) if defect.kind()==DefectKind::Library(LibraryDefect::ProviderWorkersInsufficient))
        );
        let native = native_async_runtime::Cancellation::new();
        let cancel = Cancellation::new();
        assert!(native.request());
        let expected: Exit<(), usize> = Exit::FinalizationFailed(Finalization {
            primary: PrimaryOutcome::WasInterrupted,
            first: CleanupFailure::ReleaseFailure(7),
            rest: vec![CleanupFailure::ReleaseInterrupted],
        });
        let outcome = futures_executor::block_on(effect_native_runtime::linked(
            Arc::clone(&native),
            cancel.clone(),
            async { expected },
        ));
        assert!(cancel.is_requested());
        let total = effect_native_runtime::complete(
            outcome,
            |()| 0,
            |_| 0,
            |_| 0,
            || 0,
            |details| {
                assert_eq!(details.first, CleanupFailure::ReleaseFailure(7));
                details.rest.len()
            },
        );
        assert_eq!(total, 1);
        let owner = native_async_runtime::RuntimeOwner::new(2, 4)
            .map_err(|error| format!("native owner: {error:?}"))?;
        let tasks = effect_native_runtime::NativeTasks::new(owner.handle(), diagnostics())
            .map_err(|error| format!("native adapter: {error:?}"))?;
        struct NativeEnv(effect_native_runtime::NativeTasks);
        impl HasTaskSpawner for NativeEnv {
            type Tasks = effect_native_runtime::NativeTasks;
            fn task_spawner(&self) -> &Self::Tasks {
                &self.0
            }
        }
        let program: Effect<usize, (), NativeEnv> = effect_scope! {
body: |scope| {
            effect! {
 bind;
 program: scope
                .fork(effect_sync! { body: || 7, }),
 body: |fiber| fiber.join(),
 }
        },
};
        assert_eq!(
            owner.block_on_sync(program.run(Arc::new(NativeEnv(tasks)))),
            Exit::Success(7)
        );
        owner.block_on_sync(owner.stop().wait());
        Ok(())
    }
    #[test]
    fn bounded_load_preserves_order_and_single_admission_reuse_has_no_retained_work()
    -> Result<(), String> {
        let (runtime, env) = fixture(2, 4)?;
        let program = { let __dsl_program: Effect < :: std :: vec :: Vec < usize > , () , Env > = effect_for_each_concurrent! { concurrency: nz(4), values: (0usize..256).collect(), body: |index| {
                let index = *index;
                effect! { Effect::sleep(Duration::ZERO)?; let value=effect_cpu! { once; body: move |_| std::hint::black_box(index).wrapping_add(1), }?; Effect::sleep(Duration::ZERO)?; value }
            }, }; __dsl_program };
        let outcome = runtime
            .block_on(async {
                tokio::time::timeout(Duration::from_secs(10), program.run(Arc::clone(&env))).await
            })
            .map_err(|error| format!("bounded load settles: {error}"))?;
        assert_eq!(outcome, Exit::Success((1..=256).collect()));
        runtime.block_on(async {
            env.tasks.stop().wait().await;
            env.cpu.stop().wait().await;
            env.blocking.stop().wait().await;
        });
        assert_eq!(env.tasks.active(), 0);
        assert_eq!(env.cpu.active(), 0);
        assert_eq!(env.blocking.active(), 0);
        let tasks = TokioTasks::new(
            Arc::clone(&runtime),
            AdmissionCapacity::Bounded(NonZeroUsize::MIN),
        )
        .map_err(|error| format!("single admission: {error:?}"))?;
        let env = Arc::new(Env {
            tasks,
            ..env.as_ref().clone()
        });
        let program: Effect<(), (), Env> = effect_scope! {
body: |scope| effect! { let scope=scope.clone(); for index in 0..100 { let child=scope.fork(effect_sync! { body: move||index, })?; let value=child.join()?; assert_eq!(value,index); } },
};
        assert_eq!(
            runtime.block_on(program.run(Arc::clone(&env))),
            Exit::Success(())
        );
        runtime.block_on(env.tasks.stop().wait());
        assert_eq!(env.tasks.active(), 0);
        Ok(())
    }
    #[test]
    fn vendor_jobs_keep_expected_errors_panics_and_async_branch_failures_distinct()
    -> Result<(), String> {
        let (runtime, env) = fixture(2, 4)?;
        let expected: Effect<usize, usize, Env> = effect_try_cpu! { body: |_| Err(7usize), on_error: |error| error, };
        assert_eq!(
            runtime.block_on(expected.run(Arc::clone(&env))),
            Exit::Fail(7)
        );
        let panic: Effect<usize, usize, Env> =
            effect_cpu! { body: |_| std::panic::resume_unwind(Box::new("intentional worker panic")), };
        assert!(
            matches!(runtime.block_on(panic.run(Arc::clone(&env))),Exit::Die(defect) if defect.kind()==DefectKind::CallbackPanic)
        );
        let branch: Effect<usize, usize, Env> =
            effect_try_future! { body: |_| async { Err::<usize, _>(11usize) }, on_error: |error| error, };
        let program = effect_all_concurrent! { concurrency: nz(2), items: vec![effect_cpu! { body: |_| 1, }, branch], };
        assert_eq!(
            runtime.block_on(program.run(Arc::clone(&env))),
            Exit::Fail(11)
        );
        runtime.block_on(async {
            env.tasks.stop().wait().await;
            env.cpu.stop().wait().await;
            env.blocking.stop().wait().await;
        });
        Ok(())
    }
}
