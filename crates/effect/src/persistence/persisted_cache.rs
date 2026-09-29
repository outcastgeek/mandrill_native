//! Composition of memory cache with stored results.

use super::internal::cache_gate::CacheGate;
use super::{
    HasKeyValueStore, KeyValueStore, Persistable, PersistedRecordError, StoreRead, StoredValue,
};
use crate::{
    Cache, CacheCapacity, CacheConfig, Effect, HasClock, HasTaskSpawner, HasWallClock, KeyDomain,
    KeyError, Release, ResourceBinding, SchemaIssue, Scope, SemaphoreError, SharedValue,
    StoredOutcome, TimeError, WallClock,
};
use std::marker::PhantomData;
use std::sync::Arc;
use std::time::Duration;

/// The cache's process-local single flight and the backing store have separate
/// lifetimes. Sharing the backing store does not coordinate cache misses across
/// processes, or between two independently constructed cache instances.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CorruptCacheRecord {
    /// Reject the conflicting input without applying its proposed state change.
    Reject,
    /// Discard the corrupt cache record and load fresh data.
    EvictAndReload,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// The expiry, capacity, and version rules for a cache backed by a store.
pub struct PersistedCachePolicy {
    /// Expiry policy applied to records in the backing store. Type: `Option<Duration>`.
    pub persistent_ttl: Option<Duration>,
    /// Whether expected load failures are retained in the cache. Type: `bool`.
    pub persist_failures: bool,
    /// Policy for a stored record that fails schema decoding. Type: `CorruptCacheRecord`.
    pub corrupt_record: CorruptCacheRecord,
}

/// Memory retention for a persisted cache. Stable encoded identity determines
/// equality for both memory and backing records, so it is not configurable.
///
/// # Type parameters
/// - `E`: Domain-error type.
pub struct PersistedCacheMemory<E> {
    /// Time for which the stored value remains valid. Type: `Option<Duration>`.
    pub ttl: Option<Duration>,
    /// The maximum admitted capacity. Type: `CacheCapacity`.
    pub capacity: CacheCapacity,
    /// Predicate selecting expected errors that remain cached. Type: `Arc<dyn Fn(&E) -> bool + Send + Sync>`.
    pub cache_failure: Arc<dyn Fn(&E) -> bool + Send + Sync>,
}

/// Stable identity used by persistence persisted cache operations.
///
/// # Type parameters
/// - `K`: Key type used for identity or lookup.
struct CacheKey<K> {
    request: Arc<K>,
    identity: Arc<[u8]>,
}

#[derive(Debug)]
/// A schema, policy, or store failure while using a persisted cache.
///
/// # Type parameters
/// - `E`: Domain-error type.
/// - `Backend`: Backend-specific error or provider type.
/// - `Clock`: Clock provider type.
pub enum PersistedCacheError<E, Backend, Clock> {
    /// The application’s expected domain operation failed. Retains the `E` detail.
    Domain(E),
    /// The stable identity is invalid or incompatible. Retains the `KeyError` detail.
    Identity(KeyError),
    /// Encoding the typed value failed. Retains the `SchemaIssue` detail.
    Encode(SchemaIssue),
    /// Decoding the stored or transported value failed. Retains the `SchemaIssue` detail.
    Decode(SchemaIssue),
    /// The recorded version differs from the declared version.
    VersionMismatch,
    /// The stored outcome is not supported.
    UnsupportedStoredOutcome,
    /// The selected time conversion or clock operation failed. Retains the `TimeError` detail.
    Time(TimeError),
    /// The selected clock could not schedule or observe the requested time. Retains the `Clock` detail.
    Clock(Clock),
    /// Coordinating accepted work through the provider failed. Retains the `SemaphoreError` detail.
    Coordination(SemaphoreError),
    /// Reading the selected source failed. Retains the `Backend` detail.
    Read(Backend),
    /// The provider may have committed before its acknowledgement was lost.
    UncertainWrite {
        /// Key used to address the stored value. Type: `Arc<[u8]>`.
        key: Arc<[u8]>,
        /// The underlying failure or provider cause. Type: `Backend`.
        source: Backend,
    },
    /// The provider may have removed the record before acknowledgement was lost.
    UncertainRemoval {
        /// Key used to address the stored value. Type: `Arc<[u8]>`.
        key: Arc<[u8]>,
        /// The underlying failure or provider cause. Type: `Backend`.
        source: Backend,
    },
    /// An owned input was already consumed.
    InputConsumed,
}

/// A per-environment cache. Construct one instance for each environment whose
/// loader or service projections differ; the memory cache captures the creating
/// environment as its scoped load context.
///
/// # Type parameters
/// - `K`: Key type used for identity or lookup.
/// - `A`: Application value type.
/// - `E`: Domain-error type.
/// - `Backend`: Backend-specific error or provider type.
/// - `ClockError`: Error returned by the clock capability.
/// - `D`: Identity-domain type.
pub struct PersistedCache<K, A, E, Backend, ClockError, D>
where
    K: Send + Sync + 'static,
    A: Send + 'static,
    E: Send + 'static,
    Backend: Send + 'static,
    ClockError: Send + 'static,
    D: KeyDomain,
{
    memory: Cache<CacheKey<K>, A, PersistedCacheError<E, Backend, ClockError>>,
    definition: Persistable<K, A, E, D>,
    gate: CacheGate,
    _errors: PhantomData<fn() -> (Backend, ClockError)>,
}

impl<K, A, E, Backend, ClockError, D> Clone for PersistedCache<K, A, E, Backend, ClockError, D>
where
    K: Send + Sync + 'static,
    A: Send + 'static,
    E: Send + 'static,
    Backend: Send + 'static,
    ClockError: Send + 'static,
    D: KeyDomain,
{
    ///
    /// # Returns
    /// Type: `Self`.
    fn clone(&self) -> Self {
        Self {
            memory: self.memory.clone(),
            definition: self.definition.clone(),
            gate: self.gate.clone(),
            _errors: PhantomData,
        }
    }
}

impl<K, A, E, Backend, ClockError, D> PersistedCache<K, A, E, Backend, ClockError, D>
where
    K: Send + Sync + 'static,
    A: Send + 'static,
    E: Send + 'static,
    Backend: Send + 'static,
    ClockError: Send + 'static,
    D: KeyDomain,
{
    /// Create a `PersistedCache` through the declared Effect and environment capabilities.
    ///
    /// # Type parameters
    /// - `Backend`: Backend-specific error or provider type.
    /// - `ClockError`: Error returned by the clock capability.
    /// - `R`: Environment type. Additional bound: `HasClock + HasTaskSpawner + HasKeyValueStore + HasWallClock + Send + Sync + 'static`. Additional bound: `:Store: KeyValueStore<Error = Backend>`.
    /// - `A`: Success or application-value type.
    /// - `D`: Stable-key domain type.
    /// - `E`: Domain-error type.
    /// - `K`: Type supplied by the enclosing impl or trait.
    ///
    /// # Parameters
    /// - `scope` (`Scope<SharedValue<PersistedCacheError<E, Backend, ClockError>>>`): Owner that joins child work and releases resources at close.
    /// - `memory` (`PersistedCacheMemory<PersistedCacheError<E, Backend, ClockError>>`): The memory supplied to this operation.
    /// - `definition` (`Persistable<K, A, E, D>`): The typed definition used by this operation.
    /// - `policy` (`PersistedCachePolicy`): The caller-supplied behavior policy.
    /// - `load` (`impl Fn(&K) -> Effect<A, E, R> + Send + Sync + 'static`): The load supplied to this operation.
    ///
    /// # Returns
    /// Type: `Effect<Self, SharedValue<PersistedCacheError<E, Backend, ClockError>>, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    pub fn make<R>(
        scope: Scope<SharedValue<PersistedCacheError<E, Backend, ClockError>>>,
        memory: PersistedCacheMemory<PersistedCacheError<E, Backend, ClockError>>,
        definition: Persistable<K, A, E, D>,
        policy: PersistedCachePolicy,
        load: impl Fn(&K) -> Effect<A, E, R> + Send + Sync + 'static,
    ) -> Effect<Self, SharedValue<PersistedCacheError<E, Backend, ClockError>>, R>
    where
        R: HasClock + HasTaskSpawner + HasKeyValueStore + HasWallClock + Send + Sync + 'static,
        R::Store: KeyValueStore<Error = Backend>,
        <R as HasWallClock>::Clock: WallClock<Error = ClockError>,
    {
        let loader = Arc::new(load);
        let memory = CacheConfig {
            equal: Arc::new(|left: &CacheKey<K>, right: &CacheKey<K>| {
                left.identity == right.identity
            }),
            ttl: memory.ttl,
            capacity: memory.capacity,
            cache_failure: memory.cache_failure,
        };
        let load_definition = definition.clone();
        Cache::make(scope, memory, move |request: &CacheKey<K>| {
            let key = Arc::clone(&request.identity);
            let definition = load_definition.clone();
            let loader = Arc::clone(&loader);
            let application = loader(&request.request);
            load_persisted(application, key, definition, policy)
        })
        .map(move |memory| Self {
            memory,
            definition: definition.clone(),
            gate: CacheGate::new(),
            _errors: PhantomData,
        })
    }

    /// Read the current value through the declared storage or state capability.
    ///
    /// # Type parameters
    /// - `Backend`: Backend-specific error or provider type.
    /// - `ClockError`: Error returned by the clock capability.
    /// - `D`: Identity-domain type.
    /// - `R`: Environment type. Bounds: `Send + Sync + 'static`.
    /// - `A`: Success or application-value type.
    /// - `E`: Domain-error type.
    /// - `K`: Type supplied by the enclosing impl or trait.
    ///
    /// # Parameters
    /// - `key` (`K`): Stable key used to locate or identify the selected value.
    ///
    /// # Returns
    /// Type: `Effect<SharedValue<A>, SharedValue<PersistedCacheError<E, Backend, ClockError>>, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    pub fn get<R: Send + Sync + 'static>(
        &self,
        key: K,
    ) -> Effect<SharedValue<A>, SharedValue<PersistedCacheError<E, Backend, ClockError>>, R> {
        let key = Arc::new(key);
        let identity_key = Arc::clone(&key);
        let definition = self.definition.clone();
        let memory = self.memory.clone();
        let gate = self.gate.clone();
        Effect::try_sync(
            move || definition.key(&identity_key).map(|key| key.into_bytes()),
            |error| SharedValue::new(PersistedCacheError::Identity(error)),
        )
        .flat_map(move |identity| {
            let memory = memory.clone();
            let key = Arc::clone(&key);
            let identity_for_memory = Arc::clone(&identity);
            ResourceBinding::new(
                gate.acquire(identity)
                    .map_error(|error| SharedValue::new(PersistedCacheError::Coordination(error))),
                |permit, _| Release::sync(move || drop(permit)),
            )
            .use_with(move |_| {
                memory.get_shared(Arc::new(CacheKey {
                    request: Arc::clone(&key),
                    identity: Arc::clone(&identity_for_memory),
                }))
            })
        })
    }

    /// Invalidate this cache instance before removing the backing record.
    /// Lookups and invalidation for the same stable identity are serialized
    /// within this instance. Separate cache instances and processes still
    /// require coordination from their backing-store capability.
    ///
    /// # Type parameters
    /// - `A`: Application value type.
    /// - `Backend`: Backend-specific error or provider type.
    /// - `ClockError`: Error returned by the clock capability.
    /// - `D`: Identity-domain type.
    /// - `R`: Environment type. Additional bound: `HasKeyValueStore + Send + Sync + 'static`. Additional bound: `:Store: KeyValueStore<Error = Backend>`.
    /// - `E`: Domain-error type.
    /// - `K`: Type supplied by the enclosing impl or trait.
    ///
    /// # Parameters
    /// - `key` (`K`): Stable key used to locate or identify the selected value.
    ///
    /// # Returns
    /// Type: `Effect<(), SharedValue<PersistedCacheError<E, Backend, ClockError>>, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    pub fn invalidate<R>(
        &self,
        key: K,
    ) -> Effect<(), SharedValue<PersistedCacheError<E, Backend, ClockError>>, R>
    where
        R: HasKeyValueStore + Send + Sync + 'static,
        R::Store: KeyValueStore<Error = Backend>,
    {
        let key = Arc::new(key);
        let identity_key = Arc::clone(&key);
        let definition = self.definition.clone();
        let memory = self.memory.clone();
        let gate = self.gate.clone();
        Effect::try_sync(
            move || {
                definition
                    .key(&identity_key)
                    .map(|value| value.into_bytes())
            },
            |error| SharedValue::new(PersistedCacheError::Identity(error)),
        )
        .flat_map(move |identity| {
            let key = Arc::clone(&key);
            let memory = memory.clone();
            let removal_key = Arc::clone(&identity);
            let identity_for_memory = Arc::clone(&identity);
            ResourceBinding::new(
                gate.acquire(identity)
                    .map_error(|error| SharedValue::new(PersistedCacheError::Coordination(error))),
                |permit, _| Release::sync(move || drop(permit)),
            )
            .use_with(move |_| {
                let removal_key = Arc::clone(&removal_key);
                memory
                    .invalidate_shared(Arc::new(CacheKey {
                        request: Arc::clone(&key),
                        identity: Arc::clone(&identity_for_memory),
                    }))
                    .flat_map(move |_| {
                        let key = Arc::clone(&removal_key);
                        Effect::service(|env: &R| env.key_value_store()).flat_map(move |store| {
                            let error_key = Arc::clone(&key);
                            store
                                .remove(Arc::clone(&key))
                                .map(|_| ())
                                .map_error(move |source| {
                                    SharedValue::new(PersistedCacheError::UncertainRemoval {
                                        key: Arc::clone(&error_key),
                                        source,
                                    })
                                })
                                .provide_some(|_: &R| ())
                        })
                    })
            })
        })
    }

    /// Stop accepting new work and describe the settlement of owned work.
    ///
    /// # Type parameters
    /// - `K`: Key type used for identity or lookup.
    /// - `A`: Application value type.
    /// - `Backend`: Backend-specific error or provider type.
    /// - `ClockError`: Error returned by the clock capability.
    /// - `D`: Identity-domain type.
    /// - `R`: Environment type. Bounds: `Send + Sync + 'static`.
    /// - `E`: Domain-error type.
    ///
    /// # Returns
    /// Type: `Effect<crate::PoolEndStatus, SharedValue<PersistedCacheError<E, Backend, ClockError>>, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    pub fn shutdown<R: Send + Sync + 'static>(
        &self,
    ) -> Effect<crate::PoolEndStatus, SharedValue<PersistedCacheError<E, Backend, ClockError>>, R>
    {
        self.memory.shutdown()
    }
}

///
/// # Type parameters
/// - `K`: Key type used for identity or lookup. Additional bound: `Send + Sync + 'static`.
/// - `A`: Successful application-value type. Additional bound: `Send + 'static`.
/// - `E`: Expected application-error type. Additional bound: `Send + 'static`.
/// - `B`: Transformed success-value type. Additional bound: `Send + 'static`.
/// - `W`: Workflow or writer type. Additional bound: `Send + 'static`.
/// - `D`: Stable identity-domain type. Additional bound: `KeyDomain`.
/// - `R`: Environment type supplying required capabilities. Additional bound: `HasKeyValueStore + HasWallClock + Send + Sync + 'static`. Additional bound: `:Store: KeyValueStore<Error = B>`. Additional bound: `:Clock: WallClock<Error = W>`.
///
/// # Parameters
/// - `application` (`Effect<A, E, R>`): The application supplied to this operation.
/// - `key` (`Arc<[u8]>`): Stable key used to locate or identify the selected value.
/// - `definition` (`Persistable<K, A, E, D>`): The definition supplied to this operation.
/// - `policy` (`PersistedCachePolicy`): The caller-supplied behavior policy.
///
/// # Returns
/// Type: `Effect<A, PersistedCacheError<E, B, W>, R>`.
/// Returns a lazy `Effect` with the declared success, error, and environment types.
fn load_persisted<K, A, E, B, W, D, R>(
    application: Effect<A, E, R>,
    key: Arc<[u8]>,
    definition: Persistable<K, A, E, D>,
    policy: PersistedCachePolicy,
) -> Effect<A, PersistedCacheError<E, B, W>, R>
where
    K: Send + Sync + 'static,
    A: Send + 'static,
    E: Send + 'static,
    B: Send + 'static,
    W: Send + 'static,
    D: KeyDomain,
    R: HasKeyValueStore + HasWallClock + Send + Sync + 'static,
    R::Store: KeyValueStore<Error = B>,
    R::Clock: WallClock<Error = W>,
{
    Effect::wall_now()
        .map_error(PersistedCacheError::Clock)
        .flat_map(move |now| {
            let key_for_read = Arc::clone(&key);
            let key_for_result = Arc::clone(&key);
            let definition = definition.clone();
            let application = application.clone();
            Effect::service(|env: &R| env.key_value_store()).flat_map(move |store| {
                let key = Arc::clone(&key_for_result);
                let definition = definition.clone();
                let application = application.clone();
                store
                    .get(Arc::clone(&key_for_read), now)
                    .map_error(PersistedCacheError::Read)
                    .provide_some(|_: &R| ())
                    .flat_map(move |read| match read {
                        StoreRead::Missing | StoreRead::Expired(_) => load_and_store(
                            application.clone(),
                            Arc::clone(&key),
                            definition.clone(),
                            policy,
                        ),
                        StoreRead::Found(entry) => match definition.decode(&entry.value.bytes) {
                            Ok(StoredOutcome::Success(value)) => Effect::sync_once(move || value),
                            Ok(StoredOutcome::Failure(error)) => {
                                Effect::from_result_once(Err(PersistedCacheError::Domain(error)))
                            }
                            Ok(
                                StoredOutcome::Defect(_)
                                | StoredOutcome::Interrupted
                                | StoredOutcome::FinalizationFailed(_),
                            ) => handle_bad_record(
                                PersistedCacheError::UnsupportedStoredOutcome,
                                Arc::clone(&key),
                                application.clone(),
                                definition.clone(),
                                policy,
                            ),
                            Err(PersistedRecordError::Codec(issue)) => handle_bad_record(
                                PersistedCacheError::Decode(issue),
                                Arc::clone(&key),
                                application.clone(),
                                definition.clone(),
                                policy,
                            ),
                            Err(PersistedRecordError::VersionMismatch) => handle_bad_record(
                                PersistedCacheError::VersionMismatch,
                                Arc::clone(&key),
                                application.clone(),
                                definition.clone(),
                                policy,
                            ),
                        },
                    })
            })
        })
}

///
/// # Type parameters
/// - `K`: Key type used for identity or lookup. Additional bound: `Send + Sync + 'static`.
/// - `A`: Successful application-value type. Additional bound: `Send + 'static`.
/// - `E`: Expected application-error type. Additional bound: `Send + 'static`.
/// - `B`: Transformed success-value type. Additional bound: `Send + 'static`.
/// - `W`: Workflow or writer type. Additional bound: `Send + 'static`.
/// - `D`: Stable identity-domain type. Additional bound: `KeyDomain`.
/// - `R`: Environment type supplying required capabilities. Additional bound: `HasKeyValueStore + HasWallClock + Send + Sync + 'static`. Additional bound: `:Store: KeyValueStore<Error = B>`. Additional bound: `:Clock: WallClock<Error = W>`.
///
/// # Parameters
/// - `error` (`PersistedCacheError<E, B, W>`): Expected or provider error preserved in the returned outcome.
/// - `key` (`Arc<[u8]>`): Stable key used to locate or identify the selected value.
/// - `application` (`Effect<A, E, R>`): The application supplied to this operation.
/// - `definition` (`Persistable<K, A, E, D>`): The definition supplied to this operation.
/// - `policy` (`PersistedCachePolicy`): The caller-supplied behavior policy.
///
/// # Returns
/// Type: `Effect<A, PersistedCacheError<E, B, W>, R>`.
/// Returns a lazy `Effect` with the declared success, error, and environment types.
fn handle_bad_record<K, A, E, B, W, D, R>(
    error: PersistedCacheError<E, B, W>,
    key: Arc<[u8]>,
    application: Effect<A, E, R>,
    definition: Persistable<K, A, E, D>,
    policy: PersistedCachePolicy,
) -> Effect<A, PersistedCacheError<E, B, W>, R>
where
    K: Send + Sync + 'static,
    A: Send + 'static,
    E: Send + 'static,
    B: Send + 'static,
    W: Send + 'static,
    D: KeyDomain,
    R: HasKeyValueStore + HasWallClock + Send + Sync + 'static,
    R::Store: KeyValueStore<Error = B>,
    R::Clock: WallClock<Error = W>,
{
    match policy.corrupt_record {
        CorruptCacheRecord::Reject => Effect::from_result_once(Err(error)),
        CorruptCacheRecord::EvictAndReload => Effect::service(|env: &R| env.key_value_store())
            .flat_map(move |store| {
                let remove_key = Arc::clone(&key);
                let error_key = Arc::clone(&key);
                let load_key = Arc::clone(&key);
                let definition = definition.clone();
                let application = application.clone();
                store
                    .remove(remove_key)
                    .map_error(move |source| PersistedCacheError::UncertainRemoval {
                        key: Arc::clone(&error_key),
                        source,
                    })
                    .provide_some(|_: &R| ())
                    .flat_map(move |_| {
                        load_and_store(
                            application.clone(),
                            Arc::clone(&load_key),
                            definition.clone(),
                            policy,
                        )
                    })
            }),
    }
}

///
/// # Type parameters
/// - `K`: Key type used for identity or lookup. Additional bound: `Send + Sync + 'static`.
/// - `A`: Successful application-value type. Additional bound: `Send + 'static`.
/// - `E`: Expected application-error type. Additional bound: `Send + 'static`.
/// - `B`: Transformed success-value type. Additional bound: `Send + 'static`.
/// - `W`: Workflow or writer type. Additional bound: `Send + 'static`.
/// - `D`: Stable identity-domain type. Additional bound: `KeyDomain`.
/// - `R`: Environment type supplying required capabilities. Additional bound: `HasKeyValueStore + HasWallClock + Send + Sync + 'static`. Additional bound: `:Store: KeyValueStore<Error = B>`. Additional bound: `:Clock: WallClock<Error = W>`.
///
/// # Parameters
/// - `application` (`Effect<A, E, R>`): The application supplied to this operation.
/// - `key` (`Arc<[u8]>`): Stable key used to locate or identify the selected value.
/// - `definition` (`Persistable<K, A, E, D>`): The definition supplied to this operation.
/// - `policy` (`PersistedCachePolicy`): The caller-supplied behavior policy.
///
/// # Returns
/// Type: `Effect<A, PersistedCacheError<E, B, W>, R>`.
/// Returns a lazy `Effect` with the declared success, error, and environment types.
fn load_and_store<K, A, E, B, W, D, R>(
    application: Effect<A, E, R>,
    key: Arc<[u8]>,
    definition: Persistable<K, A, E, D>,
    policy: PersistedCachePolicy,
) -> Effect<A, PersistedCacheError<E, B, W>, R>
where
    K: Send + Sync + 'static,
    A: Send + 'static,
    E: Send + 'static,
    B: Send + 'static,
    W: Send + 'static,
    D: KeyDomain,
    R: HasKeyValueStore + HasWallClock + Send + Sync + 'static,
    R::Store: KeyValueStore<Error = B>,
    R::Clock: WallClock<Error = W>,
{
    application
        .either()
        .map_error(PersistedCacheError::Domain)
        .flat_map(move |outcome| {
            if outcome.is_err() && !policy.persist_failures {
                return Effect::from_result_once(outcome.map_err(PersistedCacheError::Domain));
            }
            let key = Arc::clone(&key);
            let definition = definition.clone();
            let pending = std::sync::Mutex::new(Some(outcome));
            Effect::wall_now()
                .map_error(PersistedCacheError::Clock)
                .flat_map(move |now| {
                    let outcome = pending
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .take();
                    let Some(outcome) = outcome else {
                        return Effect::from_result_once(Err(PersistedCacheError::InputConsumed));
                    };
                    let stored = match outcome {
                        Ok(value) => StoredOutcome::Success(value),
                        Err(error) => StoredOutcome::Failure(error),
                    };
                    let encoded = match definition.encode(&stored) {
                        Ok(bytes) => bytes,
                        Err(issue) => {
                            return Effect::from_result_once(Err(PersistedCacheError::Encode(
                                issue,
                            )));
                        }
                    };
                    let outcome = match stored {
                        StoredOutcome::Success(value) => Ok(value),
                        StoredOutcome::Failure(error) => Err(error),
                        StoredOutcome::Defect(_)
                        | StoredOutcome::Interrupted
                        | StoredOutcome::FinalizationFailed(_) => {
                            return Effect::from_result_once(Err(
                                PersistedCacheError::UnsupportedStoredOutcome,
                            ));
                        }
                    };
                    let expires_at = match policy
                        .persistent_ttl
                        .map(|ttl| now.checked_add(ttl))
                        .transpose()
                    {
                        Ok(value) => value,
                        Err(error) => {
                            return Effect::from_result_once(Err(PersistedCacheError::Time(error)));
                        }
                    };
                    let outcome = std::sync::Mutex::new(Some(outcome));
                    let key_for_store = Arc::clone(&key);
                    Effect::service(|env: &R| env.key_value_store()).flat_map(move |store| {
                        let value = StoredValue {
                            bytes: Arc::clone(&encoded),
                            expires_at,
                        };
                        let write_key = Arc::clone(&key_for_store);
                        let error_key = Arc::clone(&key_for_store);
                        let outcome = outcome
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner)
                            .take();
                        match outcome {
                            Some(outcome) => {
                                let completed = std::sync::Mutex::new(Some(outcome));
                                store
                                    .put(write_key, value)
                                    .map_error(move |source| PersistedCacheError::UncertainWrite {
                                        key: Arc::clone(&error_key),
                                        source,
                                    })
                                    .provide_some(|_: &R| ())
                                    .flat_map(move |_| {
                                        let result = completed
                                            .lock()
                                            .unwrap_or_else(std::sync::PoisonError::into_inner)
                                            .take();
                                        match result {
                                            Some(result) => Effect::from_result_once(
                                                result.map_err(PersistedCacheError::Domain),
                                            ),
                                            None => Effect::from_result_once(Err(
                                                PersistedCacheError::InputConsumed,
                                            )),
                                        }
                                    })
                            }
                            None => {
                                Effect::from_result_once(Err(PersistedCacheError::InputConsumed))
                            }
                        }
                    })
                })
        })
}
