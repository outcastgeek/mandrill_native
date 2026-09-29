//! Typed future binding used by procedural macro expansion.

use super::runtime::{self, Context, Step};
use super::workflow::Workflow;
use crate::scope::Registered;
use crate::{Defect, Effect, Exit, LibraryDefect, ResourceBinding, ResourceValue, Scope};
use futures_util::FutureExt;
use std::future::{Future, poll_fn};
use std::panic::AssertUnwindSafe;
use std::sync::{Arc, Mutex};
use std::task::Poll;

/// Generated-code bridge. Contains run-derived state, never provider settings.
#[doc(hidden)]
///
/// # Type parameters
/// - `E`: Domain-error type. Bound or type: `Send + 'static`.
/// - `R`: Environment type. Bound or type: `Send + Sync + 'static`.
pub struct MacroContext<E: Send + 'static, R: Send + Sync + 'static> {
    env: Arc<R>,
    context: Context,
    scope: Scope<E>,
}
mod sealed {
    /// Seals asynchronous macro bindings to supported future operations.
    pub trait Sealed {}
    impl<A: Send + 'static, E: Send + 'static, R: Send + Sync + 'static> Sealed
        for crate::Effect<A, E, R>
    {
    }
    impl<T: Send + Sync + 'static, E: Send + 'static, R: Send + Sync + 'static> Sealed
        for crate::ResourceBinding<T, E, R>
    {
    }
}
#[doc(hidden)]
/// Sealed binding contract used by the asynchronous Effect macro expansion.
pub trait AsyncBindable<E: Send + 'static, R: Send + Sync + 'static>: sealed::Sealed {
    /// Value admitted by this sealed macro binding.
    type Value: Send + 'static;
    /// # Type parameters
    /// - `E`: Domain-error type.
    /// - `R`: Environment type.
    ///
    /// # Parameters
    /// - `scope` (`Scope<E>`): Owner that joins child work and releases resources at close.
    ///
    /// # Returns
    /// Type: `Effect<Self::Value, E, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    fn into_scoped_effect(self, scope: Scope<E>) -> Effect<Self::Value, E, R>;
}
impl<A: Send + 'static, E: Send + 'static, R: Send + Sync + 'static> AsyncBindable<E, R>
    for Effect<A, E, R>
{
    /// Type alias for value in internal macro future.
    type Value = A;
    ///
    /// # Type parameters
    /// - `E`: Domain-error type.
    ///
    /// # Returns
    /// Type: `Self`.
    fn into_scoped_effect(self, _: Scope<E>) -> Self {
        self
    }
}
impl<T: Send + Sync + 'static, E: Send + 'static, R: Send + Sync + 'static> AsyncBindable<E, R>
    for ResourceBinding<T, E, R>
{
    /// Type alias for value in internal macro future.
    type Value = ResourceValue<T>;
    ///
    /// # Type parameters
    /// - `E`: Domain-error type.
    /// - `R`: Environment type.
    ///
    /// # Parameters
    /// - `scope` (`Scope<E>`): Owner that joins child work and releases resources at close.
    ///
    /// # Returns
    /// Type: `Effect<Self::Value, E, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    fn into_scoped_effect(self, scope: Scope<E>) -> Effect<Self::Value, E, R> {
        self.acquire_scoped(scope)
    }
}
impl<E: Send + 'static, R: Send + Sync + 'static> MacroContext<E, R> {
    /// Sequence the next macro-expanded step after a successful Effect result.
    ///
    /// # Type parameters
    /// - `R`: Environment type. Bound: `Send + Sync + 'static`.
    /// - `P`: Payload type. Bounds: `AsyncBindable<E, R>`.
    /// - `A`: Application value type. Bounds: `Send + 'static`.
    /// - `E`: Domain-error type.
    ///
    /// # Parameters
    /// - `program` (`P`): Lazy Effect description executed or transformed here.
    ///
    /// # Returns
    /// Type: `Result<P::Value, Exit<A, E>>`.
    pub async fn bind<P: AsyncBindable<E, R>, A: Send + 'static>(
        &self,
        program: P,
    ) -> Result<P::Value, Exit<A, E>> {
        runtime::evaluate_composed(
            program.into_scoped_effect(self.scope.clone()),
            Arc::clone(&self.env),
            &self.context,
        )
        .await
        .into_failure()
    }
    #[must_use]
    /// Observe cancellation at a macro-generated execution boundary.
    ///
    /// # Type parameters
    /// - `E`: Domain-error type. Bound: `Send + 'static`.
    /// - `R`: Environment type. Bound: `Send + Sync + 'static`.
    ///
    /// # Returns
    /// Type: `usize`.
    pub fn checkpoint(&self) -> usize {
        self.scope.checkpoint()
    }
    /// End this claim or lease and return its settlement status.
    ///
    /// # Type parameters
    /// - `R`: Environment type. Bound: `Send + Sync + 'static`.
    /// - `A`: Application value type. Bounds: `Send + 'static`.
    /// - `E`: Domain-error type.
    ///
    /// # Parameters
    /// - `checkpoint` (`usize`): The checkpoint supplied to this operation.
    ///
    /// # Returns
    /// Type: `Result<(), Exit<A, E>>`.
    pub async fn release<A: Send + 'static>(&self, checkpoint: usize) -> Result<(), Exit<A, E>> {
        let releases = self.scope.take_since(checkpoint)?;
        if releases.is_empty() {
            return Ok(());
        }
        let context = self.context.protected();
        evaluate(
            release_work(releases, Exit::Success(())),
            Arc::clone(&self.env),
            &context,
        )
        .await
        .into_failure()
    }
    /// Yield execution so long-running macro control flow remains cooperative.
    ///
    /// # Type parameters
    /// - `R`: Environment type. Bound: `Send + Sync + 'static`.
    /// - `A`: Application value type. Bounds: `Send + 'static`.
    /// - `E`: Domain-error type.
    ///
    /// # Returns
    /// Type: `Result<(), Exit<A, E>>`.
    pub async fn cooperate<A: Send + 'static>(&self) -> Result<(), Exit<A, E>> {
        /// Variants of turn used by internal macro future.
        enum Turn {
            Yield,
            Resume,
        }
        let mut turn = Turn::Yield;
        poll_fn(|cx| {
            if self.context.cancelled() {
                return Poll::Ready(Err(Exit::Interrupted));
            }
            match turn {
                Turn::Yield => {
                    turn = Turn::Resume;
                    cx.waker().wake_by_ref();
                    Poll::Pending
                }
                Turn::Resume => Poll::Ready(Ok(())),
            }
        })
        .await
    }
}
///
/// # Type parameters
/// - `E`: Bounds: `Send + 'static`.
/// - `R`: Bounds: `Send + Sync + 'static`.
///
/// # Parameters
/// - `releases` (`Vec<Registered<E>>`): The releases supplied to this operation.
/// - `cleanup` (`Exit<(), E>`): The cleanup supplied to this operation.
///
/// # Returns
/// Type: `Workflow<(), E, R>`.
/// Returns `Workflow<(), E, R>`.
fn release_work<E: Send + 'static, R: Send + Sync + 'static>(
    mut releases: Vec<Registered<E>>,
    cleanup: Exit<(), E>,
) -> Workflow<(), E, R> {
    Workflow::new(move |env, context, finish| match releases.pop() {
        Some(release) => release.execute(runtime::continuation(move |result| {
            release_work(releases, super::outcome::append_cleanup(cleanup, result))
                .execute(env, context, finish)
        })),
        None => runtime::finish(finish, cleanup),
    })
}
///
/// # Type parameters
/// - `A`: Bounds: `Send + 'static`.
/// - `E`: Bounds: `Send + 'static`.
/// - `R`: Bounds: `Send + Sync + 'static`.
///
/// # Parameters
/// - `workflow` (`Workflow<A, E, R>`): The workflow supplied to this operation.
/// - `env` (`Arc<R>`): The environment supplying this operation's capabilities.
/// - `context` (`&Context`): Execution context carrying cancellation, scope, and trace state.
///
/// # Returns
/// Type: `Exit<A, E>`.
/// Returns `Exit<A, E>`.
async fn evaluate<A: Send + 'static, E: Send + 'static, R: Send + Sync + 'static>(
    workflow: Workflow<A, E, R>,
    env: Arc<R>,
    context: &Context,
) -> Exit<A, E> {
    let candidate = Mutex::new(Some(workflow));
    let program = Effect::new(move |env, context, finish| {
        match candidate
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
        {
            Some(workflow) => workflow.execute(env, context, finish),
            None => runtime::finish(
                finish,
                Exit::Die(Defect::from(LibraryDefect::WorkflowConsumed)),
            ),
        }
    });
    runtime::evaluate_composed(program, env, context).await
}
#[doc(hidden)]
/// Lower a Rust block into one lazy asynchronous Effect description.
///
/// # Type parameters
/// - `A`: Application value type. Additional bound: `Send + 'static`.
/// - `E`: Domain-error type. Additional bound: `Send + 'static`.
/// - `R`: Environment type. Additional bound: `Send + Sync + 'static`.
/// - `F`: Callback or field-definition type. Additional bound: `Future<Output = Exit<A, E>> + Send + 'static`.
///
/// # Parameters
/// - `factory` (`impl FnOnce(MacroContext<E, R>) -> F + Send + 'static`): Factory invoked to create fresh data for an execution.
///
/// # Returns
/// Type: `Workflow<A, E, R>`.
pub fn async_workflow<A, E, R, F>(
    factory: impl FnOnce(MacroContext<E, R>) -> F + Send + 'static,
) -> Workflow<A, E, R>
where
    A: Send + 'static,
    E: Send + 'static,
    R: Send + Sync + 'static,
    F: Future<Output = Exit<A, E>> + Send + 'static,
{
    Workflow::new(move |env, context, finish| {
        let scope = Scope::new();
        let macro_context = MacroContext {
            env: Arc::clone(&env),
            context: context.clone(),
            scope: scope.clone(),
        };
        let body = Workflow::new(move |_, _, finish| {
            Step::Composed(Box::pin(async move {
                let result = AssertUnwindSafe(async { factory(macro_context).await })
                    .catch_unwind()
                    .await
                    .unwrap_or_else(|panic| Exit::Die(Defect::from_panic(panic)));
                runtime::finish(finish, result)
            }))
        });
        scope.execute_workflow(body, env, context, finish)
    })
}
