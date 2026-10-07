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
