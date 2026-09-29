//! Construction and scoped ownership of environment services.

use crate::internal::workflow::{self, bind, success};
use crate::{Effect, Release, ResourceValue, Scope};
use std::sync::Arc;

/// A scoped constructor for dependencies that are provided to an Effect subtree.
///
/// # Type parameters
/// - `I`: Input type. Bound or type: `Send + Sync + 'static`.
/// - `E`: Domain-error type. Bound or type: `Send + 'static`.
/// - `O`: Output type. Bound or type: `Send + 'static`.
pub struct Layer<I: Send + Sync + 'static, E: Send + 'static, O: Send + 'static> {
    build: Arc<dyn Fn(Scope<E>) -> Effect<O, E, I> + Send + Sync>,
}
impl<I: Send + Sync + 'static, E: Send + 'static, O: Send + 'static> Clone for Layer<I, E, O> {
    ///
    /// # Returns
    /// Type: `Self`.
    fn clone(&self) -> Self {
        Self {
            build: Arc::clone(&self.build),
        }
    }
}
impl<I: Send + Sync + 'static, E: Send + 'static, O: Send + 'static> Layer<I, E, O> {
    #[must_use]
    /// Adapt an existing Effect while preserving its typed outcome channels.
    ///
    /// # Type parameters
    /// - `E`: Domain-error type.
    /// - `I`: Input type.
    /// - `O`: Output type.
    ///
    /// # Parameters
    /// - `program` (`Effect<O, E, I>`): Lazy Effect description executed or transformed here.
    ///
    /// # Returns
    /// Type: `Self`.
    pub fn from_effect(program: Effect<O, E, I>) -> Self {
        Self::new(move |_| program.clone())
    }
    #[must_use]
    /// Create a `Layer` using the supplied factory and capability environment.
    ///
    /// # Type parameters
    /// - `I`: Input type. Bound: `Send + Sync + 'static`.
    /// - `E`: Domain-error type. Bound: `Send + 'static`.
    /// - `O`: Output type.
    ///
    /// # Parameters
    /// - `factory` (`impl Fn() -> O + Send + Sync + 'static`): Factory invoked to create fresh data for an execution.
    ///
    /// # Returns
    /// Type: `Self`.
    pub fn make_with(factory: impl Fn() -> O + Send + Sync + 'static) -> Self {
        Self::from_effect(Effect::sync(factory))
    }
    #[must_use]
    /// Construct a `Layer` from its required inputs and explicit policies.
    ///
    /// # Type parameters
    /// - `E`: Domain-error type.
    /// - `I`: Input type.
    /// - `O`: Output type.
    ///
    /// # Parameters
    /// - `build` (`impl Fn(Scope<E>) -> Effect<O, E, I> + Send + Sync + 'static`): The build supplied to this operation.
    ///
    /// # Returns
    /// Type: `Self`.
    pub fn new(build: impl Fn(Scope<E>) -> Effect<O, E, I> + Send + Sync + 'static) -> Self {
        Self {
            build: Arc::new(build),
        }
    }
    /// Acquire this layer’s dependencies within the supplied scope.
    ///
    /// # Type parameters
    /// - `E`: Domain-error type.
    /// - `I`: Input type.
    /// - `O`: Output type.
    ///
    /// # Parameters
    /// - `scope` (`Scope<E>`): Owner that joins child work and releases resources at close.
    ///
    /// # Returns
    /// Type: `Effect<O, E, I>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    pub fn build(&self, scope: Scope<E>) -> Effect<O, E, I> {
        let build = Arc::clone(&self.build);
        Effect::defer(move || build(scope.clone()))
    }
    #[must_use]
    /// Build two layers in one scope and combine their environments.
    ///
    /// # Type parameters
    /// - `P`: Payload type. Bounds: `Send + 'static`.
    /// - `Q`: Request type. Bounds: `Send + 'static`.
    /// - `E`: Domain-error type.
    /// - `I`: Input type.
    /// - `O`: Output type.
    ///
    /// # Parameters
    /// - `right` (`Layer<I, E, P>`): Right-hand operation composed with this one.
    /// - `combine` (`impl Fn(O, P) -> Q + Send + Sync + 'static`): The combine supplied to this operation.
    ///
    /// # Returns
    /// Type: `Layer<I, E, Q>`.
    pub fn merge<P: Send + 'static, Q: Send + 'static>(
        self,
        right: Layer<I, E, P>,
        combine: impl Fn(O, P) -> Q + Send + Sync + 'static,
    ) -> Layer<I, E, Q> {
        let combine = Arc::new(combine);
        Layer::new(move |scope| {
            let left = self.build(scope.clone());
            let right = right.build(scope);
            let combine = Arc::clone(&combine);
            workflow::reusable(move || {
                let right = right.clone();
                let combine = Arc::clone(&combine);
                bind(left.clone(), move |a| {
                    bind(right, move |b| success(combine(a, b)))
                })
            })
        })
    }
}
impl<I: Send + Sync + 'static, E: Send + 'static, O: Send + Sync + 'static> Layer<I, E, O> {
    /// Build this layer and provide its environment to the selected Effect.
    ///
    /// # Type parameters
    /// - `A`: Application value type. Bounds: `Send + 'static`.
    /// - `E`: Domain-error type.
    /// - `I`: Input type.
    /// - `O`: Output type.
    ///
    /// # Parameters
    /// - `program` (`Effect<A, E, O>`): Lazy Effect description executed or transformed here.
    ///
    /// # Returns
    /// Type: `Effect<A, E, I>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    pub fn provide_to<A: Send + 'static>(&self, program: Effect<A, E, O>) -> Effect<A, E, I> {
        let layer = self.clone();
        Scope::scoped(move |scope| {
            let build = layer.build(scope);
            let program = program.clone();
            workflow::reusable(move || {
                let program = program.clone();
                bind(build.clone(), move |env| {
                    bind(
                        program.provide(Arc::new(env)).provide_some(|_: &I| ()),
                        success,
                    )
                })
            })
        })
    }
    #[must_use]
    /// Supply this layer’s environment to a dependent program.
    ///
    /// # Type parameters
    /// - `P`: Payload type. Bounds: `Send + 'static`.
    /// - `E`: Domain-error type.
    /// - `I`: Input type.
    /// - `O`: Output type.
    ///
    /// # Parameters
    /// - `second` (`Layer<O, E, P>`): The second supplied to this operation.
    ///
    /// # Returns
    /// Type: `Layer<I, E, P>`.
    pub fn provide<P: Send + 'static>(self, second: Layer<O, E, P>) -> Layer<I, E, P> {
        Layer::new(move |scope| {
            let build = self.build(scope.clone());
            let second = second.clone();
            workflow::reusable(move || {
                let second = second.clone();
                let scope = scope.clone();
                bind(build.clone(), move |env| {
                    bind(
                        second
                            .build(scope)
                            .provide(Arc::new(env))
                            .provide_some(|_: &I| ()),
                        success,
                    )
                })
            })
        })
    }
}
impl<I: Send + Sync + 'static, E: Send + 'static, T: Send + Sync + 'static>
    Layer<I, E, ResourceValue<T>>
{
    #[must_use]
    /// Construct dependencies whose lifetime is owned by a scope.
    ///
    /// # Type parameters
    /// - `E`: Domain-error type.
    /// - `I`: Input type.
    /// - `T`: Value or item type.
    ///
    /// # Parameters
    /// - `acquire` (`Effect<T, E, I>`): Description that obtains the resource before its body runs.
    /// - `release` (`impl for<'a> Fn(T, crate::PrimaryOutcomeRef<'a, E>) -> Release<E, I> + Send + Sync + 'static`): The release supplied to this operation.
    ///
    /// # Returns
    /// Type: `Self`.
    pub fn scoped(
        acquire: Effect<T, E, I>,
        release: impl for<'a> Fn(T, crate::PrimaryOutcomeRef<'a, E>) -> Release<E, I>
        + Send
        + Sync
        + 'static,
    ) -> Self {
        let release = Arc::new(release);
        Self::new(move |scope| {
            let release = Arc::clone(&release);
            crate::ResourceBinding::new(acquire.clone(), move |value, reason| {
                release(value, reason)
            })
            .acquire_scoped(scope)
        })
    }
}
