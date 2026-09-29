//! The lazy Effect program and its typed execution combinators.

use crate::LibraryDefect;
use crate::internal::destruction::FlatDrop;
use crate::internal::outcome;
use crate::internal::runtime::{self, Context, Finish, Step};
use crate::internal::workflow::{self, Workflow};
use crate::{
    Cancellation, Defect, Exit, HasCancellation, PrimaryOutcomeRef, Release, ResourceBinding,
    RunDriver, RunObserver,
};
use futures_util::FutureExt;
use std::future::Future;
use std::ops::ControlFlow;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;

/// Shared callback representation. Calling it uses Rust trait-object dispatch;
/// `A`, `E`, and `R` remain statically typed and the vtable is never stored.
///
/// # Type parameters
/// - `A`: Application value type.
/// - `E`: Domain-error type.
/// - `R`: Environment type.
type Operation<A, E, R> = Arc<dyn Fn(Arc<R>, Context, Finish<A, E>) -> Step + Send + Sync>;

/// An opaque, lazy recipe. Cloning shares the recipe, not a running future.
/// The callback has a Rust trait-object vtable; its layout is not part of the
/// Effect API or any persisted representation.
#[must_use = "an Effect is a description; call run to execute it"]
///
/// # Type parameters
/// - `A`: Application value type. Bound or type: `Send + 'static`.
/// - `E`: Domain-error type. Bound or type: `Send + 'static`.
/// - `R`: Environment type. Bound or type: `Send + Sync + 'static = ()`.
pub struct Effect<A: Send + 'static, E: Send + 'static, R: Send + Sync + 'static = ()> {
    operation: FlatDrop<Operation<A, E, R>>,
}

impl<A: Send + 'static, E: Send + 'static, R: Send + Sync + 'static> Clone for Effect<A, E, R> {
    ///
    /// # Returns
    /// Type: `Self`.
    fn clone(&self) -> Self {
        Self {
            operation: self.operation.clone(),
        }
    }
}

impl<A: Send + 'static, E: Send + 'static, R: Send + Sync + 'static> Effect<A, E, R> {
    /// # Type parameters
    /// - `A`: Success or application-value type.
    /// - `E`: Domain-error type.
    /// - `R`: Environment type.
    ///
    /// # Parameters
    /// - `operation` (`impl Fn(Arc<R>, Context, Finish<A, E>) -> Step + Send + Sync + 'static`): Typed operation definition or callback selected for execution.
    ///
    /// # Returns
    /// Type: `Self`.
    pub(crate) fn new(
        operation: impl Fn(Arc<R>, Context, Finish<A, E>) -> Step + Send + Sync + 'static,
    ) -> Self {
        Self {
            operation: FlatDrop::new(Arc::new(operation)),
        }
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
    pub(crate) fn execute(self, env: Arc<R>, context: Context, finish: Finish<A, E>) -> Step {
        runtime::more(move || {
            context.begin_operation();
            if context.cancelled() {
                return runtime::finish(finish, Exit::Interrupted);
            }
            match self.operation.as_ref() {
                Some(operation) => operation(env, context, finish),
                None => runtime::finish(
                    finish,
                    Exit::Die(Defect::from(LibraryDefect::MissingEffectRecipe)),
                ),
            }
        })
    }

    /// # Type parameters
    /// - `A`: Success or application-value type.
    /// - `E`: Domain-error type.
    /// - `R`: Environment type.
    ///
    /// # Parameters
    /// - `callback` (`impl Fn(&R, &Context) -> Exit<A, E> + Send + Sync + 'static`): The callback invoked by this operation.
    ///
    /// # Returns
    /// Type: `Self`.
    pub(crate) fn ready(
        callback: impl Fn(&R, &Context) -> Exit<A, E> + Send + Sync + 'static,
    ) -> Self {
        Self::new(move |env, context, finish| {
            let outcome = catch_unwind(AssertUnwindSafe(|| callback(&env, &context)))
                .unwrap_or_else(|panic| Exit::Die(Defect::from_panic(panic)));
            runtime::finish(finish, outcome)
        })
    }

    /// Fresh owned values can be produced without a Clone bound on A.
    ///
    /// # Type parameters
    /// - `E`: Domain-error type. Bound: `Send + 'static`.
    /// - `R`: Environment type. Bound: `Send + Sync + 'static`.
    /// - `A`: Success or application-value type.
    ///
    /// # Parameters
    /// - `callback` (`impl Fn() -> A + Send + Sync + 'static`): The callback invoked by this operation.
    ///
    /// # Returns
    /// Type: `Self`.
    pub fn sync(callback: impl Fn() -> A + Send + Sync + 'static) -> Self {
        Self::ready(move |_, _| Exit::Success(callback()))
    }

    /// Build a reusable program that succeeds with a clone of `value` on each run.
    ///
    /// # Type parameters
    /// - `E`: Domain-error type. Bound: `Send + 'static`.
    /// - `R`: Environment type. Bound: `Send + Sync + 'static`.
    /// - `A`: Success or application-value type.
    ///
    /// # Parameters
    /// - `value` (`A`): Value read, transformed, encoded, or stored by this operation.
    ///
    /// # Returns
    /// Type: `Self`.
    pub fn succeed(value: A) -> Self
    where
        A: Clone + Sync,
    {
        Self::sync(move || value.clone())
    }

    /// Build a reusable program that fails with a clone of the expected error on each run.
    ///
    /// # Type parameters
    /// - `A`: Application value type. Bound: `Send + 'static`.
    /// - `R`: Environment type. Bound: `Send + Sync + 'static`.
    /// - `E`: Domain-error type.
    ///
    /// # Parameters
    /// - `error` (`E`): The error to propagate or record.
    ///
    /// # Returns
    /// Type: `Self`.
    pub fn fail(error: E) -> Self
    where
        E: Clone + Sync,
    {
        Self::fail_with(move || error.clone())
    }

    /// Call the error factory on each run to produce a fresh expected failure.
    ///
    /// # Type parameters
    /// - `A`: Application value type. Bound: `Send + 'static`.
    /// - `R`: Environment type. Bound: `Send + Sync + 'static`.
    /// - `E`: Domain-error type.
    ///
    /// # Parameters
    /// - `callback` (`impl Fn() -> E + Send + Sync + 'static`): The callback invoked by this operation.
    ///
    /// # Returns
    /// Type: `Self`.
    pub fn fail_with(callback: impl Fn() -> E + Send + Sync + 'static) -> Self {
        Self::ready(move |_, _| Exit::Fail(callback()))
    }

    /// Terminate with an unrecoverable defect instead of an expected `E` failure.
    ///
    /// # Type parameters
    /// - `A`: Application value type. Bound: `Send + 'static`.
    /// - `E`: Domain-error type. Bound: `Send + 'static`.
    /// - `R`: Environment type. Bound: `Send + Sync + 'static`.
    ///
    /// # Parameters
    /// - `defect` (`Defect`): The defect supplied to this operation.
    ///
    /// # Returns
    /// Type: `Self`.
    pub fn die(defect: Defect) -> Self {
        Self::ready(move |_, _| Exit::Die(defect.clone()))
    }

    /// Result errors are mapped explicitly; unwinding panics remain defects.
    ///
    /// # Type parameters
    /// - `R`: Environment type. Bound: `Send + Sync + 'static`.
    /// - `X`: Source error type mapped into the declared error channel.
    /// - `A`: Success or application-value type.
    /// - `E`: Domain-error type.
    ///
    /// # Parameters
    /// - `callback` (`impl Fn() -> Result<A, X> + Send + Sync + 'static`): The callback invoked by this operation.
    /// - `on_error` (`impl Fn(X) -> E + Send + Sync + 'static`): Callback that maps a source error into the declared error type.
    ///
    /// # Returns
    /// Type: `Self`.
    pub fn try_sync<X>(
        callback: impl Fn() -> Result<A, X> + Send + Sync + 'static,
        on_error: impl Fn(X) -> E + Send + Sync + 'static,
    ) -> Self {
        Self::ready(move |_, _| match callback() {
            Ok(value) => Exit::Success(value),
            Err(error) => Exit::Fail(on_error(error)),
        })
    }

    /// Call `factory` on each run, then execute the newly constructed program.
    ///
    /// # Type parameters
    /// - `A`: Application value type. Bound: `Send + 'static`.
    /// - `E`: Domain-error type. Bound: `Send + 'static`.
    /// - `R`: Environment type. Bound: `Send + Sync + 'static`.
    ///
    /// # Parameters
    /// - `factory` (`impl Fn() -> Self + Send + Sync + 'static`): Factory invoked to create fresh data for an execution.
    ///
    /// # Returns
    /// Type: `Self`.
    pub fn defer(factory: impl Fn() -> Self + Send + Sync + 'static) -> Self {
        Self::new(
            move |env, context, finish| match catch_unwind(AssertUnwindSafe(&factory)) {
                Ok(program) => program.execute(env, context, finish),
                Err(panic) => runtime::finish(finish, Exit::Die(Defect::from_panic(panic))),
            },
        )
    }

    /// Await protected cleanup after every started body outcome.
    ///
    /// # Type parameters
    /// - `A`: Application value type. Bound: `Send + 'static`.
    /// - `E`: Domain-error type.
    /// - `R`: Environment type.
    ///
    /// # Parameters
    /// - `finalizer` (`Effect<(), E, R>`): Cleanup description awaited after the primary outcome.
    ///
    /// # Returns
    /// Type: `Self`.
    pub fn ensuring(self, finalizer: Effect<(), E, R>) -> Self {
        self.ensuring_with(move |_| Release::from_effect(finalizer.clone()))
    }

    /// Inspect the borrowed primary outcome when constructing single-use
    /// cleanup, without requiring Clone on an expected error.
    ///
    /// # Type parameters
    /// - `A`: Application value type. Bound: `Send + 'static`.
    /// - `E`: Domain-error type.
    /// - `R`: Environment type.
    ///
    /// # Parameters
    /// - `finalizer` (`impl for<'outcome> Fn(PrimaryOutcomeRef<'outcome, E>) -> Release<E, R> + Send + Sync + 'static`): Cleanup description awaited after the primary outcome.
    ///
    /// # Returns
    /// Type: `Self`.
    pub fn ensuring_with(
        self,
        finalizer: impl for<'outcome> Fn(PrimaryOutcomeRef<'outcome, E>) -> Release<E, R>
        + Send
        + Sync
        + 'static,
    ) -> Self {
        ResourceBinding::new(Effect::succeed(()), move |(), reason| finalizer(reason))
            .use_with(move |_| self.clone())
    }

    /// The factory receives cooperative cancellation and runs once per execution.
    /// Wrapped futures may still require their own reactor/runtime services.
    ///
    /// # Type parameters
    /// - `A`: Application value type. Bound: `Send + 'static`.
    /// - `E`: Domain-error type. Bound: `Send + 'static`.
    /// - `R`: Environment type. Bound: `Send + Sync + 'static`.
    /// - `F`: Bounds: `Future<Output = A> + Send + 'static`.
    ///
    /// # Parameters
    /// - `factory` (`impl Fn(Cancellation) -> F + Send + Sync + 'static`): Factory invoked to create fresh data for an execution.
    ///
    /// # Returns
    /// Type: `Self`.
    pub fn from_future<F: Future<Output = A> + Send + 'static>(
        factory: impl Fn(Cancellation) -> F + Send + Sync + 'static,
    ) -> Self {
        Self::future_exit(move |token| {
            let pending = factory(token);
            async move { Exit::Success(pending.await) }
        })
    }

    /// Run a freshly created fallible future and map its error into `E`.
    ///
    /// # Type parameters
    /// - `A`: Application value type. Bound: `Send + 'static`.
    /// - `R`: Environment type. Bound: `Send + Sync + 'static`.
    /// - `X`: Bounds: `Send + 'static`.
    /// - `F`: Bounds: `Future<Output = Result<A, X>> + Send + 'static`.
    /// - `E`: Domain-error type.
    ///
    /// # Parameters
    /// - `factory` (`impl Fn(Cancellation) -> F + Send + Sync + 'static`): Factory invoked to create fresh data for an execution.
    /// - `on_error` (`impl Fn(X) -> E + Send + Sync + 'static`): Callback that maps a source error into the declared error type.
    ///
    /// # Returns
    /// Type: `Self`.
    pub fn try_future<X: Send + 'static, F: Future<Output = Result<A, X>> + Send + 'static>(
        factory: impl Fn(Cancellation) -> F + Send + Sync + 'static,
        on_error: impl Fn(X) -> E + Send + Sync + 'static,
    ) -> Self {
        let on_error = Arc::new(on_error);
        Self::future_exit(move |token| {
            let pending = factory(token);
            let on_error = Arc::clone(&on_error);
            async move {
                match pending.await {
                    Ok(value) => Exit::Success(value),
                    Err(error) => Exit::Fail(on_error(error)),
                }
            }
        })
    }

    ///
    /// # Type parameters
    /// - `F`: Bounds: `Future<Output = Exit<A, E>> + Send + 'static`.
    ///
    /// # Parameters
    /// - `factory` (`impl Fn(Cancellation) -> F + Send + Sync + 'static`): Factory invoked to create fresh data for an execution.
    ///
    /// # Returns
    /// Type: `Self`.
    fn future_exit<F: Future<Output = Exit<A, E>> + Send + 'static>(
        factory: impl Fn(Cancellation) -> F + Send + Sync + 'static,
    ) -> Self {
        Self::new(move |_, context, finish| {
            match catch_unwind(AssertUnwindSafe(|| factory(context.cancellation.clone()))) {
                Err(panic) => runtime::finish(finish, Exit::Die(Defect::from_panic(panic))),
                Ok(pending) => Step::Suspended(Box::pin(async move {
                    let outcome = AssertUnwindSafe(pending)
                        .catch_unwind()
                        .await
                        .unwrap_or_else(|panic| Exit::Die(Defect::from_panic(panic)));
                    runtime::finish(finish, outcome)
                })),
            }
        })
    }

    /// Read a service or value from the supplied environment when the program runs.
    ///
    /// # Type parameters
    /// - `E`: Domain-error type. Bound: `Send + 'static`.
    /// - `A`: Success or application-value type.
    /// - `R`: Environment type.
    ///
    /// # Parameters
    /// - `project` (`impl Fn(&R) -> A + Send + Sync + 'static`): The project supplied to this operation.
    ///
    /// # Returns
    /// Type: `Self`.
    pub fn service(project: impl Fn(&R) -> A + Send + Sync + 'static) -> Self {
        Self::ready(move |env, _| Exit::Success(project(env)))
    }

    /// Capture one shared environment and remove its requirement from the returned Effect.
    ///
    /// # Type parameters
    /// - `A`: Success or application-value type.
    /// - `E`: Domain-error type.
    /// - `R`: Environment type.
    ///
    /// # Parameters
    /// - `env` (`Arc<R>`): The environment supplying this operation's capabilities.
    ///
    /// # Returns
    /// Type: `Effect<A, E, ()>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    pub fn provide(self, env: Arc<R>) -> Effect<A, E, ()> {
        Effect::new(move |_, context, finish| {
            self.clone().execute(Arc::clone(&env), context, finish)
        })
    }

    /// Project a parent environment into `R` for this subtree while preserving its run context.
    ///
    /// # Type parameters
    /// - `L`: Bounds: `Send + Sync + 'static`.
    /// - `A`: Success or application-value type.
    /// - `E`: Domain-error type.
    /// - `R`: Environment type.
    ///
    /// # Parameters
    /// - `project` (`impl Fn(&L) -> R + Send + Sync + 'static`): The project supplied to this operation.
    ///
    /// # Returns
    /// Type: `Effect<A, E, L>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    pub fn provide_some<L: Send + Sync + 'static>(
        self,
        project: impl Fn(&L) -> R + Send + Sync + 'static,
    ) -> Effect<A, E, L> {
        Effect::new(move |env, context, finish| {
            match catch_unwind(AssertUnwindSafe(|| project(&env))) {
                Ok(env) => self.clone().execute(Arc::new(env), context, finish),
                Err(panic) => runtime::finish(finish, Exit::Die(Defect::from_panic(panic))),
            }
        })
    }

    /// Run this program, then build and run the dependent program from its success value.
    ///
    /// # Type parameters
    /// - `B`: Transformed value type. Bounds: `Send + 'static`.
    /// - `A`: Success or application-value type.
    /// - `E`: Domain-error type.
    /// - `R`: Environment type.
    ///
    /// # Parameters
    /// - `next` (`impl Fn(A) -> Effect<B, E, R> + Send + Sync + 'static`): Next value or state produced by the transition.
    ///
    /// # Returns
    /// Type: `Effect<B, E, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    pub fn flat_map<B: Send + 'static>(
        self,
        next: impl Fn(A) -> Effect<B, E, R> + Send + Sync + 'static,
    ) -> Effect<B, E, R> {
        let next = Arc::new(next);
        workflow::reusable(move || {
            let next = Arc::clone(&next);
            workflow::bind(self.clone(), move |value| {
                let program = next(value);
                Workflow::new(move |env, context, finish| program.execute(env, context, finish))
            })
        })
    }

    /// Transform only the success value; preserve failures, defects, and interruption.
    ///
    /// # Type parameters
    /// - `B`: Transformed value type. Bounds: `Send + 'static`.
    /// - `A`: Success or application-value type.
    /// - `E`: Domain-error type.
    /// - `R`: Environment type.
    ///
    /// # Parameters
    /// - `transform` (`impl Fn(A) -> B + Send + Sync + 'static`): Callback applied to the selected value or error.
    ///
    /// # Returns
    /// Type: `Effect<B, E, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    pub fn map<B: Send + 'static>(
        self,
        transform: impl Fn(A) -> B + Send + Sync + 'static,
    ) -> Effect<B, E, R> {
        let transform = Arc::new(transform);
        workflow::reusable(move || {
            let transform = Arc::clone(&transform);
            workflow::bind(self.clone(), move |value| {
                workflow::success(transform(value))
            })
        })
    }

    ///
    /// # Type parameters
    /// - `B`: Bounds: `Send + 'static`.
    /// - `E2`: Bounds: `Send + 'static`.
    /// - `A`: Success or application-value type.
    /// - `E`: Domain-error type.
    /// - `R`: Environment type.
    ///
    /// # Parameters
    /// - `next` (`impl Fn(Exit<A, E>) -> Workflow<B, E2, R> + Send + Sync + 'static`): Next value or state produced by the transition.
    ///
    /// # Returns
    /// Type: `Effect<B, E2, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    fn bind_exit<B: Send + 'static, E2: Send + 'static>(
        self,
        next: impl Fn(Exit<A, E>) -> Workflow<B, E2, R> + Send + Sync + 'static,
    ) -> Effect<B, E2, R> {
        let next = Arc::new(next);
        Effect::new(move |env, context, finish| {
            let next = Arc::clone(&next);
            self.clone().execute(
                Arc::clone(&env),
                context.clone(),
                runtime::continuation(move |outcome| {
                    runtime::more(
                        move || match catch_unwind(AssertUnwindSafe(|| next(outcome))) {
                            Ok(program) => program.execute(env, context, finish),
                            Err(panic) => {
                                runtime::finish(finish, Exit::Die(Defect::from_panic(panic)))
                            }
                        },
                    )
                }),
            )
        })
    }

    /// Transform expected errors, including expected errors retained by failed cleanup.
    ///
    /// # Type parameters
    /// - `E2`: Bounds: `Send + 'static`.
    /// - `A`: Success or application-value type.
    /// - `E`: Domain-error type.
    /// - `R`: Environment type.
    ///
    /// # Parameters
    /// - `transform` (`impl Fn(E) -> E2 + Send + Sync + 'static`): Callback applied to the selected value or error.
    ///
    /// # Returns
    /// Type: `Effect<A, E2, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    pub fn map_error<E2: Send + 'static>(
        self,
        transform: impl Fn(E) -> E2 + Send + Sync + 'static,
    ) -> Effect<A, E2, R> {
        self.bind_exit(move |outcome| Workflow::from_exit(outcome.map_error(&transform)))
    }

    /// Recover an expected `Fail(E)` by running another Effect; leave other exits unchanged.
    ///
    /// # Type parameters
    /// - `A`: Application value type. Bound: `Send + 'static`.
    /// - `R`: Environment type. Bound: `Send + Sync + 'static`.
    /// - `E`: Domain-error type.
    ///
    /// # Parameters
    /// - `recover` (`impl Fn(E) -> Self + Send + Sync + 'static`): The recover supplied to this operation.
    ///
    /// # Returns
    /// Type: `Self`.
    pub fn catch_all(self, recover: impl Fn(E) -> Self + Send + Sync + 'static) -> Self {
        let recover = Arc::new(recover);
        self.bind_exit(move |outcome| match outcome {
            Exit::Fail(error) => {
                let recover = Arc::clone(&recover);
                Workflow::new(move |env, context, finish| {
                    match catch_unwind(AssertUnwindSafe(|| recover(error))) {
                        Ok(program) => program.execute(env, context, finish),
                        Err(panic) => runtime::finish(finish, Exit::Die(Defect::from_panic(panic))),
                    }
                })
            }
            other @ (Exit::Success(_)
            | Exit::Die(_)
            | Exit::Interrupted
            | Exit::FinalizationFailed(_)) => Workflow::from_exit(other),
        })
    }

    /// Turn an expected failure into `Err(E)` data and a success into `Ok(A)` data.
    ///
    /// # Type parameters
    /// - `A`: Success or application-value type.
    /// - `E`: Domain-error type.
    /// - `R`: Environment type.
    ///
    /// # Returns
    /// Type: `Effect<Result<A, E>, E, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    pub fn either(self) -> Effect<Result<A, E>, E, R> {
        self.bind_exit(|outcome| match outcome {
            Exit::Success(value) => workflow::success(Ok(value)),
            Exit::Fail(error) => workflow::success(Err(error)),
            Exit::Die(defect) => Workflow::from_exit(Exit::Die(defect)),
            Exit::Interrupted => Workflow::from_exit(Exit::Interrupted),
            Exit::FinalizationFailed(details) => {
                Workflow::from_exit(Exit::FinalizationFailed(details))
            }
        })
    }

    /// Observe the complete exit as data, retaining expected errors, defects,
    /// interruption and finalization failures without erasing their channels.
    /// This is useful for diagnostics and reactive state descriptions.
    ///
    /// # Type parameters
    /// - `A`: Success or application-value type.
    /// - `E`: Domain-error type.
    /// - `R`: Environment type.
    ///
    /// # Returns
    /// Type: `Effect<Exit<A, E>, std::convert::Infallible, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    pub fn materialize_exit(self) -> Effect<Exit<A, E>, std::convert::Infallible, R> {
        self.bind_exit(workflow::success)
    }

    /// Expose success or expected failure as a Rust `Result` value.
    ///
    /// # Type parameters
    /// - `A`: Success or application-value type.
    /// - `E`: Domain-error type.
    /// - `R`: Environment type.
    ///
    /// # Returns
    /// Type: `Effect<Result<A, E>, E, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    pub fn result(self) -> Effect<Result<A, E>, E, R> {
        self.either()
    }
    /// Expose success as `Some(A)` and an expected failure as `None`.
    ///
    /// # Type parameters
    /// - `A`: Success or application-value type.
    /// - `E`: Domain-error type.
    /// - `R`: Environment type.
    ///
    /// # Returns
    /// Type: `Effect<Option<A>, E, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    pub fn option(self) -> Effect<Option<A>, E, R> {
        self.either().map(Result::ok)
    }

    /// Map a top-level defect into the expected-error channel using `on_defect`.
    ///
    /// # Type parameters
    /// - `A`: Application value type. Bound: `Send + 'static`.
    /// - `R`: Environment type. Bound: `Send + Sync + 'static`.
    /// - `E`: Domain-error type.
    ///
    /// # Parameters
    /// - `on_defect` (`impl Fn(Defect) -> E + Send + Sync + 'static`): The on defect supplied to this operation.
    ///
    /// # Returns
    /// Type: `Self`.
    pub fn sandbox(self, on_defect: impl Fn(Defect) -> E + Send + Sync + 'static) -> Self {
        self.bind_exit(move |outcome| {
            Workflow::from_exit(match outcome {
                Exit::Die(defect) => Exit::Fail(on_defect(defect)),
                other @ (Exit::Success(_)
                | Exit::Fail(_)
                | Exit::Interrupted
                | Exit::FinalizationFailed(_)) => other,
            })
        })
    }

    /// Mask cancellation once this operation starts, then publish normally to
    /// its parent. A pre-cancelled run still does not start protected work.
    ///
    /// # Type parameters
    /// - `A`: Application value type. Bound: `Send + 'static`.
    /// - `E`: Domain-error type. Bound: `Send + 'static`.
    /// - `R`: Environment type. Bound: `Send + Sync + 'static`.
    ///
    /// # Returns
    /// Type: `Self`.
    pub fn uninterruptible(self) -> Self {
        Self::new(move |env, context, finish| {
            self.clone().execute(
                env,
                context.protected(),
                runtime::continuation(move |outcome| {
                    context.record_protected_failure(&outcome);
                    runtime::finish(finish, outcome)
                }),
            )
        })
    }

    /// Await protected cleanup if this operation's primary outcome is interrupted.
    /// Cleanup failures are appended without replacing that primary outcome.
    ///
    /// # Type parameters
    /// - `A`: Application value type. Bound: `Send + 'static`.
    /// - `E`: Domain-error type.
    /// - `R`: Environment type.
    ///
    /// # Parameters
    /// - `finalizer` (`Effect<(), E, R>`): Cleanup description awaited after the primary outcome.
    ///
    /// # Returns
    /// Type: `Self`.
    pub fn on_interrupt(self, finalizer: Effect<(), E, R>) -> Self {
        Self::new(move |env, context, finish| {
            let finalizer = finalizer.clone();
            self.clone().execute(
                Arc::clone(&env),
                context.clone(),
                runtime::continuation(move |result| {
                    let result = runtime::publish(result, &context);
                    if !outcome::interrupted(&result) {
                        return runtime::finish(finish, result);
                    }
                    finalizer.execute(
                        env,
                        context.protected(),
                        runtime::continuation(move |cleanup| {
                            runtime::finish(finish, outcome::append_cleanup(result, cleanup))
                        }),
                    )
                }),
            )
        })
    }

    /// Run two programs sequentially and return both successful values as a tuple.
    ///
    /// # Type parameters
    /// - `B`: Transformed value type. Bounds: `Send + 'static`.
    /// - `A`: Success or application-value type.
    /// - `E`: Domain-error type.
    /// - `R`: Environment type.
    ///
    /// # Parameters
    /// - `right` (`Effect<B, E, R>`): Right-hand operation composed with this one.
    ///
    /// # Returns
    /// Type: `Effect<(A, B), E, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    pub fn zip<B: Send + 'static>(self, right: Effect<B, E, R>) -> Effect<(A, B), E, R> {
        workflow::reusable(move || {
            let right = right.clone();
            workflow::bind(self.clone(), move |left| {
                workflow::bind(right, move |right| workflow::success((left, right)))
            })
        })
    }

    /// Run the supplied programs sequentially and collect their successful values.
    ///
    /// # Type parameters
    /// - `A`: Success or application-value type.
    /// - `E`: Domain-error type.
    /// - `R`: Environment type.
    ///
    /// # Parameters
    /// - `programs` (`Vec<Self>`): The programs supplied to this operation.
    ///
    /// # Returns
    /// Type: `Effect<Vec<A>, E, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    pub fn all(programs: Vec<Self>) -> Effect<Vec<A>, E, R> {
        workflow::reusable(move || collect(programs.clone().into_iter(), Vec::new()))
    }

    /// Build and run one program per input in order, collecting the results.
    ///
    /// # Type parameters
    /// - `T`: Value or item type. Bounds: `Clone + Send + Sync + 'static`.
    /// - `A`: Success or application-value type.
    /// - `E`: Domain-error type.
    /// - `R`: Environment type.
    ///
    /// # Parameters
    /// - `values` (`Vec<T>`): Owned values processed in their declared order.
    /// - `transform` (`impl Fn(T) -> Self + Send + Sync + 'static`): Callback applied to the selected value or error.
    ///
    /// # Returns
    /// Type: `Effect<Vec<A>, E, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    pub fn for_each<T: Clone + Send + Sync + 'static>(
        values: Vec<T>,
        transform: impl Fn(T) -> Self + Send + Sync + 'static,
    ) -> Effect<Vec<A>, E, R> {
        let transform = Arc::new(transform);
        workflow::reusable(move || {
            traverse(
                values.clone().into_iter(),
                Arc::clone(&transform),
                Vec::new(),
            )
        })
    }

    /// Lift a stored Rust `Result` into a reusable Effect; each run clones its payload.
    ///
    /// # Type parameters
    /// - `R`: Environment type. Bound: `Send + Sync + 'static`.
    /// - `A`: Success or application-value type.
    /// - `E`: Domain-error type.
    ///
    /// # Parameters
    /// - `value` (`Result<A, E>`): Value read, transformed, encoded, or stored by this operation.
    ///
    /// # Returns
    /// Type: `Self`.
    pub fn from_result(value: Result<A, E>) -> Self
    where
        A: Clone + Sync,
        E: Clone + Sync,
    {
        Self::ready(move |_, _| match &value {
            Ok(value) => Exit::Success(value.clone()),
            Err(error) => Exit::Fail(error.clone()),
        })
    }

    /// Lift a stored `Option` into a reusable Effect, producing `E` when it is absent.
    ///
    /// # Type parameters
    /// - `R`: Environment type. Bound: `Send + Sync + 'static`.
    /// - `A`: Success or application-value type.
    /// - `E`: Domain-error type.
    ///
    /// # Parameters
    /// - `value` (`Option<A>`): Value read, transformed, encoded, or stored by this operation.
    /// - `on_none` (`impl Fn() -> E + Send + Sync + 'static`): The on none supplied to this operation.
    ///
    /// # Returns
    /// Type: `Self`.
    pub fn from_option(value: Option<A>, on_none: impl Fn() -> E + Send + Sync + 'static) -> Self
    where
        A: Clone + Sync,
    {
        Self::ready(move |_, _| match &value {
            Some(value) => Exit::Success(value.clone()),
            None => Exit::Fail(on_none()),
        })
    }

    /// Execution starts when polled. Dropping this future abandons execution;
    /// use [`Self::run_cancellable`] when the host needs to request cancellation.
    /// This boundary creates a fresh, internal request source and does not read
    /// cancellation from the environment. It requires no execution services.
    ///
    /// # Type parameters
    /// - `A`: Success or application-value type.
    /// - `E`: Domain-error type.
    /// - `R`: Environment type.
    ///
    /// # Parameters
    /// - `env` (`Arc<R>`): The environment supplying this operation's capabilities.
    ///
    /// # Returns
    /// Type: `impl Future<Output = Exit<A, E>> + Send + 'static + use<A, E, R>`.
    pub fn run(
        &self,
        env: Arc<R>,
    ) -> impl Future<Output = Exit<A, E>> + Send + 'static + use<A, E, R> {
        runtime::evaluate(self.clone(), env, Cancellation::new())
    }

    /// Run with the external cancellation source supplied by `R`.
    ///
    /// The accessor and all work are lazy. Request cancellation through the
    /// environment and continue awaiting to observe settlement. Environment
    /// projection within a run preserves its existing cancellation state.
    ///
    /// # Type parameters
    /// - `A`: Success or application-value type.
    /// - `E`: Domain-error type.
    /// - `R`: Environment type.
    ///
    /// # Parameters
    /// - `env` (`Arc<R>`): The environment supplying this operation's capabilities.
    ///
    /// # Returns
    /// Type: `impl Future<Output = Exit<A, E>> + Send + 'static + use<A, E, R>`.
    pub fn run_cancellable(
        &self,
        env: Arc<R>,
    ) -> impl Future<Output = Exit<A, E>> + Send + 'static + use<A, E, R>
    where
        R: HasCancellation,
    {
        let program = self.clone();
        async move {
            let cancellation = match catch_unwind(AssertUnwindSafe(|| env.cancellation().clone())) {
                Ok(cancellation) => cancellation,
                Err(panic) => return Exit::Die(Defect::from_panic(panic)),
            };
            runtime::evaluate(program, env, cancellation).await
        }
    }

    /// Separate the host's execution ownership from observation of the exit.
    /// The host polls the driver even if the observer is dropped.
    ///
    /// # Type parameters
    /// - `A`: Success or application-value type.
    /// - `E`: Domain-error type.
    /// - `R`: Environment type.
    ///
    /// # Parameters
    /// - `env` (`Arc<R>`): The environment supplying this operation's capabilities.
    ///
    /// # Returns
    /// Type: `(RunDriver<A, E>, RunObserver<A, E>)`.
    pub fn owned_run(&self, env: Arc<R>) -> (RunDriver<A, E>, RunObserver<A, E>) {
        crate::owned_run::owned(self.run(env))
    }

    /// An owned run whose cancellation source is supplied solely through `R`.
    ///
    /// # Type parameters
    /// - `A`: Success or application-value type.
    /// - `E`: Domain-error type.
    /// - `R`: Environment type.
    ///
    /// # Parameters
    /// - `env` (`Arc<R>`): The environment supplying this operation's capabilities.
    ///
    /// # Returns
    /// Type: `(RunDriver<A, E>, RunObserver<A, E>)`.
    pub fn owned_run_cancellable(&self, env: Arc<R>) -> (RunDriver<A, E>, RunObserver<A, E>)
    where
        R: HasCancellation,
    {
        crate::owned_run::owned(self.run_cancellable(env))
    }
}

impl<E: Send + 'static, R: Send + Sync + 'static> Effect<(), E, R> {
    /// Walk a fresh iterator sequentially without collecting per-item
    /// programs or results. The factory is reevaluated for each Effect run.
    ///
    /// # Type parameters
    /// - `E`: Domain-error type. Bound: `Send + 'static`.
    /// - `R`: Environment type. Bound: `Send + Sync + 'static`.
    /// - `Items`: Input collection or iterator type. Additional bound: `IntoIterator + 'static`. Additional bound: `:Item: Send + 'static`. Additional bound: `:IntoIter: Send + 'static`.
    ///
    /// # Parameters
    /// - `items` (`impl Fn() -> Items + Send + Sync + 'static`): The items supplied to this operation.
    /// - `body` (`impl Fn(Items::Item) -> Self + Send + Sync + 'static`): Application callback or effect executed inside the owning scope.
    ///
    /// # Returns
    /// Type: `Self`.
    pub fn for_each_sequential<Items>(
        items: impl Fn() -> Items + Send + Sync + 'static,
        body: impl Fn(Items::Item) -> Self + Send + Sync + 'static,
    ) -> Self
    where
        Items: IntoIterator + 'static,
        Items::Item: Send + 'static,
        Items::IntoIter: Send + 'static,
    {
        let body = Arc::new(body);
        workflow::reusable(move || walk_unit(items().into_iter(), Arc::clone(&body)))
    }

    /// Check a lazy guard before each iteration and create a fresh body only
    /// when it succeeds. Failures and cancellation stop further iterations.
    ///
    /// # Type parameters
    /// - `E`: Domain-error type. Bound: `Send + 'static`.
    /// - `R`: Environment type. Bound: `Send + Sync + 'static`.
    ///
    /// # Parameters
    /// - `guard` (`impl Fn() -> bool + Send + Sync + 'static`): The guard supplied to this operation.
    /// - `body` (`impl Fn() -> Self + Send + Sync + 'static`): Application callback or effect executed inside the owning scope.
    ///
    /// # Returns
    /// Type: `Self`.
    pub fn while_loop(
        guard: impl Fn() -> bool + Send + Sync + 'static,
        body: impl Fn() -> Self + Send + Sync + 'static,
    ) -> Self {
        let step = Arc::new(move |()| {
            if guard() {
                body().map(|()| ControlFlow::Continue(()))
            } else {
                Effect::succeed(ControlFlow::Break(()))
            }
        });
        workflow::reusable(move || iterate((), Arc::clone(&step)))
    }
}

///
/// # Type parameters
/// - `I`: Workflow or codec input type. Additional bound: `Iterator + Send + 'static`. Additional bound: `:Item: Send + 'static`.
/// - `E`: Expected application-error type. Additional bound: `Send + 'static`.
/// - `R`: Environment type supplying required capabilities. Additional bound: `Send + Sync + 'static`.
/// - `F`: Callback or field-definition type. Additional bound: `Fn(I::Item) -> Effect<(), E, R> + Send + Sync + 'static`.
///
/// # Parameters
/// - `items` (`I`): The items supplied to this operation.
/// - `body` (`Arc<F>`): Application callback or effect executed inside the owning scope.
///
/// # Returns
/// Type: `Workflow<(), E, R>`.
/// Returns `Workflow<(), E, R>`.
fn walk_unit<I, E, R, F>(mut items: I, body: Arc<F>) -> Workflow<(), E, R>
where
    I: Iterator + Send + 'static,
    I::Item: Send + 'static,
    E: Send + 'static,
    R: Send + Sync + 'static,
    F: Fn(I::Item) -> Effect<(), E, R> + Send + Sync + 'static,
{
    match items.next() {
        Some(item) => workflow::bind(body(item), move |()| walk_unit(items, body)),
        None => workflow::success(()),
    }
}

///
/// # Type parameters
/// - `S`: State or store value type. Additional bound: `Send + 'static`.
/// - `A`: Successful application-value type. Additional bound: `Send + 'static`.
/// - `E`: Expected application-error type. Additional bound: `Send + 'static`.
/// - `R`: Environment type supplying required capabilities. Additional bound: `Send + Sync + 'static`.
/// - `F`: Callback or field-definition type. Additional bound: `Fn(S) -> Effect<ControlFlow<A, S>, E, R> + Send + Sync + 'static`.
///
/// # Parameters
/// - `state` (`S`): Current or replacement state owned by this operation.
/// - `step` (`Arc<F>`): The step supplied to this operation.
///
/// # Returns
/// Type: `Workflow<A, E, R>`.
/// Returns `Workflow<A, E, R>`.
fn iterate<S, A, E, R, F>(state: S, step: Arc<F>) -> Workflow<A, E, R>
where
    S: Send + 'static,
    A: Send + 'static,
    E: Send + 'static,
    R: Send + Sync + 'static,
    F: Fn(S) -> Effect<ControlFlow<A, S>, E, R> + Send + Sync + 'static,
{
    let program = step(state);
    workflow::bind(program, move |next| match next {
        ControlFlow::Continue(state) => iterate(state, step),
        ControlFlow::Break(value) => workflow::success(value),
    })
}

///
/// # Type parameters
/// - `A`: Bounds: `Send + 'static`.
/// - `E`: Bounds: `Send + 'static`.
/// - `R`: Bounds: `Send + Sync + 'static`.
///
/// # Parameters
/// - `programs` (`std::vec::IntoIter<Effect<A, E, R>>`): The programs supplied to this operation.
/// - `values` (`Vec<A>`): Owned values processed in their declared order.
///
/// # Returns
/// Type: `Workflow<Vec<A>, E, R>`.
/// Returns `Workflow<Vec<A>, E, R>`.
fn collect<A: Send + 'static, E: Send + 'static, R: Send + Sync + 'static>(
    mut programs: std::vec::IntoIter<Effect<A, E, R>>,
    mut values: Vec<A>,
) -> Workflow<Vec<A>, E, R> {
    match programs.next() {
        Some(program) => workflow::bind(program, move |value| {
            values.push(value);
            collect(programs, values)
        }),
        None => workflow::success(values),
    }
}

///
/// # Type parameters
/// - `T`: Bounds: `Send + 'static`.
/// - `A`: Bounds: `Send + 'static`.
/// - `E`: Bounds: `Send + 'static`.
/// - `R`: Bounds: `Send + Sync + 'static`.
/// - `F`: Callback or field-definition type. Additional bound: `Fn(T) -> Effect<A, E, R> + Send + Sync + 'static`.
///
/// # Parameters
/// - `input` (`std::vec::IntoIter<T>`): Owned input decoded or consumed by this operation.
/// - `transform` (`Arc<F>`): Callback applied to the selected value or error.
/// - `values` (`Vec<A>`): Owned values processed in their declared order.
///
/// # Returns
/// Type: `Workflow<Vec<A>, E, R>`.
/// Returns `Workflow<Vec<A>, E, R>`.
fn traverse<T: Send + 'static, A: Send + 'static, E: Send + 'static, R: Send + Sync + 'static, F>(
    mut input: std::vec::IntoIter<T>,
    transform: Arc<F>,
    mut values: Vec<A>,
) -> Workflow<Vec<A>, E, R>
where
    F: Fn(T) -> Effect<A, E, R> + Send + Sync + 'static,
{
    match input.next() {
        Some(value) => {
            let program = transform(value);
            workflow::bind(program, move |value| {
                values.push(value);
                traverse(input, transform, values)
            })
        }
        None => workflow::success(values),
    }
}

impl<A: Send + 'static, E: Send + 'static, R: Send + Sync + 'static> Effect<A, E, R> {
    /// An explicitly consumed callback for owned captures. Construct inside a
    /// reusable effect! factory to get fresh inputs on each application run.
    ///
    /// # Type parameters
    /// - `E`: Domain-error type. Bound: `Send + 'static`.
    /// - `R`: Environment type. Bound: `Send + Sync + 'static`.
    /// - `A`: Success or application-value type.
    ///
    /// # Parameters
    /// - `callback` (`impl FnOnce() -> A + Send + 'static`): The callback invoked by this operation.
    ///
    /// # Returns
    /// Type: `Self`.
    pub fn sync_once(callback: impl FnOnce() -> A + Send + 'static) -> Self {
        Self::try_sync_once(
            move || Ok::<A, std::convert::Infallible>(callback()),
            |never| match never {},
        )
    }
    /// Consume a synchronous fallible callback exactly once, allowing owned non-Clone captures.
    ///
    /// # Type parameters
    /// - `R`: Environment type. Bound: `Send + Sync + 'static`.
    /// - `X`: Source error type mapped into the declared error channel.
    /// - `A`: Success or application-value type.
    /// - `E`: Domain-error type.
    ///
    /// # Parameters
    /// - `callback` (`impl FnOnce() -> Result<A, X> + Send + 'static`): The callback invoked by this operation.
    /// - `on_error` (`impl FnOnce(X) -> E + Send + 'static`): Callback that maps a source error into the declared error type.
    ///
    /// # Returns
    /// Type: `Self`.
    pub fn try_sync_once<X>(
        callback: impl FnOnce() -> Result<A, X> + Send + 'static,
        on_error: impl FnOnce(X) -> E + Send + 'static,
    ) -> Self {
        let candidate = std::sync::Mutex::new(Some((callback, on_error)));
        Self::ready(move |_, _| {
            let candidate = candidate
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take();
            match candidate {
                Some((callback, on_error)) => match callback() {
                    Ok(value) => Exit::Success(value),
                    Err(error) => Exit::Fail(on_error(error)),
                },
                None => Exit::Die(Defect::from(LibraryDefect::SyncCallbackConsumed)),
            }
        })
    }
    /// Move a stored Rust `Result` into one execution without cloning its payload.
    ///
    /// # Type parameters
    /// - `R`: Environment type. Bound: `Send + Sync + 'static`.
    /// - `A`: Success or application-value type.
    /// - `E`: Domain-error type.
    ///
    /// # Parameters
    /// - `value` (`Result<A, E>`): Value read, transformed, encoded, or stored by this operation.
    ///
    /// # Returns
    /// Type: `Self`.
    pub fn from_result_once(value: Result<A, E>) -> Self {
        Self::try_sync_once(move || value, |error| error)
    }
    /// Move a stored `Option` into one execution without cloning its payload.
    ///
    /// # Type parameters
    /// - `R`: Environment type. Bound: `Send + Sync + 'static`.
    /// - `A`: Success or application-value type.
    /// - `E`: Domain-error type.
    ///
    /// # Parameters
    /// - `value` (`Option<A>`): Value read, transformed, encoded, or stored by this operation.
    /// - `on_none` (`impl FnOnce() -> E + Send + 'static`): The on none supplied to this operation.
    ///
    /// # Returns
    /// Type: `Self`.
    pub fn from_option_once(
        value: Option<A>,
        on_none: impl FnOnce() -> E + Send + 'static,
    ) -> Self {
        Self::try_sync_once(move || value.ok_or_else(on_none), |error| error)
    }
}

impl<A: Send + 'static, E: Send + 'static, R: Send + Sync + 'static> Effect<A, E, R> {
    /// Create a future on each run with access to that run’s borrowed environment.
    ///
    /// # Type parameters
    /// - `A`: Application value type. Bound: `Send + 'static`.
    /// - `E`: Domain-error type. Bound: `Send + 'static`.
    /// - `F`: Bounds: `Future<Output = A> + Send + 'static`.
    /// - `R`: Environment type.
    ///
    /// # Parameters
    /// - `factory` (`impl Fn(Arc<R>, Cancellation) -> F + Send + Sync + 'static`): Factory invoked to create fresh data for an execution.
    ///
    /// # Returns
    /// Type: `Self`.
    pub fn from_future_with_env<F: Future<Output = A> + Send + 'static>(
        factory: impl Fn(Arc<R>, Cancellation) -> F + Send + Sync + 'static,
    ) -> Self {
        Self::try_future_with_env(
            move |env, cancellation| {
                let future = factory(env, cancellation);
                async move { Ok::<A, std::convert::Infallible>(future.await) }
            },
            |never| match never {},
        )
    }
    /// Create a fallible future on each run using its environment and cancellation token.
    ///
    /// # Type parameters
    /// - `A`: Application value type. Bound: `Send + 'static`.
    /// - `X`: Bounds: `Send + 'static`.
    /// - `F`: Bounds: `Future<Output = Result<A, X>> + Send + 'static`.
    /// - `E`: Domain-error type.
    /// - `R`: Environment type.
    ///
    /// # Parameters
    /// - `factory` (`impl Fn(Arc<R>, Cancellation) -> F + Send + Sync + 'static`): Factory invoked to create fresh data for an execution.
    /// - `on_error` (`impl Fn(X) -> E + Send + Sync + 'static`): Callback that maps a source error into the declared error type.
    ///
    /// # Returns
    /// Type: `Self`.
    pub fn try_future_with_env<
        X: Send + 'static,
        F: Future<Output = Result<A, X>> + Send + 'static,
    >(
        factory: impl Fn(Arc<R>, Cancellation) -> F + Send + Sync + 'static,
        on_error: impl Fn(X) -> E + Send + Sync + 'static,
    ) -> Self {
        let factory = Arc::new(factory);
        let on_error = Arc::new(on_error);
        Self::new(move |env, context, finish| {
            let factory = Arc::clone(&factory);
            let on_error = Arc::clone(&on_error);
            Step::Suspended(Box::pin(async move {
                let outcome = AssertUnwindSafe(async {
                    match factory(Arc::clone(&env), context.cancellation.clone()).await {
                        Ok(value) => Exit::Success(value),
                        Err(error) => Exit::Fail(on_error(error)),
                    }
                })
                .catch_unwind()
                .await
                .unwrap_or_else(|panic| Exit::Die(Defect::from_panic(panic)));
                runtime::finish(finish, outcome)
            }))
        })
    }
    /// Move one future into a single execution, allowing owned non-Clone captures.
    ///
    /// # Type parameters
    /// - `A`: Application value type. Bound: `Send + 'static`.
    /// - `E`: Domain-error type. Bound: `Send + 'static`.
    /// - `R`: Environment type. Bound: `Send + Sync + 'static`.
    /// - `F`: Bounds: `Future<Output = A> + Send + 'static`.
    ///
    /// # Parameters
    /// - `factory` (`impl FnOnce(Cancellation) -> F + Send + 'static`): Factory invoked to create fresh data for an execution.
    ///
    /// # Returns
    /// Type: `Self`.
    pub fn from_future_once<F: Future<Output = A> + Send + 'static>(
        factory: impl FnOnce(Cancellation) -> F + Send + 'static,
    ) -> Self {
        Self::try_future_once(
            move |cancel| {
                let future = factory(cancel);
                async move { Ok::<A, std::convert::Infallible>(future.await) }
            },
            |never| match never {},
        )
    }
    /// Move one fallible future into a single execution and map its typed error.
    ///
    /// # Type parameters
    /// - `A`: Application value type. Bound: `Send + 'static`.
    /// - `R`: Environment type. Bound: `Send + Sync + 'static`.
    /// - `X`: Bounds: `Send + 'static`.
    /// - `F`: Bounds: `Future<Output = Result<A, X>> + Send + 'static`.
    /// - `E`: Domain-error type.
    ///
    /// # Parameters
    /// - `factory` (`impl FnOnce(Cancellation) -> F + Send + 'static`): Factory invoked to create fresh data for an execution.
    /// - `on_error` (`impl FnOnce(X) -> E + Send + 'static`): Callback that maps a source error into the declared error type.
    ///
    /// # Returns
    /// Type: `Self`.
    pub fn try_future_once<X: Send + 'static, F: Future<Output = Result<A, X>> + Send + 'static>(
        factory: impl FnOnce(Cancellation) -> F + Send + 'static,
        on_error: impl FnOnce(X) -> E + Send + 'static,
    ) -> Self {
        let candidate = std::sync::Mutex::new(Some((factory, on_error)));
        Self::new(move |_, context, finish| {
            let candidate = candidate
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take();
            match candidate {
                Some((factory, on_error)) => Step::Suspended(Box::pin(async move {
                    let outcome = AssertUnwindSafe(async {
                        match factory(context.cancellation).await {
                            Ok(value) => Exit::Success(value),
                            Err(error) => Exit::Fail(on_error(error)),
                        }
                    })
                    .catch_unwind()
                    .await
                    .unwrap_or_else(|panic| Exit::Die(Defect::from_panic(panic)));
                    runtime::finish(finish, outcome)
                })),
                None => runtime::finish(
                    finish,
                    Exit::Die(Defect::from(LibraryDefect::FutureFactoryConsumed)),
                ),
            }
        })
    }
}
