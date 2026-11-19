//! Scoped origin-data caches using the existing core engines.
//!
//! Keys must include each tenant/security/culture/representation partition that
//! affects the value. They are typed application identities, never credentials or
//! arbitrary object stringification. This module owns no replacement cache map.

use crate::{WebError, admission::independent};
use crate::{
    effect_die, effect_fail, effect_finalizer, effect_future, effect_map, effect_map_error,
    effect_sync, effect_try_sync,
};
use effect::memory::{Bytes, MemoryError, PreparedMemory, RegionId};
use effect::{
    AdmissionCapacity, AdmissionError, Cache, CacheAdmission, CacheCapacity, CacheConfig, CacheKey,
    CacheLookup, CacheObservation, CacheOptions, CacheTtl, DependencyCache, DependencyCacheError,
    DependencyCachePolicy, Effect, Exit, HasClock, HasTaskSpawner, PreparedCacheCapacity,
    PreparedCacheRefusals, PreparedCacheStorage, PreparedDependencyCache, PreparedDependencyPolicy,
    PreparedDependencyPool, PreparedDependencyStorage, PreparedDependencyValue,
    PreparedDependencyWriter, ProviderAdmission, Scope, SharedValue, effect,
};
use std::num::NonZeroUsize;
use std::sync::Arc;
use std::time::Duration;

/// Settled value behavior, separate from the maximum number of active loaders.
#[derive(Clone, Copy)]
#[must_use]
pub enum Retention {
    /// Every lookup calls the loader independently under immediate core admission.
    Disabled,
    /// Equivalent active lookups share one loader; settled values are not retained.
    Coalescing,
    /// Retain at most the positive caller-selected number of settled entries.
    Bounded(NonZeroUsize),
}
/// Caller-selected optional retention duration; None means no time expiry.
#[derive(Clone, Copy)]
#[must_use]
pub struct Retain {
    /// Explicit value/active-sharing mode.
    mode: Retention,
    /// Elapsed retention measured with the selected environment clock.
    ttl: Option<Duration>,
}
impl Retain {
    /// Reject zero finite retention rather than treating it as an arbitrary quantum.
    pub fn new(mode: Retention, ttl: Option<Duration>) -> Result<Self, WebError> {
        if ttl.is_some_and(|duration| duration.is_zero()) {
            return Err(WebError::Configuration("cache retention duration".into()));
        }
        Ok(Self { mode, ttl })
    }

    /// Settled entry capacity used when budgeting the selected core index.
    #[must_use]
    pub const fn cache_capacity(self) -> CacheCapacity {
        match self.mode {
            Retention::Bounded(maximum) => CacheCapacity::Bounded(maximum),
            Retention::Coalescing | Retention::Disabled => CacheCapacity::NoRetention,
        }
    }
}
/// Explicit active-loader admission using typed capacity failures.
#[must_use]
pub struct LoadAdmission<E> {
    /// Maximum independent producer count; shared waiters consume no extra slot.
    pub maximum: NonZeroUsize,
    /// Application rejection factory; closed/full/overflow are never loader absence.
    pub on_full: Arc<dyn Fn(AdmissionError) -> E + Send + Sync>,
}
impl<E> LoadAdmission<E> {
    /// Select the active-loader bound and the callback for admission failures.
    pub fn new(
        maximum: NonZeroUsize,
        on_full: impl Fn(AdmissionError) -> E + Send + Sync + 'static,
    ) -> Self {
        Self {
            maximum,
            on_full: Arc::new(on_full),
        }
    }
}
impl<E> Clone for LoadAdmission<E> {
    /// Share only the immutable rejection callback and count bound.
    fn clone(&self) -> Self {
        Self {
            maximum: self.maximum,
            on_full: self.on_full.clone(),
        }
    }
}
/// Actual engine selection: either core cache ownership or core admission only.
enum Origin<
    K: Send + Sync + 'static,
    A: Send + 'static,
    E: Send + 'static,
    R: Send + Sync + 'static,
> {
    /// Reuse the existing shared-load/retained engine.
    Cached(Cache<K, A, E>),
    PreparedIndependent(Cache<K, A, E>),
    /// No cache state is kept; each call owns its loader and core permit.
    Disabled {
        /// Core producer admission and drain owner.
        owner: ProviderAdmission,
        /// Caller selected loader; R remains the runtime boundary.
        load: Arc<dyn Fn(Arc<K>) -> Effect<A, E, R> + Send + Sync>,
        /// Explicit typed capacity mapping.
        on_full: Arc<dyn Fn(AdmissionError) -> E + Send + Sync>,
    },
}
/// Scoped read-through handle. Payloads are shared by the core cache or freshly
/// owned by independent calls; no blanket K/A/E Clone requirement is added.
#[must_use]
pub struct ReadThrough<
    K: Send + Sync + 'static,
    A: Send + 'static,
    E: Send + 'static,
    R: Send + Sync + 'static,
> {
    /// One selected owner; handle clones preserve that identity.
    origin: Arc<Origin<K, A, E, R>>,
}
impl<K: Send + Sync + 'static, A: Send + 'static, E: Send + 'static, R: Send + Sync + 'static> Clone
    for ReadThrough<K, A, E, R>
{
    /// Share service ownership without duplicating keys, values or errors.
    fn clone(&self) -> Self {
        Self {
            origin: self.origin.clone(),
        }
    }
}
impl<
    K: Send + Sync + 'static,
    A: Send + 'static,
    E: Send + 'static,
    R: HasClock + HasTaskSpawner + Send + Sync + 'static,
> ReadThrough<K, A, E, R>
{
    /// Lazily acquire the selected core owner in `scope`. Inline loaders receive an
    /// Arc of the existing key so deferred binds cannot escape a temporary borrow.
    pub fn scoped(
        scope: Scope<SharedValue<E>>,
        keys: impl Fn(&K, &K) -> bool + Send + Sync + 'static,
        retain: Retain,
        admission: LoadAdmission<E>,
        cache_failures: impl Fn(&E) -> bool + Send + Sync + 'static,
        loader: impl Fn(Arc<K>) -> Effect<A, E, R> + Send + Sync + 'static,
    ) -> Effect<Self, SharedValue<E>, R> {
        match retain.mode {
            Retention::Disabled => {
                let loader: Arc<dyn Fn(Arc<K>) -> Effect<A, E, R> + Send + Sync> = Arc::new(loader);
                let maximum = admission.maximum;
                let on_full = admission.on_full;
                effect! {
                    let scope = scope.clone();
                    let loader = loader.clone();
                    let on_full = on_full.clone();
                    let owner = effect_sync! {
                        body { ProviderAdmission::new(AdmissionCapacity::Bounded(maximum)) },
                    }?;
                    let release_owner = owner.clone();
                    effect_finalizer! {
                        scope: scope,
                        program: effect_future! {
                            exit_with_context;
                            body: move |_, _| {
                                let drain = release_owner.stop();
                                async move {
                                    drain.wait().await;
                                    Exit::Success(())
                                }
                            },
                        },
                    }?;
                    Self {
                        origin: Arc::new(Origin::Disabled {
                            owner, load: loader, on_full,
                        }),
                    }
                }
            }
            Retention::Coalescing | Retention::Bounded(_) => {
                let capacity = match retain.mode {
                    Retention::Bounded(maximum) => CacheCapacity::Bounded(maximum),
                    Retention::Coalescing | Retention::Disabled => CacheCapacity::NoRetention,
                };
                let on_full = admission.on_full.clone();
                let cache = effect::cache! {
                    scope: scope,
                    config: CacheConfig {
                        equal: Arc::new(keys),
                        ttl: retain.ttl,
                        capacity,
                        cache_failure: Arc::new(cache_failures),
                    },
                    options: CacheOptions {
                        ttl: CacheTtl::Fixed(retain.ttl),
                        admission: CacheAdmission::Bounded {
                            maximum: admission.maximum,
                            capacity_error: Arc::new(move || on_full(AdmissionError::Full)),
                        },
                    },
                    load_shared: loader,
                };
                effect_map! {
                    program: cache,
                    map: |cache| Self { origin: Arc::new(Origin::Cached(cache)) },
                }
            }
        }
    }

    /// Acquire the selected retained, coalescing or independent cache using startup backing.
    pub fn scoped_prepared(
        scope: Scope<SharedValue<E>>,
        keys: impl Fn(&K, &K) -> bool + Send + Sync + 'static,
        retain: Retain,
        admission: LoadAdmission<E>,
        (memory, refusals): (PreparedCacheStorage<K, A, E>, PreparedCacheRefusals<E>),
        cache_failures: impl Fn(&E) -> bool + Send + Sync + 'static,
        loader: impl Fn(CacheKey<K>) -> Effect<A, E, R> + Send + Sync + 'static,
    ) -> Effect<Self, SharedValue<E>, R> {
        let independent = matches!(retain.mode, Retention::Disabled);
        let cache = effect::cache! {
            scope: scope,
            config: CacheConfig {
                equal: Arc::new(keys), ttl: retain.ttl,
                capacity: retain.cache_capacity(), cache_failure: Arc::new(cache_failures),
            },
            options: CacheOptions {
                ttl: CacheTtl::Fixed(retain.ttl),
                admission: CacheAdmission::Bounded {
                    maximum: admission.maximum,
                    capacity_error: Arc::new(move || (admission.on_full)(AdmissionError::Full)),
                },
            },
            memory: memory, refusals: refusals, independent: independent, load_shared: loader,
        };
        effect_map! {
            program: cache,
            map: move |cache| {
                let owner = if independent { Origin::PreparedIndependent(cache) } else { Origin::Cached(cache) };
                Self { origin: Arc::new(owner) }
            },
        }
    }
    /// Get/load one key and atomically classify its selected acquisition path.
    pub fn get_observed(&self, key: K) -> Effect<CacheLookup<A>, SharedValue<E>, R> {
        match self.origin.as_ref() {
            Origin::Cached(cache) | Origin::PreparedIndependent(cache) => effect::cache! { cache: cache, get_observed: key, },
            Origin::Disabled {
                owner,
                load,
                on_full,
            } => {
                let key = Arc::new(key);
                let load = load.clone();
                let on_full = on_full.clone();
                effect_map! {
                    program: effect_map_error! {
                    program: independent(
                    owner.clone(),
                    move || load(key.clone()),
                    move |error| on_full(error),
                ), map: SharedValue::new, }, map: |value| CacheLookup {
                    value: SharedValue::new(value),
                    observation: CacheObservation::Loaded,
                }, }
            }
        }
    }
    /// Return the shared value without copying its application payload.
    pub fn get(&self, key: K) -> Effect<SharedValue<A>, SharedValue<E>, R> {
        effect_map! { program: self.get_observed(key), map: |lookup| lookup.value, }
    }
    /// Project a borrowed cache payload while the core shared owner is retained.
    /// References cannot escape the callback; poisoning remains an Effect defect.
    pub fn project<B: Send + 'static>(
        &self,
        key: K,
        inspect: impl Fn(&A) -> B + Send + Sync + 'static,
    ) -> Effect<B, SharedValue<E>, R> {
        effect! {
        bind;
        program: self.get(key),
        body: move |value| match value.with(&inspect) {
                       Ok(value) => effect::effect_from_result! { once; result: Ok(value), },
                       Err(defect) => effect_die! { defect: defect, },
                   },
        }
    }
    /// Read a retained success only. Disabled/coalescing policies have no settled value.
    pub fn try_get(&self, key: K) -> Effect<Option<SharedValue<A>>, SharedValue<E>, R> {
        match self.origin.as_ref() {
            Origin::Cached(cache) => effect::cache! { cache: cache, try_get: key, },
            Origin::Disabled { .. } | Origin::PreparedIndependent(_) => {
                effect_sync! { body { None }, }
            }
        }
    }
    /// Fence and join any earlier core producer for this key. Disabled calls are
    /// independent and have no retained association to invalidate.
    pub fn invalidate(&self, key: K) -> Effect<(), SharedValue<E>, R> {
        match self.origin.as_ref() {
            Origin::Cached(cache) => effect::cache! { cache: cache, invalidate: key, },
            Origin::Disabled { .. } | Origin::PreparedIndependent(_) => effect_sync! { body { () }, },
        }
    }
    /// Replace through the same core owner; disabled mode deliberately retains nothing.
    pub fn set(&self, key: K, value: A) -> Effect<(), SharedValue<E>, R> {
        match self.origin.as_ref() {
            Origin::Cached(cache) => effect::cache! { cache: cache, set: key, value: value, },
            Origin::Disabled { .. } | Origin::PreparedIndependent(_) => {
                effect_map! { program: effect::effect_from_result! { once; result: Ok(value), }, map: |_| (), }
            }
        }
    }
    /// Fence/join all active core cache producers and remove retained entries.
    pub fn clear(&self) -> Effect<(), SharedValue<E>, R> {
        match self.origin.as_ref() {
            Origin::Cached(cache) => effect::cache! { cache: cache, clear: (), },
            Origin::Disabled { .. } | Origin::PreparedIndependent(_) => effect_sync! { body { () }, },
        }
    }
    /// Close and join the selected core owner. No native task is detached on drop.
    pub fn shutdown(&self) -> Effect<(), SharedValue<E>, R> {
        match self.origin.as_ref() {
            Origin::Cached(cache) | Origin::PreparedIndependent(cache) => effect_map! { program: effect::cache! { cache: cache, shutdown: (), }, map: |_| (), },
            Origin::Disabled { owner, .. } => {
                let owner = owner.clone();
                effect_future! {
                    exit_with_context; body: move |_, _| {
                    let drain = owner.stop();
                    async move {
                        drain.wait().await;
                        Exit::Success(())
                    }
                }, }
            }
        }
    }
}
/// Shared loader description; each invocation retains its key owner and returns
/// owned value/dependency payloads through the caller's original E and R.
type DependencyLoader<K, D, A, E, R> =
    Arc<dyn Fn(Arc<K>) -> Effect<(A, Vec<D>), E, R> + Send + Sync>;

/// Selected region backing for a dependency read-through and its finite writers.
pub struct PreparedDependentReadThroughStorage<
    K: Send + Sync + 'static,
    D: Send + Sync + 'static,
    A: Send + 'static,
    E: Send + 'static,
> {
    cache: PreparedDependencyStorage<K, D, A, E>,
    writers: Arc<PreparedDependencyPool<D>>,
    maximum_dependencies: NonZeroUsize,
}

impl<K: Send + Sync + 'static, D: Send + Sync + 'static, A: Send + 'static, E: Send + 'static>
    PreparedDependentReadThroughStorage<K, D, A, E>
{
    /// Region bytes for cache records, values, and retained dependency identities.
    pub fn storage_bytes(
        cache: PreparedCacheCapacity,
        values: NonZeroUsize,
        writers: NonZeroUsize,
        maximum_dependencies: NonZeroUsize,
    ) -> Result<Bytes, MemoryError> {
        PreparedDependencyStorage::<K, D, A, E>::storage_bytes(cache, values)?.checked_add(
            PreparedDependencyPool::<D>::storage_bytes(writers.get(), maximum_dependencies.get())?,
        )
    }

    /// Reserve storage before Web startup begins accepting requests.
    pub fn prepare(
        memory: &PreparedMemory<'static>,
        region: RegionId,
        cache: PreparedCacheCapacity,
        values: NonZeroUsize,
        writers: NonZeroUsize,
        maximum_dependencies: NonZeroUsize,
    ) -> Result<Self, MemoryError> {
        let _ = Self::storage_bytes(cache, values, writers, maximum_dependencies)?;
        Ok(Self {
            cache: PreparedDependencyStorage::prepare(memory, region, cache, values)?,
            writers: Arc::new(
                PreparedDependencyPool::prepare(
                    memory,
                    region,
                    writers.get(),
                    maximum_dependencies.get(),
                )?
                .with_logical_width_errors(),
            ),
            maximum_dependencies,
        })
    }
}

/// Core dependency owner or deliberately independent disabled origin loads.
enum DependentOrigin<
    K: Send + Sync + 'static,
    D: Send + Sync + 'static,
    A: Send + 'static,
    E: Send + 'static,
    R: Send + Sync + 'static,
> {
    /// Existing cache/dependency index owns all coalescing/retained associations.
    Core(DependencyCache<K, D, A, E>),
    Prepared {
        cache: PreparedDependencyCache<K, D, A, E>,
        independent: bool,
        maximum_dependencies: NonZeroUsize,
        refusals: PreparedCacheRefusals<DependencyCacheError<E>>,
    },
    /// Independent producers retain no dependency metadata or settled values.
    Disabled {
        /// Core active-loader reservation/drain capability.
        owner: ProviderAdmission,
        /// Original selected loader; each invocation owns its returned identities.
        load: DependencyLoader<K, D, A, E, R>,
        /// Explicit typed admission failure callback.
        on_full: Arc<dyn Fn(AdmissionError) -> E + Send + Sync>,
        /// Positive dependency width checked even when associations are not retained.
        maximum_dependencies: NonZeroUsize,
    },
}
/// Dependency read-through delegates active-load fencing and retained metadata to
/// the core owner. Disabled mode is independent core admission, never a fake index.
#[must_use]
pub struct DependentReadThrough<
    K: Send + Sync + 'static,
    D: Send + Sync + 'static,
    A: Send + 'static,
    E: Send + 'static,
    R: Send + Sync + 'static,
> {
    /// Shared selected owner; clone does not copy any application payload.
    origin: Arc<DependentOrigin<K, D, A, E, R>>,
    /// Selected caller injection for core active-capacity decisions, never backend failures.
    admission_error: Arc<dyn Fn(AdmissionError) -> E + Send + Sync>,
}
impl<
    K: Send + Sync + 'static,
    D: Send + Sync + 'static,
    A: Send + 'static,
    E: Send + 'static,
    R: Send + Sync + 'static,
> Clone for DependentReadThrough<K, D, A, E, R>
{
    /// Share existing service ownership without acquiring new producer work.
    fn clone(&self) -> Self {
        Self {
            origin: self.origin.clone(),
            admission_error: self.admission_error.clone(),
        }
    }
}
impl<
    K: Send + Sync + 'static,
    D: Send + Sync + 'static,
    A: Send + 'static,
    E: Send + 'static,
    R: HasClock + HasTaskSpawner + Send + Sync + 'static,
> DependentReadThrough<K, D, A, E, R>
{
    /// Acquire the actual core retained/coalescing owner or explicit independent
    /// disabled mode. Loaders must include empty collection membership identities.
    /// `callbacks` contains key equality, dependency equality and failure retention,
    /// in that order; each callback retains its original type and ownership.
    pub fn scoped(
        scope: Scope<SharedValue<DependencyCacheError<E>>>,
        retain: Retain,
        admission: LoadAdmission<E>,
        maximum_dependencies: NonZeroUsize,
        callbacks: (
            impl Fn(&K, &K) -> bool + Send + Sync + 'static,
            impl Fn(&D, &D) -> bool + Send + Sync + 'static,
            impl Fn(&E) -> bool + Send + Sync + 'static,
        ),
        loader: impl Fn(Arc<K>) -> Effect<(A, Vec<D>), E, R> + Send + Sync + 'static,
    ) -> Effect<Self, SharedValue<DependencyCacheError<E>>, R> {
        let (keys, dependencies, cache_failures) = callbacks;
        let admission_error = admission.on_full.clone();
        match retain.mode {
            Retention::Bounded(retention) => effect_map! {
                program: effect::dependency_cache! {
                    scope: scope,
                    policy: DependencyCachePolicy {
                        retention,
                        maximum_active: admission.maximum,
                        maximum_dependencies,
                        key_equal: Arc::new(keys),
                        dependency_equal: Arc::new(dependencies),
                        cache_failure: Arc::new(cache_failures),
                        ttl: CacheTtl::Fixed(retain.ttl),
                    },
                    load_shared: loader,
                },
                map: move |cache| Self {
                    origin: Arc::new(DependentOrigin::Core(cache)),
                    admission_error: admission_error.clone(),
                },
            },
            Retention::Coalescing => effect_map! {
                program: effect::dependency_cache! {
                    scope: scope,
                    coalescing: effect::DependencyCacheCoalescingPolicy {
                        maximum_active: admission.maximum,
                        maximum_dependencies,
                        key_equal: Arc::new(keys),
                        dependency_equal: Arc::new(dependencies),
                    },
                    load_shared: loader,
                },
                map: move |cache| Self {
                    origin: Arc::new(DependentOrigin::Core(cache)),
                    admission_error: admission_error.clone(),
                },
            },
            Retention::Disabled => {
                let loader: DependencyLoader<K, D, A, E, R> = Arc::new(loader);
                let maximum = admission.maximum;
                let on_full = admission.on_full;
                effect! {
                    let scope = scope.clone();
                    let loader = loader.clone();
                    let on_full = on_full.clone();
                    let admission_error = admission_error.clone();
                    let owner = effect_sync! {
                        body { ProviderAdmission::new(AdmissionCapacity::Bounded(maximum)) },
                    }?;
                    let release_owner = owner.clone();
                    effect_finalizer! {
                        scope: scope,
                        program: effect_future! {
                            exit_with_context;
                            body: move |_, _| {
                                let drain = release_owner.stop();
                                async move {
                                    drain.wait().await;
                                    Exit::Success(())
                                }
                            },
                        },
                    }?;
                    Self {
                        origin: Arc::new(DependentOrigin::Disabled {
                            owner, load: loader, on_full, maximum_dependencies,
                        }),
                        admission_error,
                    }
                }
            }
        }
    }
    /// Select dependency membership from a loaded value while borrowing the original
    /// key/value owners. Empty result collections still require an explicit membership
    /// identity from the callback. This composes over the same core loader/index.
    /// `callbacks` contains key equality, dependency equality and failure retention,
    /// in the same order as [`Self::scoped`].
    pub fn scoped_by(
        scope: Scope<SharedValue<DependencyCacheError<E>>>,
        retain: Retain,
        admission: LoadAdmission<E>,
        maximum_dependencies: NonZeroUsize,
        callbacks: (
            impl Fn(&K, &K) -> bool + Send + Sync + 'static,
            impl Fn(&D, &D) -> bool + Send + Sync + 'static,
            impl Fn(&E) -> bool + Send + Sync + 'static,
        ),
        loader: impl Fn(Arc<K>) -> Effect<A, E, R> + Send + Sync + 'static,
        select_dependencies: impl Fn(&K, &A) -> Vec<D> + Send + Sync + 'static,
    ) -> Effect<Self, SharedValue<DependencyCacheError<E>>, R> {
        let select = Arc::new(select_dependencies);
        Self::scoped(
            scope,
            retain,
            admission,
            maximum_dependencies,
            callbacks,
            move |key| {
                let selection_key = key.clone();
                let select = select.clone();
                effect_map! {
                    program: loader(key),
                    map: move |value| {
                        let dependencies = select(&selection_key, &value);
                        (value, dependencies)
                    },
                }
            },
        )
    }

    /// Acquire the selected cache and pass its reserved dependency writer to each loader.
    /// The writer must be returned with the value so the exact written identities can be retained.
    pub fn scoped_prepared(
        scope: Scope<SharedValue<DependencyCacheError<E>>>,
        retain: Retain,
        admission: LoadAdmission<E>,
        maximum_dependencies: NonZeroUsize,
        callbacks: (
            impl Fn(&K, &K) -> bool + Send + Sync + 'static,
            impl Fn(&D, &D) -> bool + Send + Sync + 'static,
            impl Fn(&E) -> bool + Send + Sync + 'static,
        ),
        (memory, refusals): (
            PreparedDependentReadThroughStorage<K, D, A, E>,
            PreparedCacheRefusals<DependencyCacheError<E>>,
        ),
        loader: impl Fn(
            CacheKey<K>,
            PreparedDependencyWriter<D>,
        ) -> Effect<(A, PreparedDependencyWriter<D>), E, R>
        + Send
        + Sync
        + 'static,
    ) -> Effect<Self, SharedValue<DependencyCacheError<E>>, R> {
        let independent = matches!(retain.mode, Retention::Disabled);
        if memory.maximum_dependencies != maximum_dependencies {
            return effect_fail! { error: refusals.memory(MemoryError::UnsupportedRunContract), };
        }
        let (keys, dependencies, cache_failures) = callbacks;
        let admission_error = admission.on_full.clone();
        let policy = PreparedDependencyPolicy {
            config: CacheConfig {
                equal: Arc::new(keys),
                ttl: retain.ttl,
                capacity: retain.cache_capacity(),
                cache_failure: Arc::new(move |error| match error {
                    DependencyCacheError::Load(error) => cache_failures(error),
                    DependencyCacheError::DependencyCapacity { .. }
                    | DependencyCacheError::ActiveCapacity { .. }
                    | DependencyCacheError::Memory(_) => false,
                }),
            },
            options: CacheOptions {
                ttl: CacheTtl::Fixed(retain.ttl),
                admission: CacheAdmission::Bounded {
                    maximum: admission.maximum,
                    capacity_error: Arc::new(move || {
                        DependencyCacheError::Load((admission.on_full)(AdmissionError::Full))
                    }),
                },
            },
            dependency_equal: Arc::new(dependencies),
        };
        let cache = if independent {
            PreparedDependencyCache::make_prepared_independent(
                scope,
                policy,
                memory.cache,
                memory.writers,
                refusals.clone(),
                loader,
            )
        } else {
            PreparedDependencyCache::make_prepared_with_refusals(
                scope,
                policy,
                memory.cache,
                memory.writers,
                refusals.clone(),
                loader,
            )
        };
        effect_map! {
            program: cache,
            map: move |cache| Self {
                origin: Arc::new(DependentOrigin::Prepared {
                    cache, independent, maximum_dependencies, refusals: refusals.clone(),
                }),
                admission_error: admission_error.clone(),
            },
        }
    }

    /// Select dependencies from a borrowed successful value while retaining the
    /// resulting identities in the reserved writer. Selector-owned Vec backing
    /// remains with the callback until the identities move into the writer.
    pub fn scoped_prepared_by(
        scope: Scope<SharedValue<DependencyCacheError<E>>>,
        retain: Retain,
        admission: LoadAdmission<E>,
        maximum_dependencies: NonZeroUsize,
        callbacks: (
            impl Fn(&K, &K) -> bool + Send + Sync + 'static,
            impl Fn(&D, &D) -> bool + Send + Sync + 'static,
            impl Fn(&E) -> bool + Send + Sync + 'static,
        ),
        prepared: (
            PreparedDependentReadThroughStorage<K, D, A, E>,
            PreparedCacheRefusals<DependencyCacheError<E>>,
        ),
        (loader, select): (
            impl Fn(CacheKey<K>) -> Effect<A, E, R> + Send + Sync + 'static,
            impl Fn(&K, &A) -> Vec<D> + Send + Sync + 'static,
        ),
    ) -> Effect<Self, SharedValue<DependencyCacheError<E>>, R> {
        let select = Arc::new(select);
        Self::scoped_prepared(
            scope,
            retain,
            admission,
            maximum_dependencies,
            callbacks,
            prepared,
            move |key, writer| {
                let select = select.clone();
                let loaded = loader(key.clone());
                let writer = effect_sync! { once; body: move || writer, };
                effect! {
                    let select = select.clone();
                    let key = key.clone();
                    let loaded = loaded.clone();
                    let writer = writer.clone();
                    let mut writer = writer?;
                    let value = loaded?;
                    effect_sync! {
                        once;
                        body: move || {
                            for dependency in select(&key, &value) {
                                let _ = writer.push(dependency);
                            }
                            (value, writer)
                        },
                    }?
                }
            },
        )
    }
    /// Acquire a value and its atomic core Loaded/Shared/Retained classification.
    pub fn get_observed(
        &self,
        key: K,
    ) -> Effect<CacheLookup<A>, SharedValue<DependencyCacheError<E>>, R> {
        match self.origin.as_ref() {
            DependentOrigin::Core(cache) => {
                let injection = self.admission_error.clone();
                effect_map_error! {
                    program: effect::cache! { cache: cache, get_observed: key, }, map: move |error| {
                    if error
                        .with(|error| matches!(error, DependencyCacheError::ActiveCapacity { .. }))
                        .is_ok_and(|capacity| capacity)
                    {
                        SharedValue::new(DependencyCacheError::Load(injection(
                            AdmissionError::Full,
                        )))
                    } else {
                        error
                    }
                }, }
            }
            DependentOrigin::Prepared { cache, .. } => {
                effect! {
                    bind;
                    program: effect::cache! { cache: cache, get_observed: key, },
                    body: move |lookup: CacheLookup<PreparedDependencyValue<A, D>>| {
                        match lookup.value.with(|value| value.value.clone()) {
                            Ok(value) => effect_sync! { once; body: move || CacheLookup { value, observation: lookup.observation }, },
                            Err(defect) => effect_die! { defect: defect, },
                        }
                    },
                }
            }
            DependentOrigin::Disabled {
                owner,
                load,
                on_full,
                maximum_dependencies,
            } => {
                let key = Arc::new(key);
                let load = load.clone();
                let on_full = on_full.clone();
                let maximum = *maximum_dependencies;
                effect_map_error! {
                                   program: effect! {
                bind;
                program: effect_map_error! {
                    program: independent(
                                   owner.clone(),
                                   move || load(key.clone()),
                                   move |error| on_full(error),
                               ), map: DependencyCacheError::Load, },
                body: move |(value, dependencies)| {
                                   if dependencies.len() > maximum.get() {
                                       effect::effect_from_result! {
                                           once;
                                           result: Err(DependencyCacheError::DependencyCapacity {
                                                   maximum: maximum.get(),
                                                   observed: dependencies.len(),
                                               }),
                                       }
                                   } else {
                                       effect::effect_from_result! {
                                           once;
                                           result: Ok(CacheLookup {
                                                   value: SharedValue::new(value),
                                                   observation: CacheObservation::Loaded,
                                               }),
                                       }
                                   }
                               },
                }, map: SharedValue::new, }
            }
        }
    }
    /// Get/load payload ownership without copying A.
    pub fn get(&self, key: K) -> Effect<SharedValue<A>, SharedValue<DependencyCacheError<E>>, R> {
        effect_map! { program: self.get_observed(key), map: |lookup| lookup.value, }
    }
    /// Project a borrowed dependency-cache payload without copying A or retaining
    /// a temporary reference. The same typed core acquisition failures are preserved.
    pub fn project<B: Send + 'static>(
        &self,
        key: K,
        inspect: impl Fn(&A) -> B + Send + Sync + 'static,
    ) -> Effect<B, SharedValue<DependencyCacheError<E>>, R> {
        effect! {
        bind;
        program: self.get(key),
        body: move |value| match value.with(&inspect) {
                       Ok(value) => effect::effect_from_result! { once; result: Ok(value), },
                       Err(defect) => effect_die! { defect: defect, },
                   },
        }
    }
    /// Read only a settled retained success; disabled mode has none by contract.
    pub fn try_get(
        &self,
        key: K,
    ) -> Effect<Option<SharedValue<A>>, SharedValue<DependencyCacheError<E>>, R> {
        match self.origin.as_ref() {
            DependentOrigin::Core(cache) => effect::cache! { cache: cache, try_get: key, },
            DependentOrigin::Prepared { cache, independent: false, .. } => {
                effect! {
                    bind;
                    program: effect::cache! { cache: cache, try_get: key, },
                    body: move |result: Option<SharedValue<PreparedDependencyValue<A, D>>>| {
                        match result.map(|value| value.with(|value| value.value.clone())).transpose() {
                            Ok(value) => effect_sync! { once; body: move || value, },
                            Err(defect) => effect_die! { defect: defect, },
                        }
                    },
                }
            }
            DependentOrigin::Disabled { .. } | DependentOrigin::Prepared { independent: true, .. } => {
                effect_sync! { body { None }, }
            }
        }
    }
    /// Fence and join earlier associated core loads before owned replacement.
    pub fn set(
        &self,
        key: K,
        value: A,
        dependencies: Vec<D>,
    ) -> Effect<(), SharedValue<DependencyCacheError<E>>, R> {
        match self.origin.as_ref() {
            DependentOrigin::Core(cache) => effect::cache! { cache: cache, set: key, value: value, dependencies: dependencies, },
            DependentOrigin::Prepared { cache, independent, maximum_dependencies, refusals } => {
                let independent = *independent;
                let maximum = maximum_dependencies.get();
                let cache = cache.clone();
                let refusals = refusals.clone();
                effect! {
                    bind;
                    program: effect_sync! { once; body: move || (key, value, dependencies), },
                    body: move |(key, value, dependencies): (K, A, Vec<D>)| {
                        let writer = if dependencies.len() > maximum {
                            Err(cache.dependency_capacity_error(maximum, dependencies.len()))
                        } else if independent {
                            return effect_sync! { body { () }, };
                        } else {
                            cache.try_writer().and_then(|mut writer| {
                                for dependency in dependencies {
                                    writer.push(dependency)?;
                                }
                                Ok(writer)
                            }).map_err(|error| refusals.memory(error))
                        };
                        match writer {
                            Ok(writer) => effect::cache! { cache: cache, set: key, value: value, dependencies: writer, },
                            Err(error) => effect_try_sync! { once; body: move || Err(error), },
                        }
                    },
                }
            }
            DependentOrigin::Disabled {
                maximum_dependencies,
                ..
            } => {
                let maximum = maximum_dependencies.get();
                effect! {
                bind;
                program: effect::effect_from_result! { once; result: Ok((value, dependencies)), },
                body: move |(_, dependencies)| {
                                       if dependencies.len() > maximum {
                                           effect::effect_from_result! {
                                               once;
                                               result: Err(SharedValue::new(
                                                       DependencyCacheError::DependencyCapacity {
                                                           maximum,
                                                           observed: dependencies.len(),
                                                       },
                                                   )),
                                           }
                                       } else {
                                           effect_sync! { body { () }, }
                                       }
                                   },
                }
            }
        }
    }
    /// Invalidate one associated key through the actual core engine.
    pub fn invalidate(&self, key: K) -> Effect<(), SharedValue<DependencyCacheError<E>>, R> {
        match self.origin.as_ref() {
            DependentOrigin::Core(cache) => effect::cache! { cache: cache, invalidate: key, },
            DependentOrigin::Prepared { cache, independent: false, .. } => effect::cache! { cache: cache, invalidate: key, },
            DependentOrigin::Disabled { .. } | DependentOrigin::Prepared { independent: true, .. } => effect_sync! { body { () }, },
        }
    }
    /// Core dependency invalidation conservatively fences/cancels/joins earlier
    /// active loads with unknown identities. Disabled mode has no associations.
    pub fn invalidate_dependency(
        &self,
        dependency: D,
    ) -> Effect<(), SharedValue<DependencyCacheError<E>>, R> {
        match self.origin.as_ref() {
            DependentOrigin::Core(cache) => effect::cache! { cache: cache, invalidate_dependency: dependency, },
            DependentOrigin::Prepared { cache, independent: false, .. } => effect::cache! { cache: cache, invalidate_dependency: dependency, },
            DependentOrigin::Disabled { .. } | DependentOrigin::Prepared { independent: true, .. } => effect_sync! { body { () }, },
        }
    }
    /// Remove retained associations and fence/join earlier active core producers.
    pub fn clear(&self) -> Effect<(), SharedValue<DependencyCacheError<E>>, R> {
        match self.origin.as_ref() {
            DependentOrigin::Core(cache) => effect::cache! { cache: cache, clear: (), },
            DependentOrigin::Prepared { cache, independent: false, .. } => effect::cache! { cache: cache, clear: (), },
            DependentOrigin::Disabled { .. } | DependentOrigin::Prepared { independent: true, .. } => effect_sync! { body { () }, },
        }
    }
    /// Close selected admission and join producer cleanup through the original owner.
    pub fn shutdown(&self) -> Effect<(), SharedValue<DependencyCacheError<E>>, R> {
        match self.origin.as_ref() {
            DependentOrigin::Core(cache) => effect_map! { program: effect::cache! { cache: cache, shutdown: (), }, map: |_| (), },
            DependentOrigin::Prepared { cache, .. } => effect_map! { program: effect::cache! { cache: cache, shutdown: (), }, map: |_| (), },
            DependentOrigin::Disabled { owner, .. } => {
                let owner = owner.clone();
                effect_future! {
                    exit_with_context; body: move |_, _| {
                    let drain = owner.stop();
                    async move {
                        drain.wait().await;
                        Exit::Success(())
                    }
                }, }
            }
        }
    }
}
