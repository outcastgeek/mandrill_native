//! Reusable tracked programs which carry one evaluation's read context through Effect composition.

use super::{Atom, AtomDependencyError, AtomReadContext, AtomRef, AtomRefError};
use crate::internal::macro_future::{AsyncBindable, MacroContext};
use crate::{Effect, Exit, ResourceBinding, SharedValue};
use crate::{effect, effect_map, effect_sync};
use std::future::Future;
use std::sync::Arc;

/// Context-dependent Effect factory; clones share the description, never the read observations.
type Run<A, E, R> = dyn Fn(AtomReadContext) -> Effect<A, E, R> + Send + Sync;

/// A reusable tracked description. `evaluate` joins the caller's Effect execution and retains
/// the supplied context through helper calls; the owning Atom creates that context afresh.
///
/// ```
/// use effect::{Effect, atom};
/// use effect::reactivity::AtomProgram;
/// fn helper() -> AtomProgram<i32, (), ()> {
///     atom! { let value = Effect::sync(|| 4)?; value + 1 }
/// }
/// let program: AtomProgram<i32, (), ()> = atom! { helper()? };
/// # let _ = program;
/// ```
pub struct AtomProgram<A: Send + 'static, E: Send + 'static, R: Send + Sync + 'static> {
    /// Typed factory executed only when an evaluation Effect runs.
    run: Arc<Run<A, E, R>>,
}
impl<A: Send + 'static, E: Send + 'static, R: Send + Sync + 'static> Clone
    for AtomProgram<A, E, R>
{
    /// Share a reusable description without requiring Clone on the result or error.
    fn clone(&self) -> Self {
        Self {
            run: Arc::clone(&self.run),
        }
    }
}
impl<A: Send + 'static, E: Send + 'static, R: Send + Sync + 'static> AtomProgram<A, E, R> {
    /// Describe evaluation under the active read context; constructing it performs no reads.
    pub fn new(run: impl Fn(AtomReadContext) -> Effect<A, E, R> + Send + Sync + 'static) -> Self {
        Self { run: Arc::new(run) }
    }
    /// Lazily execute with the supplied evaluation context, preserving dependency observations.
    pub fn evaluate(&self, context: AtomReadContext) -> Effect<A, E, R> {
        let run = Arc::clone(&self.run);
        {
            let __dsl_factory = move || run(context.clone());
            effect! { __dsl_factory()? }
        }
    }
    /// Lift an ordinary Effect; later tracked reads still use the current context.
    #[must_use]
    pub fn from_effect(effect: Effect<A, E, R>) -> Self {
        Self::new(move |_| effect.clone())
    }
    /// Produce a fresh value per evaluation, avoiding a payload Clone requirement.
    pub fn succeed_with(value: impl Fn() -> A + Send + Sync + 'static) -> Self {
        Self::from_effect(effect_sync! { body: value, })
    }
    /// Rebuild a helper description on each evaluation, including its captured local state.
    pub fn defer(factory: impl Fn() -> Self + Send + Sync + 'static) -> Self {
        Self::new(move |context| factory().evaluate(context))
    }
    /// Transform the success channel while retaining the active tracking context.
    pub fn map<B: Send + 'static>(
        self,
        map: impl Fn(A) -> B + Send + Sync + 'static,
    ) -> AtomProgram<B, E, R> {
        let map = Arc::new(map);
        AtomProgram::new(move |context| {
            let map = Arc::clone(&map);
            effect_map! { program: self.evaluate(context), map: move |value| map(value), }
        })
    }
    /// Map only expected errors, leaving defects, interruption and shared read observations intact.
    pub fn map_error<F: Send + 'static>(
        self,
        map: impl Fn(E) -> F + Send + Sync + 'static,
    ) -> AtomProgram<A, F, R> {
        let map = Arc::new(map);
        AtomProgram::new(move |context| {
            let map = Arc::clone(&map);
            self.evaluate(context).map_error(move |error| map(error))
        })
    }
    /// Build a dependent helper under the same read context; only executed reads are tracked.
    pub fn flat_map<B: Send + 'static>(
        self,
        next: impl Fn(A) -> AtomProgram<B, E, R> + Send + Sync + 'static,
    ) -> AtomProgram<B, E, R> {
        let next = Arc::new(next);
        AtomProgram::new(move |context| {
            let next = Arc::clone(&next);
            {
                let __dsl_program = self.evaluate(context.clone());
                effect! { let context = context.clone(); let next = next.clone(); let value = __dsl_program.clone()?; next(value).evaluate(context.clone())? }
            }
        })
    }
    /// Generated native body bridge. Locals are fresh and resources remain owned by this evaluation.
    #[doc(hidden)]
    pub fn from_body<F>(
        factory: impl Fn(AtomProgramContext<E, R>) -> F + Send + Sync + 'static,
    ) -> Self
    where
        F: Future<Output = Result<A, Exit<A, E>>> + Send + 'static,
    {
        let factory = Arc::new(factory);
        Self::new(move |tracking| {
            let factory = Arc::clone(&factory);
            crate::internal::workflow::reusable(move || {
                let factory = Arc::clone(&factory);
                let tracking = tracking.clone();
                crate::internal::macro_future::async_workflow(move |context| async move {
                    match factory(AtomProgramContext { context, tracking }).await {
                        Ok(value) => Exit::Success(value),
                        Err(failure) => failure,
                    }
                })
            })
        })
    }
}
impl<A: Send + Sync + 'static, R: Send + Sync + 'static> AtomProgram<Arc<A>, AtomRefError, R> {
    /// Read a source through the active tracking context using the existing shared-value boundary.
    #[must_use]
    pub fn read(source: AtomRef<A>) -> Self {
        Self::new(move |context| context.read(&source))
    }
}
impl<A: Send + 'static, E: Send + 'static, R: Send + Sync + 'static>
    AtomProgram<SharedValue<A>, AtomDependencyError<E>, R>
{
    /// Read a derived dependency through the active context with its typed dependency failures.
    #[must_use]
    pub fn read_derived(source: Atom<A, E, R>) -> Self {
        Self::new(move |context| context.read_derived(&source))
    }
}

/// Sealing keeps macro inputs limited to tracked programs and existing Effect/resource bindings.
mod sealed {
    /// Internal admission marker; consumers cannot introduce untracked interpreters.
    pub trait Sealed {}
    impl<A: Send + 'static, E: Send + 'static, R: Send + Sync + 'static> Sealed
        for super::AtomProgram<A, E, R>
    {
    }
    impl<A: Send + 'static, E: Send + 'static, R: Send + Sync + 'static> Sealed
        for crate::Effect<A, E, R>
    {
    }
    impl<T: Send + Sync + 'static, E: Send + 'static, R: Send + Sync + 'static> Sealed
        for crate::ResourceBinding<T, E, R>
    {
    }
}
/// Typed macro admission retaining the context for tracked helpers and scope for resources.
#[doc(hidden)]
pub trait AtomBindable<E: Send + 'static, R: Send + Sync + 'static>: sealed::Sealed {
    /// Value moved from the bound operation.
    type Value: Send + 'static;
    /// Convert to an Effect under the active tracking and evaluation scope.
    fn in_context(
        self,
        tracking: AtomReadContext,
        scope: crate::Scope<E>,
    ) -> Effect<Self::Value, E, R>;
}
impl<A: Send + 'static, E: Send + 'static, R: Send + Sync + 'static> AtomBindable<E, R>
    for AtomProgram<A, E, R>
{
    /// The tracked helper's success value.
    type Value = A;
    /// Reuse the current observations rather than creating a replacement context.
    fn in_context(self, tracking: AtomReadContext, _: crate::Scope<E>) -> Effect<A, E, R> {
        self.evaluate(tracking)
    }
}
impl<A: Send + 'static, E: Send + 'static, R: Send + Sync + 'static> AtomBindable<E, R>
    for Effect<A, E, R>
{
    /// Ordinary Effect success value.
    type Value = A;
    /// Ordinary Effects do not discard observations belonging to subsequent tracked reads.
    fn in_context(self, _: AtomReadContext, _: crate::Scope<E>) -> Effect<A, E, R> {
        self
    }
}
impl<T: Send + Sync + 'static, E: Send + 'static, R: Send + Sync + 'static> AtomBindable<E, R>
    for ResourceBinding<T, E, R>
{
    /// Checked resource handle, without copying its payload.
    type Value = crate::ResourceValue<T>;
    /// Register cleanup in the evaluation's scope before exposing the handle.
    fn in_context(self, _: AtomReadContext, scope: crate::Scope<E>) -> Effect<Self::Value, E, R> {
        self.into_scoped_effect(scope)
    }
}
/// Generated-code context sharing the existing Effect runtime and active dependency observations.
#[doc(hidden)]
pub struct AtomProgramContext<E: Send + 'static, R: Send + Sync + 'static> {
    /// Existing runtime bridge used for cancellation and scoped execution.
    context: MacroContext<E, R>,
    /// Read observations shared only within this evaluation.
    tracking: AtomReadContext,
}
impl<E: Send + 'static, R: Send + Sync + 'static> AtomProgramContext<E, R> {
    /// Bind a tracked helper or an ordinary Effect without losing observations.
    pub async fn bind<P: AtomBindable<E, R>, A: Send + 'static>(
        &self,
        program: P,
    ) -> Result<P::Value, Exit<A, E>> {
        self.context
            .bind(program.in_context(self.tracking.clone(), self.context.scope()))
            .await
    }
    /// Cooperatively observe interruption between native loop iterations.
    pub async fn cooperate<A: Send + 'static>(&self) -> Result<(), Exit<A, E>> {
        self.context.cooperate().await
    }
}
