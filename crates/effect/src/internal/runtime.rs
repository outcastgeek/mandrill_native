//! Private Effect runtime coordination.

use super::destruction::FlatDrop;
use crate::LibraryDefect;
use crate::{Cancellation, Defect, Effect, Exit};
use std::future::{Future, poll_fn};
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::task::Poll;

/// Type alias for finish in internal runtime.
///
/// # Type parameters
/// - `A`: Application value type.
/// - `E`: Domain-error type.
pub(crate) type Finish<A, E> = FlatDrop<Box<dyn FnOnce(Exit<A, E>) -> Step + Send>>;

/// Variants of step used by internal runtime.
pub(crate) enum Step {
    Done,
    More(FlatDrop<Box<dyn FnOnce() -> Step + Send>>),
    Suspended(Pin<Box<dyn Future<Output = Step> + Send>>),
    Composed(Pin<Box<dyn Future<Output = Step> + Send>>),
}

#[derive(Clone)]
/// Typed context used by internal runtime.
pub(crate) struct Context {
    pub(crate) cancellation: Cancellation,
    origin: Arc<FailureOrigin>,
    pub(crate) trace: crate::TraceContext,
    clock_frontier: ClockFrontier,
}

/// Typed failure origin used by internal runtime.
struct FailureOrigin {
    protected: AtomicBool,
    fiber: bool,
    frontier: crate::clock::Frontier,
}

impl FailureOrigin {
    ///
    /// # Returns
    /// Type: `Self`.
    const fn root() -> Self {
        Self {
            protected: AtomicBool::new(false),
            fiber: false,
            frontier: crate::clock::Frontier::new(),
        }
    }
}

#[derive(Clone)]
/// Variants of clock frontier used by internal runtime.
enum ClockFrontier {
    Own,
    Parent(Arc<FailureOrigin>),
}

impl Context {
    /// # Returns
    /// Type: `bool`.
    /// Returns whether the requested condition holds.
    pub(crate) fn cancelled(&self) -> bool {
        self.cancellation.is_requested()
    }

    /// # Returns
    /// Type: `()`.
    pub(crate) fn begin_operation(&self) {
        self.origin.protected.store(false, Ordering::Relaxed);
    }

    /// # Returns
    /// Type: `Self`.
    pub(crate) fn protected(&self) -> Self {
        Self {
            cancellation: Cancellation::new(),
            trace: self.trace.clone(),
            origin: Arc::new(const { FailureOrigin::root() }),
            clock_frontier: ClockFrontier::Parent(Arc::clone(self.clock_origin())),
        }
    }

    /// # Returns
    /// Type: `Self`.
    pub(crate) fn child(&self) -> Self {
        Self {
            cancellation: self.cancellation.child(),
            trace: self.trace.clone(),
            origin: Arc::new(FailureOrigin {
                protected: AtomicBool::new(false),
                fiber: true,
                frontier: const { crate::clock::Frontier::new() },
            }),
            clock_frontier: ClockFrontier::Own,
        }
    }

    /// Parallel branches inherit whether their caller is itself a fiber; unlike
    /// a scoped fork they do not create a new fiber failure-publication boundary.
    ///
    /// # Parameters
    /// - `cancellation` (`Cancellation`): The cancellation token for this operation.
    ///
    /// # Returns
    /// Type: `Self`.
    pub(crate) fn parallel(&self, cancellation: Cancellation) -> Self {
        Self {
            cancellation,
            trace: self.trace.clone(),
            origin: Arc::new(FailureOrigin {
                protected: AtomicBool::new(false),
                fiber: self.origin.fiber,
                frontier: const { crate::clock::Frontier::new() },
            }),
            clock_frontier: ClockFrontier::Own,
        }
    }

    /// # Returns
    /// Type: `bool`.
    /// Returns whether the requested condition holds.
    pub(crate) fn was_protected(&self) -> bool {
        self.origin.protected.load(Ordering::Relaxed)
    }

    ///
    /// # Returns
    /// Type: `&Arc<FailureOrigin>`.
    fn clock_origin(&self) -> &Arc<FailureOrigin> {
        match &self.clock_frontier {
            ClockFrontier::Own => &self.origin,
            ClockFrontier::Parent(origin) => origin,
        }
    }
    ///
    /// # Returns
    /// Type: `()`.
    fn acknowledge_clock(&self) {
        self.clock_origin().frontier.acknowledge();
    }
    /// # Parameters
    /// - `tick` (`crate::ClockTick`): The tick supplied to this operation.
    ///
    /// # Returns
    /// Type: `()`.
    pub(crate) fn clock_tick(&self, tick: crate::ClockTick) {
        self.clock_origin().frontier.retain(tick);
    }

    /// # Parameters
    /// - `protected` (`bool`): The protected supplied to this operation.
    ///
    /// # Returns
    /// Type: `()`.
    pub(crate) fn inherit_protection(&self, protected: bool) {
        self.origin.protected.store(protected, Ordering::Relaxed);
    }

    /// # Type parameters
    /// - `A`: Application value type.
    /// - `E`: Domain-error type.
    ///
    /// # Parameters
    /// - `outcome` (`&Exit<A, E>`): Complete terminal outcome, including expected errors and cleanup failures.
    ///
    /// # Returns
    /// Type: `()`.
    pub(crate) fn record_protected_failure<A, E>(&self, outcome: &Exit<A, E>) {
        self.origin
            .protected
            .store(matches!(outcome, Exit::Fail(_)), Ordering::Relaxed);
    }
}

/// # Parameters
/// - `next` (`impl FnOnce() -> Step + Send + 'static`): Next value or state produced by the transition.
///
/// # Returns
/// Type: `Step`.
pub(crate) fn more(next: impl FnOnce() -> Step + Send + 'static) -> Step {
    Step::More(FlatDrop::new(Box::new(next)))
}

/// # Type parameters
/// - `A`: Application value type. Bounds: `Send + 'static`.
/// - `E`: Domain-error type. Bounds: `Send + 'static`.
///
/// # Parameters
/// - `callback` (`Finish<A, E>`): The callback invoked by this operation.
/// - `outcome` (`Exit<A, E>`): Complete terminal outcome, including expected errors and cleanup failures.
///
/// # Returns
/// Type: `Step`.
pub(crate) fn finish<A: Send + 'static, E: Send + 'static>(
    callback: Finish<A, E>,
    outcome: Exit<A, E>,
) -> Step {
    more(move || match callback.take() {
        Some(callback) => callback(outcome),
        None => Step::Done,
    })
}

/// # Type parameters
/// - `A`: Application value type. Bounds: `Send + 'static`.
/// - `E`: Domain-error type. Bounds: `Send + 'static`.
///
/// # Parameters
/// - `callback` (`impl FnOnce(Exit<A, E>) -> Step + Send + 'static`): The callback invoked by this operation.
///
/// # Returns
/// Type: `Finish<A, E>`.
pub(crate) fn continuation<A: Send + 'static, E: Send + 'static>(
    callback: impl FnOnce(Exit<A, E>) -> Step + Send + 'static,
) -> Finish<A, E> {
    FlatDrop::new(Box::new(callback))
}

/// # Type parameters
/// - `A`: Application value type.
/// - `E`: Domain-error type.
///
/// # Parameters
/// - `outcome` (`Exit<A, E>`): Complete terminal outcome, including expected errors and cleanup failures.
/// - `context` (`&Context`): Execution context carrying cancellation, scope, and trace state.
///
/// # Returns
/// Type: `Exit<A, E>`.
pub(crate) fn publish<A, E>(outcome: Exit<A, E>, context: &Context) -> Exit<A, E> {
    if !context.cancelled() {
        return outcome;
    }
    match outcome {
        Exit::Success(_) => Exit::Interrupted,
        Exit::Fail(error)
            if context.origin.fiber && context.origin.protected.load(Ordering::Relaxed) =>
        {
            Exit::Fail(error)
        }
        Exit::Fail(_) => Exit::Interrupted,
        Exit::Die(defect) => Exit::Die(defect),
        Exit::Interrupted => Exit::Interrupted,
        Exit::FinalizationFailed(details) => Exit::FinalizationFailed(details),
    }
}

/// # Type parameters
/// - `A`: Application value type. Additional bound: `Send + 'static`.
/// - `E`: Domain-error type. Additional bound: `Send + 'static`.
/// - `R`: Environment type. Additional bound: `Send + Sync + 'static`.
///
/// # Parameters
/// - `program` (`Effect<A, E, R>`): Lazy Effect description executed or transformed here.
/// - `env` (`Arc<R>`): The environment supplying this operation's capabilities.
/// - `cancellation` (`Cancellation`): The cancellation token for this operation.
///
/// # Returns
/// Type: `Exit<A, E>`.
pub(crate) async fn evaluate<A, E, R>(
    program: Effect<A, E, R>,
    env: Arc<R>,
    cancellation: Cancellation,
) -> Exit<A, E>
where
    A: Send + 'static,
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    let context = Context {
        cancellation,
        trace: crate::TraceContext::default(),
        origin: Arc::new(const { FailureOrigin::root() }),
        clock_frontier: ClockFrontier::Own,
    };
    evaluate_context(program, env, &context).await
}

/// # Type parameters
/// - `A`: Application value type. Additional bound: `Send + 'static`.
/// - `E`: Domain-error type. Additional bound: `Send + 'static`.
/// - `R`: Environment type. Additional bound: `Send + Sync + 'static`.
///
/// # Parameters
/// - `program` (`Effect<A, E, R>`): Lazy Effect description executed or transformed here.
/// - `env` (`Arc<R>`): The environment supplying this operation's capabilities.
/// - `context` (`&Context`): Execution context carrying cancellation, scope, and trace state.
///
/// # Returns
/// Type: `Exit<A, E>`.
pub(crate) async fn evaluate_context<A, E, R>(
    program: Effect<A, E, R>,
    env: Arc<R>,
    context: &Context,
) -> Exit<A, E>
where
    A: Send + 'static,
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    evaluate_inner(program, env, context, Publication::Publish).await
}

/// # Type parameters
/// - `A`: Application value type. Bounds: `Send + 'static`.
/// - `E`: Domain-error type. Bounds: `Send + 'static`.
/// - `R`: Environment type. Bounds: `Send + Sync + 'static`.
///
/// # Parameters
/// - `program` (`Effect<A, E, R>`): Lazy Effect description executed or transformed here.
/// - `env` (`Arc<R>`): The environment supplying this operation's capabilities.
/// - `context` (`&Context`): Execution context carrying cancellation, scope, and trace state.
///
/// # Returns
/// Type: `Exit<A, E>`.
pub(crate) async fn evaluate_raw<A: Send + 'static, E: Send + 'static, R: Send + Sync + 'static>(
    program: Effect<A, E, R>,
    env: Arc<R>,
    context: &Context,
) -> Exit<A, E> {
    evaluate_inner(program, env, context, Publication::Raw).await
}
// A composed workflow owns the continuation after this sub-interpreter's Done.
// Native waits still acknowledge virtual time; a local bind completion does not.
/// # Type parameters
/// - `A`: Application value type. Bounds: `Send + 'static`.
/// - `E`: Domain-error type. Bounds: `Send + 'static`.
/// - `R`: Environment type. Bounds: `Send + Sync + 'static`.
///
/// # Parameters
/// - `program` (`Effect<A, E, R>`): Lazy Effect description executed or transformed here.
/// - `env` (`Arc<R>`): The environment supplying this operation's capabilities.
/// - `context` (`&Context`): Execution context carrying cancellation, scope, and trace state.
///
/// # Returns
/// Type: `Exit<A, E>`.
pub(crate) async fn evaluate_composed<
    A: Send + 'static,
    E: Send + 'static,
    R: Send + Sync + 'static,
>(
    program: Effect<A, E, R>,
    env: Arc<R>,
    context: &Context,
) -> Exit<A, E> {
    evaluate_inner(program, env, context, Publication::Continue).await
}
/// Variants of publication used by internal runtime.
enum Publication {
    Publish,
    Raw,
    Continue,
}
///
/// # Type parameters
/// - `A`: Bounds: `Send + 'static`.
/// - `E`: Bounds: `Send + 'static`.
/// - `R`: Bounds: `Send + Sync + 'static`.
///
/// # Parameters
/// - `program` (`Effect<A, E, R>`): Lazy Effect description executed or transformed here.
/// - `env` (`Arc<R>`): The environment supplying this operation's capabilities.
/// - `context` (`&Context`): Execution context carrying cancellation, scope, and trace state.
/// - `publication` (`Publication`): The publication supplied to this operation.
///
/// # Returns
/// Type: `Exit<A, E>`.
/// Returns `Exit<A, E>`.
async fn evaluate_inner<A: Send + 'static, E: Send + 'static, R: Send + Sync + 'static>(
    program: Effect<A, E, R>,
    env: Arc<R>,
    context: &Context,
    publication: Publication,
) -> Exit<A, E> {
    let output = Arc::new(Mutex::new(None));
    let completed = Arc::clone(&output);
    let mut step = program.execute(
        env,
        context.clone(),
        continuation(move |outcome| {
            *completed
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(outcome);
            Step::Done
        }),
    );
    poll_fn(move |cx| {
        // Advance the current typed transition, then return control to the
        // host before executing its successor. No arbitrary batching quantum
        // configures scheduling outside R. A native wait keeps its own waker.
        let current = std::mem::replace(&mut step, Step::Done);
        match current {
            Step::Done => {}
            Step::More(next) => {
                step = match next.take() {
                    Some(next) => next(),
                    None => Step::Done,
                };
            }
            Step::Suspended(mut pending) => match pending.as_mut().poll(cx) {
                Poll::Ready(next) => step = next,
                Poll::Pending => {
                    step = Step::Suspended(pending);
                    context.acknowledge_clock();
                    return Poll::Pending;
                }
            },
            Step::Composed(mut pending) => match pending.as_mut().poll(cx) {
                Poll::Ready(next) => step = next,
                Poll::Pending => {
                    step = Step::Composed(pending);
                    return Poll::Pending;
                }
            },
        }
        if matches!(step, Step::Done) {
            if !matches!(publication, Publication::Continue) {
                context.acknowledge_clock();
            }
            let outcome = output
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take();
            let outcome = outcome.unwrap_or_else(|| {
                Exit::Die(Defect::from(LibraryDefect::MissingInterpreterOutcome))
            });
            return Poll::Ready(match publication {
                Publication::Publish | Publication::Continue => publish(outcome, context),
                Publication::Raw => outcome,
            });
        }
        cx.waker().wake_by_ref();
        Poll::Pending
    })
    .await
}
