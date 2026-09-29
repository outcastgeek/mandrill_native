//! Coalesced invalidation keys and query reruns.

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
    /// Wrap a typed stable key while preserving its identity bytes.
    ///
    /// # Type parameters
    /// - `D`: Identity-domain type. Bounds: `KeyDomain`.
    ///
    /// # Parameters
    /// - `key` (`StableKey<D>`): Stable key used to locate or identify the selected value.
    ///
    /// # Returns
    /// Type: `Self`.
    pub fn from_stable<D: KeyDomain>(key: StableKey<D>) -> Self {
        Self(key.into_bytes())
    }

    #[must_use]
    /// Borrow the stable encoded bytes without transferring ownership.
    ///
    /// # Returns
    /// Type: `&[u8]`.
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

/// A set of stable keys published together to mark dependent state stale.
pub struct InvalidationBatch {
    /// The recorded storage revision. Type: `NonZeroU64`.
    pub revision: NonZeroU64,
    /// Stable keys whose dependent values must be resnapshotted. Type: `Arc<[InvalidationKey]>`.
    pub keys: Arc<[InvalidationKey]>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// A capacity or publication failure while sending invalidations.
pub enum InvalidationError {
    /// The publication channel rejected or failed the operation. Retains the `PubSubError` detail.
    PubSub(PubSubError),
    /// The provider rejected work before accepting ownership. Retains the `SemaphoreError` detail.
    Admission(SemaphoreError),
    /// No further revision can be allocated without exceeding its range.
    RevisionExhausted,
    /// An owned input was already consumed.
    InputConsumed,
}

/// Typed registry state used by reactivity invalidation.
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
    ///
    /// # Returns
    /// Type: `Self`.
    fn clone(&self) -> Self {
        Self {
            hub: self.hub.clone(),
            order: self.order.clone(),
            state: Arc::clone(&self.state),
        }
    }
}

impl InvalidationRegistry {
    /// Construct a `InvalidationRegistry` from its required inputs and explicit policies.
    ///
    /// # Parameters
    /// - `options` (`PubSubOptions`): Caller-selected bounds and behavior for this operation.
    ///
    /// # Returns
    /// Type: `Result<Self, InvalidationError>`.
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

    /// Register an observer for future committed changes.
    ///
    /// # Type parameters
    /// - `R`: Environment type. Bounds: `Send + Sync + 'static`.
    ///
    /// # Returns
    /// Type: `Effect<Subscription<InvalidationBatch>, InvalidationError, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    pub fn subscribe<R: Send + Sync + 'static>(
        &self,
    ) -> Effect<Subscription<InvalidationBatch>, InvalidationError, R> {
        self.hub.subscribe().map_error(InvalidationError::PubSub)
    }

    /// Register a subscriber owned by the supplied scope.
    ///
    /// # Type parameters
    /// - `R`: Environment type. Bounds: `Send + Sync + 'static`.
    ///
    /// # Parameters
    /// - `scope` (`&Scope<InvalidationError>`): Owner that joins child work and releases resources at close.
    ///
    /// # Returns
    /// Type: `Effect<ResourceValue<Subscription<InvalidationBatch>>, InvalidationError, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
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
    ///
    /// # Type parameters
    /// - `R`: Environment type. Bounds: `Send + Sync + 'static`.
    ///
    /// # Parameters
    /// - `keys` (`Vec<InvalidationKey>`): The keys supplied to this operation.
    ///
    /// # Returns
    /// Type: `Effect<Option<PublishReport>, InvalidationError, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
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
    ///
    /// # Type parameters
    /// - `R`: Environment type. Bounds: `Send + Sync + 'static`.
    ///
    /// # Parameters
    /// - `keys` (`Arc<[InvalidationKey]>`): The keys supplied to this operation.
    ///
    /// # Returns
    /// Type: `Effect<Option<NonZeroU64>, InvalidationError, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
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

    /// Stop accepting new work and describe the settlement of owned work.
    ///
    /// # Type parameters
    /// - `R`: Environment type. Bounds: `Send + Sync + 'static`.
    ///
    /// # Parameters
    /// - `mode` (`PubSubShutdown`): Selected behavior for this operation.
    ///
    /// # Returns
    /// Type: `Effect<bool, InvalidationError, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    pub fn shutdown<R: Send + Sync + 'static>(
        &self,
        mode: PubSubShutdown,
    ) -> Effect<bool, InvalidationError, R> {
        self.hub.shutdown(mode).map_error(InvalidationError::PubSub)
    }
}

/// Type alias for invalidation subscription in reactivity invalidation.
pub type InvalidationSubscription = Subscription<InvalidationBatch>;
