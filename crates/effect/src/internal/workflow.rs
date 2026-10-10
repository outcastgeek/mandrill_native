//! Typed binding bridge for the effect macro.

use super::destruction::FlatDrop;
use super::runtime::{self, Context, Finish, Step};
use crate::LibraryDefect;
use crate::{Defect, Effect, Exit, ResourceBinding, ResourceValue};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;

/// Type alias for operation in internal workflow.
///
/// # Type parameters
/// - `A`: Application value type.
/// - `E`: Domain-error type.
/// - `R`: Environment type.
type Operation<A, E, R> = Box<dyn FnOnce(Arc<R>, Context, Finish<A, E>) -> Step + Send>;

/// Single-run typed construction used by `effect!`; contains no public runtime access.
///
/// # Type parameters
/// - `A`: Application value type. Bound or type: `Send + 'static`.
/// - `E`: Domain-error type. Bound or type: `Send + 'static`.
/// - `R`: Environment type. Bound or type: `Send + Sync + 'static`.
pub struct Workflow<A: Send + 'static, E: Send + 'static, R: Send + Sync + 'static> {
    operation: FlatDrop<Operation<A, E, R>>,
    check_cancellation: bool,
}

impl<A: Send + 'static, E: Send + 'static, R: Send + Sync + 'static> Workflow<A, E, R> {
    /// # Type parameters
    /// - `A`: Success or application-value type.
    /// - `E`: Domain-error type.
    /// - `R`: Environment type.
    ///
    /// # Parameters
    /// - `operation` (`impl FnOnce(Arc<R>, Context, Finish<A, E>) -> Step + Send + 'static`): Typed operation definition or callback selected for execution.
    ///
    /// # Returns
    /// Type: `Self`.
    pub(crate) fn new(
        operation: impl FnOnce(Arc<R>, Context, Finish<A, E>) -> Step + Send + 'static,
    ) -> Self {
        Self {
            operation: FlatDrop::new(Box::new(operation)),
            check_cancellation: true,
        }
    }

    /// # Type parameters
    /// - `R`: Environment type. Bound: `Send + Sync + 'static`.
    /// - `A`: Success or application-value type.
    /// - `E`: Domain-error type.
    ///
    /// # Parameters
    /// - `outcome` (`Exit<A, E>`): Complete terminal outcome, including expected errors and cleanup failures.
    ///
    /// # Returns
    /// Type: `Self`.
    pub(crate) fn from_exit(outcome: Exit<A, E>) -> Self {
        let mut workflow = Self::new(move |_, _, finish| runtime::finish(finish, outcome));
        workflow.check_cancellation = false;
        workflow
    }

    /// # Type parameters
    /// - `A`: Success or application-value type.
    /// - `E`: Domain-error type.
    /// - `R`: Environment type.
    ///
    /// # Parameters
    /// - `env` (`Arc<R>`): The environment supplying this operation's capabilities.
    /// - `context` (`Context`): Execution context carrying cancellation, scope, and trace state.
    /// - `finish` (`Finish<A, E>`): Continuation receiving the complete terminal outcome.
    ///
    /// # Returns
    /// Type: `Step`.
    /// Preserve native instrumentation across this one-shot transition and restore
    /// its entry context before continuing through the shared trampoline.
    pub(crate) fn execute(self, env: Arc<R>, context: Context, finish: Finish<A, E>) -> Step {
        let instrumentation_context = context.clone();
        let finish = runtime::instrument_finish(finish, context.clone());
        let step = runtime::more(move || {
            if self.check_cancellation && context.cancelled() {
                return runtime::finish(finish, Exit::Interrupted);
            }
            match self.operation.take() {
                Some(operation) => operation(env, context, finish),
                None => runtime::finish(
                    finish,
                    Exit::Die(Defect::from(LibraryDefect::WorkflowConsumed)),
                ),
            }
        });
        runtime::instrument_step(step, instrumentation_context)
    }
}

/// Move this run's value into its success channel without requiring Clone.
#[must_use]
/// # Type parameters
/// - `A`: Application value type. Bounds: `Send + 'static`.
/// - `E`: Domain-error type. Bounds: `Send + 'static`.
/// - `R`: Environment type. Bounds: `Send + Sync + 'static`.
///
/// # Parameters
/// - `value` (`A`): Value read, transformed, encoded, or stored by this operation.
///
/// # Returns
/// Type: `Workflow<A, E, R>`.
pub fn success<A: Send + 'static, E: Send + 'static, R: Send + Sync + 'static>(
    value: A,
) -> Workflow<A, E, R> {
    Workflow::from_exit(Exit::Success(value))
}

/// Single-use lazy block for control-flow expansion.
///
/// # Type parameters
/// - `A`: Application value type. Additional bound: `Send + 'static`.
/// - `E`: Domain-error type. Additional bound: `Send + 'static`.
/// - `R`: Environment type. Additional bound: `Send + Sync + 'static`.
///
/// # Parameters
/// - `factory` (`impl FnOnce() -> Workflow<A, E, R> + Send + 'static`): Factory invoked to create fresh data for an execution.
///
/// # Returns
/// Type: `Workflow<A, E, R>`.
pub fn defer<A, E, R>(
    factory: impl FnOnce() -> Workflow<A, E, R> + Send + 'static,
) -> Workflow<A, E, R>
where
    A: Send + 'static,
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    Workflow::new(
        move |env, context, finish| match catch_unwind(AssertUnwindSafe(factory)) {
            Ok(workflow) => workflow.execute(env, context, finish),
            Err(panic) => runtime::finish(finish, Exit::Die(Defect::from_panic(panic))),
        },
    )
}

/// Typed binding; the continuation is invoked once for this run and only on success.
#[must_use]
/// # Type parameters
/// - `P`: Payload type. Additional bound: `Bindable<E, R>`.
/// - `B`: Transformed value type. Additional bound: `Send + 'static`.
/// - `E`: Domain-error type. Additional bound: `Send + 'static`.
/// - `R`: Environment type. Additional bound: `Send + Sync + 'static`.
///
/// # Parameters
/// - `program` (`P`): Lazy Effect description executed or transformed here.
/// - `next` (`impl FnOnce(P::Value) -> Workflow<B, E, R> + Send + 'static`): Next value or state produced by the transition.
///
/// # Returns
/// Type: `Workflow<B, E, R>`.
pub fn bind<P, B, E, R>(
    program: P,
    next: impl FnOnce(P::Value) -> Workflow<B, E, R> + Send + 'static,
) -> Workflow<B, E, R>
where
    P: Bindable<E, R>,
    B: Send + 'static,
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    program.bind_workflow(next)
}

mod sealed {
    /// Seals the Effect binding bridge to supported workflow operations.
    pub trait Sealed {}
    impl<A: Send + 'static, E: Send + 'static, R: Send + Sync + 'static> Sealed
        for crate::Effect<A, E, R>
    {
    }
    impl<T: Send + Sync + 'static, E: Send + 'static, R: Send + Sync + 'static> Sealed
        for crate::ResourceBinding<T, E, R>
    {
    }
    impl<A: Send + 'static, E: Send + 'static, R: Send + Sync + 'static> Sealed
        for super::Workflow<A, E, R>
    {
    }
}

/// Safe macro dispatch for ordinary effects and typed ownership declarations.
pub trait Bindable<E: Send + 'static, R: Send + Sync + 'static>: sealed::Sealed {
    /// Value admitted by this sealed macro binding.
    type Value: Send + 'static;
    /// # Type parameters
    /// - `B`: Transformed value type. Bounds: `Send + 'static`.
    /// - `E`: Domain-error type.
    /// - `R`: Environment type.
    ///
    /// # Parameters
    /// - `next` (`impl FnOnce(Self::Value) -> Workflow<B, E, R> + Send + 'static`): Next value or state produced by the transition.
    ///
    /// # Returns
    /// Type: `Workflow<B, E, R>`.
    fn bind_workflow<B: Send + 'static>(
        self,
        next: impl FnOnce(Self::Value) -> Workflow<B, E, R> + Send + 'static,
    ) -> Workflow<B, E, R>;
}

impl<A: Send + 'static, E: Send + 'static, R: Send + Sync + 'static> Bindable<E, R>
    for Effect<A, E, R>
{
    /// Type alias for value in internal workflow.
    type Value = A;
    ///
    /// # Type parameters
    /// - `B`: Bounds: `Send + 'static`.
    /// - `A`: Success or application-value type.
    /// - `E`: Domain-error type.
    /// - `R`: Environment type.
    ///
    /// # Parameters
    /// - `next` (`impl FnOnce(A) -> Workflow<B, E, R> + Send + 'static`): Next value or state produced by the transition.
    ///
    /// # Returns
    /// Type: `Workflow<B, E, R>`.
    /// Returns `Workflow<B, E, R>`.
    fn bind_workflow<B: Send + 'static>(
        self,
        next: impl FnOnce(A) -> Workflow<B, E, R> + Send + 'static,
    ) -> Workflow<B, E, R> {
        bind_effect(self, next)
    }
}

impl<T: Send + Sync + 'static, E: Send + 'static, R: Send + Sync + 'static> Bindable<E, R>
    for ResourceBinding<T, E, R>
{
    /// Type alias for value in internal workflow.
    type Value = ResourceValue<T>;
    ///
    /// # Type parameters
    /// - `B`: Bounds: `Send + 'static`.
    /// - `E`: Domain-error type.
    /// - `R`: Environment type.
    /// - `T`: Value or item type.
    ///
    /// # Parameters
    /// - `next` (`impl FnOnce(ResourceValue<T>) -> Workflow<B, E, R> + Send + 'static`): Next value or state produced by the transition.
    ///
    /// # Returns
    /// Type: `Workflow<B, E, R>`.
    /// Returns `Workflow<B, E, R>`.
    fn bind_workflow<B: Send + 'static>(
        self,
        next: impl FnOnce(ResourceValue<T>) -> Workflow<B, E, R> + Send + 'static,
    ) -> Workflow<B, E, R> {
        self.bind_run(next)
    }
}

impl<A: Send + 'static, E: Send + 'static, R: Send + Sync + 'static> Bindable<E, R>
    for Workflow<A, E, R>
{
    /// Type alias for value in internal workflow.
    type Value = A;
    ///
    /// # Type parameters
    /// - `B`: Bounds: `Send + 'static`.
    /// - `A`: Success or application-value type.
    /// - `E`: Domain-error type.
    /// - `R`: Environment type.
    ///
    /// # Parameters
    /// - `next` (`impl FnOnce(A) -> Workflow<B, E, R> + Send + 'static`): Next value or state produced by the transition.
    ///
    /// # Returns
    /// Type: `Workflow<B, E, R>`.
    /// Returns `Workflow<B, E, R>`.
    fn bind_workflow<B: Send + 'static>(
        self,
        next: impl FnOnce(A) -> Workflow<B, E, R> + Send + 'static,
    ) -> Workflow<B, E, R> {
        bind_operation(
            move |env, context, finish| self.execute(env, context, finish),
            next,
        )
    }
}

///
/// # Type parameters
/// - `A`: Successful application-value type. Additional bound: `Send + 'static`.
/// - `B`: Transformed success-value type. Additional bound: `Send + 'static`.
/// - `E`: Expected application-error type. Additional bound: `Send + 'static`.
/// - `R`: Environment type supplying required capabilities. Additional bound: `Send + Sync + 'static`.
///
/// # Parameters
/// - `program` (`Effect<A, E, R>`): Lazy Effect description executed or transformed here.
/// - `next` (`impl FnOnce(A) -> Workflow<B, E, R> + Send + 'static`): Next value or state produced by the transition.
///
/// # Returns
/// Type: `Workflow<B, E, R>`.
/// Returns `Workflow<B, E, R>`.
fn bind_effect<A, B, E, R>(
    program: Effect<A, E, R>,
    next: impl FnOnce(A) -> Workflow<B, E, R> + Send + 'static,
) -> Workflow<B, E, R>
where
    A: Send + 'static,
    B: Send + 'static,
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    bind_operation(
        move |env, context, finish| program.execute(env, context, finish),
        next,
    )
}

///
/// # Type parameters
/// - `A`: Successful application-value type. Additional bound: `Send + 'static`.
/// - `B`: Transformed success-value type. Additional bound: `Send + 'static`.
/// - `E`: Expected application-error type. Additional bound: `Send + 'static`.
/// - `R`: Environment type supplying required capabilities. Additional bound: `Send + Sync + 'static`.
///
/// # Parameters
/// - `execute` (`impl FnOnce(Arc<R>, Context, Finish<A, E>) -> Step + Send + 'static`): The execute supplied to this operation.
/// - `next` (`impl FnOnce(A) -> Workflow<B, E, R> + Send + 'static`): Next value or state produced by the transition.
///
/// # Returns
/// Type: `Workflow<B, E, R>`.
/// Returns `Workflow<B, E, R>`.
fn bind_operation<A, B, E, R>(
    execute: impl FnOnce(Arc<R>, Context, Finish<A, E>) -> Step + Send + 'static,
    next: impl FnOnce(A) -> Workflow<B, E, R> + Send + 'static,
) -> Workflow<B, E, R>
where
    A: Send + 'static,
    B: Send + 'static,
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    Workflow::new(move |env, context, finish| {
        let next = FlatDrop::new(Box::new(next));
        execute(
            Arc::clone(&env),
            context.clone(),
            runtime::continuation(move |outcome| {
                runtime::more(move || match outcome.into_failure() {
                    Err(failure) => runtime::finish(finish, failure),
                    Ok(_) if context.cancelled() => runtime::finish(finish, Exit::Interrupted),
                    Ok(value) => {
                        let workflow =
                            catch_unwind(AssertUnwindSafe(|| next.take().map(|next| next(value))));
                        match workflow {
                            Ok(Some(workflow)) => workflow.execute(env, context, finish),
                            Ok(None) => runtime::finish(
                                finish,
                                Exit::Die(Defect::from(LibraryDefect::BindingConsumed)),
                            ),
                            Err(panic) => {
                                runtime::finish(finish, Exit::Die(Defect::from_panic(panic)))
                            }
                        }
                    }
                })
            }),
        )
    })
}

/// A fresh workflow factory is invoked lazily on every run.
///
/// # Type parameters
/// - `A`: Application value type. Additional bound: `Send + 'static`.
/// - `E`: Domain-error type. Additional bound: `Send + 'static`.
/// - `R`: Environment type. Additional bound: `Send + Sync + 'static`.
///
/// # Parameters
/// - `factory` (`impl Fn() -> Workflow<A, E, R> + Send + Sync + 'static`): Factory invoked to create fresh data for an execution.
///
/// # Returns
/// Type: `Effect<A, E, R>`.
/// Returns a lazy `Effect` with the declared success, error, and environment types.
pub fn reusable<A, E, R>(
    factory: impl Fn() -> Workflow<A, E, R> + Send + Sync + 'static,
) -> Effect<A, E, R>
where
    A: Send + 'static,
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    Effect::new(
        move |env, context, finish| match catch_unwind(AssertUnwindSafe(&factory)) {
            Ok(workflow) => workflow.execute(env, context, finish),
            Err(panic) => runtime::finish(finish, Exit::Die(Defect::from_panic(panic))),
        },
    )
}

/// One-shot deferred body used when an RPC/activity callback transfers its owned input.
/// The factory runs on execution, never callback construction. Reusing the resulting Effect
/// reports the existing consumed-workflow defect instead of cloning the input payload.
#[doc(hidden)]
pub fn once<A, E, R>(
    factory: impl FnOnce() -> Workflow<A, E, R> + Send + 'static,
) -> Effect<A, E, R>
where
    A: Send + 'static,
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    let factory = std::sync::Mutex::new(Some(factory));
    Effect::new(move |env, context, finish| {
        let factory = factory
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        match factory {
            Some(factory) => match catch_unwind(AssertUnwindSafe(factory)) {
                Ok(body) => body.execute(env, context, finish),
                Err(panic) => runtime::finish(finish, Exit::Die(Defect::from_panic(panic))),
            },
            None => runtime::finish(
                finish,
                Exit::Die(Defect::from(LibraryDefect::WorkflowConsumed)),
            ),
        }
    })
}

/// Adapt a reusable one-input body factory into a lazy callback. The owned input
/// enters the existing one-shot Effect adapter; user initializers run only at execution.
/// Cloning descriptions shares only the factory handle and never copies input payloads.
#[doc(hidden)]
pub fn defer_callback1<I, A, E, R>(
    factory: impl Fn(I) -> Effect<A, E, R> + Send + Sync + 'static,
) -> impl Fn(I) -> Effect<A, E, R> + Send + Sync + 'static
where
    I: Send + 'static,
    A: Send + 'static,
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    let factory = std::sync::Arc::new(factory);
    move |input| {
        let factory = std::sync::Arc::clone(&factory);
        Effect::sync_once(move || input).flat_map(move |input| factory(input))
    }
}
/// Adapt a zero-input body factory without invoking it when its Effect is constructed.
#[doc(hidden)]
pub fn defer_callback0<A, E, R>(
    factory: impl Fn() -> Effect<A, E, R> + Send + Sync + 'static,
) -> impl Fn() -> Effect<A, E, R> + Send + Sync + 'static
where
    A: Send + 'static,
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    let factory = std::sync::Arc::new(factory);
    move || {
        let factory = std::sync::Arc::clone(&factory);
        Effect::defer(move || factory())
    }
}
/// Adapt a typed two-input body factory while moving both invocation values once.
/// The pair is an internal ownership carrier, never an erased runtime message.
#[doc(hidden)]
pub fn defer_callback2<I, J, A, E, R>(
    factory: impl Fn(I, J) -> Effect<A, E, R> + Send + Sync + 'static,
) -> impl Fn(I, J) -> Effect<A, E, R> + Send + Sync + 'static
where
    I: Send + 'static,
    J: Send + 'static,
    A: Send + 'static,
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    let invoke = defer_callback1(move |(left, right)| factory(left, right));
    move |left, right| invoke((left, right))
}
