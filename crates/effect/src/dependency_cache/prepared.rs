//! Fixed dependency identity storage over the scoped cache engine.

use super::{DependencyCacheError, Equal};
use crate::memory::{
    Bytes, FixedVector, MemoryError, PreparedMemory, RegionId, SharedSlot, SlotId, SlotLease,
    SlotPool,
};
use crate::{
    Cache, CacheConfig, CacheKey, CacheLookup, CacheOptions, Effect, HasClock, HasTaskSpawner,
    PreparedCacheCapacity, PreparedCacheIndex, PreparedCacheRefusals, PreparedCacheStorage, Scope,
    SharedValue, cache, effect, effect_map, effect_map_error, effect_try_sync,
};
use std::alloc::Layout;
use std::num::NonZeroUsize;
use std::sync::{Arc, Mutex};

type DependencySlot<D> = FixedVector<'static, D>;

/// Prepared payload cells bounded independently of the cache index.
pub struct PreparedDependencyPool<D: Send + Sync + 'static> {
    slots: SlotPool<'static, DependencySlot<D>>,
    logical_width: bool,
}
impl<D: Send + Sync + 'static> PreparedDependencyPool<D> {
    /// Region bytes for the slot metadata and inline dependency identities.
    pub fn storage_bytes(slots: usize, identities: usize) -> Result<Bytes, MemoryError> {
        let items = Layout::array::<D>(identities).map_err(|_| MemoryError::InvalidLayout)?;
        SlotPool::<DependencySlot<D>>::storage_bytes(slots)?
            .checked_add(Bytes::new(items.size()).checked_mul(slots)?)
    }

    /// Prepare fixed cells before any loader or cache invocation.
    pub fn prepare(
        memory: &PreparedMemory<'static>,
        region: RegionId,
        slots: usize,
        identities: usize,
    ) -> Result<Self, MemoryError> {
        let _ = Self::storage_bytes(slots, identities)?;
        let slots = memory.slots(region, slots, |_| {
            FixedVector::prepare(memory.allocator(region)?, identities)
        })?;
        Ok(Self {
            slots,
            logical_width: false,
        })
    }

    /// Select logical dependency-width errors for a returned writer that overflowed.
    #[must_use]
    pub fn with_logical_width_errors(mut self) -> Self {
        self.logical_width = true;
        self
    }

    /// Acquire an owned writer for explicit replacement or loader output.
    pub fn try_writer(&self) -> Result<PreparedDependencyWriter<D>, MemoryError> {
        let slot = self.slots.try_checkout()?;
        let maximum = slot.capacity();
        Ok(PreparedDependencyWriter {
            slot: Some(slot),
            failure: None,
            maximum,
            observed: 0,
        })
    }

    fn accepts(&self, writer: &PreparedDependencyWriter<D>) -> bool {
        writer
            .slot
            .as_ref()
            .is_some_and(|slot| slot.belongs_to(&self.slots))
    }
}

/// Unique dependency writer. Dropping an unfinished invocation releases its identities.
pub struct PreparedDependencyWriter<D: Send + Sync + 'static> {
    slot: Option<SlotLease<'static, DependencySlot<D>>>,
    failure: Option<MemoryError>,
    maximum: usize,
    observed: usize,
}
impl<D: Send + Sync + 'static> PreparedDependencyWriter<D> {
    /// Append one identity. An overflow makes publication fail even if ignored by the loader.
    pub fn push(&mut self, identity: D) -> Result<(), MemoryError> {
        self.observed = self.observed.saturating_add(1);
        if let Some(error) = self.failure {
            return Err(error);
        }
        let slot = self.slot.as_mut().ok_or(MemoryError::ConstructionUnbound)?;
        slot.push(identity).map_err(|_| {
            self.failure = Some(MemoryError::NoSlot);
            MemoryError::NoSlot
        })
    }

    fn id(&self) -> Option<SlotId> {
        self.slot.as_ref().map(SlotLease::id)
    }

    fn finish(mut self) -> Result<PreparedDependencies<D>, MemoryError> {
        if let Some(error) = self.failure {
            return Err(error);
        }
        let slot = self.slot.take().ok_or(MemoryError::ConstructionUnbound)?;
        Ok(PreparedDependencies {
            slot: Some(slot.share()),
        })
    }
}
impl<D: Send + Sync + 'static> Drop for PreparedDependencyWriter<D> {
    fn drop(&mut self) {
        if let Some(slot) = &mut self.slot {
            slot.clear();
        }
    }
}

/// Immutable identities retaining their prepared slot through the last reader.
pub struct PreparedDependencies<D: Send + Sync + 'static> {
    slot: Option<SharedSlot<'static, DependencySlot<D>>>,
}
impl<D: Send + Sync + 'static> PreparedDependencies<D> {
    /// Read identities without cloning them.
    #[must_use]
    pub fn as_slice(&self) -> &[D] {
        self.slot.as_ref().map_or(&[], |slot| slot.as_slice())
    }

    /// Retain the actual slot independently of cache eviction.
    pub fn try_clone(&self) -> Result<Self, MemoryError> {
        Ok(Self {
            slot: self.slot.as_ref().map(SharedSlot::try_clone).transpose()?,
        })
    }
}
impl<D: Send + Sync + 'static> Drop for PreparedDependencies<D> {
    fn drop(&mut self) {
        if let Some(mut slot) = self.slot.take().and_then(SharedSlot::into_last) {
            slot.clear();
        }
    }
}

/// A cached value and its physically retained dependency identities.
pub struct PreparedDependencyValue<A: Send + 'static, D: Send + Sync + 'static> {
    /// The original shared cache value.
    pub value: SharedValue<A>,
    /// Dependency ownership that may outlive cache membership.
    pub dependencies: PreparedDependencies<D>,
}

/// Selected scoped cache policy for a dependency writer.
pub struct PreparedDependencyPolicy<K, D: Send + Sync + 'static, A: Send + 'static, E> {
    /// Existing equality, retention and failure policy.
    pub config: CacheConfig<K, DependencyCacheError<E>>,
    /// Existing outcome expiry and active producer admission policy.
    pub options: CacheOptions<PreparedDependencyValue<A, D>, DependencyCacheError<E>>,
    /// Application equality used to fence dependency changes.
    pub dependency_equal: Equal<D>,
}

/// Physical indexes for a cache whose loaders write into fixed dependency backing.
pub type PreparedDependencyPayloadIndex<K, D, A, E> =
    PreparedCacheIndex<K, PreparedDependencyValue<A, D>, DependencyCacheError<E>>;
type PreparedCache<K, D, A, E> = Cache<K, PreparedDependencyValue<A, D>, DependencyCacheError<E>>;
type RetainedPreparedValue<A, D> = Option<SharedValue<PreparedDependencyValue<A, D>>>;

type ValueCells<A> = SlotPool<'static, Option<Mutex<A>>>;
type ValueLease<A> = SlotLease<'static, Option<Mutex<A>>>;

/// Full cache backing plus independent value slots for values cloned from the dependency result.
pub struct PreparedDependencyStorage<
    K: Send + Sync + 'static,
    D: Send + Sync + 'static,
    A: Send + 'static,
    E: Send + 'static,
> {
    cache: PreparedCacheStorage<K, PreparedDependencyValue<A, D>, DependencyCacheError<E>>,
    values: ValueCells<A>,
}
impl<K: Send + Sync + 'static, D: Send + Sync + 'static, A: Send + 'static, E: Send + 'static>
    PreparedDependencyStorage<K, D, A, E>
{
    /// Checked region demand, plus the separately selected dependency writer pool.
    pub fn storage_bytes(
        cache: PreparedCacheCapacity,
        values: NonZeroUsize,
    ) -> Result<Bytes, MemoryError> {
        PreparedCacheStorage::<K, PreparedDependencyValue<A, D>, DependencyCacheError<E>>::storage_bytes(cache)?
            .checked_add(ValueCells::<A>::storage_bytes(values.get())?)
    }

    /// Provision both the outer cache result and the independently shareable value.
    pub fn prepare(
        memory: &PreparedMemory<'static>,
        region: RegionId,
        cache: PreparedCacheCapacity,
        values: NonZeroUsize,
    ) -> Result<Self, MemoryError> {
        Self::storage_bytes(cache, values)?;
        Ok(Self {
            cache: PreparedCacheStorage::prepare(memory, region, cache)?,
            values: memory.slots(region, values.get(), |_| Ok(None))?,
        })
    }
}

/// Dependency invalidation over the existing scoped cache and fixed dependency cells.
pub struct PreparedDependencyCache<
    K: Send + Sync + 'static,
    D: Send + Sync + 'static,
    A: Send + 'static,
    E: Send + 'static,
> {
    cache: PreparedCache<K, D, A, E>,
    pool: Arc<PreparedDependencyPool<D>>,
    values: Option<Arc<ValueCells<A>>>,
    refusals: Option<PreparedCacheRefusals<DependencyCacheError<E>>>,
    equal: Equal<D>,
}
impl<K: Send + Sync + 'static, D: Send + Sync + 'static, A: Send + 'static, E: Send + 'static> Clone
    for PreparedDependencyCache<K, D, A, E>
{
    fn clone(&self) -> Self {
        Self {
            cache: self.cache.clone(),
            pool: self.pool.clone(),
            values: self.values.clone(),
            refusals: self.refusals.clone(),
            equal: self.equal.clone(),
        }
    }
}

impl<K: Send + Sync + 'static, D: Send + Sync + 'static, A: Send + 'static, E: Send + 'static>
    PreparedDependencyCache<K, D, A, E>
{
    /// Configure a loader that receives an actual dependency writer before invocation.
    /// The returned writer must be the same owned slot. Caller pointees retain their own policy.
    pub fn make<R: HasClock + HasTaskSpawner + Send + Sync + 'static>(
        scope: Scope<SharedValue<DependencyCacheError<E>>>,
        policy: PreparedDependencyPolicy<K, D, A, E>,
        index: PreparedDependencyPayloadIndex<K, D, A, E>,
        pool: Arc<PreparedDependencyPool<D>>,
        load: impl Fn(
            Arc<K>,
            PreparedDependencyWriter<D>,
        ) -> Effect<(A, PreparedDependencyWriter<D>), E, R>
        + Send
        + Sync
        + 'static,
    ) -> Effect<Self, SharedValue<DependencyCacheError<E>>, R> {
        let PreparedDependencyPolicy {
            config,
            options,
            dependency_equal,
        } = policy;
        let load = Self::wrap_loader(pool.clone(), None, load);
        let cache = cache! {
            scope: scope,
            config: config,
            options: options,
            prepared_index: index,
            on_capacity: DependencyCacheError::Memory,
            load_shared: load,
        };
        effect_map! {
            program: cache,
            map: move |cache| Self { cache, pool: pool.clone(), values: None, refusals: None, equal: dependency_equal.clone() },
        }
    }

    /// Reserve the cache key and outcome cells. Each loader receives its own
    /// dependency writer after the separate value slot is reserved.
    pub fn make_prepared<R: HasClock + HasTaskSpawner + Send + Sync + 'static>(
        scope: Scope<SharedValue<DependencyCacheError<E>>>,
        policy: PreparedDependencyPolicy<K, D, A, E>,
        storage: PreparedDependencyStorage<K, D, A, E>,
        pool: Arc<PreparedDependencyPool<D>>,
        load: impl Fn(
            CacheKey<K>,
            PreparedDependencyWriter<D>,
        ) -> Effect<(A, PreparedDependencyWriter<D>), E, R>
        + Send
        + Sync
        + 'static,
    ) -> Effect<Self, SharedValue<DependencyCacheError<E>>, R> {
        Self::make_configured(scope, policy, storage, pool, None, false, load)
    }

    /// Use startup-provisioned shared errors for independent admission refusals.
    pub fn make_prepared_with_refusals<R: HasClock + HasTaskSpawner + Send + Sync + 'static>(
        scope: Scope<SharedValue<DependencyCacheError<E>>>,
        policy: PreparedDependencyPolicy<K, D, A, E>,
        storage: PreparedDependencyStorage<K, D, A, E>,
        pool: Arc<PreparedDependencyPool<D>>,
        refusals: PreparedCacheRefusals<DependencyCacheError<E>>,
        load: impl Fn(
            CacheKey<K>,
            PreparedDependencyWriter<D>,
        ) -> Effect<(A, PreparedDependencyWriter<D>), E, R>
        + Send
        + Sync
        + 'static,
    ) -> Effect<Self, SharedValue<DependencyCacheError<E>>, R> {
        Self::make_configured(scope, policy, storage, pool, Some(refusals), false, load)
    }

    /// Prepare independent dependency loads with a drain-only close.
    pub fn make_prepared_independent<R: HasClock + HasTaskSpawner + Send + Sync + 'static>(
        scope: Scope<SharedValue<DependencyCacheError<E>>>,
        policy: PreparedDependencyPolicy<K, D, A, E>,
        storage: PreparedDependencyStorage<K, D, A, E>,
        pool: Arc<PreparedDependencyPool<D>>,
        refusals: PreparedCacheRefusals<DependencyCacheError<E>>,
        load: impl Fn(CacheKey<K>, PreparedDependencyWriter<D>) -> Effect<(A, PreparedDependencyWriter<D>), E, R>
            + Send + Sync + 'static,
    ) -> Effect<Self, SharedValue<DependencyCacheError<E>>, R> {
        Self::make_configured(scope, policy, storage, pool, Some(refusals), true, load)
    }

    fn make_configured<R: HasClock + HasTaskSpawner + Send + Sync + 'static>(
        scope: Scope<SharedValue<DependencyCacheError<E>>>,
        policy: PreparedDependencyPolicy<K, D, A, E>,
        storage: PreparedDependencyStorage<K, D, A, E>,
        pool: Arc<PreparedDependencyPool<D>>,
        refusals: Option<PreparedCacheRefusals<DependencyCacheError<E>>>,
        independent: bool,
        load: impl Fn(
            CacheKey<K>,
            PreparedDependencyWriter<D>,
        ) -> Effect<(A, PreparedDependencyWriter<D>), E, R>
        + Send
        + Sync
        + 'static,
    ) -> Effect<Self, SharedValue<DependencyCacheError<E>>, R> {
        let PreparedDependencyPolicy {
            config,
            options,
            dependency_equal,
        } = policy;
        let values = Arc::new(storage.values);
        let load = Self::wrap_loader(pool.clone(), Some(values.clone()), load);
        let cache = match refusals.clone() {
            Some(refusals) => cache! {
                scope: scope, config: config, options: options,
                memory: storage.cache, refusals: refusals, independent: independent, load_shared: load,
            },
            None => cache! {
                scope: scope, config: config, options: options,
                memory: storage.cache, on_capacity: DependencyCacheError::Memory, load_shared: load,
            },
        };
        effect_map! {
            program: cache,
            map: move |cache| Self {
                cache, pool: pool.clone(), values: Some(values.clone()),
                refusals: refusals.clone(), equal: dependency_equal.clone(),
            },
        }
    }

    fn wrap_loader<Key: Clone + Send + Sync + 'static, R: Send + Sync + 'static>(
        pool: Arc<PreparedDependencyPool<D>>,
        values: Option<Arc<ValueCells<A>>>,
        load: impl Fn(
            Key,
            PreparedDependencyWriter<D>,
        ) -> Effect<(A, PreparedDependencyWriter<D>), E, R>
        + Send
        + Sync
        + 'static,
    ) -> impl Fn(Key) -> Effect<PreparedDependencyValue<A, D>, DependencyCacheError<E>, R> {
        let load = Arc::new(load);
        move |key: Key| {
            let load = load.clone();
            let pool = pool.clone();
            let values = values.clone();
            effect! {
                let pool = pool.clone();
                let selected = pool.clone();
                let load = load.clone();
                let values = values.clone();
                let request = key.clone();
                let (writer, reservation) = effect_try_sync! {
                    body: move || {
                        let writer = selected.try_writer()?;
                        let reservation = Self::reserve_value(values.as_deref())?;
                        Ok((writer, reservation))
                    },
                    on_error: DependencyCacheError::Memory,
                }?;
                let expected = writer.id();
                let (value, writer) = effect_map_error! {
                    program: load(request, writer),
                    map: DependencyCacheError::Load,
                }?;
                let result = if writer.id() == expected && pool.accepts(&writer) {
                    if pool.logical_width && writer.observed > writer.maximum {
                        Err(DependencyCacheError::DependencyCapacity {
                            maximum: writer.maximum, observed: writer.observed,
                        })
                    } else {
                        writer.finish().map(|dependencies| PreparedDependencyValue {
                            value: Self::publish_value(value, reservation), dependencies,
                        }).map_err(DependencyCacheError::Memory)
                    }
                } else {
                    Err(DependencyCacheError::Memory(MemoryError::UnsupportedRunContract))
                };
                effect_try_sync! {
                    once;
                    body: move || result,
                    on_error: ::core::convert::identity,
                }?
            }
        }
    }

    fn reserve_value(values: Option<&ValueCells<A>>) -> Result<Option<ValueLease<A>>, MemoryError> {
        values.map(SlotPool::try_checkout).transpose()
    }

    fn publish_value(value: A, reservation: Option<ValueLease<A>>) -> SharedValue<A> {
        match reservation {
            Some(reservation) => SharedValue::from_reserved(reservation, value),
            None => SharedValue::new(value),
        }
    }

    /// Observe the shared value and its exact retained dependency identities.
    pub fn get<R: Send + Sync + 'static>(
        &self,
        key: K,
    ) -> Effect<SharedValue<PreparedDependencyValue<A, D>>, SharedValue<DependencyCacheError<E>>, R>
    {
        self.cache.get(key)
    }

    /// Observe whether a value was retained, joined or started by this invocation.
    pub fn get_observed<R: Send + Sync + 'static>(
        &self,
        key: K,
    ) -> Effect<CacheLookup<PreparedDependencyValue<A, D>>, SharedValue<DependencyCacheError<E>>, R>
    {
        self.cache.get_observed(key)
    }

    /// Inspect a retained success and its dependency ownership without loading.
    pub fn try_get<R: Send + Sync + 'static>(
        &self,
        key: K,
    ) -> Effect<RetainedPreparedValue<A, D>, SharedValue<DependencyCacheError<E>>, R> {
        self.cache.try_get(key)
    }

    /// Conservatively invalidate pending/failed entries and successes with equal identities.
    pub fn invalidate_dependency<R: Send + Sync + 'static>(
        &self,
        dependency: D,
    ) -> Effect<(), SharedValue<DependencyCacheError<E>>, R> {
        let equal = self.equal.clone();
        self.cache.invalidate_matching(move |_, value, pending| {
            if pending {
                return true;
            }
            let dependencies = value.and_then(|value| {
                value
                    .with(|value| value.dependencies.try_clone())
                    .ok()?
                    .ok()
            });
            dependencies.is_none_or(|dependencies| {
                dependencies
                    .as_slice()
                    .iter()
                    .any(|candidate| equal(candidate, &dependency))
            })
        })
    }

    /// Check out a writer for replacing an entry without moving through a Vec.
    pub fn try_writer(&self) -> Result<PreparedDependencyWriter<D>, MemoryError> {
        self.pool.try_writer()
    }

    /// Share an exact logical width failure through a prepared error slot.
    /// Exhausted error storage returns the configured memory refusal.
    #[must_use]
    pub fn dependency_capacity_error(&self, maximum: usize, observed: usize) -> SharedValue<DependencyCacheError<E>> {
        match self.cache.share_expected_error(DependencyCacheError::DependencyCapacity { maximum, observed }) {
            Ok(error) => error,
            Err(error) => self.refusals.as_ref().map_or_else(
                || SharedValue::new(DependencyCacheError::Memory(error)),
                |refusals| refusals.memory(error),
            ),
        }
    }

    /// Replace one value with its explicitly filled dependency slot.
    pub fn set<R: Send + Sync + 'static>(
        &self,
        key: K,
        value: A,
        writer: PreparedDependencyWriter<D>,
    ) -> Effect<(), SharedValue<DependencyCacheError<E>>, R> {
        let publication = if self.pool.accepts(&writer) {
            Self::reserve_value(self.values.as_deref()).and_then(|reservation| {
                writer.finish().map(|dependencies| PreparedDependencyValue {
                    value: Self::publish_value(value, reservation),
                    dependencies,
                })
            })
        } else {
            Err(MemoryError::UnsupportedRunContract)
        };
        match publication {
            Ok(publication) => self.cache.set(key, publication),
            Err(error) => {
                let error = self.refusals.as_ref().map_or_else(
                    || SharedValue::new(DependencyCacheError::Memory(error)),
                    |refusals| refusals.memory(error),
                );
                effect_try_sync! { once; body: move || Err(error), }
            }
        }
    }

    /// Fence one key and join the old producer.
    pub fn invalidate<R: Send + Sync + 'static>(
        &self,
        key: K,
    ) -> Effect<(), SharedValue<DependencyCacheError<E>>, R> {
        self.cache.invalidate(key)
    }

    /// Clear retained entries and join earlier producers through the same owner.
    pub fn clear<R: Send + Sync + 'static>(
        &self,
    ) -> Effect<(), SharedValue<DependencyCacheError<E>>, R> {
        self.cache.clear()
    }

    /// Stop admission and join scoped producer cleanup.
    pub fn shutdown<R: Send + Sync + 'static>(
        &self,
    ) -> Effect<crate::PoolEndStatus, SharedValue<DependencyCacheError<E>>, R> {
        self.cache.shutdown()
    }
}
