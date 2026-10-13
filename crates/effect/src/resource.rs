//! Acquisition, use, and awaited release of scoped resources.

use crate::LibraryDefect;
use crate::internal::destruction::FlatDrop;
use crate::internal::outcome;
use crate::internal::runtime::{self, Context, Finish, Step};
use crate::internal::workflow::{self, Workflow};
use crate::scope::{Registered, Scope};
use crate::{Cancellation, Defect, Effect, Exit, PrimaryOutcomeRef};
use futures_util::FutureExt;
use std::future::Future;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Arc, RwLock};

/// Awaited, single-use cleanup. Its factories can consume an owned resource
/// without requiring Clone or constructing a reusable future around it.
#[must_use = "a release must be returned to its resource binding"]
///
/// # Type parameters
/// - `E`: Domain-error type. Bound or type: `Send + 'static`.
/// - `R`: Environment type. Bound or type: `Send + Sync + 'static = ()`.
pub struct Release<E: Send + 'static, R: Send + Sync + 'static = ()> {
    workflow: Workflow<(), E, R>,
}

impl<E: Send + 'static, R: Send + Sync + 'static> Release<E, R> {
    /// Describe a single-use synchronous release callback for an owned resource.
    ///
    /// # Type parameters
    /// - `E`: Domain-error type. Bound: `Send + 'static`.
    /// - `R`: Environment type. Bound: `Send + Sync + 'static`.
    ///
    /// # Parameters
    /// - `callback` (`impl FnOnce() + Send + 'static`): The callback invoked by this operation.
    ///
    /// # Returns
    /// Type: `Self`.
    pub fn sync(callback: impl FnOnce() + Send + 'static) -> Self {
        Self::try_sync(move || {
            callback();
            Ok(())
        })
    }

    /// Describe a single-use synchronous release that can return an expected error.
    ///
    /// # Type parameters
    /// - `R`: Environment type. Bound: `Send + Sync + 'static`.
    /// - `E`: Domain-error type.
    ///
    /// # Parameters
    /// - `callback` (`impl FnOnce() -> Result<(), E> + Send + 'static`): The callback invoked by this operation.
    ///
    /// # Returns
    /// Type: `Self`.
    pub fn try_sync(callback: impl FnOnce() -> Result<(), E> + Send + 'static) -> Self {
        Self {
            workflow: Workflow::new(move |_, _, finish| {
                let result = catch_unwind(AssertUnwindSafe(callback));
                runtime::finish(
                    finish,
                    match result {
                        Ok(Ok(())) => Exit::Success(()),
                        Ok(Err(error)) => Exit::Fail(error),
                        Err(panic) => Exit::Die(Defect::from_panic(panic)),
                    },
                )
            }),
        }
    }

    /// Create a single-use release from a future factory, preserving resource ownership.
    ///
    /// # Type parameters
    /// - `E`: Domain-error type. Bound: `Send + 'static`.
    /// - `R`: Environment type. Bound: `Send + Sync + 'static`.
    /// - `F`: Bounds: `Future<Output = ()> + Send + 'static`.
    ///
    /// # Parameters
    /// - `factory` (`impl FnOnce(Cancellation) -> F + Send + 'static`): Factory invoked to create fresh data for an execution.
    ///
    /// # Returns
    /// Type: `Self`.
    pub fn from_future<F: Future<Output = ()> + Send + 'static>(
        factory: impl FnOnce(Cancellation) -> F + Send + 'static,
    ) -> Self {
        Self::try_future(move |token| async move {
            factory(token).await;
            Ok(())
        })
    }

    /// Create a single-use fallible release from a future factory.
    ///
    /// # Type parameters
    /// - `E`: Domain-error type. Bound: `Send + 'static`.
    /// - `R`: Environment type. Bound: `Send + Sync + 'static`.
    /// - `F`: Bounds: `Future<Output = Result<(), E>> + Send + 'static`.
    ///
    /// # Parameters
    /// - `factory` (`impl FnOnce(Cancellation) -> F + Send + 'static`): Factory invoked to create fresh data for an execution.
    ///
    /// # Returns
    /// Type: `Self`.
    pub fn try_future<F: Future<Output = Result<(), E>> + Send + 'static>(
        factory: impl FnOnce(Cancellation) -> F + Send + 'static,
    ) -> Self {
        Self {
            workflow: Workflow::new(move |_, context, finish| {
                match catch_unwind(AssertUnwindSafe(|| factory(context.cancellation.clone()))) {
                    Err(panic) => runtime::finish(finish, Exit::Die(Defect::from_panic(panic))),
                    Ok(pending) => Step::Suspended(Box::pin(async move {
                        let result = AssertUnwindSafe(pending).catch_unwind().await;
                        runtime::finish(
                            finish,
                            match result {
                                Ok(Ok(())) => Exit::Success(()),
                                Ok(Err(error)) => Exit::Fail(error),
                                Err(panic) => Exit::Die(Defect::from_panic(panic)),
                            },
                        )
                    })),
                }
            }),
        }
    }

    /// Adapt an existing Effect while preserving its typed outcome channels.
    ///
    /// # Type parameters
    /// - `E`: Domain-error type.
    /// - `R`: Environment type.
    ///
    /// # Parameters
    /// - `program` (`Effect<(), E, R>`): Lazy Effect description executed or transformed here.
    ///
    /// # Returns
    /// Type: `Self`.
    pub fn from_effect(program: Effect<(), E, R>) -> Self {
        Self {
            workflow: Workflow::new(move |env, context, finish| {
                program.execute(env, context, finish)
            }),
        }
    }

    /// # Type parameters
    /// - `E`: Domain-error type.
    /// - `R`: Environment type.
    ///
    /// # Parameters
    /// - `env` (`Arc<R>`): The environment supplying this operation's capabilities.
    /// - `context` (`Context`): Execution context carrying cancellation, scope, and trace state.
    /// - `finish` (`Finish<(), E>`): Continuation receiving the complete terminal outcome.
    ///
    /// # Returns
    /// Type: `Step`.
    pub(crate) fn execute(self, env: Arc<R>, context: Context, finish: Finish<(), E>) -> Step {
        self.workflow.execute(env, context, finish)
    }
}

/// Access handle valid only until the binding's body finishes. Cloning or
/// returning this handle does not extend the underlying resource lifetime.
///
/// # Type parameters
/// - `T`: Value or item type.
pub struct ResourceValue<T> {
    value: Arc<RwLock<Option<T>>>,
}

impl<T> Clone for ResourceValue<T> {
    ///
    /// # Returns
    /// Type: `Self`.
    fn clone(&self) -> Self {
        Self {
            value: Arc::clone(&self.value),
        }
    }
}

impl<T> ResourceValue<T> {
    ///
    /// # Type parameters
    /// - `T`: Value or item type.
    ///
    /// # Parameters
    /// - `value` (`T`): Value read, transformed, encoded, or stored by this operation.
    ///
    /// # Returns
    /// Type: `Self`.
    fn new(value: T) -> Self {
        Self {
            value: Arc::new(RwLock::new(Some(value))),
        }
    }

    /// Synchronous, short access. A borrowed reference cannot escape this call.
    /// Never wait for resource release while inside this callback.
    ///
    /// # Type parameters
    /// - `A`: Application value type.
    /// - `T`: Value or item type.
    ///
    /// # Parameters
    /// - `access` (`impl FnOnce(&T) -> A`): The access supplied to this operation.
    ///
    /// # Returns
    /// Type: `Result<A, Defect>`.
    pub fn with<A>(&self, access: impl FnOnce(&T) -> A) -> Result<A, Defect> {
        let value = self
            .value
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        match value.as_ref() {
            Some(value) => Ok(access(value)),
            None => Err(Defect::from(LibraryDefect::ResourceScopeEnded)),
        }
    }

    ///
    /// # Type parameters
    /// - `T`: Value or item type.
    ///
    /// # Returns
    /// Type: `Option<T>`.
    /// Returns the value when present, or `None`.
    fn invalidate(&self) -> Option<T> {
        self.value
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
    }
}

impl<T: Send + Sync + 'static> ResourceValue<T> {
    /// Effectful access reports a closed handle as a defect. Copy owned data
    /// while inside the callback; references cannot cross an async suspension.
    ///
    /// # Type parameters
    /// - `A`: Application value type. Additional bound: `Send + 'static`.
    /// - `E`: Domain-error type. Additional bound: `Send + 'static`.
    /// - `R`: Environment type. Additional bound: `Send + Sync + 'static`.
    /// - `T`: Value or item type.
    ///
    /// # Parameters
    /// - `access` (`impl Fn(&T) -> A + Send + Sync + 'static`): The access supplied to this operation.
    ///
    /// # Returns
    /// Type: `Effect<A, E, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    pub fn access<A, E, R>(
        &self,
        access: impl Fn(&T) -> A + Send + Sync + 'static,
    ) -> Effect<A, E, R>
    where
        A: Send + 'static,
        E: Send + 'static,
        R: Send + Sync + 'static,
    {
        let handle = self.clone();
        Effect::ready(move |_, _| match handle.with(&access) {
            Ok(value) => Exit::Success(value),
            Err(defect) => Exit::Die(defect),
        })
    }
}

// Abandonment cannot await release, but must invalidate every escaped handle
// and relinquish the owned value. Run/scope guards report ownership loss.
/// Typed resource owner used by resource.
///
/// # Type parameters
/// - `T`: Value or item type.
struct ResourceOwner<T> {
    handle: ResourceValue<T>,
}

impl<T> ResourceOwner<T> {
    ///
    /// # Type parameters
    /// - `T`: Value or item type.
    ///
    /// # Parameters
    /// - `handle` (`ResourceValue<T>`): Owned or borrowed capability handle kept live through this operation.
    ///
    /// # Returns
    /// Type: `Self`.
    fn new(handle: ResourceValue<T>) -> Self {
        Self { handle }
    }

    ///
    /// # Type parameters
    /// - `T`: Value or item type.
    ///
    /// # Returns
    /// Type: `Option<T>`.
    /// Returns the value when present, or `None`.
    fn take(&self) -> Option<T> {
        self.handle.invalidate()
    }
}

impl<T> Drop for ResourceOwner<T> {
    ///
    /// # Returns
    /// Type: `()`.
    fn drop(&mut self) {
        drop(self.take());
    }
}

/// Type alias for releaser in resource.
///
/// # Type parameters
/// - `T`: Value or item type.
/// - `E`: Domain-error type.
/// - `R`: Environment type.
type Releaser<T, E, R> =
    Arc<dyn for<'outcome> Fn(T, PrimaryOutcomeRef<'outcome, E>) -> Release<E, R> + Send + Sync>;

/// Declares acquisition and release together before work begins.
/// The body receives an invalidatable handle, not an escaping `Arc<T>`.
#[must_use = "use the resource binding to establish its release obligation"]
///
/// # Type parameters
/// - `T`: Value or item type. Bound or type: `Send + Sync + 'static`.
/// - `E`: Domain-error type. Bound or type: `Send + 'static`.
/// - `R`: Environment type. Bound or type: `Send + Sync + 'static = ()`.
pub struct ResourceBinding<
    T: Send + Sync + 'static,
    E: Send + 'static,
    R: Send + Sync + 'static = (),
> {
    acquire: Effect<T, E, R>,
    release: Releaser<T, E, R>,
}

impl<T: Send + Sync + 'static, E: Send + 'static, R: Send + Sync + 'static> Clone
    for ResourceBinding<T, E, R>
{
    ///
    /// # Returns
    /// Type: `Self`.
    fn clone(&self) -> Self {
        Self {
            acquire: self.acquire.clone(),
            release: Arc::clone(&self.release),
        }
    }
}

impl<T: Send + Sync + 'static, E: Send + 'static, R: Send + Sync + 'static>
    ResourceBinding<T, E, R>
{
    /// Construct a `ResourceBinding` from its required inputs and explicit policies.
    ///
    /// # Type parameters
    /// - `E`: Domain-error type.
    /// - `R`: Environment type.
    /// - `T`: Value or item type.
    ///
    /// # Parameters
    /// - `acquire` (`Effect<T, E, R>`): Description that obtains the resource before its body runs.
    /// - `release` (`impl for<'outcome> Fn(T, PrimaryOutcomeRef<'outcome, E>) -> Release<E, R> + Send + Sync + 'static`): The release supplied to this operation.
    ///
    /// # Returns
    /// Type: `Self`.
    pub fn new(
        acquire: Effect<T, E, R>,
        release: impl for<'outcome> Fn(T, PrimaryOutcomeRef<'outcome, E>) -> Release<E, R>
        + Send
        + Sync
        + 'static,
    ) -> Self {
        Self {
            acquire,
            release: Arc::new(release),
        }
    }

    /// Acquire an owned resource lazily, run its body, then execute protected release.
    ///
    /// `use_resource` receives a checked ResourceValue handle, not the underlying
    /// payload. Release consumes that payload and receives the body's borrowed primary
    /// outcome. Construction panics, body interruption and release failures preserve
    /// primary/cleanup channels; no payload cloning is required.
    pub fn use_with<A: Send + 'static>(
        self,
        use_resource: impl Fn(ResourceValue<T>) -> Effect<A, E, R> + Send + Sync + 'static,
    ) -> Effect<A, E, R> {
        let use_resource = Arc::new(use_resource);
        workflow::reusable(move || {
            let use_resource = Arc::clone(&use_resource);
            self.clone().bind_run(move |handle| {
                let body = use_resource(handle);
                Workflow::new(move |env, context, finish| body.execute(env, context, finish))
            })
        })
    }

    /// # Type parameters
    /// - `A`: Application value type. Bounds: `Send + 'static`.
    /// - `E`: Domain-error type.
    /// - `R`: Environment type.
    /// - `T`: Value or item type.
    ///
    /// # Parameters
    /// - `use_resource` (`impl FnOnce(ResourceValue<T>) -> Workflow<A, E, R> + Send + 'static`): The use resource supplied to this operation.
    ///
    /// # Returns
    /// Type: `Workflow<A, E, R>`.
    pub(crate) fn bind_run<A: Send + 'static>(
        self,
        use_resource: impl FnOnce(ResourceValue<T>) -> Workflow<A, E, R> + Send + 'static,
    ) -> Workflow<A, E, R> {
        Workflow::new(move |env, context, finish| {
            let release = Arc::clone(&self.release);
            let use_resource = FlatDrop::new(Box::new(use_resource));
            self.acquire.clone().execute(
                Arc::clone(&env),
                context.clone(),
                runtime::continuation(move |acquired| {
                    match acquired.into_failure() {
                        Err(failure) => runtime::finish(finish, failure),
                        Ok(resource) => {
                            // Ownership is installed before checking cancellation
                            // or invoking the user body factory after raw success.
                            let handle = ResourceValue::new(resource);
                            let owner = ResourceOwner::new(handle.clone());
                            let release_env = Arc::clone(&env);
                            let release_context = context.clone();
                            let after_body = runtime::continuation(move |result| {
                                let result = runtime::publish(result, &release_context);
                                let protected = release_context.protected();
                                let cleanup = catch_unwind(AssertUnwindSafe(|| {
                                    owner.take().map(|value| release(value, result.primary()))
                                }));
                                let cleanup = match cleanup {
                                    Ok(Some(cleanup)) => cleanup,
                                    Ok(None) => {
                                        return runtime::finish(
                                            finish,
                                            outcome::append_cleanup(
                                                result,
                                                Exit::Die(Defect::from(
                                                    LibraryDefect::ResourceReleasedTwice,
                                                )),
                                            ),
                                        );
                                    }
                                    Err(panic) => {
                                        return runtime::finish(
                                            finish,
                                            outcome::append_cleanup(
                                                result,
                                                Exit::Die(Defect::from_panic(panic)),
                                            ),
                                        );
                                    }
                                };
                                cleanup.execute(
                                    release_env,
                                    protected,
                                    runtime::continuation(move |cleanup| {
                                        let result = runtime::publish(result, &release_context);
                                        runtime::finish(
                                            finish,
                                            outcome::append_cleanup(result, cleanup),
                                        )
                                    }),
                                )
                            });
                            if context.cancelled() {
                                return runtime::finish(after_body, Exit::Interrupted);
                            }
                            match catch_unwind(AssertUnwindSafe(|| {
                                use_resource.take().map(|use_resource| use_resource(handle))
                            })) {
                                Ok(Some(body)) => body.execute(env, context, after_body),
                                Ok(None) => runtime::finish(
                                    after_body,
                                    Exit::Die(Defect::from(LibraryDefect::ResourceBodyConsumed)),
                                ),
                                Err(panic) => runtime::finish(
                                    after_body,
                                    Exit::Die(Defect::from_panic(panic)),
                                ),
                            }
                        }
                    }
                }),
            )
        })
    }

    /// Transfer acquisition into scope ownership before publishing a checked handle; construct and execute release in protected cleanup, preserving defects.
    pub(crate) fn acquire_scoped(self, scope: Scope<E>) -> Effect<ResourceValue<T>, E, R> {
        Effect::new(move |env, context, finish| {
            let scope = scope.clone();
            let release = Arc::clone(&self.release);
            self.acquire.clone().execute(
                Arc::clone(&env),
                context.clone(),
                runtime::continuation(move |acquired| match acquired.into_failure() {
                    Err(failure) => runtime::finish(finish, failure),
                    Ok(resource) => {
                        // Register raw ownership before any cancellation checkpoint.
                        let handle = ResourceValue::new(resource);
                        let owner = ResourceOwner::new(handle.clone());
                        let abandoned = handle.clone();
                        let context = context.protected();
                        let finalizer = Registered::with_abandon(
                            move |finish| match catch_unwind(AssertUnwindSafe(|| {
                                owner
                                    .take()
                                    .map(|resource| release(resource, PrimaryOutcomeRef::Succeeded))
                            })) {
                                Ok(Some(cleanup)) => cleanup.execute(env, context, finish),
                                Ok(None) => runtime::finish(
                                    finish,
                                    Exit::Die(Defect::from(LibraryDefect::ResourceReleasedTwice)),
                                ),
                                Err(panic) => {
                                    runtime::finish(finish, Exit::Die(Defect::from_panic(panic)))
                                }
                            },
                            move || drop(abandoned.invalidate()),
                        );
                        scope.register_or_release(finalizer, handle, finish)
                    }
                }),
            )
        })
    }
}

impl<E: Send + 'static, R: Send + Sync + 'static> Release<E, R> {
    /// An owned cleanup future that can preserve several failures and all Exit
    /// families. Factory/poll unwinds become release defects under protection.
    ///
    /// # Type parameters
    /// - `E`: Domain-error type. Bound: `Send + 'static`.
    /// - `R`: Environment type. Bound: `Send + Sync + 'static`.
    /// - `F`: Bounds: `Future<Output = Exit<(), E>> + Send + 'static`.
    ///
    /// # Parameters
    /// - `factory` (`impl FnOnce(Cancellation) -> F + Send + 'static`): Factory invoked to create fresh data for an execution.
    ///
    /// # Returns
    /// Type: `Self`.
    pub fn from_exit_future<F: Future<Output = Exit<(), E>> + Send + 'static>(
        factory: impl FnOnce(Cancellation) -> F + Send + 'static,
    ) -> Self {
        Self {
            workflow: Workflow::new(move |_, context, finish| {
                Step::Suspended(Box::pin(async move {
                    let result = AssertUnwindSafe(async { factory(context.cancellation).await })
                        .catch_unwind()
                        .await
                        .unwrap_or_else(|panic| Exit::Die(Defect::from_panic(panic)));
                    runtime::finish(finish, result)
                }))
            }),
        }
    }
}

/// Adapt an existing Effect cleanup description into a single-use protected release.
/// This conversion does not run cleanup or change expected-error/defect channels.
impl<E: Send + 'static, R: Send + Sync + 'static> From<Effect<(), E, R>> for Release<E, R> {
    /// Preserve the Effect description until protected release execution.
    fn from(program: Effect<(), E, R>) -> Self {
        Self::from_effect(program)
    }
}
