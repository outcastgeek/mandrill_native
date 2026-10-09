//! Resources owned by one entity activation.

use super::ClusterOwner;
use crate::{Effect, Release, ResourceValue, Scope};
use std::sync::Arc;

/// Type alias for entity acquire in cluster entity resource.
///
/// # Type parameters
/// - `T`: Value or item type.
/// - `E`: Domain-error type.
/// - `R`: Environment type.
type EntityAcquire<T, E, R> = dyn Fn(&ClusterOwner) -> Effect<T, E, R> + Send + Sync;

/// Scoped live state for one activation. The descriptor is reusable, while
/// every open reacquires a fresh T and registers exactly one awaited release.
/// T is never placed in a message-store record or activation token.
///
/// # Type parameters
/// - `T`: Value or item type.
/// - `E`: Domain-error type.
/// - `R`: Environment type.
pub struct EntityResource<T, E, R>
where
    T: Send + Sync + 'static,
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    acquire: Arc<EntityAcquire<T, E, R>>,
    release: Arc<dyn Fn(T) -> Effect<(), E, R> + Send + Sync>,
}

impl<T, E, R> Clone for EntityResource<T, E, R>
where
    T: Send + Sync + 'static,
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    ///
    /// # Returns
    /// Type: `Self`.
    fn clone(&self) -> Self {
        Self {
            acquire: Arc::clone(&self.acquire),
            release: Arc::clone(&self.release),
        }
    }
}

impl<T, E, R> EntityResource<T, E, R>
where
    T: Send + Sync + 'static,
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    /// Describe owner-dependent acquisition and owned release callbacks without acquiring a resource; open later rechecks activation before exposing it.
    #[must_use]
    pub fn new(
        acquire: impl Fn(&ClusterOwner) -> Effect<T, E, R> + Send + Sync + 'static,
        release: impl Fn(T) -> Effect<(), E, R> + Send + Sync + 'static,
    ) -> Self {
        Self {
            acquire: Arc::new(acquire),
            release: Arc::new(release),
        }
    }
}

/// Acquisition, release, clock or activation-store failures while opening a fenced resource.
pub enum EntityResourceError<E, S> {
    /// Resource acquisition/release or the supplied time effect failed in its original typed channel.
    Resource(E),
    /// The activation store failed while checking the owner after acquisition.
    Store(S),
    /// Acquisition completed after activation ownership expired or was replaced; the scope still owns disposal.
    StaleOwnership,
}
impl<T: Send + Sync + 'static, E: Send + 'static, R: Send + Sync + 'static>
    EntityResource<T, E, R>
{
    /// Lazily acquire into scope, then recheck the exact activation owner before exposing live state.
    ///
    /// `now` supplies the check time in R. A stale or failed check never publishes the
    /// resource to the caller; scope ownership still guarantees its eventual protected
    /// release. Store and resource failures remain distinct typed channels.
    pub fn open<S: Send + 'static>(
        &self,
        scope: &Scope<EntityResourceError<E, S>>,
        owner: ClusterOwner,
        now: impl Fn() -> Effect<crate::UtcMillis, E, R> + Send + Sync + 'static,
    ) -> Effect<ResourceValue<T>, EntityResourceError<E, S>, R>
    where
        R: super::HasClusterActivationStore,
        R::Store: super::ClusterActivationStore<Error = S>,
    {
        use super::ClusterActivationStore;
        let acquire = Arc::clone(&self.acquire);
        let release = Arc::clone(&self.release);
        let check_owner = owner.clone();
        let now = Arc::new(now);
        scope
            .acquire(
                Effect::defer(move || acquire(&owner)).map_error(EntityResourceError::Resource),
                move |value| {
                    Release::from_effect(release(value).map_error(EntityResourceError::Resource))
                },
            )
            .flat_map(move |value| {
                let owner = check_owner.clone();
                let check_owner = check_owner.clone();
                let now = Arc::clone(&now);
                Effect::defer(move || now())
                    .map_error(EntityResourceError::Resource)
                    .zip(Effect::from_result_once(Ok(value)))
                    .flat_map(move |(now, value)| {
                        let owner = owner.clone();
                        let check_owner = check_owner.clone();
                        Effect::service(|env: &R| env.cluster_activation_store())
                            .flat_map(move |store| {
                                store
                                    .inspect_activation(owner.address.clone())
                                    .map_error(EntityResourceError::Store)
                                    .provide_some(|_: &R| ())
                            })
                            .zip(Effect::from_result_once(Ok(value)))
                            .flat_map(move |(status, value)| {
                                Effect::from_result_once(
                                    if status.is_some_and(|status| {
                                        status.phase == super::ActivationPhase::Active
                                            && status.lease.owner == check_owner
                                            && now < status.lease.lease_until
                                    }) {
                                        Ok(value)
                                    } else {
                                        Err(EntityResourceError::StaleOwnership)
                                    },
                                )
                            })
                    })
            })
    }
}

/// Reusable acquisition factory sharing the activation's owner and scope.
type PlanAcquire<A, E, R> = dyn Fn(Scope<E>, ClusterOwner) -> Effect<A, E, R> + Send + Sync;

/// A reusable activation-owned resource program. Construction is inert; `acquire` executes
/// under the supplied scope. Nested plans retain that same owner, and cleanup belongs to
/// the activation rather than the lexical block which assembles its live state.
pub struct EntityResourcePlan<A: Send + 'static, E: Send + 'static, R: Send + Sync + 'static> {
    /// Factory retaining typed acquisition and cleanup errors in one scope channel.
    acquire: Arc<PlanAcquire<A, E, R>>,
}
impl<A: Send + 'static, E: Send + 'static, R: Send + Sync + 'static> Clone
    for EntityResourcePlan<A, E, R>
{
    /// Share the plan description without copying acquired values.
    fn clone(&self) -> Self {
        Self {
            acquire: Arc::clone(&self.acquire),
        }
    }
}
impl<A: Send + 'static, E: Send + 'static, R: Send + Sync + 'static> EntityResourcePlan<A, E, R> {
    /// Describe acquisition into an existing activation scope; the callback must retain that owner.
    pub fn new(
        acquire: impl Fn(Scope<E>, ClusterOwner) -> Effect<A, E, R> + Send + Sync + 'static,
    ) -> Self {
        Self {
            acquire: Arc::new(acquire),
        }
    }
    /// Execute lazily in the supplied scope. Partial acquisition remains protected by its owner.
    pub fn acquire(&self, scope: Scope<E>, owner: ClusterOwner) -> Effect<A, E, R> {
        let acquire = Arc::clone(&self.acquire);
        Effect::defer(move || acquire(scope.clone(), owner.clone()))
    }
    /// Lift an ordinary Effect without replacing the enclosing activation scope.
    #[must_use]
    pub fn from_effect(effect: Effect<A, E, R>) -> Self {
        Self::new(move |_, _| effect.clone())
    }
    /// Reconstruct a helper on every acquisition rather than eagerly creating its locals.
    pub fn defer(factory: impl Fn() -> Self + Send + Sync + 'static) -> Self {
        Self::new(move |scope, owner| factory().acquire(scope, owner))
    }
    /// Produce a fresh value on acquisition, with no payload Clone requirement.
    pub fn succeed_with(value: impl Fn() -> A + Send + Sync + 'static) -> Self {
        Self::from_effect(Effect::sync(value))
    }
    /// Transform live state while retaining the existing resource owner.
    pub fn map<B: Send + 'static>(
        self,
        map: impl Fn(A) -> B + Send + Sync + 'static,
    ) -> EntityResourcePlan<B, E, R> {
        let map = Arc::new(map);
        EntityResourcePlan::new(move |scope, owner| {
            let map = Arc::clone(&map);
            self.acquire(scope, owner).map(move |value| map(value))
        })
    }
    /// Compose dependent acquisitions under the same activation and scope.
    pub fn flat_map<B: Send + 'static>(
        self,
        next: impl Fn(A) -> EntityResourcePlan<B, E, R> + Send + Sync + 'static,
    ) -> EntityResourcePlan<B, E, R> {
        let next = Arc::new(next);
        EntityResourcePlan::new(move |scope, owner| {
            let next = Arc::clone(&next);
            self.acquire(scope.clone(), owner.clone())
                .flat_map(move |value| next(value).acquire(scope.clone(), owner.clone()))
        })
    }
    /// Map acquisition and registered finalizer failures together, before changing the scope type.
    ///
    /// A child scope retains the original error type and registers one protected close in the
    /// parent scope. Already registered finalizers are never moved to an incompatible owner.
    pub fn map_error<F: Send + 'static>(
        self,
        map: impl Fn(E) -> F + Send + Sync + 'static,
    ) -> EntityResourcePlan<A, F, R> {
        let map = Arc::new(map);
        EntityResourcePlan::new(move |parent, owner| {
            let child = Scope::new();
            let cleanup = child.clone();
            let map_cleanup = Arc::clone(&map);
            let map_acquire = Arc::clone(&map);
            let acquisition = self
                .acquire(child, owner)
                .map_error(move |error| map_acquire(error));
            parent
                .add_finalizer(
                    cleanup
                        .close_owned()
                        .map_error(move |error| map_cleanup(error)),
                )
                .flat_map(move |_| acquisition.clone())
        })
    }
    /// Generated native body bridge. Resources remain in the externally supplied activation scope.
    #[doc(hidden)]
    pub fn from_body<F>(
        factory: impl Fn(EntityPlanContext<E, R>) -> F + Send + Sync + 'static,
    ) -> Self
    where
        F: std::future::Future<Output = Result<A, crate::Exit<A, E>>> + Send + 'static,
    {
        let factory = Arc::new(factory);
        Self::new(move |scope, owner| {
            let factory = Arc::clone(&factory);
            Effect::new(move |env, context, finish| {
                use crate::internal::runtime::{self, Step};
                use futures_util::FutureExt;
                let bridge = crate::internal::macro_future::MacroContext::with_scope(
                    env,
                    context,
                    scope.clone(),
                );
                let owner = owner.clone();
                let factory = Arc::clone(&factory);
                Step::Composed(Box::pin(async move {
                    let result = std::panic::AssertUnwindSafe(async {
                        factory(EntityPlanContext {
                            context: bridge,
                            owner,
                        })
                        .await
                    })
                    .catch_unwind()
                    .await;
                    runtime::finish(
                        finish,
                        match result {
                            Ok(Ok(value)) => crate::Exit::Success(value),
                            Ok(Err(failure)) => failure,
                            Err(panic) => crate::Exit::Die(crate::Defect::from_panic(panic)),
                        },
                    )
                }))
            })
        })
    }
}
impl<T: Send + Sync + 'static, E: Send + 'static, R: Send + Sync + 'static>
    EntityResource<T, E, R>
{
    /// Adapt a fenced resource with an explicitly typed check-time Effect. Both acquisition
    /// and release use EntityResourceError, retaining the post-acquisition ownership check.
    pub fn plan<S: Send + 'static>(
        self,
        now: impl Fn() -> Effect<crate::UtcMillis, E, R> + Send + Sync + 'static,
    ) -> EntityResourcePlan<ResourceValue<T>, EntityResourceError<E, S>, R>
    where
        R: super::HasClusterActivationStore,
        R::Store: super::ClusterActivationStore<Error = S>,
    {
        let now = Arc::new(now);
        EntityResourcePlan::new(move |scope, owner| {
            let now = Arc::clone(&now);
            self.open(&scope, owner, move || now())
        })
    }
}

/// Admission is sealed to existing Effects, resources and nested activation plans.
mod plan_sealed {
    /// Prevents consumers from installing an alternate evaluator through macro binding.
    pub trait Sealed {}
    impl<A: Send + 'static, E: Send + 'static, R: Send + Sync + 'static> Sealed
        for super::EntityResourcePlan<A, E, R>
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
    impl<T: Send + Sync + 'static, E: Send + 'static, R: Send + Sync + 'static> Sealed
        for super::EntityResource<T, E, R>
    {
    }
}
/// Typed activation macro bridge retaining the owner, scope and failure channel.
#[doc(hidden)]
pub trait EntityPlanBindable<E: Send + 'static, R: Send + Sync + 'static>:
    plan_sealed::Sealed
{
    /// Value moved out of successful acquisition.
    type Value: Send + 'static;
    /// Execute under the current owner and scope, registering cleanup before publishing resources.
    fn in_activation(self, scope: Scope<E>, owner: ClusterOwner) -> Effect<Self::Value, E, R>;
}
impl<A: Send + 'static, E: Send + 'static, R: Send + Sync + 'static> EntityPlanBindable<E, R>
    for EntityResourcePlan<A, E, R>
{
    /// Nested plan success value.
    type Value = A;
    /// Reuse rather than replace the owning activation scope.
    fn in_activation(self, scope: Scope<E>, owner: ClusterOwner) -> Effect<A, E, R> {
        self.acquire(scope, owner)
    }
}
impl<A: Send + 'static, E: Send + 'static, R: Send + Sync + 'static> EntityPlanBindable<E, R>
    for Effect<A, E, R>
{
    /// Ordinary Effect success value.
    type Value = A;
    /// Execute ordinary initialization without ending the activation's resource lifetime.
    fn in_activation(self, _: Scope<E>, _: ClusterOwner) -> Effect<A, E, R> {
        self
    }
}
impl<T: Send + Sync + 'static, E: Send + 'static, R: Send + Sync + 'static> EntityPlanBindable<E, R>
    for crate::ResourceBinding<T, E, R>
{
    /// Checked resource handle sharing only ownership metadata.
    type Value = ResourceValue<T>;
    /// Register the binding exactly once in the externally owned scope.
    fn in_activation(self, scope: Scope<E>, _: ClusterOwner) -> Effect<Self::Value, E, R> {
        self.acquire_scoped(scope)
    }
}
impl<T, E, S, R> EntityPlanBindable<EntityResourceError<E, S>, R> for EntityResource<T, E, R>
where
    T: Send + Sync + 'static,
    E: Send + 'static,
    S: Send + 'static,
    R: super::HasClusterActivationStore + crate::HasWallClock + Send + Sync + 'static,
    R::Store: super::ClusterActivationStore<Error = S>,
    R::Clock: crate::WallClock<Error = E>,
{
    /// Fenced resource handle; state itself is not copied.
    type Value = ResourceValue<T>;
    /// Use the environment's wall clock for the existing post-acquisition fencing check.
    fn in_activation(
        self,
        scope: Scope<EntityResourceError<E, S>>,
        owner: ClusterOwner,
    ) -> Effect<Self::Value, EntityResourceError<E, S>, R> {
        self.open(&scope, owner, Effect::wall_now)
    }
}
/// Generated-code context which retains activation ownership throughout dependent binds.
#[doc(hidden)]
pub struct EntityPlanContext<E: Send + 'static, R: Send + Sync + 'static> {
    /// Existing runtime, trace context and externally owned scope.
    context: crate::internal::macro_future::MacroContext<E, R>,
    /// Fencing identity shared by all nested acquisitions.
    owner: ClusterOwner,
}
impl<E: Send + 'static, R: Send + Sync + 'static> EntityPlanContext<E, R> {
    /// Bind only supported operations while sharing this activation's owner and scope.
    pub async fn bind<P: EntityPlanBindable<E, R>, A: Send + 'static>(
        &self,
        program: P,
    ) -> Result<P::Value, crate::Exit<A, E>> {
        self.context
            .bind(program.in_activation(self.context.scope(), self.owner.clone()))
            .await
    }
    /// Observe interruption between native loop iterations without releasing activation resources.
    pub async fn cooperate<A: Send + 'static>(&self) -> Result<(), crate::Exit<A, E>> {
        self.context.cooperate().await
    }
}
