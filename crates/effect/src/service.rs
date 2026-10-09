//! Selected service operations whose provider failures are mapped at their own typed boundary.

use crate::internal::macro_future::MacroContext;
use crate::{Effect, Exit, Release, ResourceBinding, ResourceValue, Scope};
use std::future::Future;
use std::sync::Arc;

/// Typed operation invoked against the selected native receiver without a method-name registry.
type Operation<S, A, P, R> = dyn Fn(&S) -> Effect<A, P, R> + Send + Sync;
/// Reusable provider operation. The service DSL maps only this operation's provider error;
/// ordinary application Effects retain their own already-adapted error channel.
pub struct ServiceOperation<S, A: Send + 'static, P: Send + 'static, R: Send + Sync + 'static> {
    /// Receiver-based operation description, invoked lazily by a service body.
    operation: Arc<Operation<S, A, P, R>>,
}
impl<S, A: Send + 'static, P: Send + 'static, R: Send + Sync + 'static>
    ServiceOperation<S, A, P, R>
{
    /// Describe a native receiver operation without selecting a runtime or invoking it.
    pub fn new(operation: impl Fn(&S) -> Effect<A, P, R> + Send + Sync + 'static) -> Self {
        Self {
            operation: Arc::new(operation),
        }
    }
}
/// Selected resource with provider-typed acquisition and disposal. Disposal consumes its payload;
/// scope ownership registers one finalizer and preserves the enclosing application's primary outcome.
pub struct ServiceResource<S, T: Send + Sync + 'static, P: Send + 'static, R: Send + Sync + 'static>
{
    /// Receiver-based acquisition description.
    acquire: Arc<Operation<S, T, P, R>>,
    /// Provider disposal description, independent of the application's error representation.
    release: Arc<dyn Fn(T) -> Effect<(), P, R> + Send + Sync>,
}
impl<S, T: Send + Sync + 'static, P: Send + 'static, R: Send + Sync + 'static>
    ServiceResource<S, T, P, R>
{
    /// Describe acquisition and release without running either callback.
    pub fn new(
        acquire: impl Fn(&S) -> Effect<T, P, R> + Send + Sync + 'static,
        release: impl Fn(T) -> Effect<(), P, R> + Send + Sync + 'static,
    ) -> Self {
        Self {
            acquire: Arc::new(acquire),
            release: Arc::new(release),
        }
    }
}
/// Admission is limited to the existing typed operation and resource boundaries.
mod sealed {
    /// Prevents consumers from adding an alternate interpreter to selected-service binding.
    pub trait Sealed {}
    impl<A: Send + 'static, E: Send + 'static, R: Send + Sync + 'static> Sealed
        for crate::Effect<A, E, R>
    {
    }
    impl<T: Send + Sync + 'static, E: Send + 'static, R: Send + Sync + 'static> Sealed
        for crate::ResourceBinding<T, E, R>
    {
    }
    impl<S, A: Send + 'static, P: Send + 'static, R: Send + Sync + 'static> Sealed
        for super::ServiceOperation<S, A, P, R>
    {
    }
    impl<S, T: Send + Sync + 'static, P: Send + 'static, R: Send + Sync + 'static> Sealed
        for super::ServiceResource<S, T, P, R>
    {
    }
}
/// Typed selected-service bind contract. The mapper is applied only to provider operations.
#[doc(hidden)]
pub trait ServiceBindable<S, P: Send + 'static, E: Send + 'static, R: Send + Sync + 'static>:
    sealed::Sealed
{
    /// Owned successful value or checked resource handle.
    type Value: Send + 'static;
    /// Prepare execution against the same selected service and owner scope.
    fn selected(
        self,
        service: &S,
        mapper: Arc<dyn Fn(P) -> E + Send + Sync>,
        scope: Scope<E>,
    ) -> Effect<Self::Value, E, R>;
}
impl<S, P: Send + 'static, E: Send + 'static, R: Send + Sync + 'static, A: Send + 'static>
    ServiceBindable<S, P, E, R> for Effect<A, E, R>
{
    /// Application success value.
    type Value = A;
    /// Application errors are already typed and must not be provider-mapped again.
    fn selected(self, _: &S, _: Arc<dyn Fn(P) -> E + Send + Sync>, _: Scope<E>) -> Self {
        self
    }
}
impl<S, P: Send + 'static, E: Send + 'static, R: Send + Sync + 'static, T: Send + Sync + 'static>
    ServiceBindable<S, P, E, R> for ResourceBinding<T, E, R>
{
    /// Checked application-resource handle.
    type Value = ResourceValue<T>;
    /// Register the existing binding once, retaining its original application-error channel.
    fn selected(
        self,
        _: &S,
        _: Arc<dyn Fn(P) -> E + Send + Sync>,
        scope: Scope<E>,
    ) -> Effect<Self::Value, E, R> {
        self.acquire_scoped(scope)
    }
}
impl<S, A: Send + 'static, P: Send + 'static, E: Send + 'static, R: Send + Sync + 'static>
    ServiceBindable<S, P, E, R> for ServiceOperation<S, A, P, R>
{
    /// Provider operation success value.
    type Value = A;
    /// Map the provider operation's declared failure once while retaining all other channels.
    fn selected(
        self,
        service: &S,
        mapper: Arc<dyn Fn(P) -> E + Send + Sync>,
        _: Scope<E>,
    ) -> Effect<A, E, R> {
        (self.operation)(service).map_error(move |error| mapper(error))
    }
}
impl<S, T: Send + Sync + 'static, P: Send + 'static, E: Send + 'static, R: Send + Sync + 'static>
    ServiceBindable<S, P, E, R> for ServiceResource<S, T, P, R>
{
    /// Checked provider-resource handle.
    type Value = ResourceValue<T>;
    /// Map acquisition and release consistently and register exactly one owned finalizer.
    fn selected(
        self,
        service: &S,
        mapper: Arc<dyn Fn(P) -> E + Send + Sync>,
        scope: Scope<E>,
    ) -> Effect<Self::Value, E, R> {
        let acquire_mapper = Arc::clone(&mapper);
        let release = self.release;
        scope.acquire(
            (self.acquire)(service).map_error(move |error| acquire_mapper(error)),
            move |value| {
                let mapper = Arc::clone(&mapper);
                Release::from_effect(release(value).map_error(move |error| mapper(error)))
            },
        )
    }
}
/// Generated service-body constructor; the returned Effect is one-shot for this selected handle.
#[doc(hidden)]
pub struct ServiceBody;
impl ServiceBody {
    /// Execute an owned selection with a typed mapper and the application's original runtime context.
    /// Locals and native receiver operations are deferred until the Effect executes.
    pub fn from_body<S, P, A, E, R, F>(
        service: S,
        mapper: Arc<dyn Fn(P) -> E + Send + Sync>,
        factory: impl FnOnce(ServiceContext<S, P, E, R>) -> F + Send + 'static,
    ) -> Effect<A, E, R>
    where
        S: Send + Sync + 'static,
        P: Send + 'static,
        A: Send + 'static,
        E: Send + 'static,
        R: Send + Sync + 'static,
        F: Future<Output = Result<A, Exit<A, E>>> + Send + 'static,
    {
        crate::internal::workflow::once(move || {
            crate::internal::macro_future::async_workflow(move |context| async move {
                match factory(ServiceContext {
                    service,
                    mapper,
                    context,
                })
                .await
                {
                    Ok(value) => Exit::Success(value),
                    Err(failure) => failure,
                }
            })
        })
    }
}
/// Typed context of one selection; it borrows its owned receiver only inside the evaluation future.
#[doc(hidden)]
pub struct ServiceContext<S, P: Send + 'static, E: Send + 'static, R: Send + Sync + 'static> {
    /// Selected receiver; never a second environment/runtime selector.
    service: S,
    /// Explicit provider-to-application failure conversion.
    mapper: Arc<dyn Fn(P) -> E + Send + Sync>,
    /// Existing Effect execution and lexical resource owner.
    context: MacroContext<E, R>,
}
impl<S, P: Send + 'static, E: Send + 'static, R: Send + Sync + 'static> ServiceContext<S, P, E, R> {
    /// Borrow the owned service inside this evaluation, avoiding hidden receiver copies.
    pub fn service(&self) -> &S {
        &self.service
    }
    /// Bind provider operations or ordinary application Effects through their distinct typed adapters.
    pub async fn bind<B: ServiceBindable<S, P, E, R>, A: Send + 'static>(
        &self,
        program: B,
    ) -> Result<B::Value, Exit<A, E>> {
        self.context
            .bind(program.selected(
                &self.service,
                Arc::clone(&self.mapper),
                self.context.scope(),
            ))
            .await
    }
    /// Observe interruption between native loop iterations under the same runtime.
    pub async fn cooperate<A: Send + 'static>(&self) -> Result<(), Exit<A, E>> {
        self.context.cooperate().await
    }
}
