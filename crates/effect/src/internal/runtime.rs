//! Private Effect runtime coordination.

use super::runtime_storage::{Consume, Once, Pending, TaggedStep};
mod prepared;
use crate::LibraryDefect;
use crate::memory::{FramePool, FrameReservation, MemoryError, RunFrames, SharedRuntimeFrame};
use crate::{Cancellation, Defect, Effect, Exit};
#[doc(hidden)]
pub use prepared::graph as prepared_graph;
pub use prepared::{
    ConstructionShape, ConstructionSite, ConstructionVacancy, DynamicRunContract, LeasedRunFuture,
    LeasedRunResult, PreparedCancellation, PreparedCancellationObservation,
    PreparedCancellationSignalWait, PreparedCancellationWait, PreparedConstructionPool,
    PreparedDriver, PreparedInvocation, PreparedNativeAdmission, PreparedNativeFuture,
    PreparedNativeReservation, PreparedOutput, PreparedOutputOwnerReservation,
    PreparedOutputOwnerView, PreparedOwnedDriver, PreparedPollHook, PreparedPollInstrumentation,
    PreparedRun, PreparedRunContext,
    PreparedRunDemand, PreparedRunPlan, ReservedOutputOrigins, construction_site,
    join_prepared_native,
};
pub(crate) use prepared::{NativeGraphOrigins, panic_defect};
use std::future::{Future, poll_fn};
use std::ops::Deref;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::task::Poll;

/// Type alias for finish in internal runtime.
///
/// # Type parameters
/// - `A`: Application value type.
/// - `E`: Domain-error type.
pub(crate) type Finish<A, E> = Once<Exit<A, E>, Step>;

/// Variants of step used by internal runtime.
pub(crate) enum Step {
    Done,
    More(Once<(), Step>),
    Suspended(Pending),
    Composed(Pending),
    /// Select native context for the next transition without wrapping its callback.
    Contextual(
        /// Next executable transition; consumed iteratively before polling it.
        TaggedStep,
        /// Native context to select for that transition and its current wait.
        PollContext,
    ),
}

impl Step {
    pub(crate) fn suspended(pending: Pin<Box<dyn Future<Output = Step> + Send>>) -> Self {
        Self::Suspended(Pending::System(pending))
    }
    pub(crate) fn composed(pending: Pin<Box<dyn Future<Output = Step> + Send>>) -> Self {
        Self::Composed(Pending::System(pending))
    }
}

/// Native state of the current trampoline transition, independent of its owner.
/// Replaced at context tags; it never accumulates prior transition callbacks.
pub(crate) struct PollContext {
    /// The trace snapshot belonging to this transition or native wait.
    trace: crate::TraceContext,
    /// Environment-selected native hook, absent for ordinary core execution.
    instrumentation: Option<Arc<dyn crate::PollInstrumentation>>,
}

#[derive(Clone)]
/// Typed context used by internal runtime.
pub(crate) struct Context {
    pub(crate) cancellation: Cancellation,
    origin: FailureOriginOwner,
    pub(crate) trace: crate::TraceContext,
    /// Optional native poll context chosen through the application environment.
    pub(crate) instrumentation: Option<Arc<dyn crate::PollInstrumentation>>,
    clock_frontier: ClockFrontier,
    cleanup: bool,
}

/// Typed failure origin used by internal runtime.
struct FailureOrigin {
    protected: AtomicBool,
    fiber: bool,
    frontier: crate::clock::Frontier,
    storage: Option<RunStorage>,
}
enum RunStorage {
    Frames(RunFrames),
    Inherited(FailureOriginOwner),
}

enum FailureOriginOwner {
    Ordinary(Arc<FailureOrigin>),
    Prepared(SharedRuntimeFrame),
}

impl Clone for FailureOriginOwner {
    #[allow(clippy::expect_used)] // Only an unrepresentable live reference count can fail.
    fn clone(&self) -> Self {
        match self {
            Self::Ordinary(origin) => Self::Ordinary(Arc::clone(origin)),
            Self::Prepared(origin) => {
                Self::Prepared(origin.try_clone().expect("parallel origin reference count"))
            }
        }
    }
}

impl Deref for FailureOriginOwner {
    type Target = FailureOrigin;

    fn deref(&self) -> &FailureOrigin {
        match self {
            Self::Ordinary(origin) => origin,
            Self::Prepared(origin) => {
                // SAFETY: ParallelContextReservation alone constructs this
                // variant by sharing a fully initialized FailureOrigin.
                unsafe { &*origin.pointer().cast::<FailureOrigin>().as_ptr() }
            }
        }
    }
}

pub(crate) struct ParallelContextReservation(FrameReservation<'static, FailureOrigin>);

impl ParallelContextReservation {
    pub(crate) fn layout() -> std::alloc::Layout {
        std::alloc::Layout::new::<FailureOrigin>()
    }

    pub(crate) fn reserve(frames: &FramePool<'static>) -> Result<Self, MemoryError> {
        frames.reserve().map(Self)
    }
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
            storage: None,
        }
    }
}

#[derive(Clone)]
/// Variants of clock frontier used by internal runtime.
enum ClockFrontier {
    Own,
    Parent(FailureOriginOwner),
}

impl Context {
    fn storage_origin(&self) -> Option<&FailureOriginOwner> {
        let mut origin = &self.origin;
        loop {
            match &origin.storage {
                Some(RunStorage::Frames(_)) => return Some(origin),
                Some(RunStorage::Inherited(parent)) => origin = parent,
                None => return None,
            }
        }
    }
    fn inherit_storage(&self) -> Option<RunStorage> {
        self.storage_origin()
            .map(|origin| RunStorage::Inherited(origin.clone()))
    }
    pub(crate) fn frames(&self) -> Option<&FramePool<'static>> {
        self.storage_origin()
            .and_then(|origin| match &origin.storage {
                Some(RunStorage::Frames(pools)) => Some(if self.cleanup {
                    &pools.cleanup
                } else {
                    &pools.work
                }),
                Some(RunStorage::Inherited(_)) | None => None,
            })
    }
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
            cleanup: true,
            trace: self.trace.clone(),
            instrumentation: self.instrumentation.clone(),
            origin: FailureOriginOwner::Ordinary(Arc::new(FailureOrigin {
                storage: self.inherit_storage(),
                ..FailureOrigin::root()
            })),
            clock_frontier: ClockFrontier::Parent(self.clock_origin().clone()),
        }
    }

    /// # Returns
    /// Type: `Self`.
    pub(crate) fn child(&self) -> Self {
        Self {
            cancellation: self.cancellation.child(),
            cleanup: self.cleanup,
            trace: self.trace.clone(),
            instrumentation: self.instrumentation.clone(),
            origin: FailureOriginOwner::Ordinary(Arc::new(FailureOrigin {
                protected: AtomicBool::new(false),
                fiber: true,
                frontier: const { crate::clock::Frontier::new() },
                storage: self.inherit_storage(),
            })),
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
        self.parallel_with(
            cancellation,
            FailureOriginOwner::Ordinary(Arc::new(self.parallel_origin(self.origin.fiber))),
        )
    }

    pub(crate) fn parallel_prepared(
        &self,
        cancellation: Cancellation,
        reservation: ParallelContextReservation,
    ) -> Self {
        self.parallel_with(
            cancellation,
            FailureOriginOwner::Prepared(
                reservation.0.share(self.parallel_origin(self.origin.fiber)),
            ),
        )
    }

    pub(crate) fn child_prepared(
        &self,
        cancellation: Cancellation,
        reservation: ParallelContextReservation,
    ) -> Self {
        self.parallel_with(
            cancellation,
            FailureOriginOwner::Prepared(reservation.0.share(self.parallel_origin(true))),
        )
    }

    fn parallel_origin(&self, fiber: bool) -> FailureOrigin {
        FailureOrigin {
            protected: AtomicBool::new(false),
            fiber,
            frontier: const { crate::clock::Frontier::new() },
            storage: self.inherit_storage(),
        }
    }

    fn parallel_with(&self, cancellation: Cancellation, origin: FailureOriginOwner) -> Self {
        Self {
            cancellation,
            cleanup: self.cleanup,
            trace: self.trace.clone(),
            instrumentation: self.instrumentation.clone(),
            origin,
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
    /// Type: `&FailureOriginOwner`.
    fn clock_origin(&self) -> &FailureOriginOwner {
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
    Step::More(Once::system(move |()| next()))
}

/// Tag the next transition with its native context. A more specific tag already
/// returned by a nested execution wins; no closure calls another tagged closure.
/// The interpreter consumes tags iteratively and keeps only its current context.
pub(crate) fn instrument_step(step: Step, context: Context) -> Step {
    match step {
        Step::Done | Step::Contextual(_, _) => step,
        Step::More(_) | Step::Suspended(_) | Step::Composed(_) => Step::Contextual(
            TaggedStep::System(Box::new(step)),
            PollContext {
                trace: context.trace,
                instrumentation: context.instrumentation,
            },
        ),
    }
}

/// Enter an operation with the ordinary cancellation, clock and native
/// instrumentation boundaries. The selected owner supplies the operation.
pub(crate) fn operation_entry<A, E>(
    context: Context,
    finish: Finish<A, E>,
    operation: impl FnOnce(Context, Finish<A, E>) -> Step + Send + 'static,
) -> Step
where
    A: Send + 'static,
    E: Send + 'static,
{
    let instrumentation_context = context.clone();
    let finish = instrument_finish(finish, context.clone());
    let step = more(move || {
        if !begin_operation(&context) {
            return self::finish(finish, Exit::Interrupted);
        }
        operation(context, finish)
    });
    instrument_step(step, instrumentation_context)
}

pub(crate) fn begin_operation(context: &Context) -> bool {
    context.begin_operation();
    !context.cancelled()
}

/// Same context-tag protocol using a pre-held physical tag; a nested tag still
/// wins exactly as in the System adapter. No allocation or capacity check here.
pub(crate) fn instrument_step_reserved(
    step: Step,
    context: Context,
    tag: crate::memory::FrameReservation<'static, Step>,
) -> Step {
    match step {
        Step::Done | Step::Contextual(_, _) => step,
        Step::More(_) | Step::Suspended(_) | Step::Composed(_) => Step::Contextual(
            TaggedStep::prepared(tag, step),
            PollContext {
                trace: context.trace,
                instrumentation: context.instrumentation,
            },
        ),
    }
}

/// Restore the entry context before invoking an operation's caller continuation.
/// The extra boundary is a trampoline transition, not a recursive callback call.
/// This makes a completed child span restore its parent's context and can clear
/// a locally selected hook when its enclosing caller had no instrumentation.
pub(crate) fn instrument_finish<A: Send + 'static, E: Send + 'static>(
    callback: Finish<A, E>,
    context: Context,
) -> Finish<A, E> {
    continuation(move |outcome| instrument_step(finish(callback, outcome), context))
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
    more(move || callback.call(outcome).unwrap_or(Step::Done))
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
    Once::system(callback)
}

/// Attach a checked callback reservation to the existing terminal continuation.
/// Payload consumption alone does not return settlement capacity. Captures drop
/// before the retained frame on abandonment; no extra continuation is introduced.
pub(crate) fn continuation_retained<A: Send + 'static, E: Send + 'static>(
    callback: impl FnOnce(Exit<A, E>) -> Step + Send + 'static,
    frame: Option<crate::memory::RuntimeFrame>,
) -> Finish<A, E> {
    Once::retained_system(callback, frame)
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
    publish_flags(
        outcome,
        context.cancelled(),
        context.origin.fiber,
        context.origin.protected.load(Ordering::Relaxed),
    )
}
fn publish_flags<A, E>(
    outcome: Exit<A, E>,
    cancelled: bool,
    fiber: bool,
    protected: bool,
) -> Exit<A, E> {
    if !cancelled {
        return outcome;
    }
    match outcome {
        Exit::Success(_) => Exit::Interrupted,
        Exit::Fail(error) if fiber && protected => Exit::Fail(error),
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
        instrumentation: None,
        origin: FailureOriginOwner::Ordinary(Arc::new(const { FailureOrigin::root() })),
        clock_frontier: ClockFrontier::Own,
        cleanup: false,
    };
    evaluate_context(program, env, &context).await
}

/// Select explicit prepared frame storage for this run. Context, output and
/// legacy operation ownership still use their existing domains; this entry does
/// not claim that an opaque Effect has a complete prepared allocation contract.
pub(crate) async fn evaluate_frames<
    A: Send + 'static,
    E: Send + 'static,
    R: Send + Sync + 'static,
>(
    program: Effect<A, E, R>,
    env: Arc<R>,
    cancellation: Cancellation,
    frames: RunFrames,
) -> Exit<A, E> {
    let context = Context {
        cancellation,
        trace: crate::TraceContext::default(),
        instrumentation: None,
        origin: FailureOriginOwner::Ordinary(Arc::new(FailureOrigin {
            storage: Some(RunStorage::Frames(frames)),
            ..FailureOrigin::root()
        })),
        clock_frontier: ClockFrontier::Own,
        cleanup: false,
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

type NestedOutput<A, E> = Mutex<Option<Exit<A, E>>>;

struct NestedStore<A: Send + 'static, E: Send + 'static> {
    output: crate::memory::SharedRuntimeFrame,
    marker: std::marker::PhantomData<fn() -> (A, E)>,
}

fn nested_output<A: Send + 'static, E: Send + 'static>(
    output: &crate::memory::SharedRuntimeFrame,
) -> &NestedOutput<A, E> {
    // SAFETY: only PreparedNestedReservation creates these owners, placing
    // this exact Mutex<Option<Exit<A,E>>> in the shared typed frame.
    unsafe { &*output.pointer().as_ptr().cast::<NestedOutput<A, E>>() }
}

impl<A: Send + 'static, E: Send + 'static> Consume<Exit<A, E>, Step> for NestedStore<A, E> {
    fn consume(self, outcome: Exit<A, E>) -> Step {
        *nested_output::<A, E>(&self.output)
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(outcome);
        Step::Done
    }
}

pub(crate) struct PreparedNestedReservation<A: Send + 'static, E: Send + 'static> {
    output: crate::memory::SharedRuntimeFrame,
    callback: crate::memory::SharedRuntimeFrame,
    store: crate::memory::FrameReservation<'static, NestedStore<A, E>>,
}
impl<A: Send + 'static, E: Send + 'static> PreparedNestedReservation<A, E> {
    pub(crate) fn layouts() -> (std::alloc::Layout, std::alloc::Layout) {
        (
            std::alloc::Layout::new::<NestedOutput<A, E>>(),
            std::alloc::Layout::new::<NestedStore<A, E>>(),
        )
    }
    pub(crate) fn reserve(pool: &FramePool<'static>) -> Result<Self, crate::memory::MemoryError> {
        let output = pool.reserve::<NestedOutput<A, E>>()?.share(Mutex::new(None));
        let callback = output.try_clone()?;
        let store = pool.reserve::<NestedStore<A, E>>()?;
        Ok(Self {
            output,
            callback,
            store,
        })
    }
}

pub(crate) async fn evaluate_context_reserved<A, E>(
    context: &Context,
    reservation: PreparedNestedReservation<A, E>,
    entry: impl FnOnce(Finish<A, E>) -> Step,
) -> Exit<A, E>
where
    A: Send + 'static,
    E: Send + 'static,
{
    evaluate_reserved(context, reservation, entry, Publication::Publish).await
}

pub(crate) async fn evaluate_raw_reserved<A: Send + 'static, E: Send + 'static>(
    context: &Context,
    reservation: PreparedNestedReservation<A, E>,
    entry: impl FnOnce(Finish<A, E>) -> Step,
) -> Exit<A, E> {
    evaluate_reserved(context, reservation, entry, Publication::Raw).await
}

async fn evaluate_reserved<A: Send + 'static, E: Send + 'static>(
    context: &Context,
    reservation: PreparedNestedReservation<A, E>,
    entry: impl FnOnce(Finish<A, E>) -> Step,
    publication: Publication,
) -> Exit<A, E> {
    let PreparedNestedReservation {
        output,
        callback,
        store,
    } = reservation;
    let step = entry(Once::prepared(
        store,
        NestedStore {
            output: callback,
            marker: std::marker::PhantomData,
        },
    ));
    drive(step, context, publication, || {
        let outcome = nested_output::<A, E>(&output)
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
            .unwrap_or_else(|| Exit::Die(Defect::from(LibraryDefect::MissingInterpreterOutcome)));
        match publication {
            Publication::Publish | Publication::Continue => publish(outcome, context),
            Publication::Raw => outcome,
        }
    }).await
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
#[derive(Clone, Copy)]
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
    evaluate_with(context, publication, |finish| program.execute(env, context.clone(), finish)).await
}

pub(crate) async fn evaluate_raw_with<A: Send + 'static, E: Send + 'static>(
    context: &Context,
    entry: impl FnOnce(Finish<A, E>) -> Step,
) -> Exit<A, E> {
    evaluate_with(context, Publication::Raw, entry).await
}

pub(crate) async fn evaluate_context_with<A: Send + 'static, E: Send + 'static>(
    context: &Context,
    entry: impl FnOnce(Finish<A, E>) -> Step,
) -> Exit<A, E> {
    evaluate_with(context, Publication::Publish, entry).await
}

async fn evaluate_with<A: Send + 'static, E: Send + 'static>(
    context: &Context,
    publication: Publication,
    entry: impl FnOnce(Finish<A, E>) -> Step,
) -> Exit<A, E> {
    let output = Arc::new(Mutex::new(None));
    let completed = Arc::clone(&output);
    let step = entry(
        continuation(move |outcome| {
            *completed
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(outcome);
            Step::Done
        }),
    );
    drive(step, context, publication, move || {
        let outcome = output
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
            .unwrap_or_else(|| Exit::Die(Defect::from(LibraryDefect::MissingInterpreterOutcome)));
        match publication {
            Publication::Publish | Publication::Continue => publish(outcome, context),
            Publication::Raw => outcome,
        }
    })
    .await
}

// One evaluator for ordinary and prepared root ownership. Finish remains inside
// the final native hook guard, preserving publication/drop instrumentation order.
trait DriveContext {
    fn poll_context(&self) -> PollContext;
    fn prepared_instrumentation(
        &self,
    ) -> Option<(&PreparedPollInstrumentation, Option<&crate::PreparedTraceContext>)> {
        None
    }
    fn acknowledge_clock(&self);
}
impl DriveContext for Context {
    fn poll_context(&self) -> PollContext {
        PollContext {
            trace: self.trace.clone(),
            instrumentation: self.instrumentation.clone(),
        }
    }
    fn acknowledge_clock(&self) {
        Context::acknowledge_clock(self);
    }
}
async fn drive<T>(
    mut step: Step,
    context: &impl DriveContext,
    publication: Publication,
    mut finish: impl FnMut() -> T,
) -> T {
    let mut poll_context = context.poll_context();
    poll_fn(move |cx| {
        poll_drive(
            &mut step,
            context,
            &mut poll_context,
            publication,
            &mut finish,
            cx,
        )
    })
    .await
}
fn poll_drive<T>(
    step: &mut Step,
    context: &impl DriveContext,
    poll_context: &mut PollContext,
    publication: Publication,
    finish: &mut impl FnMut() -> T,
    cx: &mut std::task::Context<'_>,
) -> Poll<T> {
    // Advance the current typed transition, then return control to the
    // host before executing its successor. No arbitrary batching quantum
    // configures scheduling outside R. A native wait keeps its own waker.
    let mut current = std::mem::replace(step, Step::Done);
    // Context tags are data, consumed iteratively before one real transition.
    // No historical wrapper/guard survives after its transition returns.
    while let Step::Contextual(inner, selected) = current {
        *poll_context = selected;
        current = inner.take();
    }
    let guard = poll_context
        .instrumentation
        .as_ref()
        .map(|hook| hook.enter(&poll_context.trace));
    let run = || poll_transition(current, step, context, poll_context, publication, finish, cx);
    if guard.is_none() && let Some((hook, trace)) = context.prepared_instrumentation() {
        return hook.with_scope(trace, run);
    }
    run()
}

fn poll_transition<T>(
    current: Step,
    step: &mut Step,
    context: &impl DriveContext,
    poll_context: &mut PollContext,
    publication: Publication,
    finish: &mut impl FnMut() -> T,
    cx: &mut std::task::Context<'_>,
) -> Poll<T> {
    match current {
        Step::Done => {}
        Step::Contextual(inner, selected) => {
            // The loop above consumes every tag; retaining this exhaustive
            // arm also preserves trampoline behavior if tag handling evolves.
            *poll_context = selected;
            *step = inner.take();
        }
        Step::More(next) => {
            *step = next.call(()).unwrap_or(Step::Done);
        }
        Step::Suspended(mut pending) => match pending.poll(cx) {
            Poll::Ready(next) => *step = next,
            Poll::Pending => {
                *step = Step::Suspended(pending);
                context.acknowledge_clock();
                return Poll::Pending;
            }
        },
        Step::Composed(mut pending) => match pending.poll(cx) {
            Poll::Ready(next) => *step = next,
            Poll::Pending => {
                *step = Step::Composed(pending);
                return Poll::Pending;
            }
        },
    }
    if matches!(*step, Step::Done) {
        if !matches!(publication, Publication::Continue) {
            context.acknowledge_clock();
        }
        return Poll::Ready(finish());
    }
    cx.waker().wake_by_ref();
    Poll::Pending
}
