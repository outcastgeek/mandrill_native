use crate::internal::outcome;
use crate::internal::runtime::{self, Context, Finish, Step};
use crate::{Cancellation, Defect, Effect, Exit, LibraryDefect};
use std::future::poll_fn;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Arc, Mutex};
use std::task::{Poll, Waker};

enum JobAction {
    Run(Cancellation),
    Lost,
}

type SettlementAction = Option<Box<dyn FnOnce() + Send>>;
type Action = Box<dyn FnOnce(JobAction, SettlementAction) + Send>;
struct PendingJob {
    cancellation: Cancellation,
    action: Action,
    settlement: Option<Box<dyn FnOnce() + Send>>,
}

/// An owned synchronous job with typed completion retained by Effect.
///
/// Execute on an independent worker, never inline during admission. `run`
/// consumes this guard, catches callback unwinds and skips already-cancelled
/// callbacks. Dropping an unexecuted guard settles its completion as a typed
/// ownership defect, after releasing callback captures. It is not cancellation.
#[must_use = "accepted jobs must remain owned through settlement"]
pub struct ScheduledJob {
    pending: Option<PendingJob>,
}

impl ScheduledJob {
    /// Run on the worker selected by the provider. No async executor is needed.
    /// Callback/capture failures enter the Effect outcome. An unwind from wake
    /// notification or destruction of an unobserved completed outcome is instead
    /// returned to the provider; it cannot change an already published outcome.
    pub fn run(mut self) -> Result<(), Defect> {
        catch_unwind(AssertUnwindSafe(|| {
            if let Some(pending) = self.pending.take() {
                (pending.action)(JobAction::Run(pending.cancellation), pending.settlement);
            }
        }))
        .map_err(Defect::from_panic)
    }

    pub fn on_settled(mut self, action: impl FnOnce() + Send + 'static) -> Self {
        if let Some(pending) = &mut self.pending {
            let previous = pending.settlement.take();
            pending.settlement = Some(Box::new(move || {
                if let Some(previous) = previous {
                    previous();
                }
                action();
            }));
        }
        self
    }

    /// Inspect the derived cooperative token without selecting another provider.
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.pending
            .as_ref()
            .is_some_and(|pending| pending.cancellation.is_requested())
    }

    /// Retire requested queued work without starting its user callback. If no
    /// request exists, return the still-owned job to its provider unchanged.
    /// Ok carries the same worker-boundary diagnostic as run. Err retains a job
    /// whose cancellation has not been requested.
    pub fn cancel_before_start(self) -> Result<Result<(), Defect>, Self> {
        if self.is_cancelled() {
            Ok(self.run())
        } else {
            Err(self)
        }
    }
}

impl Drop for ScheduledJob {
    fn drop(&mut self) {
        if let Some(pending) = self.pending.take() {
            // The loss outcome commits before notification. Drop has no result
            // channel for a panicking wake or unobserved payload destructor.
            let _ = catch_unwind(AssertUnwindSafe(|| {
                (pending.action)(JobAction::Lost, pending.settlement)
            }));
        }
    }
}

/// CPU jobs supplied only through the current environment R.
///
/// Submission must return promptly without running user work or waiting for its
/// completion. Ok transfers ownership until run/cancel-before-start/drop. Err
/// rejects without starting and drops the job before returning. Admission and
/// actual worker limits are explicit provider policy. No fallback pool exists.
/// Queued cancellation may use cancel_before_start; running callbacks receive
/// cooperative cancellation and must return before Effect publishes settlement.
pub trait CpuExecutor: Send + Sync + 'static {
    fn submit(&self, job: ScheduledJob) -> Result<(), Defect>;
}

pub trait HasCpuExecutor {
    type Executor: CpuExecutor + ?Sized;
    fn cpu_executor(&self) -> &Self::Executor;
}

/// Blocking jobs have the same ownership/admission contract as CpuExecutor,
/// with a separately selected provider and capacity policy in R. Providers must
/// not occupy async poll workers or hold CPU admission while waiting for I/O.
pub trait BlockingExecutor: Send + Sync + 'static {
    fn submit(&self, job: ScheduledJob) -> Result<(), Defect>;
}

pub trait HasBlockingExecutor {
    type Executor: BlockingExecutor + ?Sized;
    fn blocking_executor(&self) -> &Self::Executor;
}

trait Capability<R> {
    fn submit(env: &R, job: ScheduledJob) -> Result<(), Defect>;
}
struct Cpu;
struct Blocking;
impl<R: HasCpuExecutor> Capability<R> for Cpu {
    fn submit(env: &R, job: ScheduledJob) -> Result<(), Defect> {
        env.cpu_executor().submit(job)
    }
}
impl<R: HasBlockingExecutor> Capability<R> for Blocking {
    fn submit(env: &R, job: ScheduledJob) -> Result<(), Defect> {
        env.blocking_executor().submit(job)
    }
}

struct Completion<A, E> {
    state: Mutex<CompletionState<A, E>>,
}
struct CompletionState<A, E> {
    outcome: Option<Exit<A, E>>,
    waiter: Option<Waker>,
}
impl<A, E> Completion<A, E> {
    fn publish(&self, outcome: Exit<A, E>) {
        let waiter = {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            state.outcome = Some(outcome);
            state.waiter.take()
        };
        // There is exactly one owned receiver. Remove its wake registration
        // before invocation, so a panicking waker cannot corrupt event bookkeeping.
        if let Some(waiter) = waiter {
            waiter.wake();
        }
    }
    async fn wait(&self) -> Exit<A, E> {
        poll_fn(|cx| {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if let Some(outcome) = state.outcome.take() {
                return Poll::Ready(outcome);
            }
            if !state
                .waiter
                .as_ref()
                .is_some_and(|waker| waker.will_wake(cx.waker()))
            {
                state.waiter = Some(cx.waker().clone());
            }
            Poll::Pending
        })
        .await
    }
}

struct Owner {
    cancellation: Cancellation,
    settled: bool,
}
impl Drop for Owner {
    fn drop(&mut self) {
        if !self.settled {
            let _ = catch_unwind(AssertUnwindSafe(|| self.cancellation.request()));
        }
        self.cancellation.unlink();
    }
}

fn discarded<A, E>(callback: impl Send, primary: Exit<A, E>) -> Exit<A, E> {
    match catch_unwind(AssertUnwindSafe(|| drop(callback))) {
        Ok(()) => primary,
        Err(panic) => outcome::append_cleanup(primary, Exit::Die(Defect::from_panic(panic))),
    }
}

fn dispatch<A, E, R, C>(
    env: Arc<R>,
    context: Context,
    finish: Finish<A, E>,
    callback: impl FnOnce(Cancellation) -> Exit<A, E> + Send + 'static,
) -> Step
where
    A: Send + 'static,
    E: Send + 'static,
    R: Send + Sync + 'static,
    C: Capability<R>,
{
    let completion = Arc::new(Completion {
        state: Mutex::new(CompletionState {
            outcome: None,
            waiter: None,
        }),
    });
    let completed = Arc::clone(&completion);
    // Accepted jobs retain their provider environment even if their awaiting
    // interpreter is abandoned. No second provider configuration is introduced.
    let job_env = Arc::clone(&env);
    let mut owner = Owner {
        cancellation: context.cancellation.child(),
        settled: false,
    };
    let job = ScheduledJob {
        pending: Some(PendingJob {
            cancellation: owner.cancellation.clone(),
            settlement: None,
            action: Box::new(move |action, settlement| {
                let outcome = match action {
                    JobAction::Run(cancellation) if cancellation.is_requested() => {
                        discarded(callback, Exit::Interrupted)
                    }
                    JobAction::Run(cancellation) => {
                        catch_unwind(AssertUnwindSafe(|| callback(cancellation)))
                            .unwrap_or_else(|panic| Exit::Die(Defect::from_panic(panic)))
                    }
                    JobAction::Lost => discarded(
                        callback,
                        Exit::Die(Defect::from(LibraryDefect::ScheduledJobDropped)),
                    ),
                };
                let outcome = discarded(job_env, outcome);
                let outcome = match settlement {
                    Some(settlement) => match catch_unwind(AssertUnwindSafe(settlement)) {
                        Ok(()) => outcome,
                        Err(panic) => {
                            outcome::append_cleanup(outcome, Exit::Die(Defect::from_panic(panic)))
                        }
                    },
                    None => outcome,
                };
                completed.publish(outcome);
            }),
        }),
    };
    let submission = match catch_unwind(AssertUnwindSafe(|| C::submit(&env, job))) {
        Ok(result) => result,
        Err(panic) => Err(Defect::from_panic(panic)),
    };
    Step::Suspended(Box::pin(async move {
        // Always await accepted work, including after a running callback receives
        // cancellation. Rejection/panic also waits for capture destruction.
        let completed = completion.wait().await;
        owner.settled = true;
        drop(owner);
        let result = match submission {
            Ok(()) => completed,
            Err(defect) => {
                let failures = outcome::failures(completed);
                outcome::append_failures(Exit::Die(defect), failures)
            }
        };
        drop(env); // Keep the current environment/provider alive through settlement.
        runtime::finish(finish, result)
    }))
}

fn reusable<A, E, R, C>(
    callback: impl Fn(Cancellation) -> Exit<A, E> + Send + Sync + 'static,
) -> Effect<A, E, R>
where
    A: Send + 'static,
    E: Send + 'static,
    R: Send + Sync + 'static,
    C: Capability<R>,
{
    let callback = Arc::new(callback);
    Effect::new(move |env, context, finish| {
        let callback = Arc::clone(&callback);
        dispatch::<A, E, R, C>(env, context, finish, move |cancellation| {
            callback(cancellation)
        })
    })
}

fn once<A, E, R, C>(
    callback: impl FnOnce(Cancellation) -> Exit<A, E> + Send + 'static,
) -> Effect<A, E, R>
where
    A: Send + 'static,
    E: Send + 'static,
    R: Send + Sync + 'static,
    C: Capability<R>,
{
    let callback = Mutex::new(Some(callback));
    Effect::new(move |env, context, finish| {
        let callback = callback
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        match callback {
            Some(callback) => dispatch::<A, E, R, C>(env, context, finish, callback),
            None => runtime::finish(
                finish,
                Exit::Die(Defect::from(LibraryDefect::JobCallbackConsumed)),
            ),
        }
    })
}

impl<A, E, R> Effect<A, E, R>
where
    A: Send + 'static,
    E: Send + 'static,
    R: HasCpuExecutor + Send + Sync + 'static,
{
    /// Fresh results from a reusable worker callback. Captures are shared;
    /// result/error payloads are moved and need neither Clone nor Sync.
    pub fn cpu(callback: impl Fn(Cancellation) -> A + Send + Sync + 'static) -> Self {
        reusable::<A, E, R, Cpu>(move |cancel| Exit::Success(callback(cancel)))
    }
    /// Expected errors enter E; the callback and mapping run on the CPU worker.
    pub fn try_cpu<X>(
        callback: impl Fn(Cancellation) -> Result<A, X> + Send + Sync + 'static,
        on_error: impl Fn(X) -> E + Send + Sync + 'static,
    ) -> Self {
        reusable::<A, E, R, Cpu>(move |cancel| match callback(cancel) {
            Ok(value) => Exit::Success(value),
            Err(error) => Exit::Fail(on_error(error)),
        })
    }
    /// Consume an owned, potentially non-Sync callback once. Reuse reports
    /// JobCallbackConsumed; pre-cancelled execution does not consume it.
    pub fn cpu_once(callback: impl FnOnce(Cancellation) -> A + Send + 'static) -> Self {
        once::<A, E, R, Cpu>(move |cancel| Exit::Success(callback(cancel)))
    }
    pub fn try_cpu_once<X>(
        callback: impl FnOnce(Cancellation) -> Result<A, X> + Send + 'static,
        on_error: impl FnOnce(X) -> E + Send + 'static,
    ) -> Self {
        once::<A, E, R, Cpu>(move |cancel| match callback(cancel) {
            Ok(value) => Exit::Success(value),
            Err(error) => Exit::Fail(on_error(error)),
        })
    }
}

impl<A, E, R> Effect<A, E, R>
where
    A: Send + 'static,
    E: Send + 'static,
    R: HasBlockingExecutor + Send + Sync + 'static,
{
    /// Run a reusable blocking callback on the independently selected provider.
    pub fn blocking(callback: impl Fn(Cancellation) -> A + Send + Sync + 'static) -> Self {
        reusable::<A, E, R, Blocking>(move |cancel| Exit::Success(callback(cancel)))
    }
    pub fn try_blocking<X>(
        callback: impl Fn(Cancellation) -> Result<A, X> + Send + Sync + 'static,
        on_error: impl Fn(X) -> E + Send + Sync + 'static,
    ) -> Self {
        reusable::<A, E, R, Blocking>(move |cancel| match callback(cancel) {
            Ok(value) => Exit::Success(value),
            Err(error) => Exit::Fail(on_error(error)),
        })
    }
    /// Move owned input/captures without imposing Clone or Sync. A new recipe
    /// constructed inside each effect! run can consume a fresh callback.
    pub fn blocking_once(callback: impl FnOnce(Cancellation) -> A + Send + 'static) -> Self {
        once::<A, E, R, Blocking>(move |cancel| Exit::Success(callback(cancel)))
    }
    pub fn try_blocking_once<X>(
        callback: impl FnOnce(Cancellation) -> Result<A, X> + Send + 'static,
        on_error: impl FnOnce(X) -> E + Send + 'static,
    ) -> Self {
        once::<A, E, R, Blocking>(move |cancel| match callback(cancel) {
            Ok(value) => Exit::Success(value),
            Err(error) => Exit::Fail(on_error(error)),
        })
    }
}

impl<A: Send + 'static, E: Send + 'static, R: HasCpuExecutor + Send + Sync + 'static>
    Effect<A, E, R>
{
    /// Explicit metadata travels with the selected worker job. No thread-local
    /// tracer state or second runtime configuration is installed.
    pub fn cpu_with_context(
        callback: impl Fn(Cancellation, crate::TraceContext) -> A + Send + Sync + 'static,
    ) -> Self {
        let callback = Arc::new(callback);
        Self::new(move |env, context, finish| {
            let callback = Arc::clone(&callback);
            let trace = context.trace.clone();
            dispatch::<A, E, R, Cpu>(env, context, finish, move |cancel| {
                Exit::Success(callback(cancel, trace))
            })
        })
    }
    pub fn try_cpu_with_context<X>(
        callback: impl Fn(Cancellation, crate::TraceContext) -> Result<A, X> + Send + Sync + 'static,
        on_error: impl Fn(X) -> E + Send + Sync + 'static,
    ) -> Self {
        let callback = Arc::new(callback);
        let on_error = Arc::new(on_error);
        Self::new(move |env, context, finish| {
            let callback = Arc::clone(&callback);
            let on_error = Arc::clone(&on_error);
            let trace = context.trace.clone();
            dispatch::<A, E, R, Cpu>(env, context, finish, move |cancel| {
                match callback(cancel, trace) {
                    Ok(value) => Exit::Success(value),
                    Err(error) => Exit::Fail(on_error(error)),
                }
            })
        })
    }
    pub fn cpu_once_with_context(
        callback: impl FnOnce(Cancellation, crate::TraceContext) -> A + Send + 'static,
    ) -> Self {
        let candidate = Mutex::new(Some(callback));
        Self::new(move |env, context, finish| {
            let candidate = candidate
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take();
            match candidate {
                Some(callback) => {
                    let trace = context.trace.clone();
                    dispatch::<A, E, R, Cpu>(env, context, finish, move |cancel| {
                        Exit::Success(callback(cancel, trace))
                    })
                }
                None => runtime::finish(
                    finish,
                    Exit::Die(Defect::from(LibraryDefect::JobCallbackConsumed)),
                ),
            }
        })
    }
}
impl<A: Send + 'static, E: Send + 'static, R: HasBlockingExecutor + Send + Sync + 'static>
    Effect<A, E, R>
{
    pub fn blocking_with_context(
        callback: impl Fn(Cancellation, crate::TraceContext) -> A + Send + Sync + 'static,
    ) -> Self {
        let callback = Arc::new(callback);
        Self::new(move |env, context, finish| {
            let callback = Arc::clone(&callback);
            let trace = context.trace.clone();
            dispatch::<A, E, R, Blocking>(env, context, finish, move |cancel| {
                Exit::Success(callback(cancel, trace))
            })
        })
    }
    pub fn try_blocking_with_context<X>(
        callback: impl Fn(Cancellation, crate::TraceContext) -> Result<A, X> + Send + Sync + 'static,
        on_error: impl Fn(X) -> E + Send + Sync + 'static,
    ) -> Self {
        let callback = Arc::new(callback);
        let on_error = Arc::new(on_error);
        Self::new(move |env, context, finish| {
            let callback = Arc::clone(&callback);
            let on_error = Arc::clone(&on_error);
            let trace = context.trace.clone();
            dispatch::<A, E, R, Blocking>(env, context, finish, move |cancel| {
                match callback(cancel, trace) {
                    Ok(value) => Exit::Success(value),
                    Err(error) => Exit::Fail(on_error(error)),
                }
            })
        })
    }
    pub fn blocking_once_with_context(
        callback: impl FnOnce(Cancellation, crate::TraceContext) -> A + Send + 'static,
    ) -> Self {
        let candidate = Mutex::new(Some(callback));
        Self::new(move |env, context, finish| {
            let candidate = candidate
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take();
            match candidate {
                Some(callback) => {
                    let trace = context.trace.clone();
                    dispatch::<A, E, R, Blocking>(env, context, finish, move |cancel| {
                        Exit::Success(callback(cancel, trace))
                    })
                }
                None => runtime::finish(
                    finish,
                    Exit::Die(Defect::from(LibraryDefect::JobCallbackConsumed)),
                ),
            }
        })
    }
}
