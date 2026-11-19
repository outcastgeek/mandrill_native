//! Weakly held cached values and partitioned identities.

use crate::cache::{EntryKey, IndexEntry, WeakIndexEntry};
use crate::internal::runtime::{self, Step};
use crate::memory::{
    Bytes, FixedVector, MemoryError, PreparedMemory, RegionId, SlotLease, SlotPool,
};
use crate::{
    Cache, CacheAdmission, CacheCapacity, CacheConfig, CacheKey, CacheOptions, CacheTtl, Defect,
    Effect, Exit, HasClock, HasTaskSpawner, LibraryDefect, PreparedCacheCapacity,
    PreparedCacheRefusals, PreparedCacheStorage, Release, Scope, SharedValue,
};
use crate::{
    cache, effect, effect_all, effect_die, effect_finalizer, effect_map, effect_provide_some,
    effect_try_sync, effect_uninterruptible,
};
use std::num::NonZeroUsize;
use std::sync::{
    Arc, Mutex, Weak,
    atomic::{AtomicBool, Ordering},
};

#[derive(Clone, PartialEq, Eq)]
/// A key partition and its weakly associated cached values.
pub enum CachePartition {
    /// Use the unnamed default partition.
    Default,
    /// Use a caller-supplied named partition. Contains `Arc<str>`.
    Named(Arc<str>),
}
/// Stable identity used by weak cache operations.
///
/// # Type parameters
/// - `K`: Key type used for identity or lookup. Bound or type: `Send + Sync + 'static`.
/// - `A`: Application value type. Bound or type: `Send + 'static`.
/// - `E`: Domain-error type. Bound or type: `Send + 'static`.
struct Key<K: Send + Sync + 'static, A: Send + 'static, E: Send + 'static> {
    identity: Weak<K>,
    partition: CachePartition,
    load: Mutex<Option<Effect<A, E, ()>>>,
}
/// Type alias for associations in weak cache.
///
/// # Type parameters
/// - `K`: Key type used for identity or lookup.
/// - `A`: Application value type.
/// - `E`: Domain-error type.
type WeakEntry<K, A, E> = WeakIndexEntry<Key<K, A, E>, A, E>;
type StrongEntry<K, A, E> = IndexEntry<Key<K, A, E>, A, E>;
type AssociationRows<K, A, E> = FixedVector<'static, Association<K, A, E>>;
type AssociationPool<K, A, E> = SlotPool<'static, AssociationRows<K, A, E>>;
type AssociationRetirement<K, A, E> = SlotPool<'static, FixedVector<'static, StrongEntry<K, A, E>>>;
type WeakMemoryError<E> = Arc<dyn Fn(MemoryError) -> SharedValue<E> + Send + Sync>;
type WeakCachePreparation<K, A, E> = (PreparedWeakCacheStorage<K, A, E>, WeakRefusals<E>);

enum WeakRefusals<E: Send + 'static> {
    Mapper(Arc<dyn Fn(MemoryError) -> E + Send + Sync>),
    Shared(PreparedCacheRefusals<E>),
}

/// Capacity of a prepared weak-cache index and its shared producer engine.
#[derive(Clone, Copy, Debug)]
pub struct PreparedWeakCacheCapacity {
    /// Simultaneous weak entries and unpublished association claims.
    pub associations: usize,
    /// Maximum concurrent cache loaders.
    pub active: NonZeroUsize,
    /// Concurrent lookup snapshots.
    pub lookups: NonZeroUsize,
    /// Concurrent potential or retained unobserved finalization diagnostics.
    pub diagnostics: NonZeroUsize,
    /// Concurrent suspended observations.
    pub observers: NonZeroUsize,
}

impl PreparedWeakCacheCapacity {
    fn cache(self) -> Result<PreparedCacheCapacity, MemoryError> {
        let checked = |value| NonZeroUsize::new(value).ok_or(MemoryError::ArithmeticOverflow);
        let add = |left: usize, right: usize| {
            left.checked_add(right)
                .ok_or(MemoryError::ArithmeticOverflow)
        };
        let values = add(self.associations, self.active.get())?;
        let keys = add(values, self.lookups.get())?;
        let outcomes = add(
            add(self.active.get(), self.lookups.get())?,
            self.diagnostics.get(),
        )?;
        let cleanup_entries = self.diagnostics.get();
        let errors = outcomes
            .checked_mul(add(cleanup_entries, 2)?)
            .ok_or(MemoryError::ArithmeticOverflow)?;
        Ok(PreparedCacheCapacity {
            retention: CacheCapacity::NoRetention,
            active: self.active,
            lookups: self.lookups,
            diagnostics: self.diagnostics,
            observers: self.observers,
            keys: checked(keys)?,
            entries: checked(values)?,
            values: checked(values)?,
            errors: checked(errors)?,
            cleanup_outcomes: checked(outcomes)?,
            maintenance_outcomes: self.lookups,
            cleanup_entries,
        })
    }
}

/// Fixed weak associations, cache indexes and region-backed key and outcome cells.
pub struct PreparedWeakCacheStorage<K: Send + Sync + 'static, A: Send + 'static, E: Send + 'static>
{
    cache: PreparedCacheStorage<Key<K, A, E>, A, E>,
    associations: AssociationPool<K, A, E>,
    retirement: AssociationRetirement<K, A, E>,
    active: NonZeroUsize,
}
impl<K: Send + Sync + 'static, A: Send + 'static, E: Send + 'static>
    PreparedWeakCacheStorage<K, A, E>
{
    /// Region demand using defaults derived from the existing weak-cache bounds.
    /// Entries and values use associations plus active loaders, and keys also
    /// include lookup slots. Outcomes use active loaders plus lookups and
    /// diagnostics, and maintenance outcomes use the lookup count. Each outcome
    /// allows `diagnostics` secondary entries and reserves errors for those
    /// entries plus the primary and first failure.
    pub fn storage_bytes(capacity: PreparedWeakCacheCapacity) -> Result<Bytes, MemoryError> {
        Self::storage_bytes_with_cache(capacity.associations, capacity.cache()?)
    }

    /// Region demand with independently selected payload and cleanup counts.
    pub fn storage_bytes_with_cache(
        association_count: usize,
        cache: PreparedCacheCapacity,
    ) -> Result<Bytes, MemoryError> {
        if !matches!(cache.retention, CacheCapacity::NoRetention) {
            return Err(MemoryError::UnsupportedRunContract);
        }
        let associations = std::alloc::Layout::array::<Association<K, A, E>>(association_count)
            .map_err(|_| MemoryError::InvalidLayout)?;
        let retirement = std::alloc::Layout::array::<StrongEntry<K, A, E>>(association_count)
            .map_err(|_| MemoryError::InvalidLayout)?;
        PreparedCacheStorage::<Key<K, A, E>, A, E>::storage_bytes(cache)?
            .checked_add(SlotPool::<AssociationRows<K, A, E>>::storage_bytes(1)?)?
            .checked_add(Bytes::new(associations.size()))?
            .checked_add(SlotPool::<FixedVector<'static, StrongEntry<K, A, E>>>::storage_bytes(1)?)?
            .checked_add(Bytes::new(retirement.size()))
    }

    /// Provision the selected region using the defaults documented by `storage_bytes`.
    pub fn prepare(
        memory: &PreparedMemory<'static>,
        region: RegionId,
        capacity: PreparedWeakCacheCapacity,
    ) -> Result<Self, MemoryError> {
        Self::prepare_with_cache(memory, region, capacity.associations, capacity.cache()?)
    }

    /// Prepare actual backing with explicit cache and association limits.
    /// The underlying cache must use no retention for weak ownership.
    pub fn prepare_with_cache(
        memory: &PreparedMemory<'static>,
        region: RegionId,
        association_count: usize,
        capacity: PreparedCacheCapacity,
    ) -> Result<Self, MemoryError> {
        Self::storage_bytes_with_cache(association_count, capacity)?;
        let cache = PreparedCacheStorage::prepare(memory, region, capacity)?;
        let associations = memory.slots(region, 1, |_| {
            FixedVector::prepare(memory.allocator(region)?, association_count)
        })?;
        let retirement = memory.slots(region, 1, |_| {
            FixedVector::prepare(memory.allocator(region)?, association_count)
        })?;
        Ok(Self {
            cache,
            associations,
            retirement,
            active: capacity.active,
        })
    }
}

struct Association<K: Send + Sync + 'static, A: Send + 'static, E: Send + 'static> {
    identity: Weak<K>,
    partition: CachePartition,
    entry: Option<WeakEntry<K, A, E>>,
    pending: usize,
    generation: u128,
}
impl<K: Send + Sync + 'static, A: Send + 'static, E: Send + 'static> Association<K, A, E> {
    fn is_live(&self) -> bool {
        self.pending != 0 || self.entry.as_ref().is_some_and(WeakIndexEntry::is_live)
    }

    fn upgrade(&self, stale: Option<&WeakEntry<K, A, E>>) -> Option<StrongEntry<K, A, E>> {
        self.entry
            .as_ref()
            .filter(|entry| stale.is_none_or(|stale| !entry.same_entry(stale)))
            .and_then(WeakIndexEntry::upgrade)
    }
}
struct PreparedAssociations<K: Send + Sync + 'static, A: Send + 'static, E: Send + 'static> {
    rows: SlotLease<'static, AssociationRows<K, A, E>>,
    generation: u128,
}
impl<K: Send + Sync + 'static, A: Send + 'static, E: Send + 'static> Drop
    for PreparedAssociations<K, A, E>
{
    fn drop(&mut self) {
        self.rows.clear();
    }
}
enum Associations<K: Send + Sync + 'static, A: Send + 'static, E: Send + 'static> {
    Ordinary(Vec<Association<K, A, E>>),
    Prepared(PreparedAssociations<K, A, E>),
}
impl<K: Send + Sync + 'static, A: Send + 'static, E: Send + 'static> Default
    for Associations<K, A, E>
{
    fn default() -> Self {
        Self::Ordinary(Vec::new())
    }
}
enum AssociationSelection<K: Send + Sync + 'static, A: Send + 'static, E: Send + 'static> {
    Existing(StrongEntry<K, A, E>),
    New(Option<u128>),
}
impl<K: Send + Sync + 'static, A: Send + 'static, E: Send + 'static> Associations<K, A, E> {
    fn select(
        &mut self,
        identity: &Weak<K>,
        partition: &CachePartition,
        stale: Option<&WeakEntry<K, A, E>>,
    ) -> Result<AssociationSelection<K, A, E>, MemoryError> {
        match self {
            Self::Ordinary(entries) => {
                entries.retain(|row| {
                    row.is_live()
                        && row
                            .entry
                            .as_ref()
                            .is_none_or(|entry| stale.is_none_or(|stale| !entry.same_entry(stale)))
                });
                let existing = entries
                    .iter()
                    .filter(|entry| {
                        entry.identity.ptr_eq(identity) && entry.partition == *partition
                    })
                    .find_map(|row| row.upgrade(None));
                Ok(existing.map_or(
                    AssociationSelection::New(None),
                    AssociationSelection::Existing,
                ))
            }
            Self::Prepared(table) => {
                table.rows.retain(Association::is_live);
                if let Some(association) =
                    table.rows.as_mut_slice().iter_mut().find(|association| {
                        association.identity.ptr_eq(identity) && association.partition == *partition
                    })
                {
                    if let Some(entry) = association.upgrade(stale) {
                        return Ok(AssociationSelection::Existing(entry));
                    }
                    association.pending = association
                        .pending
                        .checked_add(1)
                        .ok_or(MemoryError::OwnerLimit)?;
                    return Ok(AssociationSelection::New(Some(association.generation)));
                }
                let generation = table
                    .generation
                    .checked_add(1)
                    .ok_or(MemoryError::ArithmeticOverflow)?;
                table
                    .rows
                    .push(Association {
                        identity: identity.clone(),
                        partition: partition.clone(),
                        entry: None,
                        pending: 1,
                        generation,
                    })
                    .map_err(|_| MemoryError::NoSlot)?;
                table.generation = generation;
                Ok(AssociationSelection::New(Some(generation)))
            }
        }
    }

    fn publish(
        &mut self,
        generation: Option<u128>,
        entry: &StrongEntry<K, A, E>,
        weak: WeakEntry<K, A, E>,
    ) -> (bool, Option<StrongEntry<K, A, E>>) {
        match self {
            Self::Ordinary(entries) => {
                entries.push(Association {
                    identity: entry.key.identity.clone(),
                    partition: entry.key.partition.clone(),
                    entry: Some(weak),
                    pending: 0,
                    generation: 0,
                });
                (true, None)
            }
            Self::Prepared(table) => {
                if entry.is_invalidated() {
                    return (false, None);
                }
                let Some(row) = table
                    .rows
                    .as_mut_slice()
                    .iter_mut()
                    .find(|row| Some(row.generation) == generation)
                else {
                    return (false, None);
                };
                let previous = row.entry.as_ref().and_then(WeakIndexEntry::upgrade);
                if previous
                    .as_ref()
                    .is_none_or(|current| current.is_invalidated())
                {
                    row.entry = Some(weak);
                }
                (true, previous)
            }
        }
    }
}
/// Typed inner used by weak cache.
///
/// # Type parameters
/// - `K`: Key type used for identity or lookup. Bound or type: `Send + Sync + 'static`.
/// - `A`: Application value type. Bound or type: `Send + 'static`.
/// - `E`: Domain-error type. Bound or type: `Send + 'static`.
struct Inner<K: Send + Sync + 'static, A: Send + 'static, E: Send + 'static> {
    cache: Cache<Key<K, A, E>, A, E>,
    entries: Mutex<Associations<K, A, E>>,
    prepared_error: Option<WeakMemoryError<E>>,
    retirement: Option<Arc<AssociationRetirement<K, A, E>>>,
    closed: AtomicBool,
}

struct AssociationClaim<K: Send + Sync + 'static, A: Send + 'static, E: Send + 'static> {
    owner: Arc<Inner<K, A, E>>,
    generation: Option<u128>,
}
impl<K: Send + Sync + 'static, A: Send + 'static, E: Send + 'static> Drop
    for AssociationClaim<K, A, E>
{
    fn drop(&mut self) {
        let Some(generation) = self.generation else {
            return;
        };
        let mut entries = self
            .owner
            .entries
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Associations::Prepared(table) = &mut *entries {
            if let Some(row) = table
                .rows
                .as_mut_slice()
                .iter_mut()
                .find(|row| row.generation == generation)
            {
                row.pending -= 1;
            }
            table.rows.retain(Association::is_live);
        }
    }
}
/// A cache whose key and association ownership can disappear when consumers release them.
///
/// # Type parameters
/// - `K`: Key type used for identity or lookup. Bound or type: `Send + Sync + 'static`.
/// - `A`: Application value type. Bound or type: `Send + 'static`.
/// - `E`: Domain-error type. Bound or type: `Send + 'static`.
pub struct WeakCache<K: Send + Sync + 'static, A: Send + 'static, E: Send + 'static> {
    inner: Arc<Inner<K, A, E>>,
}
impl<K: Send + Sync + 'static, A: Send + 'static, E: Send + 'static> Clone for WeakCache<K, A, E> {
    ///
    /// # Returns
    /// Type: `Self`.
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}
/// Consumer ownership keeps a settled weak-cache association alive. The cache
/// itself has weak key AND weak entry references. This is deliberately weaker
/// retention than a tracing-GC ephemeron: dropping the last consumer handle may
/// cause a miss even while the key lives. A value-to-key reference cannot make
/// a successful settled association keep itself alive through the cache.
///
/// # Type parameters
/// - `K`: Key type used for identity or lookup. Bound or type: `Send + Sync + 'static`.
/// - `A`: Application value type. Bound or type: `Send + 'static`.
/// - `E`: Domain-error type. Bound or type: `Send + 'static`.
pub struct WeakCacheValue<K: Send + Sync + 'static, A: Send + 'static, E: Send + 'static> {
    entry: StrongEntry<K, A, E>,
    value: SharedValue<A>,
}
impl<K: Send + Sync + 'static, A: Send + 'static, E: Send + 'static> Clone
    for WeakCacheValue<K, A, E>
{
    ///
    /// # Returns
    /// Type: `Self`.
    fn clone(&self) -> Self {
        Self {
            entry: self.entry.clone(),
            value: self.value.clone(),
        }
    }
}
impl<K: Send + Sync + 'static, A: Send + 'static, E: Send + 'static> WeakCacheValue<K, A, E> {
    /// Borrow the contained value only for the duration of the supplied callback.
    ///
    /// # Type parameters
    /// - `K`: Key type used for identity or lookup. Bound: `Send + Sync + 'static`.
    /// - `E`: Domain-error type. Bound: `Send + 'static`.
    /// - `B`: Transformed value type.
    /// - `A`: Success or application-value type.
    ///
    /// # Parameters
    /// - `inspect` (`impl FnOnce(&A) -> B`): The inspect supplied to this operation.
    ///
    /// # Returns
    /// Type: `Result<B, Defect>`.
    pub fn with<B>(&self, inspect: impl FnOnce(&A) -> B) -> Result<B, Defect> {
        self.value.with(inspect)
    }
}
impl<K: Send + Sync + 'static, A: Send + 'static, E: Send + 'static> WeakCache<K, A, E> {
    /// Create a `WeakCache` through the declared Effect and environment capabilities.
    ///
    /// # Type parameters
    /// - `K`: Key type used for identity or lookup. Bound: `Send + Sync + 'static`.
    /// - `A`: Application value type. Bound: `Send + 'static`.
    /// - `R`: Environment type. Bounds: `HasClock + HasTaskSpawner + Send + Sync + 'static`.
    /// - `E`: Domain-error type.
    ///
    /// # Parameters
    /// - `scope` (`Scope<SharedValue<E>>`): Owner that joins child work and releases resources at close.
    ///
    /// # Returns
    /// Type: `Effect<Self, SharedValue<E>, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    pub fn make<R: HasClock + HasTaskSpawner + Send + Sync + 'static>(
        scope: Scope<SharedValue<E>>,
    ) -> Effect<Self, SharedValue<E>, R> {
        Self::make_configured(scope, None)
    }

    /// Select fixed weak associations and the same prepared no-retention cache engine.
    pub fn make_with_prepared_storage<R: HasClock + HasTaskSpawner + Send + Sync + 'static>(
        scope: Scope<SharedValue<E>>,
        storage: PreparedWeakCacheStorage<K, A, E>,
        on_error: impl Fn(MemoryError) -> E + Send + Sync + 'static,
    ) -> Effect<Self, SharedValue<E>, R> {
        Self::make_configured(
            scope,
            Some((storage, WeakRefusals::Mapper(Arc::new(on_error)))),
        )
    }

    /// Select startup-provisioned shared errors for both core and weak-index admission.
    pub fn make_with_prepared_refusals<R: HasClock + HasTaskSpawner + Send + Sync + 'static>(
        scope: Scope<SharedValue<E>>,
        storage: PreparedWeakCacheStorage<K, A, E>,
        refusals: PreparedCacheRefusals<E>,
    ) -> Effect<Self, SharedValue<E>, R> {
        Self::make_configured(scope, Some((storage, WeakRefusals::Shared(refusals))))
    }

    fn make_configured<R: HasClock + HasTaskSpawner + Send + Sync + 'static>(
        scope: Scope<SharedValue<E>>,
        prepared: Option<WeakCachePreparation<K, A, E>>,
    ) -> Effect<Self, SharedValue<E>, R> {
        let config = CacheConfig {
            equal: Arc::new(|left: &Key<K, A, E>, right: &Key<K, A, E>| {
                left.identity.ptr_eq(&right.identity) && left.partition == right.partition
            }),
            ttl: None,
            capacity: CacheCapacity::NoRetention,
            cache_failure: Arc::new(|_| false),
        };
        let load = |key: &Key<K, A, E>| {
            let program = key
                .load
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take();
            match program {
                Some(program) => effect_provide_some! { program: program, project: |_: &R| (), },
                None => effect_die! { defect: Defect::from(LibraryDefect::WeakCacheLoadConsumed), },
            }
        };
        let (build, associations, retirement, prepared_error) = match prepared {
            Some((storage, refusals)) => {
                let prepared_load = move |key: CacheKey<Key<K, A, E>>| load(&key);
                let (build, errors): (_, WeakMemoryError<E>) = match refusals {
                    WeakRefusals::Mapper(on_error) => {
                        let memory_error = on_error.clone();
                        let active_error = on_error.clone();
                        let options = CacheOptions {
                            ttl: CacheTtl::Fixed(None),
                            admission: CacheAdmission::Bounded {
                                maximum: storage.active,
                                capacity_error: Arc::new(move || active_error(MemoryError::NoSlot)),
                            },
                        };
                        let build = cache! {
                            scope: scope.clone(), config: config, options: options,
                            memory: storage.cache, on_capacity: move |error| memory_error(error),
                            load_shared: prepared_load,
                        };
                        (
                            build,
                            Arc::new(move |error| SharedValue::new(on_error(error))),
                        )
                    }
                    WeakRefusals::Shared(refusals) => {
                        let memory = refusals.clone();
                        let build = Cache::make_prepared_with_active_refusals(
                            scope.clone(),
                            config,
                            CacheTtl::Fixed(None),
                            storage.active,
                            storage.cache,
                            refusals,
                            prepared_load,
                        );
                        (build, Arc::new(move |error| memory.memory(error)))
                    }
                };
                (
                    build,
                    Some(Arc::new(storage.associations)),
                    Some(Arc::new(storage.retirement)),
                    Some(errors),
                )
            }
            None => {
                let options = CacheOptions {
                    ttl: CacheTtl::Fixed(None),
                    admission: CacheAdmission::Unbounded,
                };
                let ordinary_load = move |key: Arc<Key<K, A, E>>| load(&key);
                let build = cache! {
                    scope: scope.clone(), config: config, options: options, load_shared: ordinary_load,
                };
                (build, None, None, None)
            }
        };
        effect! {
            let scope = scope.clone();
            let build = build.clone();
            let associations = associations.clone();
            let prepared_error = prepared_error.clone();
            let retirement = retirement.clone();
            let entries = match associations.zip(prepared_error.clone()) {
                Some((pool, errors)) => effect_try_sync! {
                    body: move || pool.try_checkout().map(|rows| {
                        Associations::Prepared(PreparedAssociations { rows, generation: 0 })
                    }),
                    on_error: move |error| errors(error),
                }?,
                None => Associations::default(),
            };
            let cache = build?;
            let cache = Self {
                inner: Arc::new(Inner {
                    cache,
                    entries: Mutex::new(entries),
                    prepared_error,
                    retirement,
                    closed: AtomicBool::new(false),
                }),
            };
            let close = cache.clone();
            let registered = effect_finalizer! {
                release;
                scope: scope,
                release: move || {
                    let close = close.clone();
                    Release::sync(move || close.close_index())
                },
            };
            effect_map! { program: registered, map: move |()| cache.clone(), }?
        }
    }
    ///
    /// # Returns
    /// Type: `()`.
    fn close_index(&self) {
        self.inner.closed.store(true, Ordering::Release);
        let entries = std::mem::take(
            &mut *self
                .inner
                .entries
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        );
        match entries {
            Associations::Ordinary(entries) => {
                for entry in entries.into_iter().filter_map(|row| row.upgrade(None)) {
                    entry.invalidate_association();
                }
            }
            Associations::Prepared(table) => {
                for entry in table
                    .rows
                    .as_slice()
                    .iter()
                    .filter_map(|row| row.entry.as_ref().and_then(WeakIndexEntry::upgrade))
                {
                    entry.invalidate_association();
                }
            }
        }
    }
    fn memory_error<T>(&self, error: MemoryError) -> Exit<T, SharedValue<E>> {
        match &self.inner.prepared_error {
            Some(map) => Exit::Fail(map(error)),
            None => Exit::Die(Defect::from(LibraryDefect::PreparedConstructionUnbound)),
        }
    }

    fn invalidate_snapshot(&self, identity: &Weak<K>, selected: &[StrongEntry<K, A, E>]) {
        for entry in selected {
            entry.invalidate_association();
        }
        let mut entries = self
            .inner
            .entries
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Associations::Prepared(table) = &mut *entries {
            for row in table.rows.as_mut_slice() {
                if row.identity.ptr_eq(identity)
                    && row
                        .entry
                        .as_ref()
                        .is_some_and(|entry| selected.iter().any(|old| entry.matches_entry(old)))
                {
                    row.entry = None;
                }
            }
            table.rows.retain(Association::is_live);
        }
    }

    /// Read the current value through the declared storage or state capability.
    ///
    /// # Type parameters
    /// - `R`: Environment type. Bounds: `Send + Sync + 'static`.
    /// - `A`: Success or application-value type.
    /// - `E`: Domain-error type.
    /// - `K`: Type supplied by the enclosing impl or trait.
    ///
    /// # Parameters
    /// - `key` (`&Arc<K>`): Stable key used to locate or identify the selected value.
    /// - `load` (`impl Fn() -> Effect<A, E, R> + Send + Sync + 'static`): The load supplied to this operation.
    ///
    /// # Returns
    /// Type: `Effect<WeakCacheValue<K, A, E>, SharedValue<E>, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    pub fn get<R: Send + Sync + 'static>(
        &self,
        key: &Arc<K>,
        load: impl Fn() -> Effect<A, E, R> + Send + Sync + 'static,
    ) -> Effect<WeakCacheValue<K, A, E>, SharedValue<E>, R> {
        self.get_partition(CachePartition::Default, key, load)
    }
    /// Lazily acquire a weakly indexed value in this partition using the existing cache engine.
    ///
    /// Live ownership reuses producer outcomes without copying payloads. Selection
    /// preserves typed capacity rejection, defects and interruption; weak indexing does
    /// not introduce a second retention or task-provider mechanism.
    pub fn get_partition<R: Send + Sync + 'static>(
        &self,
        partition: CachePartition,
        key: &Arc<K>,
        load: impl Fn() -> Effect<A, E, R> + Send + Sync + 'static,
    ) -> Effect<WeakCacheValue<K, A, E>, SharedValue<E>, R> {
        let cache = self.clone();
        let identity = Arc::downgrade(key);
        let load = Arc::new(load);
        Effect::new(move |env, context, finish| {
            let cache = cache.clone();
            let identity = identity.clone();
            let partition = partition.clone();
            let load = Arc::clone(&load);
            Step::suspended(Box::pin(async move {
                let work = async {
                    if cache.inner.closed.load(Ordering::Acquire) || cache.inner.cache.is_closed() {
                        return Exit::Die(Defect::from(LibraryDefect::CacheClosed));
                    }
                    if identity.upgrade().is_none() {
                        return Exit::Die(Defect::from(LibraryDefect::WeakCacheKeyDropped));
                    }
                    let mut stale = None;
                    let selection = loop {
                        let selected = {
                            let mut entries = cache
                                .inner
                                .entries
                                .lock()
                                .unwrap_or_else(std::sync::PoisonError::into_inner);
                            if cache.inner.closed.load(Ordering::Acquire) {
                                return Exit::Die(Defect::from(LibraryDefect::CacheClosed));
                            }
                            entries.select(&identity, &partition, stale.as_ref())
                        };
                        match selected {
                            Ok(AssociationSelection::Existing(entry)) if entry.is_invalidated() => {
                                match entry.downgrade() {
                                    Ok(entry) => stale = Some(entry),
                                    Err(error) => return cache.memory_error(error),
                                }
                            }
                            Ok(selected) => break selected,
                            Err(error) => return cache.memory_error(error),
                        }
                    };
                    let selected = match selection {
                        AssociationSelection::Existing(entry) => {
                            match cache.inner.cache.select_existing(entry, &context.cancellation) {
                                Ok(selected) => selected,
                                Err(error) => return error.exit(),
                            }
                        }
                        AssociationSelection::New(generation) => {
                            let claim = AssociationClaim {
                                owner: cache.inner.clone(),
                                generation,
                            };
                            let recipe = Effect::defer(move || load()).provide(env);
                            let key = Key {
                                identity,
                                partition,
                                load: Mutex::new(Some(recipe)),
                            };
                            let key = if generation.is_some() {
                                match cache.inner.cache.prepare_key(key) {
                                    Ok(key) => EntryKey::Prepared(key),
                                    Err((MemoryError::Closed, _)) => {
                                        return Exit::Die(Defect::from(LibraryDefect::CacheClosed));
                                    }
                                    Err((error, key)) => {
                                        drop(key);
                                        drop(claim);
                                        return cache.memory_error(error);
                                    }
                                }
                            } else {
                                EntryKey::Ordinary(Arc::new(key))
                            };
                            match cache.inner.cache.select_entry(key, &context).await {
                                Ok(Some(selected)) => {
                                    let entry = &selected.entry;
                                    let weak = match entry.downgrade() {
                                        Ok(entry) => entry,
                                        Err(error) => {
                                            drop(claim);
                                            return cache.memory_error(error);
                                        }
                                    };
                                    let (published, previous) = {
                                        let mut entries = cache
                                            .inner
                                            .entries
                                            .lock()
                                            .unwrap_or_else(std::sync::PoisonError::into_inner);
                                        if cache.inner.closed.load(Ordering::Acquire) {
                                            (false, None)
                                        } else {
                                            entries.publish(generation, entry, weak)
                                        }
                                    };
                                    drop(previous);
                                    drop(claim);
                                    if !published {
                                        entry.invalidate_association();
                                        return Exit::Interrupted;
                                    }
                                    selected
                                }
                                Ok(None) => return Exit::Interrupted,
                                Err(error) => return error.exit(),
                            }
                        }
                    };

                    let entry = selected.entry.clone();
                    match cache
                        .inner
                        .cache
                        .observe_selected(selected, context.cancellation)
                        .await
                    {
                        Exit::Success(value) => Exit::Success(WeakCacheValue { entry, value }),
                        Exit::Fail(error) => Exit::Fail(error),
                        Exit::Die(defect) => Exit::Die(defect),
                        Exit::Interrupted => Exit::Interrupted,
                        Exit::FinalizationFailed(details) => Exit::FinalizationFailed(details),
                    }
                };
                let outcome =
                    futures_util::FutureExt::catch_unwind(std::panic::AssertUnwindSafe(work))
                        .await
                        .unwrap_or_else(|panic| Exit::Die(Defect::from_panic(panic)));
                runtime::finish(finish, outcome)
            }))
        })
    }
    /// Mark the selected cached or reactive state stale for its consumers.
    ///
    /// # Type parameters
    /// - `A`: Application value type. Bound: `Send + 'static`.
    /// - `R`: Environment type. Bounds: `Send + Sync + 'static`.
    /// - `E`: Domain-error type.
    /// - `K`: Type supplied by the enclosing impl or trait.
    ///
    /// # Parameters
    /// - `key` (`&Arc<K>`): Stable key used to locate or identify the selected value.
    ///
    /// # Returns
    /// Type: `Effect<(), SharedValue<E>, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    pub fn invalidate<R: Send + Sync + 'static>(
        &self,
        key: &Arc<K>,
    ) -> Effect<(), SharedValue<E>, R> {
        let cache = self.clone();
        let identity = Arc::downgrade(key);
        effect_uninterruptible! { program: { let __dsl_factory = move || {
            if let (Some(retirement), Some(map)) = (&cache.inner.retirement, &cache.inner.prepared_error) {
                let selected = match retirement.try_checkout() {
                    Ok(selected) => selected,
                    Err(error) => {
                        let error = map(error);
                        return effect_try_sync! { once; body: move || Err(error) };
                    }
                };
                let mut selected = std::mem::DropGuard::new(selected, |mut slot| slot.clear());
                {
                    let entries = cache.inner.entries.lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    if let Associations::Prepared(table) = &*entries {
                        for entry in table.rows.as_slice().iter()
                            .filter(|row| row.identity.ptr_eq(&identity))
                            .filter_map(|row| row.entry.as_ref().and_then(WeakIndexEntry::upgrade))
                        {
                            selected.push_reserved(entry);
                        }
                    }
                }
                // The callback only changes association state after the core
                // transaction has reserved and committed its own retirement.
                let owner = cache.clone();
                let committed_identity = identity.clone();
                let identity = identity.clone();
                return cache.inner.cache.invalidate_matching_committed(
                    move |candidate, _, _| candidate.identity.ptr_eq(&identity),
                    move || owner.invalidate_snapshot(&committed_identity, selected.as_slice()),
                );
            }
            let selected = {
                let mut entries = cache
                    .inner
                    .entries
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                let Associations::Ordinary(entries) = &mut *entries else { return effect_die! {
                    defect: Defect::from(LibraryDefect::PreparedConstructionUnbound),
                }; };
                let selected = entries
                    .iter()
                    .filter(|row| row.identity.ptr_eq(&identity))
                    .filter_map(|row| row.upgrade(None))
                    .collect::<Vec<_>>();
                entries.retain(|row| !row.identity.ptr_eq(&identity) && row.is_live());
                selected
            };
            let mut retire = Vec::new();
            for entry in selected {
                entry.invalidate_association();
                retire.push(cache.inner.cache.invalidate(Key {
                    identity: identity.clone(),
                    partition: entry.key.partition.clone(),
                    load: Mutex::new(None),
                }));
            }
            effect_map! { program: effect_all! { items: retire, }, map: |_| (), }
        }; effect! { __dsl_factory()? } }, }
    }
    /// Stop accepting new work and describe the settlement of owned work.
    ///
    /// # Type parameters
    /// - `K`: Key type used for identity or lookup. Bound: `Send + Sync + 'static`.
    /// - `A`: Application value type. Bound: `Send + 'static`.
    /// - `R`: Environment type. Bounds: `Send + Sync + 'static`.
    /// - `E`: Domain-error type.
    ///
    /// # Returns
    /// Type: `Effect<crate::PoolEndStatus, SharedValue<E>, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    pub fn shutdown<R: Send + Sync + 'static>(
        &self,
    ) -> Effect<crate::PoolEndStatus, SharedValue<E>, R> {
        let cache = self.clone();
        effect_uninterruptible! { program: { let __dsl_factory = move || {
            cache.close_index();
            cache.inner.cache.shutdown()
        }; effect! { __dsl_factory()? } }, }
    }
}
