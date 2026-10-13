//! Dependency indexing over the existing scoped cache engine. Active entries
//! with unknown dependencies and retained failures are conservatively fenced.

use crate::{
    Cache, CacheAdmission, CacheCapacity, CacheConfig, CacheLookup, CacheOptions, CacheTtl, Effect,
    HasClock, HasTaskSpawner, Scope, SharedValue,
};
use crate::{cache, effect, effect_die, effect_map, effect_map_error, effect_try_sync};
use std::num::NonZeroUsize;
use std::sync::Arc;
type Equal<A> = Arc<dyn Fn(&A, &A) -> bool + Send + Sync>;
/// Loader failures and explicit admission or dependency-width rejections.
#[derive(Debug, PartialEq, Eq)]
pub enum DependencyCacheError<E> {
    /// The application loader failed with its original typed error.
    Load(E),
    /// A successful load or replacement supplied too many dependency identities.
    DependencyCapacity {
        /// Caller-supplied dependency count limit.
        maximum: usize,
        /// Number supplied by the rejected result.
        observed: usize,
    },
    /// Admission rejected a new load before dispatch.
    ActiveCapacity {
        /// Caller-supplied maximum number of active loads.
        maximum: usize,
    },
}
/// A shared loaded payload and the dependency identities used to invalidate it.
pub struct DependencyValue<A: Send + 'static, D> {
    /// Shared ownership of the payload; copying this handle does not clone `A`.
    pub value: SharedValue<A>,
    /// Owned immutable dependency identities supplied with this successful load.
    pub dependencies: Arc<[D]>,
}
/// Explicit retention, admission, dependency equality and outcome-expiry policies.
///
/// No runtime or capacity is inferred. Equality callbacks must be stable for the
/// lifetime of cached keys and dependencies; policies run outside the cache gate.
pub struct DependencyCachePolicy<K, D, A: Send + 'static, E> {
    /// Maximum settled retained entries; active producers are bounded separately.
    pub retention: NonZeroUsize,
    /// Maximum independent active loads admitted before provider dispatch.
    pub maximum_active: NonZeroUsize,
    /// Maximum number of dependency identities returned by a load or supplied to set.
    pub maximum_dependencies: NonZeroUsize,
    /// Application equality used to coalesce and locate cache keys.
    pub key_equal: Equal<K>,
    /// Application equality used by dependency invalidation.
    pub dependency_equal: Equal<D>,
    /// Whether a loader error may be retained; capacity errors are never retained.
    pub cache_failure: Arc<dyn Fn(&E) -> bool + Send + Sync>,
    /// Optional or outcome-dependent retention lifetime, measured using the environment clock.
    pub ttl: CacheTtl<DependencyValue<A, D>, DependencyCacheError<E>>,
}
/// Explicit admission and dependency policies for active-load coalescing only.
///
/// Completed successes and failures are never retained. No retention capacity,
/// TTL approximation or failure-caching setting is needed. Equality callbacks
/// remain stable while keys/dependencies belong to active loads.
pub struct DependencyCacheCoalescingPolicy<K, D> {
    /// Maximum independent active producers; joining an existing producer uses no slot.
    pub maximum_active: NonZeroUsize,
    /// Maximum dependency identities accepted from a load or explicit replacement.
    pub maximum_dependencies: NonZeroUsize,
    /// Stable application equality used to join and locate active keys.
    pub key_equal: Equal<K>,
    /// Stable application equality used to fence dependency changes.
    pub dependency_equal: Equal<D>,
}
/// A scoped [`Cache`] with dependency-driven invalidation and owned payloads.
///
/// Clones share one engine. Active loads whose dependencies are not yet known, and
/// retained failures, are conservatively invalidated by any dependency change.
/// Runtime providers are supplied by `R` when constructing the cache.
pub struct DependencyCache<
    K: Send + Sync + 'static,
    D: Send + Sync + 'static,
    A: Send + 'static,
    E: Send + 'static,
> {
    cache: Cache<K, DependencyValue<A, D>, DependencyCacheError<E>>,
    equal: Equal<D>,
    maximum_dependencies: NonZeroUsize,
}
impl<K: Send + Sync + 'static, D: Send + Sync + 'static, A: Send + 'static, E: Send + 'static> Clone
    for DependencyCache<K, D, A, E>
{
    fn clone(&self) -> Self {
        Self {
            cache: self.cache.clone(),
            equal: Arc::clone(&self.equal),
            maximum_dependencies: self.maximum_dependencies,
        }
    }
}
impl<K: Send + Sync + 'static, D: Send + Sync + 'static, A: Send + 'static, E: Send + 'static>
    DependencyCache<K, D, A, E>
{
    /// Lazily create the cache and register shutdown with `scope`.
    ///
    /// `load` returns an owned payload and its dependency identities. Admission is
    /// checked before dispatch; oversized dependency lists fail without retention.
    /// `R` supplies the clock and task spawner. Scope closure cancels and joins owned
    /// producer work; failures remain in the shared typed error channel.
    pub fn make<R: HasClock + HasTaskSpawner + Send + Sync + 'static>(
        scope: Scope<SharedValue<DependencyCacheError<E>>>,
        policy: DependencyCachePolicy<K, D, A, E>,
        load: impl Fn(&K) -> Effect<(A, Vec<D>), E, R> + Send + Sync + 'static,
    ) -> Effect<Self, SharedValue<DependencyCacheError<E>>, R> {
        Self::make_with_shared_key(scope, policy, move |key| load(&key))
    }

    /// Create the existing dependency cache with a shared owned origin key for deferred
    /// loader bodies. The same policy, dependency bound and active-load ownership apply;
    /// the key's Arc handle is shared instead of copying its payload or borrowing a temporary.
    pub fn make_with_shared_key<R: HasClock + HasTaskSpawner + Send + Sync + 'static>(
        scope: Scope<SharedValue<DependencyCacheError<E>>>,
        policy: DependencyCachePolicy<K, D, A, E>,
        load: impl Fn(Arc<K>) -> Effect<(A, Vec<D>), E, R> + Send + Sync + 'static,
    ) -> Effect<Self, SharedValue<DependencyCacheError<E>>, R> {
        let maximum_active = policy.maximum_active;
        let failure = Arc::clone(&policy.cache_failure);
        Self::make_configured_with_shared_key(
            scope,
            CacheConfig {
                equal: policy.key_equal,
                ttl: None,
                capacity: CacheCapacity::Bounded(policy.retention),
                cache_failure: Arc::new(move |error| match error {
                    DependencyCacheError::Load(error) => failure(error),
                    DependencyCacheError::DependencyCapacity { .. }
                    | DependencyCacheError::ActiveCapacity { .. } => false,
                }),
            },
            CacheOptions {
                ttl: policy.ttl,
                admission: CacheAdmission::Bounded {
                    maximum: maximum_active,
                    capacity_error: Arc::new(move || DependencyCacheError::ActiveCapacity {
                        maximum: maximum_active.get(),
                    }),
                },
            },
            policy.maximum_dependencies,
            policy.dependency_equal,
            load,
        )
    }

    /// Lazily acquire a dependency cache sharing active loads without retaining results.
    /// The borrowed-key callback follows the same lifetime as [`Self::make`]; scope
    /// closure, cancellation, dependency-width checks and invalidation use the same
    /// core engine as the bounded retained configuration.
    pub fn make_coalescing<R: HasClock + HasTaskSpawner + Send + Sync + 'static>(
        scope: Scope<SharedValue<DependencyCacheError<E>>>,
        policy: DependencyCacheCoalescingPolicy<K, D>,
        load: impl Fn(&K) -> Effect<(A, Vec<D>), E, R> + Send + Sync + 'static,
    ) -> Effect<Self, SharedValue<DependencyCacheError<E>>, R> {
        Self::make_coalescing_with_shared_key(scope, policy, move |key| load(&key))
    }

    /// Lazily acquire active-load coalescing with an owned shared key for deferred loaders.
    /// Native producers and waiters retain the same admission, dependency fencing,
    /// ordered shutdown and non-Clone payload ownership as retained caches. A later
    /// lookup starts a fresh load after the previous producer settles; completed
    /// successes and errors are never retained, regardless of wall-clock time.
    pub fn make_coalescing_with_shared_key<R: HasClock + HasTaskSpawner + Send + Sync + 'static>(
        scope: Scope<SharedValue<DependencyCacheError<E>>>,
        policy: DependencyCacheCoalescingPolicy<K, D>,
        load: impl Fn(Arc<K>) -> Effect<(A, Vec<D>), E, R> + Send + Sync + 'static,
    ) -> Effect<Self, SharedValue<DependencyCacheError<E>>, R> {
        let maximum_active = policy.maximum_active;
        Self::make_configured_with_shared_key(
            scope,
            CacheConfig {
                equal: policy.key_equal,
                ttl: None,
                capacity: CacheCapacity::NoRetention,
                cache_failure: Arc::new(|_| false),
            },
            CacheOptions {
                ttl: CacheTtl::Fixed(None),
                admission: CacheAdmission::Bounded {
                    maximum: maximum_active,
                    capacity_error: Arc::new(move || DependencyCacheError::ActiveCapacity {
                        maximum: maximum_active.get(),
                    }),
                },
            },
            policy.maximum_dependencies,
            policy.dependency_equal,
            load,
        )
    }

    /// Build either retention mode over one scoped cache implementation, preserving
    /// common loader validation, active admission and dependency indexing behavior.
    fn make_configured_with_shared_key<R: HasClock + HasTaskSpawner + Send + Sync + 'static>(
        scope: Scope<SharedValue<DependencyCacheError<E>>>,
        config: CacheConfig<K, DependencyCacheError<E>>,
        options: CacheOptions<DependencyValue<A, D>, DependencyCacheError<E>>,
        maximum_dependencies: NonZeroUsize,
        equal: Equal<D>,
        load: impl Fn(Arc<K>) -> Effect<(A, Vec<D>), E, R> + Send + Sync + 'static,
    ) -> Effect<Self, SharedValue<DependencyCacheError<E>>, R> {
        let load = Arc::new(load);
        let cache = cache! {
            scope: scope,
            config: config,
            options: options,
            load |key: Arc<K>| {
                let load = Arc::clone(&load);
                let (value, dependencies) = effect_map_error! {
                    program: load(key),
                    map: DependencyCacheError::Load,
                }?;
                    if dependencies.len() > maximum_dependencies.get() {
                        effect_try_sync! { once; body: { let __dsl_result = Err(DependencyCacheError::DependencyCapacity {
                            maximum: maximum_dependencies.get(),
                            observed: dependencies.len(),
                        }); move || __dsl_result }, on_error: ::core::convert::identity, }?
                    } else {
                        DependencyValue {
                            value: SharedValue::new(value),
                            dependencies: dependencies.into(),
                        }
                    }
            },
        };
        effect_map! { program: cache, map: move |cache| Self {
            cache,
            equal: Arc::clone(&equal),
            maximum_dependencies,
        }, }
    }
    /// Lazily get or load a value and report whether it was retained, shared or newly loaded.
    ///
    /// Concurrent equivalent keys join one producer. Returned handles share `A`;
    /// loader and capacity errors use [`DependencyCacheError`].
    #[doc(alias = "getObserved")]
    pub fn get_observed<R: Send + Sync + 'static>(
        &self,
        key: K,
    ) -> Effect<CacheLookup<A>, SharedValue<DependencyCacheError<E>>, R> {
        let lookup = self.cache.get_observed(key);
        effect! {
            let lookup = lookup.clone();
            let lookup = lookup?;
            match lookup.value.with(|value| value.value.clone()) {
                Ok(value) => CacheLookup {
                    value,
                    observation: lookup.observation,
                },
                Err(defect) => effect_die! { defect: defect, }?,
            }
        }
    }
    /// Lazily get or load one key, sharing existing producer work and payload ownership.
    ///
    /// Use [`Self::get_observed`] when the lookup classification also matters.
    pub fn get<R: Send + Sync + 'static>(
        &self,
        key: K,
    ) -> Effect<SharedValue<A>, SharedValue<DependencyCacheError<E>>, R> {
        effect_map! { program: self.get_observed(key), map: |lookup| lookup.value, }
    }
    /// Read only a fresh retained success without starting or joining a load.
    ///
    /// Expired, absent, pending or failed entries produce `None`; this operation
    /// never registers a producer or consumes active admission.
    pub fn try_get<R: Send + Sync + 'static>(
        &self,
        key: K,
    ) -> Effect<Option<SharedValue<A>>, SharedValue<DependencyCacheError<E>>, R> {
        let lookup = self.cache.try_get(key);
        effect! {
            let lookup = lookup.clone();
            let value = lookup?;
            match value
                .map(|value| value.with(|value| value.value.clone()))
                .transpose()
            {
                Ok(value) => value,
                Err(defect) => effect_die! { defect: defect, }?,
            }
        }
    }
    /// Lazily replace a key with an owned value and explicit dependency identities.
    ///
    /// An oversized dependency list returns a capacity error. Replacement fences and
    /// joins an old active producer so its late completion cannot overwrite the value.
    pub fn set<R: Send + Sync + 'static>(
        &self,
        key: K,
        value: A,
        dependencies: Vec<D>,
    ) -> Effect<(), SharedValue<DependencyCacheError<E>>, R> {
        if dependencies.len() > self.maximum_dependencies.get() {
            return effect_try_sync! { once; body: { let __dsl_result = Err(SharedValue::new(
                DependencyCacheError::DependencyCapacity {
                    maximum: self.maximum_dependencies.get(),
                    observed: dependencies.len(),
                },
            )); move || __dsl_result }, on_error: ::core::convert::identity, };
        }
        self.cache.set(
            key,
            DependencyValue {
                value: SharedValue::new(value),
                dependencies: dependencies.into(),
            },
        )
    }
    /// Lazily evict one key, cancelling and joining producer work owned by that entry.
    pub fn invalidate<R: Send + Sync + 'static>(
        &self,
        key: K,
    ) -> Effect<(), SharedValue<DependencyCacheError<E>>, R> {
        self.cache.invalidate(key)
    }
    /// Lazily invalidate successes containing an equal dependency.
    ///
    /// Also fences pending loads and retained failures, whose complete dependency set
    /// is unavailable. Cancellation and cleanup use the existing cache engine.
    pub fn invalidate_dependency<R: Send + Sync + 'static>(
        &self,
        dependency: D,
    ) -> Effect<(), SharedValue<DependencyCacheError<E>>, R> {
        let equal = Arc::clone(&self.equal);
        self.cache.invalidate_matching(move |_, value, pending| {
            if pending {
                return true;
            }
            match value {
                None => true,
                Some(value) => match value.with(|value| Arc::clone(&value.dependencies)) {
                    Ok(dependencies) => dependencies
                        .iter()
                        .any(|candidate| equal(candidate, &dependency)),
                    Err(_) => true,
                },
            }
        })
    }
    /// Lazily remove all entries and settle their owned producer cleanup.
    pub fn clear<R: Send + Sync + 'static>(
        &self,
    ) -> Effect<(), SharedValue<DependencyCacheError<E>>, R> {
        self.cache.clear()
    }
    /// Lazily stop admission, cancel owned work and await cache shutdown.
    ///
    /// Returns the engine's [`crate::PoolEndStatus`]; shutdown errors remain shared.
    pub fn shutdown<R: Send + Sync + 'static>(
        &self,
    ) -> Effect<crate::PoolEndStatus, SharedValue<DependencyCacheError<E>>, R> {
        self.cache.shutdown()
    }
}
