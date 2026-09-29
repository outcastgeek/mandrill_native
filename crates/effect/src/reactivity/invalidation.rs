use crate::pub_sub::{
    PubSub, PubSubError, PubSubOptions, PubSubShutdown, PublishReport, Subscription,
};
use crate::{
    Effect, KeyDomain, Release, ResourceValue, Scope, Semaphore, SemaphoreError, StableKey,
};
use std::collections::{BTreeMap, BTreeSet};
use std::num::NonZeroU64;
use std::sync::{Arc, Mutex};

/// A stable namespace/domain key, not a process hash or source-location name.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct InvalidationKey(Arc<[u8]>);

impl InvalidationKey {
    #[must_use]
    pub fn from_stable<D: KeyDomain>(key: StableKey<D>) -> Self {
        Self(key.into_bytes())
    }

    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

pub struct InvalidationBatch {
    pub revision: NonZeroU64,
    pub keys: Arc<[InvalidationKey]>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InvalidationError {
    PubSub(PubSubError),
    Admission(SemaphoreError),
    RevisionExhausted,
    InputConsumed,
}

struct RegistryState {
    next_revision: Option<NonZeroU64>,
    per_key: BTreeMap<InvalidationKey, NonZeroU64>,
}

/// A registry's history, publication order and batching are local to this
/// instance. Bounded subscribers choose overload behavior through PubSubOptions;
/// a lag report requires a query to resnapshot its dependencies.
pub struct InvalidationRegistry {
    hub: PubSub<InvalidationBatch>,
    order: Semaphore,
    state: Arc<Mutex<RegistryState>>,
}

impl Clone for InvalidationRegistry {
    fn clone(&self) -> Self {
        Self {
            hub: self.hub.clone(),
            order: self.order.clone(),
            state: Arc::clone(&self.state),
        }
    }
}

impl InvalidationRegistry {
    pub fn new(options: PubSubOptions) -> Result<Self, InvalidationError> {
        Ok(Self {
            hub: PubSub::new(options).map_err(InvalidationError::PubSub)?,
            order: Semaphore::full(std::num::NonZeroUsize::MIN),
            state: Arc::new(Mutex::new(RegistryState {
                next_revision: Some(NonZeroU64::MIN),
                per_key: BTreeMap::new(),
            })),
        })
    }

    pub fn subscribe<R: Send + Sync + 'static>(
        &self,
    ) -> Effect<Subscription<InvalidationBatch>, InvalidationError, R> {
        self.hub.subscribe().map_error(InvalidationError::PubSub)
    }

    pub fn subscribe_scoped<R: Send + Sync + 'static>(
        &self,
        scope: &Scope<InvalidationError>,
    ) -> Effect<ResourceValue<Subscription<InvalidationBatch>>, InvalidationError, R> {
        scope.acquire(self.subscribe(), |subscription| {
            Release::sync(move || {
                subscription.close_now();
            })
        })
    }

    /// An empty batch is a no-op and consumes no revision. Duplicate keys are
    /// normalized before one serialized publication.
    pub fn invalidate<R: Send + Sync + 'static>(
        &self,
        keys: Vec<InvalidationKey>,
    ) -> Effect<Option<PublishReport>, InvalidationError, R> {
        let registry = self.clone();
        let keys = Mutex::new(Some(keys));
        Effect::try_sync(
            move || {
                let keys = keys
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .take()
                    .ok_or(InvalidationError::InputConsumed)?;
                Ok(keys.into_iter().collect::<BTreeSet<_>>())
            },
            std::convert::identity,
        )
        .flat_map(move |keys| {
            if keys.is_empty() {
                return Effect::sync(|| None);
            }
            let registry = registry.clone();
            let keys = Mutex::new(Some(keys));
            registry
                .order
                .acquire(std::num::NonZeroUsize::MIN)
                .map_error(InvalidationError::Admission)
                .flat_map(move |permit| {
                    let keys = keys
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .take();
                    let Some(keys) = keys else {
                        return Effect::from_result_once(Err(InvalidationError::InputConsumed));
                    };
                    let registry = registry.clone();
                    let revision = {
                        let mut state = registry
                            .state
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner);
                        match state.next_revision {
                            Some(revision) => {
                                state.next_revision =
                                    revision.get().checked_add(1).and_then(NonZeroU64::new);
                                Ok(revision)
                            }
                            None => Err(InvalidationError::RevisionExhausted),
                        }
                    };
                    let publication = match revision {
                        Ok(revision) => {
                            let keys: Arc<[InvalidationKey]> =
                                Arc::from(keys.into_iter().collect::<Vec<_>>());
                            let state = Arc::clone(&registry.state);
                            registry
                                .hub
                                .publish_with_commit(
                                    InvalidationBatch { revision, keys },
                                    move |batch| {
                                        let mut state = state
                                            .lock()
                                            .unwrap_or_else(std::sync::PoisonError::into_inner);
                                        for key in batch.keys.iter() {
                                            state.per_key.insert(key.clone(), batch.revision);
                                        }
                                    },
                                )
                                .map_error(InvalidationError::PubSub)
                                .map(Some)
                        }
                        Err(error) => Effect::from_result_once(Err(error)),
                    };
                    publication.ensuring(Effect::sync_once(move || drop(permit)))
                })
        })
    }

    /// Snapshot a dependency set at one registry-state boundary. Query code
    /// uses this before and after evaluation to detect invalidation during a
    /// pending run, even if its subscription buffer reported lag.
    pub fn revision_of<R: Send + Sync + 'static>(
        &self,
        keys: Arc<[InvalidationKey]>,
    ) -> Effect<Option<NonZeroU64>, InvalidationError, R> {
        let state = Arc::clone(&self.state);
        Effect::sync(move || {
            let state = state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            keys.iter()
                .filter_map(|key| state.per_key.get(key).copied())
                .max()
        })
    }

    pub fn shutdown<R: Send + Sync + 'static>(
        &self,
        mode: PubSubShutdown,
    ) -> Effect<bool, InvalidationError, R> {
        self.hub.shutdown(mode).map_error(InvalidationError::PubSub)
    }
}

pub type InvalidationSubscription = Subscription<InvalidationBatch>;
