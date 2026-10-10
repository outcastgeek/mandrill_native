//! Scoped origin-data caches using the existing core engines.
//!
//! Keys must include each tenant/security/culture/representation partition that
//! affects the value. They are typed application identities, never credentials or
//! arbitrary object stringification. This module owns no replacement cache map.

use crate::{WebError, admission::independent};
use effect::{
    AdmissionCapacity, AdmissionError, Cache, CacheAdmission, CacheCapacity, CacheConfig,
    CacheLookup, CacheObservation, CacheOptions, CacheTtl, DependencyCache, DependencyCacheError,
    DependencyCachePolicy, Effect, Exit, HasClock, HasTaskSpawner, ProviderAdmission, Scope,
    SharedValue,
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
}
/// Explicit active-loader admission using typed capacity failures.
#[must_use]
pub struct LoadAdmission<E> {
    /// Maximum independent producer count; shared waiters consume no extra slot.
    pub maximum: NonZeroUsize,
    /// Application rejection factory; closed/full/overflow are never loader absence.
    pub on_full: Arc<dyn Fn(AdmissionError) -> E + Send + Sync>,
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
        let loader = Arc::new(loader);
        match retain.mode {
            Retention::Disabled => {
                let loader: Arc<dyn Fn(Arc<K>) -> Effect<A, E, R> + Send + Sync> = loader;
                Effect::sync(move || {
                    ProviderAdmission::new(AdmissionCapacity::Bounded(admission.maximum))
                })
                .flat_map(move |owner| {
                    let release_owner = owner.clone();
                    let loader = loader.clone();
                    let on_full = admission.on_full.clone();
                    scope
                        .add_finalizer(Effect::from_exit_future_with_context(move |_, _| {
                            let drain = release_owner.stop();
                            async move {
                                drain.wait().await;
                                Exit::Success(())
                            }
                        }))
                        .map(move |()| Self {
                            origin: Arc::new(Origin::Disabled {
                                owner: owner.clone(),
                                load: loader.clone(),
                                on_full: on_full.clone(),
                            }),
                        })
                })
            }
            Retention::Coalescing | Retention::Bounded(_) => {
                let capacity = match retain.mode {
                    Retention::Bounded(maximum) => CacheCapacity::Bounded(maximum),
                    Retention::Coalescing | Retention::Disabled => CacheCapacity::NoRetention,
                };
                let on_full = admission.on_full.clone();
                Cache::make_with_shared_key(
                    scope,
                    CacheConfig {
                        equal: Arc::new(keys),
                        ttl: retain.ttl,
                        capacity,
                        cache_failure: Arc::new(cache_failures),
                    },
                    CacheOptions {
                        ttl: CacheTtl::Fixed(retain.ttl),
                        admission: CacheAdmission::Bounded {
                            maximum: admission.maximum,
                            capacity_error: Arc::new(move || on_full(AdmissionError::Full)),
                        },
                    },
                    move |key| loader(key),
                )
                .map(|cache| Self {
                    origin: Arc::new(Origin::Cached(cache)),
                })
            }
        }
    }
    /// Get/load one key and atomically classify its selected acquisition path.
    pub fn get_observed(&self, key: K) -> Effect<CacheLookup<A>, SharedValue<E>, R> {
        match self.origin.as_ref() {
            Origin::Cached(cache) => cache.get_observed(key),
            Origin::Disabled {
                owner,
                load,
                on_full,
            } => {
                let key = Arc::new(key);
                let load = load.clone();
                let on_full = on_full.clone();
                independent(
                    owner.clone(),
                    move || load(key.clone()),
                    move |error| on_full(error),
                )
                .map_error(SharedValue::new)
                .map(|value| CacheLookup {
                    value: SharedValue::new(value),
                    observation: CacheObservation::Loaded,
                })
            }
        }
    }
    /// Return the shared value without copying its application payload.
    pub fn get(&self, key: K) -> Effect<SharedValue<A>, SharedValue<E>, R> {
        self.get_observed(key).map(|lookup| lookup.value)
    }
    /// Project a borrowed cache payload while the core shared owner is retained.
    /// References cannot escape the callback; poisoning remains an Effect defect.
    pub fn project<B: Send + 'static>(
        &self,
        key: K,
        inspect: impl Fn(&A) -> B + Send + Sync + 'static,
    ) -> Effect<B, SharedValue<E>, R> {
        self.get(key)
            .flat_map(move |value| match value.with(&inspect) {
                Ok(value) => Effect::from_result_once(Ok(value)),
                Err(defect) => Effect::die(defect),
            })
    }
    /// Read a retained success only. Disabled/coalescing policies have no settled value.
    pub fn try_get(&self, key: K) -> Effect<Option<SharedValue<A>>, SharedValue<E>, R> {
        match self.origin.as_ref() {
            Origin::Cached(cache) => cache.try_get(key),
            Origin::Disabled { .. } => Effect::succeed(None),
        }
    }
    /// Fence and join any earlier core producer for this key. Disabled calls are
    /// independent and have no retained association to invalidate.
    pub fn invalidate(&self, key: K) -> Effect<(), SharedValue<E>, R> {
        match self.origin.as_ref() {
            Origin::Cached(cache) => cache.invalidate(key),
            Origin::Disabled { .. } => Effect::succeed(()),
        }
    }
    /// Replace through the same core owner; disabled mode deliberately retains nothing.
    pub fn set(&self, key: K, value: A) -> Effect<(), SharedValue<E>, R> {
        match self.origin.as_ref() {
            Origin::Cached(cache) => cache.set(key, value),
            Origin::Disabled { .. } => Effect::from_result_once(Ok(value)).map(|_| ()),
        }
    }
    /// Fence/join all active core cache producers and remove retained entries.
    pub fn clear(&self) -> Effect<(), SharedValue<E>, R> {
        match self.origin.as_ref() {
            Origin::Cached(cache) => cache.clear(),
            Origin::Disabled { .. } => Effect::succeed(()),
        }
    }
    /// Close and join the selected core owner. No native task is detached on drop.
    pub fn shutdown(&self) -> Effect<(), SharedValue<E>, R> {
        match self.origin.as_ref() {
            Origin::Cached(cache) => cache.shutdown().map(|_| ()),
            Origin::Disabled { owner, .. } => {
                let owner = owner.clone();
                Effect::from_exit_future_with_context(move |_, _| {
                    let drain = owner.stop();
                    async move {
                        drain.wait().await;
                        Exit::Success(())
                    }
                })
            }
        }
    }
}
/// Shared loader description; each invocation retains its key owner and returns
/// owned value/dependency payloads through the caller's original E and R.
type DependencyLoader<K, D, A, E, R> =
    Arc<dyn Fn(Arc<K>) -> Effect<(A, Vec<D>), E, R> + Send + Sync>;

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
        let loader = Arc::new(loader);
        let admission_error = admission.on_full.clone();
        match retain.mode {
            Retention::Bounded(retention) => DependencyCache::make_with_shared_key(
                scope,
                DependencyCachePolicy {
                    retention,
                    maximum_active: admission.maximum,
                    maximum_dependencies,
                    key_equal: Arc::new(keys),
                    dependency_equal: Arc::new(dependencies),
                    cache_failure: Arc::new(cache_failures),
                    ttl: CacheTtl::Fixed(retain.ttl),
                },
                move |key| loader(key),
            )
            .map(move |cache| Self {
                origin: Arc::new(DependentOrigin::Core(cache)),
                admission_error: admission_error.clone(),
            }),
            Retention::Coalescing => DependencyCache::make_coalescing_with_shared_key(
                scope,
                effect::DependencyCacheCoalescingPolicy {
                    maximum_active: admission.maximum,
                    maximum_dependencies,
                    key_equal: Arc::new(keys),
                    dependency_equal: Arc::new(dependencies),
                },
                move |key| loader(key),
            )
            .map(move |cache| Self {
                origin: Arc::new(DependentOrigin::Core(cache)),
                admission_error: admission_error.clone(),
            }),
            Retention::Disabled => {
                let loader: DependencyLoader<K, D, A, E, R> = loader;
                Effect::sync(move || {
                    ProviderAdmission::new(AdmissionCapacity::Bounded(admission.maximum))
                })
                .flat_map(move |owner| {
                    let release_owner = owner.clone();
                    let loader = loader.clone();
                    let on_full = admission.on_full.clone();
                    let admission_error = admission_error.clone();
                    scope
                        .add_finalizer(Effect::from_exit_future_with_context(move |_, _| {
                            let drain = release_owner.stop();
                            async move {
                                drain.wait().await;
                                Exit::Success(())
                            }
                        }))
                        .map(move |()| Self {
                            origin: Arc::new(DependentOrigin::Disabled {
                                owner: owner.clone(),
                                load: loader.clone(),
                                on_full: on_full.clone(),
                                maximum_dependencies,
                            }),
                            admission_error: admission_error.clone(),
                        })
                })
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
        let (keys, dependencies, cache_failures) = callbacks;
        let select = Arc::new(select_dependencies);
        Self::scoped(
            scope,
            retain,
            admission,
            maximum_dependencies,
            (keys, dependencies, cache_failures),
            move |key| {
                let selection_key = key.clone();
                let select = select.clone();
                loader(key).map(move |value| {
                    let dependencies = select(&selection_key, &value);
                    (value, dependencies)
                })
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
                cache.get_observed(key).map_error(move |error| {
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
                })
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
                independent(
                    owner.clone(),
                    move || load(key.clone()),
                    move |error| on_full(error),
                )
                .map_error(DependencyCacheError::Load)
                .flat_map(move |(value, dependencies)| {
                    if dependencies.len() > maximum.get() {
                        Effect::from_result_once(Err(DependencyCacheError::DependencyCapacity {
                            maximum: maximum.get(),
                            observed: dependencies.len(),
                        }))
                    } else {
                        Effect::from_result_once(Ok(CacheLookup {
                            value: SharedValue::new(value),
                            observation: CacheObservation::Loaded,
                        }))
                    }
                })
                .map_error(SharedValue::new)
            }
        }
    }
    /// Get/load payload ownership without copying A.
    pub fn get(&self, key: K) -> Effect<SharedValue<A>, SharedValue<DependencyCacheError<E>>, R> {
        self.get_observed(key).map(|lookup| lookup.value)
    }
    /// Project a borrowed dependency-cache payload without copying A or retaining
    /// a temporary reference. The same typed core acquisition failures are preserved.
    pub fn project<B: Send + 'static>(
        &self,
        key: K,
        inspect: impl Fn(&A) -> B + Send + Sync + 'static,
    ) -> Effect<B, SharedValue<DependencyCacheError<E>>, R> {
        self.get(key)
            .flat_map(move |value| match value.with(&inspect) {
                Ok(value) => Effect::from_result_once(Ok(value)),
                Err(defect) => Effect::die(defect),
            })
    }
    /// Read only a settled retained success; disabled mode has none by contract.
    pub fn try_get(
        &self,
        key: K,
    ) -> Effect<Option<SharedValue<A>>, SharedValue<DependencyCacheError<E>>, R> {
        match self.origin.as_ref() {
            DependentOrigin::Core(cache) => cache.try_get(key),
            DependentOrigin::Disabled { .. } => Effect::succeed(None),
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
            DependentOrigin::Core(cache) => cache.set(key, value, dependencies),
            DependentOrigin::Disabled {
                maximum_dependencies,
                ..
            } => {
                let maximum = maximum_dependencies.get();
                Effect::from_result_once(Ok((value, dependencies))).flat_map(
                    move |(_, dependencies)| {
                        if dependencies.len() > maximum {
                            Effect::from_result_once(Err(SharedValue::new(
                                DependencyCacheError::DependencyCapacity {
                                    maximum,
                                    observed: dependencies.len(),
                                },
                            )))
                        } else {
                            Effect::succeed(())
                        }
                    },
                )
            }
        }
    }
    /// Invalidate one associated key through the actual core engine.
    pub fn invalidate(&self, key: K) -> Effect<(), SharedValue<DependencyCacheError<E>>, R> {
        match self.origin.as_ref() {
            DependentOrigin::Core(cache) => cache.invalidate(key),
            DependentOrigin::Disabled { .. } => Effect::succeed(()),
        }
    }
    /// Core dependency invalidation conservatively fences/cancels/joins earlier
    /// active loads with unknown identities. Disabled mode has no associations.
    pub fn invalidate_dependency(
        &self,
        dependency: D,
    ) -> Effect<(), SharedValue<DependencyCacheError<E>>, R> {
        match self.origin.as_ref() {
            DependentOrigin::Core(cache) => cache.invalidate_dependency(dependency),
            DependentOrigin::Disabled { .. } => Effect::succeed(()),
        }
    }
    /// Remove retained associations and fence/join earlier active core producers.
    pub fn clear(&self) -> Effect<(), SharedValue<DependencyCacheError<E>>, R> {
        match self.origin.as_ref() {
            DependentOrigin::Core(cache) => cache.clear(),
            DependentOrigin::Disabled { .. } => Effect::succeed(()),
        }
    }
    /// Close selected admission and join producer cleanup through the original owner.
    pub fn shutdown(&self) -> Effect<(), SharedValue<DependencyCacheError<E>>, R> {
        match self.origin.as_ref() {
            DependentOrigin::Core(cache) => cache.shutdown().map(|_| ()),
            DependentOrigin::Disabled { owner, .. } => {
                let owner = owner.clone();
                Effect::from_exit_future_with_context(move |_, _| {
                    let drain = owner.stop();
                    async move {
                        drain.wait().await;
                        Exit::Success(())
                    }
                })
            }
        }
    }
}
