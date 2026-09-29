use super::ClusterOwner;
use crate::{Effect, Release, ResourceValue, Scope};
use std::sync::Arc;

type EntityAcquire<T, E, R> = dyn Fn(&ClusterOwner) -> Effect<T, E, R> + Send + Sync;

/// Scoped live state for one activation. The descriptor is reusable, while
/// every open reacquires a fresh T and registers exactly one awaited release.
/// T is never placed in a message-store record or activation token.
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
    pub fn new(
        acquire: impl Fn(&ClusterOwner) -> Effect<T, E, R> + Send + Sync + 'static,
        release: impl Fn(T) -> Effect<(), E, R> + Send + Sync + 'static,
    ) -> Self {
        Self {
            acquire: Arc::new(acquire),
            release: Arc::new(release),
        }
    }

    pub fn open(&self, scope: &Scope<E>, owner: ClusterOwner) -> Effect<ResourceValue<T>, E, R> {
        let acquire = Arc::clone(&self.acquire);
        let release = Arc::clone(&self.release);
        scope.acquire(Effect::defer(move || acquire(&owner)), move |value| {
            Release::from_effect(release(value))
        })
    }
}
