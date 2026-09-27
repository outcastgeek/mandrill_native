extern crate effect as fx;

use effect::{
    BlockingExecutor, Cancellation, CleanupFailure, CpuExecutor, Defect, DefectKind, Effect, Exit,
    HasBlockingExecutor, HasCancellation, HasCpuExecutor, HasTaskSpawner, LibraryDefect,
    PrimaryOutcome, ScheduledJob, ScheduledTask, Scope, TaskSpawner,
};
use futures_executor::block_on;
use std::cell::Cell;
use std::sync::{
    Arc, Condvar, Mutex,
    atomic::{AtomicUsize, Ordering},
};

#[path = "support/drive.rs"]
mod drive;
#[path = "support/job_cases.rs"]
mod portable;
#[path = "support/suspended.rs"]
mod suspension;
#[path = "support/task_cases.rs"]
pub mod tasks;

#[test]
fn external_cpu_blocking_and_async_workflows_compose_with_scoped_children() {
    block_on(portable::job_cases());
}

#[derive(Clone, Copy)]
enum Mode {
    Queued,
    Threads,
    Reject,
    Lose,
    Panic,
}
struct Workers {
    mode: Mode,
    queued: Mutex<Vec<ScheduledJob>>,
    threads: Mutex<Vec<std::thread::JoinHandle<()>>>,
}
impl Workers {
    fn new(mode: Mode) -> Self {
        Self {
            mode,
            queued: Mutex::new(Vec::new()),
            threads: Mutex::new(Vec::new()),
        }
    }
    #[allow(clippy::panic)] // Intentional provider-panic fixture.
    fn submit_job(&self, job: ScheduledJob) -> Result<(), Defect> {
        match self.mode {
            Mode::Queued => {
                self.queued
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .push(job);
                Ok(())
            }
            Mode::Threads => {
                let worker = std::thread::Builder::new()
                    .spawn(move || assert!(job.run().is_ok()))
                    .map_err(|error| Defect::new(error.to_string()))?;
                self.threads
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .push(worker);
                Ok(())
            }
            Mode::Reject => {
                drop(job);
                Err(Defect::from(LibraryDefect::JobSubmissionRejected))
            }
            Mode::Lose => {
                drop(job);
                Ok(())
            }
            Mode::Panic => {
                drop(job);
                panic!("intentional worker admission panic")
            }
        }
    }
    #[allow(clippy::panic)] // Missing queued work is a failed fixture assertion.
    fn take(&self) -> ScheduledJob {
        match self
            .queued
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .pop()
        {
            Some(job) => job,
            None => panic!("fixture expected queued work"),
        }
    }
    fn count(&self) -> usize {
        self.queued
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .len()
    }
    fn finish(&self) {
        let workers = std::mem::take(
            &mut *self
                .threads
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        );
        for worker in workers {
            assert!(worker.join().is_ok());
        }
    }
}
impl CpuExecutor for Workers {
    fn submit(&self, job: ScheduledJob) -> Result<(), Defect> {
        self.submit_job(job)
    }
}
impl BlockingExecutor for Workers {
    fn submit(&self, job: ScheduledJob) -> Result<(), Defect> {
        self.submit_job(job)
    }
}
struct Environment {
    cpu: Arc<Workers>,
    blocking: Arc<Workers>,
    cancellation: Cancellation,
}
impl Environment {
    fn new(mode: Mode) -> Self {
        Self {
            cpu: Arc::new(Workers::new(mode)),
            blocking: Arc::new(Workers::new(mode)),
            cancellation: Cancellation::new(),
        }
    }
    fn finish(&self) {
        self.cpu.finish();
        self.blocking.finish();
    }
    fn projection(&self) -> Self {
        Self {
            cpu: Arc::clone(&self.cpu),
            blocking: Arc::clone(&self.blocking),
            cancellation: self.cancellation.clone(),
        }
    }
}
impl HasCpuExecutor for Environment {
    type Executor = Workers;
    fn cpu_executor(&self) -> &Workers {
        &self.cpu
    }
}
impl HasBlockingExecutor for Environment {
    type Executor = Workers;
    fn blocking_executor(&self) -> &Workers {
        &self.blocking
    }
}
impl HasCancellation for Environment {
    fn cancellation(&self) -> &Cancellation {
        &self.cancellation
    }
}
impl TaskSpawner for Workers {
    fn spawn(&self, task: ScheduledTask) -> Result<(), Defect> {
        let worker = std::thread::Builder::new()
            .spawn(move || block_on(task))
            .map_err(|error| Defect::new(error.to_string()))?;
        self.threads
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(worker);
        Ok(())
    }
}
impl HasTaskSpawner for Environment {
    type Tasks = Workers;
    fn task_spawner(&self) -> &Workers {
        &self.blocking
    }
}

#[derive(Debug, PartialEq, Eq)]
struct Data(Cell<usize>); // Neither Clone nor Sync.

struct DropCount(Arc<AtomicUsize>);
impl Drop for DropCount {
    fn drop(&mut self) {
        let _ = self.0.fetch_add(1, Ordering::SeqCst);
    }
}
struct Gate(Mutex<bool>, Condvar);
impl Gate {
    fn new() -> Self {
        Self(Mutex::new(false), Condvar::new())
    }
    fn wait(&self) {
        let mut open = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        while !*open {
            open = self
                .1
                .wait(open)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
        }
    }
    fn open(&self) {
        *self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = true;
        self.1.notify_all();
    }
}

#[test]
fn cpu_and_blocking_move_fresh_non_clone_non_sync_results_on_independent_workers() {
    let env = Arc::new(Environment::new(Mode::Threads));
    let caller = std::thread::current().id();
    let program: Effect<Data, Data, Environment> = effect::effect! {
        let data = Effect::cpu(move |_| {
            assert_ne!(std::thread::current().id(), caller);
            Data(Cell::new(7))
        })?;
        Effect::blocking_once(move |_| Data(Cell::new(data.0.get() * 2)))?
    };
    for _ in 0..2 {
        assert_eq!(
            block_on(program.run(Arc::clone(&env))),
            Exit::Success(Data(Cell::new(14)))
        );
    }
    env.finish();
}

#[test]
fn admission_resolves_the_current_cpu_and_blocking_capabilities_separately_and_lazily() {
    let env = Arc::new(Environment::new(Mode::Queued));
    let cpu: Effect<Data, Data, Environment> = Effect::cpu(|_| Data(Cell::new(1)));
    let blocking: Effect<Data, Data, Environment> = Effect::blocking(|_| Data(Cell::new(2)));
    assert_eq!(env.cpu.count(), 0);
    assert_eq!(env.blocking.count(), 0);
    let mut first = Box::pin(cpu.run(Arc::clone(&env)));
    let mut second = Box::pin(blocking.run(Arc::clone(&env)));
    suspension::suspended(first.as_mut());
    suspension::suspended(second.as_mut());
    assert_eq!(env.cpu.count(), 1);
    assert_eq!(env.blocking.count(), 1);
    let job = env.cpu.take();
    assert!(!job.is_cancelled()); // The pending future still owns admission.
    let job = match job.cancel_before_start() {
        Err(job) => job,
        Ok(_) => unreachable_fixture(),
    };
    assert!(job.run().is_ok());
    assert!(env.blocking.take().run().is_ok());
    assert_eq!(block_on(first), Exit::Success(Data(Cell::new(1))));
    assert_eq!(block_on(second), Exit::Success(Data(Cell::new(2))));
}

#[allow(clippy::panic)]
fn unreachable_fixture<T>() -> T {
    panic!("fixture contract was violated")
}

#[test]
fn owned_callbacks_consume_input_without_clone_or_sync_and_precancellation_retains_them() {
    let env = Arc::new(Environment::new(Mode::Threads));
    let _ = env.cancellation.request();
    let input = Data(Cell::new(3));
    let program: Effect<Data, Data, Environment> = Effect::cpu_once(move |_| input);
    assert_eq!(block_on(program.run_cancellable(env)), Exit::Interrupted);
    let fresh = Arc::new(Environment::new(Mode::Threads));
    assert_eq!(
        block_on(program.run(Arc::clone(&fresh))),
        Exit::Success(Data(Cell::new(3)))
    );
    assert_eq!(
        block_on(program.run(Arc::clone(&fresh))),
        Exit::Die(Defect::from(LibraryDefect::JobCallbackConsumed))
    );
    fresh.finish();
}

#[test]
fn expected_errors_and_owned_error_mapping_move_on_the_selected_worker() {
    let env = Arc::new(Environment::new(Mode::Threads));
    let caller = std::thread::current().id();
    let cpu: Effect<Data, Data, Environment> = Effect::try_cpu(
        |_| Err(Data(Cell::new(5))),
        move |error: Data| {
            assert_ne!(std::thread::current().id(), caller);
            error
        },
    );
    assert_eq!(
        block_on(cpu.run(Arc::clone(&env))),
        Exit::Fail(Data(Cell::new(5)))
    );
    assert_eq!(
        block_on(cpu.run(Arc::clone(&env))),
        Exit::Fail(Data(Cell::new(5)))
    );
    let input = Data(Cell::new(9));
    let extra = Cell::new(1); // Non-Sync capture in a single-use mapper.
    let blocking: Effect<Data, Data, Environment> = Effect::try_blocking_once(
        move |_| Err(input),
        move |error: Data| Data(Cell::new(error.0.get() + extra.get())),
    );
    assert_eq!(
        block_on(blocking.run(Arc::clone(&env))),
        Exit::Fail(Data(Cell::new(10)))
    );
    let other: Effect<Data, Data, Environment> =
        Effect::try_blocking(|_| Err(Data(Cell::new(6))), |error| error);
    assert_eq!(
        block_on(other.run(Arc::clone(&env))),
        Exit::Fail(Data(Cell::new(6)))
    );
    let last: Effect<Data, Data, Environment> =
        Effect::try_cpu_once(|_| Err(Data(Cell::new(8))), |error| error);
    assert_eq!(
        block_on(last.run(Arc::clone(&env))),
        Exit::Fail(Data(Cell::new(8)))
    );
    env.finish();
}

#[test]
fn queued_cancellation_retires_captures_before_interruption_without_starting_user_work() {
    let env = Arc::new(Environment::new(Mode::Queued));
    let calls = Arc::new(AtomicUsize::new(0));
    let called = Arc::clone(&calls);
    let drops = Arc::new(AtomicUsize::new(0));
    let capture = DropCount(Arc::clone(&drops));
    let program: Effect<Data, Data, Environment> = Effect::cpu_once(move |_| {
        drop(capture);
        let _ = called.fetch_add(1, Ordering::SeqCst);
        Data(Cell::new(0))
    });
    let mut future = Box::pin(program.run_cancellable(Arc::clone(&env)));
    suspension::suspended(future.as_mut());
    let _ = env.cancellation.request();
    suspension::suspended(future.as_mut()); // Cancellation still awaits ownership settlement.
    let job = env.cpu.take();
    assert!(job.is_cancelled());
    assert!(matches!(job.cancel_before_start(), Ok(Ok(()))));
    assert_eq!(drops.load(Ordering::SeqCst), 1);
    assert_eq!(block_on(future), Exit::Interrupted);
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[test]
fn running_cancellation_waits_for_callback_and_finalization_before_publication() {
    let env = Arc::new(Environment::new(Mode::Threads));
    let entered = Cancellation::new();
    let reached = entered.clone();
    let gate = Arc::new(Gate::new());
    let blocked = Arc::clone(&gate);
    let finished = Arc::new(AtomicUsize::new(0));
    let done = Arc::clone(&finished);
    let finalized = Arc::clone(&finished);
    let program: Effect<Data, Data, Environment> = Effect::blocking_once(move |cancel| {
        let _ = reached.request();
        blocked.wait();
        assert!(cancel.is_requested());
        let _ = done.fetch_add(1, Ordering::SeqCst);
        Data(Cell::new(4))
    })
    .ensuring(Effect::sync(move || {
        assert_eq!(finalized.load(Ordering::SeqCst), 1);
    }));
    let mut future = Box::pin(program.run_cancellable(Arc::clone(&env)));
    drive::until(future.as_mut(), || entered.is_requested());
    let _ = env.cancellation.request();
    suspension::suspended(future.as_mut());
    assert_eq!(finished.load(Ordering::SeqCst), 0);
    gate.open();
    assert_eq!(block_on(future), Exit::Interrupted);
    env.finish();
}

#[test]
fn abandoned_waiter_requests_only_its_job_token_and_queued_work_can_retire() {
    let env = Arc::new(Environment::new(Mode::Queued));
    let program: Effect<(), (), Environment> = Effect::cpu(|_| unreachable_fixture());
    let mut future = Box::pin(program.run_cancellable(Arc::clone(&env)));
    suspension::suspended(future.as_mut());
    drop(future);
    assert!(!env.cancellation.is_requested());
    assert!(matches!(env.cpu.take().cancel_before_start(), Ok(Ok(()))));
}

#[test]
fn rejection_accepted_loss_and_submission_panic_are_distinct_defects() {
    for (mode, kind) in [
        (
            Mode::Reject,
            DefectKind::Library(LibraryDefect::JobSubmissionRejected),
        ),
        (
            Mode::Lose,
            DefectKind::Library(LibraryDefect::ScheduledJobDropped),
        ),
        (Mode::Panic, DefectKind::CallbackPanic),
    ] {
        let env = Arc::new(Environment::new(mode));
        let cpu: Effect<(), (), Environment> = Effect::cpu(|_| unreachable_fixture());
        let blocking: Effect<(), (), Environment> = Effect::blocking(|_| unreachable_fixture());
        for outcome in [
            block_on(cpu.run(Arc::clone(&env))),
            block_on(blocking.run(Arc::clone(&env))),
        ] {
            assert!(matches!(outcome, Exit::Die(ref defect) if defect.kind() == kind));
        }
    }
}

struct PanicDrop;
impl Drop for PanicDrop {
    #[allow(clippy::panic)] // Intentional capture-destruction fixture.
    fn drop(&mut self) {
        panic!("intentional callback capture destruction panic");
    }
}

#[test]
fn lost_or_cancelled_capture_destruction_failure_preserves_primary_and_cleanup() {
    for mode in [Mode::Queued, Mode::Lose, Mode::Reject] {
        let env = Arc::new(Environment::new(mode));
        let capture = PanicDrop;
        let program: Effect<(), (), Environment> = Effect::cpu_once(move |_| drop(capture));
        let result = match mode {
            Mode::Queued => {
                let mut future = Box::pin(program.run_cancellable(Arc::clone(&env)));
                suspension::suspended(future.as_mut());
                let _ = env.cancellation.request();
                assert!(env.cpu.take().run().is_ok());
                block_on(future)
            }
            Mode::Lose | Mode::Reject => block_on(program.run(Arc::clone(&env))),
            Mode::Threads | Mode::Panic => unreachable_fixture(),
        };
        match result {
            Exit::FinalizationFailed(details) => {
                assert!(
                    matches!(details.first, CleanupFailure::ReleaseDefect(ref defect) if defect.kind() == DefectKind::CallbackPanic)
                );
                assert!(details.rest.is_empty());
                match mode {
                    Mode::Queued => {
                        assert!(matches!(details.primary, PrimaryOutcome::WasInterrupted))
                    }
                    Mode::Lose => assert!(
                        matches!(details.primary, PrimaryOutcome::Defected(ref defect) if defect.kind() == DefectKind::Library(LibraryDefect::ScheduledJobDropped))
                    ),
                    Mode::Reject => assert!(
                        matches!(details.primary, PrimaryOutcome::Defected(ref defect) if defect.kind() == DefectKind::Library(LibraryDefect::JobSubmissionRejected))
                    ),
                    Mode::Threads | Mode::Panic => unreachable_fixture(),
                }
            }
            Exit::Success(()) | Exit::Fail(()) | Exit::Die(_) | Exit::Interrupted => {
                unreachable_fixture()
            }
        }
    }
}

#[test]
#[allow(clippy::panic)] // Intentional callback/mapper unwind fixtures.
fn callback_and_error_mapper_panics_settle_without_escaping_worker_threads() {
    let env = Arc::new(Environment::new(Mode::Threads));
    let cpu: Effect<(), (), Environment> =
        Effect::cpu(|_| panic!("intentional CPU callback panic"));
    let blocking: Effect<(), (), Environment> =
        Effect::try_blocking(|_| Err(()), |_| panic!("intentional mapping panic"));
    for result in [
        block_on(cpu.run(Arc::clone(&env))),
        block_on(blocking.run(Arc::clone(&env))),
    ] {
        assert!(
            matches!(result, Exit::Die(ref defect) if defect.kind() == DefectKind::CallbackPanic)
        );
    }
    env.finish();
}

#[test]
fn projected_cpu_provider_preserves_active_cancellation_instead_of_replacing_it() {
    let root = Arc::new(Environment::new(Mode::Queued));
    let subtree = Arc::new(Environment::new(Mode::Queued));
    let _ = subtree.cancellation.request(); // Must not override the root run's active token.
    let program: Effect<Data, Data, Environment> = Effect::cpu(|cancel| {
        assert!(!cancel.is_requested());
        Data(Cell::new(2))
    });
    let subtree_provider = Arc::clone(&subtree);
    let provided = program.provide_some(move |_: &Environment| subtree_provider.projection());
    let mut future = Box::pin(provided.run_cancellable(Arc::clone(&root)));
    suspension::suspended(future.as_mut());
    assert_eq!(root.cpu.count(), 0);
    assert_eq!(subtree.cpu.count(), 1);
    let job = subtree.cpu.take();
    assert!(!job.is_cancelled());
    assert!(job.run().is_ok());
    assert_eq!(block_on(future), Exit::Success(Data(Cell::new(2))));
    let projected = Arc::clone(&subtree);
    let interrupted: Effect<(), (), Environment> = Effect::cpu(|_| unreachable_fixture());
    let interrupted = interrupted.provide_some(move |_: &Environment| projected.projection());
    let mut future = Box::pin(interrupted.run_cancellable(Arc::clone(&root)));
    suspension::suspended(future.as_mut());
    let _ = root.cancellation.request();
    assert!(matches!(
        subtree.cpu.take().cancel_before_start(),
        Ok(Ok(()))
    ));
    assert_eq!(block_on(future), Exit::Interrupted);
}

#[test]
fn scope_closure_cancels_and_joins_a_running_cpu_job_through_its_child() {
    let env = Arc::new(Environment::new(Mode::Threads));
    let gate = Arc::new(Gate::new());
    let blocked = Arc::clone(&gate);
    let entered = Cancellation::new();
    let tokens = Arc::new(Mutex::new(None::<Cancellation>));
    let recorded = Arc::clone(&tokens);
    let program: Effect<(), Data, Environment> = Scope::scoped(move |scope| {
        let blocked = Arc::clone(&blocked);
        let reached = entered.clone();
        let entered = entered.clone();
        let recorded = Arc::clone(&recorded);
        scope
            .fork(Effect::cpu_once(move |cancel| {
                *recorded
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(cancel.clone());
                let _ = reached.request();
                blocked.wait();
                assert!(cancel.is_requested());
                Data(Cell::new(5))
            }))
            .flat_map(move |_| {
                let entered = entered.clone();
                Effect::from_future(move |_| {
                    let entered = entered.clone();
                    async move {
                        entered.cancelled().await;
                    }
                })
            })
    });
    let mut future = Box::pin(program.run(Arc::clone(&env)));
    drive::until(future.as_mut(), || {
        tokens
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_ref()
            .is_some_and(Cancellation::is_requested)
    });
    suspension::suspended(future.as_mut());
    gate.open();
    assert_eq!(block_on(future), Exit::Success(()));
    env.finish();
}

#[test]
fn accepted_running_work_retains_environment_after_its_waiter_is_abandoned() {
    let env = Arc::new(Environment::new(Mode::Threads));
    let weak = Arc::downgrade(&env);
    let observed = weak.clone();
    let workers = Arc::clone(&env.cpu);
    let gate = Arc::new(Gate::new());
    let blocked = Arc::clone(&gate);
    let entered = Cancellation::new();
    let reached = entered.clone();
    let program: Effect<Data, Data, Environment> = Effect::cpu_once(move |cancel| {
        let _ = reached.request();
        blocked.wait();
        assert!(cancel.is_requested());
        assert!(observed.upgrade().is_some());
        Data(Cell::new(7))
    });
    let mut future = Box::pin(program.run_cancellable(Arc::clone(&env)));
    drive::until(future.as_mut(), || entered.is_requested());
    drop(future);
    drop(env);
    drop(program);
    assert!(weak.upgrade().is_some());
    gate.open();
    workers.finish();
    assert!(weak.upgrade().is_none());
}

#[test]
fn an_unobserved_result_destructor_unwind_is_returned_to_the_provider_without_escaping_worker() {
    let env = Arc::new(Environment::new(Mode::Queued));
    let gate = Arc::new(Gate::new());
    let blocked = Arc::clone(&gate);
    let entered = Cancellation::new();
    let reached = entered.clone();
    let program: Effect<PanicDrop, (), Environment> = Effect::cpu_once(move |_| {
        let _ = reached.request();
        blocked.wait();
        PanicDrop
    });
    let mut future = Box::pin(program.run(Arc::clone(&env)));
    suspension::suspended(future.as_mut());
    let job = env.cpu.take();
    let worker = std::thread::spawn(move || job.run());
    // The explicit entry signal proves the callback started before abandonment.
    block_on(entered.cancelled());
    drop(future);
    gate.open();
    assert!(
        matches!(worker.join(), Ok(Err(ref defect)) if defect.kind() == DefectKind::CallbackPanic)
    );
}

struct PanicWake;
impl std::task::Wake for PanicWake {
    #[allow(clippy::panic)] // Intentional post-publication notification fixture.
    fn wake(self: Arc<Self>) {
        panic!("intentional worker completion wake panic")
    }
    #[allow(clippy::panic)]
    fn wake_by_ref(self: &Arc<Self>) {
        panic!("intentional worker completion wake panic")
    }
}

#[test]
fn notification_unwind_keeps_committed_completion_and_does_not_escape_job_drop() {
    use std::future::Future;
    for run in [true, false] {
        // Exercise both executed work and accepted ownership loss.
        let env = Arc::new(Environment::new(Mode::Queued));
        let program: Effect<Data, Data, Environment> = Effect::cpu(|_| Data(Cell::new(2)));
        let mut future = Box::pin(program.run(Arc::clone(&env)));
        suspension::suspended(future.as_mut());
        let waker = std::task::Waker::from(Arc::new(PanicWake));
        let mut context = std::task::Context::from_waker(&waker);
        assert!(future.as_mut().poll(&mut context).is_pending());
        let job = env.cpu.take();
        if run {
            assert!(
                matches!(job.run(), Err(ref defect) if defect.kind() == DefectKind::CallbackPanic)
            );
            assert_eq!(block_on(future), Exit::Success(Data(Cell::new(2))));
        } else {
            drop(job);
            assert_eq!(
                block_on(future),
                Exit::Die(Defect::from(LibraryDefect::ScheduledJobDropped))
            );
        }
    }
}

struct BrokenAccess;
impl HasCpuExecutor for BrokenAccess {
    type Executor = Workers;
    #[allow(clippy::panic)] // An environment accessor is a user callback boundary.
    fn cpu_executor(&self) -> &Workers {
        panic!("intentional CPU accessor panic")
    }
}
impl HasBlockingExecutor for BrokenAccess {
    type Executor = Workers;
    #[allow(clippy::panic)]
    fn blocking_executor(&self) -> &Workers {
        panic!("intentional blocking accessor panic")
    }
}

#[test]
fn accessor_panics_drop_unstarted_captures_and_settle_as_defects_without_calling_user_jobs() {
    let drops = Arc::new(AtomicUsize::new(0));
    let calls = Arc::new(AtomicUsize::new(0));
    let first = DropCount(Arc::clone(&drops));
    let called = Arc::clone(&calls);
    let cpu: Effect<(), (), BrokenAccess> = Effect::cpu_once(move |_| {
        drop(first);
        let _ = called.fetch_add(1, Ordering::SeqCst);
    });
    let second = DropCount(Arc::clone(&drops));
    let called = Arc::clone(&calls);
    let blocking: Effect<(), (), BrokenAccess> = Effect::blocking_once(move |_| {
        drop(second);
        let _ = called.fetch_add(1, Ordering::SeqCst);
    });
    for result in [
        block_on(cpu.run(Arc::new(BrokenAccess))),
        block_on(blocking.run(Arc::new(BrokenAccess))),
    ] {
        assert!(
            matches!(result, Exit::Die(ref defect) if defect.kind() == DefectKind::CallbackPanic)
        );
    }
    assert_eq!(drops.load(Ordering::SeqCst), 2); // One capture per submitted operation.
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}
