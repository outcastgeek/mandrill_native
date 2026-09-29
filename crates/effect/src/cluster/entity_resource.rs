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
    #[must_use]
    /// Construct a `EntityResource` from its required inputs and explicit policies.
    ///
    /// # Type parameters
    /// - `E`: Domain-error type.
    /// - `R`: Environment type.
    /// - `T`: Value or item type.
    ///
    /// # Parameters
    /// - `acquire` (`impl Fn(&ClusterOwner) -> Effect<T, E, R> + Send + Sync + 'static`): Description that obtains the resource before its body runs.
    /// - `release` (`impl Fn(T) -> Effect<(), E, R> + Send + Sync + 'static`): The release supplied to this operation.
    ///
    /// # Returns
    /// Type: `Self`.
    pub fn new(
        acquire: impl Fn(&ClusterOwner) -> Effect<T, E, R> + Send + Sync + 'static,
        release: impl Fn(T) -> Effect<(), E, R> + Send + Sync + 'static,
    ) -> Self {
        Self {
            acquire: Arc::new(acquire),
            release: Arc::new(release),
        }
    }

    /// Open the resource within its declared scope and ownership boundary.
    ///
    /// # Type parameters
    /// - `E`: Domain-error type.
    /// - `R`: Environment type.
    /// - `T`: Value or item type.
    ///
    /// # Parameters
    /// - `scope` (`&Scope<E>`): Owner that joins child work and releases resources at close.
    /// - `owner` (`ClusterOwner`): Worker or scope identity used to fence stale operations.
    ///
    /// # Returns
    /// Type: `Effect<ResourceValue<T>, E, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    pub fn open(&self, scope: &Scope<E>, owner: ClusterOwner) -> Effect<ResourceValue<T>, E, R> {
        let acquire = Arc::clone(&self.acquire);
        let release = Arc::clone(&self.release);
        scope.acquire(Effect::defer(move || acquire(&owner)), move |value| {
            Release::from_effect(release(value))
        })
    }
}
