//! Environment-provided CPU and blocking job execution.

use crate::internal::outcome;
use crate::internal::runtime::{self, Context, Finish, Step};
use crate::{Cancellation, Defect, Effect, Exit, LibraryDefect};
use std::future::poll_fn;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Arc, Mutex};
use std::task::{Poll, Waker};

/// Variants of job action used by job.
enum JobAction {
    Run(Cancellation),
    Lost,
}

/// Type alias for settlement action in job.
type SettlementAction = Option<Box<dyn FnOnce() + Send>>;
/// Type alias for action in job.
type Action = Box<dyn FnOnce(JobAction, SettlementAction) + Send>;
/// Typed pending job used by job.
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
    ///
    /// # Returns
    /// Type: `Result<(), Defect>`.
    pub fn run(mut self) -> Result<(), Defect> {
        catch_unwind(AssertUnwindSafe(|| {
            if let Some(pending) = self.pending.take() {
                (pending.action)(JobAction::Run(pending.cancellation), pending.settlement);
            }
        }))
        .map_err(Defect::from_panic)
    }

    /// Register a callback invoked when the scheduled job reaches settlement.
    ///
    /// # Parameters
    /// - `action` (`impl FnOnce() + Send + 'static`): State transition selected for this operation.
    ///
    /// # Returns
    /// Type: `Self`.
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
    /// # Returns
    /// Type: `bool`.
    /// Returns whether the requested condition holds.
    pub fn is_cancelled(&self) -> bool {
        self.pending
            .as_ref()
            .is_some_and(|pending| pending.cancellation.is_requested())
    }

    /// Retire requested queued work without starting its user callback. If no
    /// request exists, return the still-owned job to its provider unchanged.
    /// Ok carries the same worker-boundary diagnostic as run. Err retains a job
    /// whose cancellation has not been requested.
    ///
    /// # Returns
    /// Type: `Result<Result<(), Defect>, Self>`.
    pub fn cancel_before_start(self) -> Result<Result<(), Defect>, Self> {
        if self.is_cancelled() {
            Ok(self.run())
        } else {
            Err(self)
        }
    }
}

impl Drop for ScheduledJob {
    ///
    /// # Returns
    /// Type: `()`.
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
    /// # Parameters
    /// - `job` (`ScheduledJob`): Owned synchronous callback transferred to a provider.
    ///
    /// # Returns
    /// Type: `Result<(), Defect>`.
    fn submit(&self, job: ScheduledJob) -> Result<(), Defect>;
}

/// Selects the CPU job executor in an application environment.
pub trait HasCpuExecutor {
    /// Executor selected by this environment.
    type Executor: CpuExecutor + ?Sized;
    /// # Returns
    /// Type: `&Self::Executor`.
    fn cpu_executor(&self) -> &Self::Executor;
}

/// Blocking jobs have the same ownership/admission contract as CpuExecutor,
/// with a separately selected provider and capacity policy in R. Providers must
/// not occupy async poll workers or hold CPU admission while waiting for I/O.
pub trait BlockingExecutor: Send + Sync + 'static {
    /// # Parameters
    /// - `job` (`ScheduledJob`): Owned synchronous callback transferred to a provider.
    ///
    /// # Returns
    /// Type: `Result<(), Defect>`.
    fn submit(&self, job: ScheduledJob) -> Result<(), Defect>;
}

/// Selects the blocking job executor in an application environment.
pub trait HasBlockingExecutor {
    /// Executor selected by this environment.
    type Executor: BlockingExecutor + ?Sized;
    /// # Returns
    /// Type: `&Self::Executor`.
    fn blocking_executor(&self) -> &Self::Executor;
}

/// Submits a scheduled job through the executor capability selected by `R`.
trait Capability<R> {
    ///
    /// # Type parameters
    /// - `R`: Environment type.
    ///
    /// # Parameters
    /// - `env` (`&R`): The environment supplying this operation's capabilities.
    /// - `job` (`ScheduledJob`): Owned synchronous callback transferred to a provider.
    ///
    /// # Returns
    /// Type: `Result<(), Defect>`.
    fn submit(env: &R, job: ScheduledJob) -> Result<(), Defect>;
}
/// Typed cpu used by job.
struct Cpu;
/// Typed blocking used by job.
struct Blocking;
impl<R: HasCpuExecutor> Capability<R> for Cpu {
    ///
    /// # Type parameters
    /// - `R`: Environment type.
    ///
    /// # Parameters
    /// - `env` (`&R`): The environment supplying this operation's capabilities.
    /// - `job` (`ScheduledJob`): Owned synchronous callback transferred to a provider.
    ///
    /// # Returns
    /// Type: `Result<(), Defect>`.
    fn submit(env: &R, job: ScheduledJob) -> Result<(), Defect> {
        env.cpu_executor().submit(job)
    }
}
impl<R: HasBlockingExecutor> Capability<R> for Blocking {
    ///
    /// # Type parameters
    /// - `R`: Environment type.
    ///
    /// # Parameters
    /// - `env` (`&R`): The environment supplying this operation's capabilities.
    /// - `job` (`ScheduledJob`): Owned synchronous callback transferred to a provider.
    ///
    /// # Returns
    /// Type: `Result<(), Defect>`.
    fn submit(env: &R, job: ScheduledJob) -> Result<(), Defect> {
        env.blocking_executor().submit(job)
    }
}

/// Typed completion used by job.
///
/// # Type parameters
/// - `A`: Application value type.
/// - `E`: Domain-error type.
struct Completion<A, E> {
    state: Mutex<CompletionState<A, E>>,
}
/// Typed completion state used by job.
///
/// # Type parameters
/// - `A`: Application value type.
/// - `E`: Domain-error type.
struct CompletionState<A, E> {
    outcome: Option<Exit<A, E>>,
    waiter: Option<Waker>,
}
impl<A, E> Completion<A, E> {
    ///
    /// # Type parameters
    /// - `A`: Success or application-value type.
    /// - `E`: Domain-error type.
    ///
    /// # Parameters
    /// - `outcome` (`Exit<A, E>`): Complete terminal outcome, including expected errors and cleanup failures.
    ///
    /// # Returns
    /// Type: `()`.
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
    ///
    /// # Type parameters
    /// - `A`: Success or application-value type.
    /// - `E`: Domain-error type.
    ///
    /// # Returns
    /// Type: `Exit<A, E>`.
    /// Returns `Exit<A, E>`.
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

/// Typed owner used by job.
struct Owner {
    cancellation: Cancellation,
    settled: bool,
}
impl Drop for Owner {
    ///
    /// # Returns
    /// Type: `()`.
    fn drop(&mut self) {
        if !self.settled {
            let _ = catch_unwind(AssertUnwindSafe(|| self.cancellation.request()));
        }
        self.cancellation.unlink();
    }
}

///
/// # Type parameters
/// - `A`: Successful application-value type.
/// - `E`: Expected application-error type.
///
/// # Parameters
/// - `callback` (`impl Send`): The callback invoked by this operation.
/// - `primary` (`Exit<A, E>`): The primary supplied to this operation.
///
/// # Returns
/// Type: `Exit<A, E>`.
/// Returns `Exit<A, E>`.
fn discarded<A, E>(callback: impl Send, primary: Exit<A, E>) -> Exit<A, E> {
    match catch_unwind(AssertUnwindSafe(|| drop(callback))) {
        Ok(()) => primary,
        Err(panic) => outcome::append_cleanup(primary, Exit::Die(Defect::from_panic(panic))),
    }
}

///
/// # Type parameters
/// - `A`: Successful application-value type. Additional bound: `Send + 'static`.
/// - `E`: Expected application-error type. Additional bound: `Send + 'static`.
/// - `R`: Environment type supplying required capabilities. Additional bound: `Send + Sync + 'static`.
/// - `C`: Stream chunk or item type. Additional bound: `Capability<R>`.
///
/// # Parameters
/// - `env` (`Arc<R>`): The environment supplying this operation's capabilities.
/// - `context` (`Context`): Execution context carrying cancellation, scope, and trace state.
/// - `finish` (`Finish<A, E>`): Continuation receiving the complete terminal outcome.
/// - `callback` (`impl FnOnce(Cancellation) -> Exit<A, E> + Send + 'static`): The callback invoked by this operation.
///
/// # Returns
/// Type: `Step`.
/// Returns `Step`.
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

///
/// # Type parameters
/// - `A`: Successful application-value type. Additional bound: `Send + 'static`.
/// - `E`: Expected application-error type. Additional bound: `Send + 'static`.
/// - `R`: Environment type supplying required capabilities. Additional bound: `Send + Sync + 'static`.
/// - `C`: Stream chunk or item type. Additional bound: `Capability<R>`.
///
/// # Parameters
/// - `callback` (`impl Fn(Cancellation) -> Exit<A, E> + Send + Sync + 'static`): The callback invoked by this operation.
///
/// # Returns
/// Type: `Effect<A, E, R>`.
/// Returns a lazy `Effect` with the declared success, error, and environment types.
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

///
/// # Type parameters
/// - `A`: Successful application-value type. Additional bound: `Send + 'static`.
/// - `E`: Expected application-error type. Additional bound: `Send + 'static`.
/// - `R`: Environment type supplying required capabilities. Additional bound: `Send + Sync + 'static`.
/// - `C`: Stream chunk or item type. Additional bound: `Capability<R>`.
///
/// # Parameters
/// - `callback` (`impl FnOnce(Cancellation) -> Exit<A, E> + Send + 'static`): The callback invoked by this operation.
///
/// # Returns
/// Type: `Effect<A, E, R>`.
/// Returns a lazy `Effect` with the declared success, error, and environment types.
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
    ///
    /// # Type parameters
    /// - `E`: Domain-error type.
    /// - `R`: Environment type.
    /// - `A`: Success or application-value type.
    ///
    /// # Parameters
    /// - `callback` (`impl Fn(Cancellation) -> A + Send + Sync + 'static`): The callback invoked by this operation.
    ///
    /// # Returns
    /// Type: `Self`.
    pub fn cpu(callback: impl Fn(Cancellation) -> A + Send + Sync + 'static) -> Self {
        reusable::<A, E, R, Cpu>(move |cancel| Exit::Success(callback(cancel)))
    }
    /// Expected errors enter E; the callback and mapping run on the CPU worker.
    ///
    /// # Type parameters
    /// - `R`: Environment type.
    /// - `X`: Source error type mapped into the declared error channel.
    /// - `A`: Success or application-value type.
    /// - `E`: Domain-error type.
    ///
    /// # Parameters
    /// - `callback` (`impl Fn(Cancellation) -> Result<A, X> + Send + Sync + 'static`): The callback invoked by this operation.
    /// - `on_error` (`impl Fn(X) -> E + Send + Sync + 'static`): Callback that maps a source error into the declared error type.
    ///
    /// # Returns
    /// Type: `Self`.
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
    ///
    /// # Type parameters
    /// - `E`: Domain-error type.
    /// - `R`: Environment type.
    /// - `A`: Success or application-value type.
    ///
    /// # Parameters
    /// - `callback` (`impl FnOnce(Cancellation) -> A + Send + 'static`): The callback invoked by this operation.
    ///
    /// # Returns
    /// Type: `Self`.
    pub fn cpu_once(callback: impl FnOnce(Cancellation) -> A + Send + 'static) -> Self {
        once::<A, E, R, Cpu>(move |cancel| Exit::Success(callback(cancel)))
    }
    /// Submit one fallible CPU callback through the environment’s CPU executor.
    ///
    /// # Type parameters
    /// - `R`: Environment type.
    /// - `X`: Source error type mapped into the declared error channel.
    /// - `A`: Success or application-value type.
    /// - `E`: Domain-error type.
    ///
    /// # Parameters
    /// - `callback` (`impl FnOnce(Cancellation) -> Result<A, X> + Send + 'static`): The callback invoked by this operation.
    /// - `on_error` (`impl FnOnce(X) -> E + Send + 'static`): Callback that maps a source error into the declared error type.
    ///
    /// # Returns
    /// Type: `Self`.
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
    ///
    /// # Type parameters
    /// - `E`: Domain-error type.
    /// - `R`: Environment type.
    /// - `A`: Success or application-value type.
    ///
    /// # Parameters
    /// - `callback` (`impl Fn(Cancellation) -> A + Send + Sync + 'static`): The callback invoked by this operation.
    ///
    /// # Returns
    /// Type: `Self`.
    pub fn blocking(callback: impl Fn(Cancellation) -> A + Send + Sync + 'static) -> Self {
        reusable::<A, E, R, Blocking>(move |cancel| Exit::Success(callback(cancel)))
    }
    /// Submit a reusable fallible blocking callback through the environment.
    ///
    /// # Type parameters
    /// - `R`: Environment type.
    /// - `X`: Source error type mapped into the declared error channel.
    /// - `A`: Success or application-value type.
    /// - `E`: Domain-error type.
    ///
    /// # Parameters
    /// - `callback` (`impl Fn(Cancellation) -> Result<A, X> + Send + Sync + 'static`): The callback invoked by this operation.
    /// - `on_error` (`impl Fn(X) -> E + Send + Sync + 'static`): Callback that maps a source error into the declared error type.
    ///
    /// # Returns
    /// Type: `Self`.
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
    ///
    /// # Type parameters
    /// - `E`: Domain-error type.
    /// - `R`: Environment type.
    /// - `A`: Success or application-value type.
    ///
    /// # Parameters
    /// - `callback` (`impl FnOnce(Cancellation) -> A + Send + 'static`): The callback invoked by this operation.
    ///
    /// # Returns
    /// Type: `Self`.
    pub fn blocking_once(callback: impl FnOnce(Cancellation) -> A + Send + 'static) -> Self {
        once::<A, E, R, Blocking>(move |cancel| Exit::Success(callback(cancel)))
    }
    /// Submit one fallible blocking callback with owned captures.
    ///
    /// # Type parameters
    /// - `R`: Environment type.
    /// - `X`: Source error type mapped into the declared error channel.
    /// - `A`: Success or application-value type.
    /// - `E`: Domain-error type.
    ///
    /// # Parameters
    /// - `callback` (`impl FnOnce(Cancellation) -> Result<A, X> + Send + 'static`): The callback invoked by this operation.
    /// - `on_error` (`impl FnOnce(X) -> E + Send + 'static`): Callback that maps a source error into the declared error type.
    ///
    /// # Returns
    /// Type: `Self`.
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
    ///
    /// # Type parameters
    /// - `E`: Domain-error type. Bound: `Send + 'static`.
    /// - `R`: Environment type. Bound: `HasCpuExecutor + Send + Sync + 'static`.
    /// - `A`: Success or application-value type.
    ///
    /// # Parameters
    /// - `callback` (`impl Fn(Cancellation, crate::TraceContext) -> A + Send + Sync + 'static`): The callback invoked by this operation.
    ///
    /// # Returns
    /// Type: `Self`.
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
    /// Submit CPU work with the current trace context attached.
    ///
    /// # Type parameters
    /// - `R`: Environment type. Bound: `HasCpuExecutor + Send + Sync + 'static`.
    /// - `X`: Source error type mapped into the declared error channel.
    /// - `A`: Success or application-value type.
    /// - `E`: Domain-error type.
    ///
    /// # Parameters
    /// - `callback` (`impl Fn(Cancellation, crate::TraceContext) -> Result<A, X> + Send + Sync + 'static`): The callback invoked by this operation.
    /// - `on_error` (`impl Fn(X) -> E + Send + Sync + 'static`): Callback that maps a source error into the declared error type.
    ///
    /// # Returns
    /// Type: `Self`.
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
    /// Submit one owned CPU callback with its trace context.
    ///
    /// # Type parameters
    /// - `E`: Domain-error type. Bound: `Send + 'static`.
    /// - `R`: Environment type. Bound: `HasCpuExecutor + Send + Sync + 'static`.
    /// - `A`: Success or application-value type.
    ///
    /// # Parameters
    /// - `callback` (`impl FnOnce(Cancellation, crate::TraceContext) -> A + Send + 'static`): The callback invoked by this operation.
    ///
    /// # Returns
    /// Type: `Self`.
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
    /// Submit blocking work with the current trace context attached.
    ///
    /// # Type parameters
    /// - `E`: Domain-error type. Bound: `Send + 'static`.
    /// - `R`: Environment type. Bound: `HasBlockingExecutor + Send + Sync + 'static`.
    /// - `A`: Success or application-value type.
    ///
    /// # Parameters
    /// - `callback` (`impl Fn(Cancellation, crate::TraceContext) -> A + Send + Sync + 'static`): The callback invoked by this operation.
    ///
    /// # Returns
    /// Type: `Self`.
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
    /// Submit fallible blocking work with the current trace context.
    ///
    /// # Type parameters
    /// - `R`: Environment type. Bound: `HasBlockingExecutor + Send + Sync + 'static`.
    /// - `X`: Source error type mapped into the declared error channel.
    /// - `A`: Success or application-value type.
    /// - `E`: Domain-error type.
    ///
    /// # Parameters
    /// - `callback` (`impl Fn(Cancellation, crate::TraceContext) -> Result<A, X> + Send + Sync + 'static`): The callback invoked by this operation.
    /// - `on_error` (`impl Fn(X) -> E + Send + Sync + 'static`): Callback that maps a source error into the declared error type.
    ///
    /// # Returns
    /// Type: `Self`.
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
    /// Submit one owned blocking callback with its trace context.
    ///
    /// # Type parameters
    /// - `E`: Domain-error type. Bound: `Send + 'static`.
    /// - `R`: Environment type. Bound: `HasBlockingExecutor + Send + Sync + 'static`.
    /// - `A`: Success or application-value type.
    ///
    /// # Parameters
    /// - `callback` (`impl FnOnce(Cancellation, crate::TraceContext) -> A + Send + 'static`): The callback invoked by this operation.
    ///
    /// # Returns
    /// Type: `Self`.
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
