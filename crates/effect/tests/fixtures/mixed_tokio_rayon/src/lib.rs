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
        NonZeroUsize::new(value).expect("positive fixture capacity")
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
    ) -> (Arc<tokio::runtime::Runtime>, Arc<Env>) {
        let runtime = Arc::new(
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .max_blocking_threads(2)
                .enable_all()
                .build()
                .expect("fixture runtime"),
        );
        let pool = Arc::new(
            rayon::ThreadPoolBuilder::new()
                .num_threads(cpu_workers)
                .build()
                .expect("fixture pool"),
        );
        let env = Arc::new(Env {
            tasks: TokioTasks::new(Arc::clone(&runtime), AdmissionCapacity::Bounded(nz(8)))
                .expect("independent task provider"),
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
        (runtime, env)
    }
    #[test]
    fn reference_update_paths() {
        let (runtime, env) = fixture(2, 4);
        runtime.block_on(super::reference_update::exercise(env));
    }
    /// Execute the shared macro scenario with Tokio async/blocking and Rayon CPU providers.
    #[test]
    fn macro_composition() {
        let (runtime, env) = fixture(2, 4);
        runtime.block_on(super::composition::exercise(env));
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
            let mut open = self.open.lock().expect("gate");
            while !*open {
                open = self.changed.wait(open).expect("gate");
            }
        }
        fn release(&self) {
            *self.open.lock().expect("gate") = true;
            self.changed.notify_all();
        }
    }
    #[test]
    fn async_cpu_mixed_branches_and_blocking_resume_with_subtree_pool_override() {
        let (runtime, env) = fixture(2, 4);
        let entered = Arc::new(tokio::sync::Semaphore::new(0));
        let gate = Arc::new(Gate::default());
        let ready = Arc::clone(&entered);
        let work = Arc::clone(&gate);
        let cpu: Effect<usize, (), Env> = Effect::cpu(move |_| {
            ready.add_permits(1);
            work.wait();
            rayon::current_num_threads()
        });
        let progress = Arc::new(AtomicUsize::new(0));
        let progressed = Arc::clone(&progress);
        let signal = Arc::clone(&gate);
        let io = Effect::from_future(move |_| {
            let entered = Arc::clone(&entered);
            let signal = Arc::clone(&signal);
            let progressed = Arc::clone(&progressed);
            async move {
                let permits = entered.acquire_many(2).await.expect("workers admitted");
                permits.forget();
                progressed.fetch_add(1, Ordering::SeqCst);
                signal.release();
                10usize
            }
        });
        let alternate = RayonCpu::new(
            Arc::new(
                rayon::ThreadPoolBuilder::new()
                    .num_threads(3)
                    .build()
                    .expect("alternate pool"),
            ),
            AdmissionCapacity::Bounded(nz(2)),
            diagnostics(),
        );
        let projected = Effect::<usize, (), Env>::cpu(|_| rayon::current_num_threads())
            .provide_some(move |root: &Env| Env {
                cpu: alternate.clone(),
                ..root.clone()
            });
        let program: Effect<usize, (), Env> = Scope::scoped(move |scope| {
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
              let root_workers=Effect::cpu(|_| rayon::current_num_threads())?; assert_eq!(root_workers,2);
              let result=Effect::blocking_once(move |_| left+right+io)?;
              Effect::sleep(Duration::ZERO)?;
              result
            }
        });
        let outcome = runtime.block_on(async {
            tokio::time::timeout(Duration::from_secs(10), program.run(Arc::clone(&env)))
                .await
                .expect("mixed workflow settles")
        });
        assert_eq!(outcome, Exit::Success(14));
        assert_eq!(gate.entered.load(Ordering::SeqCst), 2);
        assert_eq!(progress.load(Ordering::SeqCst), 1);
        runtime.block_on(async {
            env.tasks.stop().wait().await;
            env.cpu.stop().wait().await;
            env.blocking.stop().wait().await;
        });
    }
    #[test]
    fn bounded_cpu_admission_queued_running_cancellation_and_shutdown_join() {
        let (runtime, env) = fixture(1, 2);
        let gate = Arc::new(Gate::default());
        let running = Arc::clone(&gate);
        let first: Effect<(), (), Env> = Effect::cpu_once(move |_| running.wait());
        let mut first = Box::pin(first.run_cancellable(Arc::clone(&env)));
        drive::until(first.as_mut(), || gate.entered.load(Ordering::SeqCst) == 1);
        let second_env = Arc::new(Env {
            cancel: Cancellation::new(),
            ..env.as_ref().clone()
        });
        let called = Arc::new(AtomicUsize::new(0));
        let executed = Arc::clone(&called);
        let second: Effect<(), (), Env> = Effect::cpu_once(move |_| {
            executed.fetch_add(1, Ordering::SeqCst);
        });
        let mut second = Box::pin(second.run_cancellable(Arc::clone(&second_env)));
        drive::until(second.as_mut(), || env.cpu.active() == 2);
        let rejected: Effect<(), (), Env> = Effect::cpu(|_| ());
        assert!(
            matches!(futures_executor::block_on(rejected.run(Arc::clone(&env))),Exit::Die(defect) if defect.kind()==DefectKind::Library(LibraryDefect::ProviderCapacityFull))
        );
        assert!(env.cancel.request());
        assert!(second_env.cancel.request());
        let mut drain = Box::pin(env.cpu.stop().wait());
        futures_executor::block_on(async {
            assert!(futures_util::poll!(drain.as_mut()).is_pending());
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
    }
    #[test]
    fn adapters_validate_independence_and_native_requests_preserve_cleanup() {
        let single = Arc::new(
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("single runtime"),
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
        let owner = native_async_runtime::RuntimeOwner::new(2, 4).expect("native owner");
        let tasks = effect_native_runtime::NativeTasks::new(owner.handle(), diagnostics())
            .expect("native adapter");
        struct NativeEnv(effect_native_runtime::NativeTasks);
        impl HasTaskSpawner for NativeEnv {
            type Tasks = effect_native_runtime::NativeTasks;
            fn task_spawner(&self) -> &Self::Tasks {
                &self.0
            }
        }
        let program: Effect<usize, (), NativeEnv> = Scope::scoped(|scope| {
            scope
                .fork(Effect::sync(|| 7))
                .flat_map(|fiber| fiber.join())
        });
        assert_eq!(
            owner.block_on_sync(program.run(Arc::new(NativeEnv(tasks)))),
            Exit::Success(7)
        );
        owner.block_on_sync(owner.stop().wait());
    }
    #[test]
    fn bounded_load_preserves_order_and_single_admission_reuse_has_no_retained_work() {
        let (runtime, env) = fixture(2, 4);
        let program = Effect::<usize, (), Env>::for_each_concurrent(
            nz(4),
            (0usize..256).collect(),
            |index| {
                let index = *index;
                effect! { Effect::sleep(Duration::ZERO)?; let value=Effect::cpu_once(move |_| std::hint::black_box(index).wrapping_add(1))?; Effect::sleep(Duration::ZERO)?; value }
            },
        );
        let outcome = runtime.block_on(async {
            tokio::time::timeout(Duration::from_secs(10), program.run(Arc::clone(&env)))
                .await
                .expect("bounded load settles")
        });
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
        .expect("single admission");
        let env = Arc::new(Env {
            tasks,
            ..env.as_ref().clone()
        });
        let program: Effect<(), (), Env> = Scope::scoped(
            |scope| effect! { let scope=scope.clone(); for index in 0..100 { let child=scope.fork(Effect::sync(move||index))?; let value=child.join()?; assert_eq!(value,index); } },
        );
        assert_eq!(
            runtime.block_on(program.run(Arc::clone(&env))),
            Exit::Success(())
        );
        runtime.block_on(env.tasks.stop().wait());
        assert_eq!(env.tasks.active(), 0);
    }
    #[test]
    fn vendor_jobs_keep_expected_errors_panics_and_async_branch_failures_distinct() {
        let (runtime, env) = fixture(2, 4);
        let expected: Effect<usize, usize, Env> = Effect::try_cpu(|_| Err(7usize), |error| error);
        assert_eq!(
            runtime.block_on(expected.run(Arc::clone(&env))),
            Exit::Fail(7)
        );
        let panic: Effect<usize, usize, Env> = Effect::cpu(|_| panic!("intentional worker panic"));
        assert!(
            matches!(runtime.block_on(panic.run(Arc::clone(&env))),Exit::Die(defect) if defect.kind()==DefectKind::CallbackPanic)
        );
        let branch: Effect<usize, usize, Env> =
            Effect::try_future(|_| async { Err::<usize, _>(11usize) }, |error| error);
        let program = Effect::all_concurrent(nz(2), vec![Effect::cpu(|_| 1), branch]);
        assert_eq!(
            runtime.block_on(program.run(Arc::clone(&env))),
            Exit::Fail(11)
        );
        runtime.block_on(async {
            env.tasks.stop().wait().await;
            env.cpu.stop().wait().await;
            env.blocking.stop().wait().await;
        });
    }
}
