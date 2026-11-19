//! Scoped in-memory cache lookup, ownership, and invalidation.

use crate::cancellation::{LoaderCancellation, OwnedCancellationWait};
use crate::execution::{PreparedTaskCompletion, TaskEvent};
use crate::internal::notification::notify_all;
use crate::internal::runtime::{self, Context, Finish, Step};
use crate::internal::runtime_storage::ReservedOnce;
use crate::internal::work_list::WorkList as CacheList;
use crate::memory::{
    Bytes, FixedVector, FramePool, MemoryError, PreparedMemory, PreparedUnitFuture, RegionId,
    SlotLease, SlotPool, WeakSlot, prepared_notification,
};
use crate::scope::Registered;
use crate::{
    Cancellation, CleanupFailure, Clock, Defect, Effect, Exit, Finalization, HasClock,
    HasTaskSpawner, LibraryDefect, PreparedCleanupAccumulator, PrimaryOutcome, ScheduledTask,
    Scope, TaskSpawner,
};
use crate::{cache, effect, effect_sync, effect_uninterruptible};
use event_listener::Event;
use futures_util::{
    FutureExt,
    future::{Either, select},
};
use std::alloc::Layout;
use std::future::Future;
use std::num::NonZeroUsize;
use std::ops::Deref;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

mod prepared;
pub use prepared::{CacheKey, PreparedCacheCapacity, PreparedCacheStorage};
use prepared::{Cell, CellLease, Cleanup, OutcomeStorage, Payloads};

/// Shared ownership of a payload without requiring `T: Clone`.
///
/// Cloning shares one protected value. [`Self::with`] borrows it for a synchronous
/// inspection; [`Self::try_into_owned`] transfers it only when this is the last
/// owner. Debug output is opaque and does not inspect the application payload.
pub struct SharedValue<T: Send + 'static> {
    value: SharedStorage<T>,
}
enum SharedStorage<T: Send + 'static> {
    Ordinary(Arc<Mutex<T>>),
    Prepared(Cell<Mutex<T>>),
}

/// Startup-prepared cells for independently retained shared values.
pub struct PreparedSharedValuePool<T: Send + 'static> {
    cells: SlotPool<'static, Option<Mutex<T>>>,
}

/// One preadmitted shared value cell, empty until publication.
pub struct PreparedSharedValueReservation<T: Send + 'static> {
    cell: SlotLease<'static, Option<Mutex<T>>>,
}

impl<T: Send + 'static> PreparedSharedValuePool<T> {
    /// Selected region bytes excluding allocator overhead.
    pub fn storage_bytes(capacity: usize) -> Result<Bytes, MemoryError> {
        SlotPool::<Option<Mutex<T>>>::storage_bytes(capacity)
    }

    /// Provision the actual cells before work that may retain a shared value.
    pub fn prepare(
        memory: &PreparedMemory<'static>,
        region: RegionId,
        capacity: usize,
    ) -> Result<Self, MemoryError> {
        Ok(Self { cells: memory.slots(region, capacity, |_| Ok(None))? })
    }

    /// Reserve one physical cell before constructing its value.
    pub fn reserve(&self) -> Result<PreparedSharedValueReservation<T>, MemoryError> {
        Ok(PreparedSharedValueReservation { cell: self.cells.try_checkout()? })
    }
}

impl<T: Send + 'static> PreparedSharedValueReservation<T> {
    /// Transfer the value into the cell retained by all shared readers.
    #[must_use]
    pub fn publish(self, value: T) -> SharedValue<T> {
        SharedValue::from_reserved(self.cell, value)
    }
}
impl<T: Send + 'static> std::fmt::Debug for SharedValue<T> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SharedValue")
            .finish_non_exhaustive()
    }
}
impl<T: Send + 'static> Clone for SharedValue<T> {
    ///
    /// # Returns
    /// Type: `Self`.
    fn clone(&self) -> Self {
        Self {
            value: match &self.value {
                SharedStorage::Ordinary(value) => SharedStorage::Ordinary(Arc::clone(value)),
                SharedStorage::Prepared(value) => SharedStorage::Prepared(value.clone()),
            },
        }
    }
}
impl<T: Send + 'static> SharedValue<T> {
    #[must_use]
    /// Construct a `SharedValue` from its required inputs and explicit policies.
    ///
    /// # Type parameters
    /// - `T`: Value or item type.
    ///
    /// # Parameters
    /// - `value` (`T`): Value read, transformed, encoded, or stored by this operation.
    ///
    /// # Returns
    /// Type: `Self`.
    pub fn new(value: T) -> Self {
        Self {
            value: SharedStorage::Ordinary(Arc::new(Mutex::new(value))),
        }
    }

    /// Region bytes for one startup-provisioned value and its cell metadata.
    pub fn prepared_storage_bytes() -> Result<Bytes, MemoryError> {
        SlotPool::<Option<Mutex<T>>>::storage_bytes(1)
    }

    /// Provision one shared value in startup-owned memory. The last handle
    /// keeps its cell checked out until the stored value is removed or dropped.
    pub fn prepare_region(
        memory: &PreparedMemory<'static>,
        region: RegionId,
        value: T,
    ) -> Result<Self, MemoryError> {
        let slots = memory.slots(region, 1, |_| Ok(None))?;
        Ok(Self::from_reserved(slots.try_checkout()?, value))
    }

    pub(crate) fn from_reserved(slot: SlotLease<'static, Option<Mutex<T>>>, value: T) -> Self {
        Self {
            value: SharedStorage::Prepared(Cell::publish(slot, Mutex::new(value))),
        }
    }
    /// Borrow the contained value only for the duration of the supplied callback.
    ///
    /// # Type parameters
    /// - `A`: Application value type.
    /// - `T`: Value or item type.
    ///
    /// # Parameters
    /// - `inspect` (`impl FnOnce(&T) -> A`): The inspect supplied to this operation.
    ///
    /// # Returns
    /// Type: `Result<A, Defect>`.
    pub fn with<A>(&self, inspect: impl FnOnce(&T) -> A) -> Result<A, Defect> {
        let value = match &self.value {
            SharedStorage::Ordinary(value) => &**value,
            SharedStorage::Prepared(value) => &**value,
        }
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
        catch_unwind(AssertUnwindSafe(|| inspect(&value))).map_err(Defect::from_panic)
    }
    /// Take the guarded value when no other shared views retain it.
    ///
    /// # Type parameters
    /// - `T`: Value or item type.
    ///
    /// # Returns
    /// Type: `Result<T, Self>`.
    pub fn try_into_owned(self) -> Result<T, Self> {
        let value = match self.value {
            SharedStorage::Ordinary(value) => {
                Arc::try_unwrap(value).map_err(SharedStorage::Ordinary)
            }
            SharedStorage::Prepared(value) => {
                value.try_into_inner().map_err(SharedStorage::Prepared)
            }
        }
        .map_err(|value| Self { value })?;
        Ok(value
            .into_inner()
            .unwrap_or_else(std::sync::PoisonError::into_inner))
    }
}
#[derive(Clone, Copy)]
/// Explicit resource bounds for cache.
pub enum CacheCapacity {
    /// No explicit capacity bound is imposed.
    Unbounded,
    /// The supplied positive capacity bound is enforced. Contains `NonZeroUsize`.
    Bounded(NonZeroUsize),
    /// No retention is available.
    NoRetention,
}
/// Type alias for equality in cache.
///
/// # Type parameters
/// - `K`: Key type used for identity or lookup.
type Equality<K> = Arc<dyn Fn(&K, &K) -> bool + Send + Sync>;
/// The equality, expiry, capacity, and failure-retention policies for a cache.
///
/// # Type parameters
/// - `K`: Key type used for identity or lookup.
/// - `E`: Domain-error type.
pub struct CacheConfig<K, E> {
    /// Key equality function used to identify cache entries. Type: `Equality<K>`.
    pub equal: Equality<K>,
    /// Time for which the stored value remains valid. Type: `Option<Duration>`.
    pub ttl: Option<Duration>,
    /// Settled-entry retention capacity; active producer admission is controlled separately.
    pub capacity: CacheCapacity,
    /// Predicate selecting expected errors that remain cached. Type: `Arc<dyn Fn(&E) -> bool + Send + Sync>`.
    pub cache_failure: Arc<dyn Fn(&E) -> bool + Send + Sync>,
}
impl<K, E> Clone for CacheConfig<K, E> {
    ///
    /// # Returns
    /// Type: `Self`.
    fn clone(&self) -> Self {
        Self {
            equal: Arc::clone(&self.equal),
            ttl: self.ttl,
            capacity: self.capacity,
            cache_failure: Arc::clone(&self.cache_failure),
        }
    }
}
/// The lookup path selected atomically when a key is acquired by [`Cache::get_observed`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CacheObservation {
    /// A fresh settled value was reused without dispatching a producer.
    Retained,
    /// The lookup joined already active producer work for an equivalent key.
    Shared,
    /// The lookup admitted new producer work for this key.
    Loaded,
}
/// A shared successful payload and its acquisition classification; no payload clone is required.
pub struct CacheLookup<A: Send + 'static> {
    /// Shared ownership of the successful cached payload.
    pub value: SharedValue<A>,
    /// How this acquisition found or started work, even if completion races with observation.
    pub observation: CacheObservation,
}
type OutcomeTtl<A, E> = Arc<dyn Fn(&Exit<A, E>) -> Result<Option<Duration>, Defect> + Send + Sync>;
/// Retention lifetime selected after producer settlement, using the environment clock.
///
/// Expiry affects retained reuse, not execution cancellation. None means no expiry;
/// failure retention still depends on [`CacheConfig::cache_failure`].
pub enum CacheTtl<A, E> {
    /// Use one optional duration for all retainable outcomes; None retains without time expiry.
    Fixed(Option<Duration>),
    /// Compute an optional duration from the full settled outcome outside the cache gate; callback errors remain defects.
    Outcome(OutcomeTtl<A, E>),
}
impl<A, E> Clone for CacheTtl<A, E> {
    fn clone(&self) -> Self {
        match self {
            Self::Fixed(ttl) => Self::Fixed(*ttl),
            Self::Outcome(callback) => Self::Outcome(Arc::clone(callback)),
        }
    }
}
/// Admission limit for independent active producers, checked before task-provider dispatch.
pub enum CacheAdmission<E> {
    /// Allow independent active producers without an explicit admission count bound.
    Unbounded,
    /// Reject new independent work at the supplied active bound; waiters joining existing work consume no additional slot.
    Bounded {
        /// Maximum admitted independent producer count.
        maximum: NonZeroUsize,
        /// Factory producing the typed rejection when active admission is exhausted.
        capacity_error: Arc<dyn Fn() -> E + Send + Sync>,
    },
}
impl<E> Clone for CacheAdmission<E> {
    fn clone(&self) -> Self {
        match self {
            Self::Unbounded => Self::Unbounded,
            Self::Bounded {
                maximum,
                capacity_error,
            } => Self::Bounded {
                maximum: *maximum,
                capacity_error: Arc::clone(capacity_error),
            },
        }
    }
}
impl<E> CacheAdmission<E> {
    fn maximum(&self) -> Option<NonZeroUsize> {
        match self {
            Self::Unbounded => None,
            Self::Bounded { maximum, .. } => Some(*maximum),
        }
    }
}
/// Outcome expiry and active admission settings used by [`Cache::make_with_options`].
pub struct CacheOptions<A, E> {
    /// Retention lifetime selection; overrides the legacy fixed TTL in CacheConfig.
    pub ttl: CacheTtl<A, E>,
    /// Active producer bound, independent of settled retention capacity.
    pub admission: CacheAdmission<E>,
}
impl<A, E> Clone for CacheOptions<A, E> {
    fn clone(&self) -> Self {
        Self {
            ttl: self.ttl.clone(),
            admission: self.admission.clone(),
        }
    }
}
pub(crate) enum CacheSelectionError<E: Send + 'static> {
    Defect(Defect),
    Capacity(SharedValue<E>),
}
impl<E: Send + 'static> CacheSelectionError<E> {
    pub(crate) fn exit<A>(self) -> Exit<A, SharedValue<E>> {
        match self {
            Self::Defect(defect) => Exit::Die(defect),
            Self::Capacity(error) => Exit::Fail(error),
        }
    }
}

type SharedOutcome<A, E> = Exit<SharedValue<A>, SharedValue<E>>;

fn clone_observed<A: Send + 'static, E: Send + 'static>(
    outcome: &SharedOutcome<A, E>,
    invalidated: bool,
    cleanup: &mut Option<Cleanup<E>>,
) -> SharedOutcome<A, E> {
    match outcome {
        Exit::FinalizationFailed(details) => {
            let mut rest = cleanup.take().unwrap_or_default();
            rest.extend(details.rest.iter().cloned());
            let primary = if invalidated {
                PrimaryOutcome::WasInterrupted
            } else {
                details.primary.clone()
            };
            Exit::FinalizationFailed(Finalization {
                primary,
                first: details.first.clone(),
                rest,
            })
        }
        Exit::Success(_) | Exit::Fail(_) | Exit::Die(_) | Exit::Interrupted => {
            if invalidated {
                Exit::Interrupted
            } else {
                outcome.clone()
            }
        }
    }
}

struct Retirement<E: Send + 'static> {
    outcome: PreparedCleanupAccumulator<(), SharedValue<E>>,
    notification: Option<prepared_notification::Registration>,
}

struct RetirementReservation<E: Send + 'static> {
    cleanup: Cleanup<E>,
    notification: prepared_notification::Registration,
}

impl<E: Send + 'static> RetirementReservation<E> {
    fn reserve<K: Send + Sync + 'static, A: Send + 'static>(
        payloads: &Payloads<K, A, E>,
    ) -> Result<Self, MemoryError> {
        Ok(Self {
            cleanup: payloads.reserve_retirement()?,
            notification: payloads.changed.listen()?,
        })
    }
}

impl<E: Send + 'static> Retirement<E> {
    fn new(reservation: Option<RetirementReservation<E>>) -> Self {
        Self::from_primary(reservation, Exit::Success(()))
    }

    fn from_primary(
        reservation: Option<RetirementReservation<E>>,
        primary: Exit<(), SharedValue<E>>,
    ) -> Self {
        let (cleanup, notification) = reservation.map_or_else(
            || (Cleanup::default(), None),
            |reservation| (reservation.cleanup, Some(reservation.notification)),
        );
        Self {
            outcome: PreparedCleanupAccumulator::from_reserved(primary, cleanup),
            notification,
        }
    }

    fn with_primary<A: Send + 'static>(
        reservation: Option<RetirementReservation<E>>,
        outcome: &SharedOutcome<A, E>,
    ) -> Self {
        let primary = match outcome.primary() {
            crate::PrimaryOutcomeRef::Succeeded => Exit::Success(()),
            crate::PrimaryOutcomeRef::Failed(error) => Exit::Fail(error.clone()),
            crate::PrimaryOutcomeRef::Defected(defect) => Exit::Die(defect.clone()),
            crate::PrimaryOutcomeRef::WasInterrupted => Exit::Interrupted,
        };
        let mut retirement = Self::from_primary(reservation, primary);
        retirement.append_exit(outcome);
        retirement
    }

    fn append_exit<A: Send + 'static>(&mut self, outcome: &SharedOutcome<A, E>) {
        if let Exit::FinalizationFailed(details) = outcome {
            for failure in std::iter::once(&details.first).chain(details.rest.iter()) {
                let failure = match failure {
                    CleanupFailure::ReleaseFailure(error) => Exit::Fail(error.clone()),
                    CleanupFailure::ReleaseDefect(defect) => Exit::Die(defect.clone()),
                    CleanupFailure::ReleaseInterrupted => Exit::Interrupted,
                };
                self.outcome.append_cleanup(failure);
            }
        }
    }

    fn finish(self) -> Exit<(), SharedValue<E>> {
        self.outcome.finish()
    }
}

pub(crate) enum EntryKey<K: Send + Sync + 'static> {
    Ordinary(Arc<K>),
    Prepared(CacheKey<K>),
}
impl<K: Send + Sync + 'static> Deref for EntryKey<K> {
    type Target = K;

    fn deref(&self) -> &K {
        match self {
            Self::Ordinary(key) => key,
            Self::Prepared(key) => key,
        }
    }
}
impl<K: Send + Sync + 'static> Clone for EntryKey<K> {
    fn clone(&self) -> Self {
        match self {
            Self::Ordinary(key) => Self::Ordinary(Arc::clone(key)),
            Self::Prepared(key) => Self::Prepared(key.clone()),
        }
    }
}

enum RecipeKey<K: Send + Sync + 'static> {
    Pending(Option<K>),
    Admitted(CacheKey<K>),
}

enum LookupKey<K: Send + Sync + 'static> {
    Owned(EntryKey<K>),
    Pending(Arc<Mutex<RecipeKey<K>>>, KeyPurpose),
}
#[derive(Clone, Copy)]
enum KeyPurpose {
    Lookup,
    Maintenance,
}
impl<K: Send + Sync + 'static> Clone for LookupKey<K> {
    fn clone(&self) -> Self {
        match self {
            Self::Owned(key) => Self::Owned(key.clone()),
            Self::Pending(key, purpose) => Self::Pending(Arc::clone(key), *purpose),
        }
    }
}

type InvalidationPredicate<K, A> =
    Arc<dyn Fn(&K, Option<SharedValue<A>>, bool) -> bool + Send + Sync>;
enum Invalidation<K, A: Send + 'static, Q> {
    Predicate(InvalidationPredicate<K, A>),
    Key(Q),
}
impl<K, A: Send + 'static, Q: Clone> Clone for Invalidation<K, A, Q> {
    fn clone(&self) -> Self {
        match self {
            Self::Predicate(predicate) => Self::Predicate(Arc::clone(predicate)),
            Self::Key(key) => Self::Key(key.clone()),
        }
    }
}
/// Typed entry state used by cache.
///
/// # Type parameters
/// - `A`: Application value type. Bound or type: `Send + 'static`.
/// - `E`: Domain-error type. Bound or type: `Send + 'static`.
struct EntryState<A: Send + 'static, E: Send + 'static> {
    outcome: Option<SharedOutcome<A, E>>,
    expires: Option<Duration>,
    invalidated: bool,
    retiring: bool,
    observed: bool,
    diagnostic: Option<SlotLease<'static, ()>>,
}
/// Typed entry used by cache.
///
/// # Type parameters
/// - `K`: Key type used for identity or lookup. Bound or type: `Send + Sync + 'static`.
/// - `A`: Application value type. Bound or type: `Send + 'static`.
/// - `E`: Domain-error type. Bound or type: `Send + 'static`.
pub(crate) struct Entry<K: Send + Sync + 'static, A: Send + 'static, E: Send + 'static> {
    pub(crate) key: EntryKey<K>,
    state: Mutex<EntryState<A, E>>,
    outcome_storage: Mutex<Option<OutcomeStorage<A, E>>>,
    produced: Mutex<Option<Exit<A, E>>>,
    submission: Mutex<Submission>,
    changed: Option<Event>,
    cancellation: Cancellation,
}

pub(crate) enum IndexEntry<K: Send + Sync + 'static, A: Send + 'static, E: Send + 'static> {
    Ordinary(Arc<Entry<K, A, E>>),
    Prepared(Cell<Entry<K, A, E>>),
}
type EntryLease<K, A, E> = CellLease<Entry<K, A, E>>;

impl<K: Send + Sync + 'static, A: Send + 'static, E: Send + 'static> Clone for IndexEntry<K, A, E> {
    fn clone(&self) -> Self {
        match self {
            Self::Ordinary(entry) => Self::Ordinary(Arc::clone(entry)),
            Self::Prepared(entry) => Self::Prepared(entry.clone()),
        }
    }
}

impl<K: Send + Sync + 'static, A: Send + 'static, E: Send + 'static> Deref for IndexEntry<K, A, E> {
    type Target = Entry<K, A, E>;

    fn deref(&self) -> &Self::Target {
        match self {
            Self::Ordinary(entry) => entry,
            Self::Prepared(entry) => entry,
        }
    }
}

impl<K: Send + Sync + 'static, A: Send + 'static, E: Send + 'static> IndexEntry<K, A, E> {
    fn publish(lease: Option<EntryLease<K, A, E>>, entry: Entry<K, A, E>) -> Self {
        match lease {
            Some(lease) => Self::Prepared(Cell::publish(lease, entry)),
            None => Self::Ordinary(Arc::new(entry)),
        }
    }

    pub(crate) fn same_entry(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Ordinary(left), Self::Ordinary(right)) => Arc::ptr_eq(left, right),
            (Self::Prepared(left), Self::Prepared(right)) => left.same_cell(right),
            _ => false,
        }
    }

    pub(crate) fn downgrade(&self) -> Result<WeakIndexEntry<K, A, E>, MemoryError> {
        match self {
            Self::Ordinary(entry) => Ok(WeakIndexEntry::Ordinary(Arc::downgrade(entry))),
            Self::Prepared(entry) => entry.downgrade().map(WeakIndexEntry::Prepared),
        }
    }
}

pub(crate) enum WeakIndexEntry<K: Send + Sync + 'static, A: Send + 'static, E: Send + 'static> {
    Ordinary(Weak<Entry<K, A, E>>),
    Prepared(WeakSlot<'static, Option<Entry<K, A, E>>>),
}

impl<K: Send + Sync + 'static, A: Send + 'static, E: Send + 'static> WeakIndexEntry<K, A, E> {
    pub(crate) fn matches_entry(&self, entry: &IndexEntry<K, A, E>) -> bool {
        match (self, entry) {
            (Self::Ordinary(weak), IndexEntry::Ordinary(entry)) => {
                std::ptr::eq(weak.as_ptr(), Arc::as_ptr(entry))
            }
            (Self::Prepared(weak), IndexEntry::Prepared(entry)) => entry.matches_weak(weak),
            _ => false,
        }
    }

    pub(crate) fn same_entry(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Ordinary(left), Self::Ordinary(right)) => left.ptr_eq(right),
            (Self::Prepared(left), Self::Prepared(right)) => left.same_slot(right),
            _ => false,
        }
    }

    pub(crate) fn upgrade(&self) -> Option<IndexEntry<K, A, E>> {
        match self {
            Self::Ordinary(entry) => entry.upgrade().map(IndexEntry::Ordinary),
            Self::Prepared(entry) => entry
                .upgrade()
                .map(|entry| IndexEntry::Prepared(Cell::from_shared(entry))),
        }
    }

    pub(crate) fn is_live(&self) -> bool {
        match self {
            Self::Ordinary(entry) => entry.strong_count() != 0,
            Self::Prepared(entry) => entry.is_live(),
        }
    }
}
type IndexTable<K, A, E> = FixedVector<'static, IndexEntry<K, A, E>>;

/// Finite retained-cache index and lookup snapshots with interchangeable backing.
pub struct PreparedCacheIndex<K: Send + Sync + 'static, A: Send + 'static, E: Send + 'static> {
    slots: SlotPool<'static, IndexTable<K, A, E>>,
    active: SlotPool<'static, IndexTable<K, A, E>>,
    maintenance: SlotPool<'static, IndexTable<K, A, E>>,
    shutdown: SlotPool<'static, IndexTable<K, A, E>>,
    diagnostics: SlotPool<'static, FixedVector<'static, Diagnostic<K, A, E>>>,
    diagnostic_permits: SlotPool<'static, ()>,
    observers: SlotPool<'static, ()>,
    width: usize,
    maximum_active: NonZeroUsize,
}
impl<K: Send + Sync + 'static, A: Send + 'static, E: Send + 'static> PreparedCacheIndex<K, A, E> {
    /// Caller-region bytes for the indexes and lookup table metadata.
    pub fn storage_bytes(
        retention: CacheCapacity,
        active: NonZeroUsize,
        concurrent_lookups: NonZeroUsize,
        diagnostics: NonZeroUsize,
        observers: NonZeroUsize,
    ) -> Result<Bytes, MemoryError> {
        let width = prepared_index_width(retention, active)?;
        let maintenance = width.checked_add(active.get())
            .and_then(|width| width.checked_add(diagnostics.get()))
            .ok_or(MemoryError::ArithmeticOverflow)?;
        let tables = concurrent_lookups.get().checked_add(1).ok_or(MemoryError::ArithmeticOverflow)?;
        let width = std::alloc::Layout::array::<IndexEntry<K, A, E>>(width)
            .map_err(|_| MemoryError::InvalidLayout)?;
        let active = std::alloc::Layout::array::<IndexEntry<K, A, E>>(active.get())
            .map_err(|_| MemoryError::InvalidLayout)?;
        let records = std::alloc::Layout::array::<Diagnostic<K, A, E>>(diagnostics.get())
            .map_err(|_| MemoryError::InvalidLayout)?;
        let maintenance = std::alloc::Layout::array::<IndexEntry<K, A, E>>(maintenance)
            .map_err(|_| MemoryError::InvalidLayout)?;
        SlotPool::<IndexTable<K, A, E>>::storage_bytes(tables)?
            .checked_add(Bytes::new(width.size()).checked_mul(tables)?)?
            .checked_add(SlotPool::<IndexTable<K, A, E>>::storage_bytes(1)?)?
            .checked_add(Bytes::new(active.size()))?
            .checked_add(SlotPool::<IndexTable<K, A, E>>::storage_bytes(1)?)?
            .checked_add(Bytes::new(active.size()))?
            .checked_add(SlotPool::<IndexTable<K, A, E>>::storage_bytes(1)?)?
            .checked_add(Bytes::new(maintenance.size()))?
            .checked_add(SlotPool::<FixedVector<'static, Diagnostic<K, A, E>>>::storage_bytes(2)?)?
            .checked_add(Bytes::new(records.size()).checked_mul(2)?)?
            .checked_add(SlotPool::<()>::storage_bytes(diagnostics.get())?)?
            .checked_add(SlotPool::<()>::storage_bytes(observers.get())?)
    }
    /// Provision a live retained index and the selected concurrent lookup snapshots.
    /// Pending producers count toward membership even when settled retention is disabled.
    pub fn prepare(
        memory: &PreparedMemory<'static>,
        region: RegionId,
        retention: CacheCapacity,
        active: NonZeroUsize,
        concurrent_lookups: NonZeroUsize,
        diagnostics: NonZeroUsize,
        observers: NonZeroUsize,
    ) -> Result<Self, MemoryError> {
        let _ = Self::storage_bytes(retention, active, concurrent_lookups, diagnostics, observers)?;
        let width = prepared_index_width(retention, active)?;
        let maintenance_width = width.checked_add(active.get())
            .and_then(|width| width.checked_add(diagnostics.get()))
            .ok_or(MemoryError::ArithmeticOverflow)?;
        let tables = concurrent_lookups.get().checked_add(1).ok_or(MemoryError::ArithmeticOverflow)?;
        let slots = memory.slots(region, tables, |_| FixedVector::prepare(memory.allocator(region)?, width))?;
        let active_slots = memory.slots(region, 1, |_| FixedVector::prepare(memory.allocator(region)?, active.get()))?;
        let shutdown = memory.slots(region, 1, |_| FixedVector::prepare(memory.allocator(region)?, active.get()))?;
        let maintenance = memory.slots(region, 1, |_| FixedVector::prepare(memory.allocator(region)?, maintenance_width))?;
        let diagnostic_slots = memory.slots(region, 2, |_| FixedVector::prepare(memory.allocator(region)?, diagnostics.get()))?;
        let diagnostic_permits = memory.slots(region, diagnostics.get(), |_| Ok(()))?;
        let observers = memory.slots(region, observers.get(), |_| Ok(()))?;
        Ok(Self {
            slots, active: active_slots, maintenance, shutdown, diagnostics: diagnostic_slots, diagnostic_permits,
            observers, width, maximum_active: active,
        })
    }

    fn reserve(&self) -> Result<CacheIndex<K, A, E>, MemoryError> {
        CacheIndex::checkout(&self.slots)
    }

    fn reserve_active(&self) -> Result<CacheIndex<K, A, E>, MemoryError> {
        CacheIndex::checkout(&self.active)
    }

    fn reserve_maintenance(&self) -> Result<CacheIndex<K, A, E>, MemoryError> {
        CacheIndex::checkout(&self.maintenance)
    }

    fn reserve_shutdown(&self) -> Result<CacheIndex<K, A, E>, MemoryError> {
        CacheIndex::checkout(&self.shutdown)
    }

    fn reserve_diagnostics(&self) -> Result<CacheList<Diagnostic<K, A, E>>, MemoryError> {
        CacheList::checkout(&self.diagnostics)
    }
}

fn prepared_index_width(retention: CacheCapacity, active: NonZeroUsize) -> Result<usize, MemoryError> {
    let settled = match retention {
        CacheCapacity::Bounded(capacity) => capacity.get(),
        CacheCapacity::NoRetention => 0,
        CacheCapacity::Unbounded => return Err(MemoryError::UnsupportedRunContract),
    };
    settled.checked_add(active.get()).ok_or(MemoryError::ArithmeticOverflow)
}

type CacheIndex<K, A, E> = CacheList<IndexEntry<K, A, E>>;

struct CacheSnapshot<K: Send + Sync + 'static, A: Send + 'static, E: Send + 'static> {
    revision: CacheRevision,
    entries: CacheIndex<K, A, E>,
}
struct ObservationReservation<E: Send + 'static> {
    observer: Option<SlotLease<'static, ()>>,
    cleanup: Option<Cleanup<E>>,
    notification: Option<prepared_notification::Registration>,
    cancelled: Option<OwnedCancellationWait>,
}

pub(crate) struct SelectedCacheEntry<K: Send + Sync + 'static, A: Send + 'static, E: Send + 'static> {
    pub(crate) entry: IndexEntry<K, A, E>,
    observation: CacheObservation,
    reservation: ObservationReservation<E>,
    independent_task: Option<ScheduledTask>,
}
enum SnapshotKind {
    Lookup,
    Maintenance { allow_closed: bool },
}

fn retain_selected<K: Send + Sync + 'static, A: Send + 'static, E: Send + 'static>(
    entries: &mut CacheIndex<K, A, E>,
    mut choose: impl FnMut(&IndexEntry<K, A, E>) -> bool,
) {
    let mut index = 0;
    while let Some(entry) = entries.get(index) {
        if choose(entry) && !entries.iter().take(index).any(|earlier| earlier.same_entry(entry)) {
            index += 1;
        } else {
            drop(entries.remove(index));
        }
    }
}

struct PreparedIndexSelection<K: Send + Sync + 'static, A: Send + 'static, E: Send + 'static> {
    index: PreparedCacheIndex<K, A, E>,
    payloads: Option<Payloads<K, A, E>>,
    on_error: Arc<dyn Fn(MemoryError) -> SharedValue<E> + Send + Sync>,
    on_active: Option<Arc<dyn Fn() -> SharedValue<E> + Send + Sync>>,
    active_limit: Option<NonZeroUsize>,
    independent: bool,
}
impl<K: Send + Sync + 'static, A: Send + 'static, E: Send + 'static>
    PreparedIndexSelection<K, A, E>
{
    fn maximum(&self, admission: &CacheAdmission<E>) -> Option<NonZeroUsize> {
        self.active_limit.or_else(|| admission.maximum())
    }
}

/// Prebuilt refusal callbacks for independent memory and active-load limits.
/// A callback can clone a startup-provisioned shared value for its exact reason.
pub struct PreparedCacheRefusals<E: Send + 'static> {
    memory: Arc<dyn Fn(MemoryError) -> SharedValue<E> + Send + Sync>,
    active: Arc<dyn Fn() -> SharedValue<E> + Send + Sync>,
}

impl<E: Send + 'static> Clone for PreparedCacheRefusals<E> {
    fn clone(&self) -> Self {
        Self {
            memory: Arc::clone(&self.memory),
            active: Arc::clone(&self.active),
        }
    }
}

impl<E: Send + 'static> PreparedCacheRefusals<E> {
    /// Select already shared results for the distinct refusal channels.
    #[must_use]
    pub fn new(
        memory: impl Fn(MemoryError) -> SharedValue<E> + Send + Sync + 'static,
        active: impl Fn() -> SharedValue<E> + Send + Sync + 'static,
    ) -> Self {
        Self {
            memory: Arc::new(memory),
            active: Arc::new(active),
        }
    }

    /// Apply the caller's shared memory-refusal policy to the original reason.
    #[must_use]
    pub fn memory(&self, reason: MemoryError) -> SharedValue<E> {
        (self.memory)(reason)
    }
}

#[derive(Clone)]
enum CacheRevision {
    Ordinary(Arc<()>),
    Prepared(u128),
    Exhausted,
}
impl CacheRevision {
    fn matches(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Ordinary(left), Self::Ordinary(right)) => Arc::ptr_eq(left, right),
            (Self::Prepared(left), Self::Prepared(right)) => left == right,
            _ => false,
        }
    }

    fn next(&self) -> Self {
        match self {
            Self::Ordinary(_) => Self::Ordinary(Arc::new(())),
            Self::Prepared(version) => version.checked_add(1).map_or(Self::Exhausted, Self::Prepared),
            Self::Exhausted => Self::Exhausted,
        }
    }
}
/// Type alias for diagnostic in cache.
///
/// # Type parameters
/// - `K`: Key type used for identity or lookup.
/// - `A`: Application value type.
/// - `E`: Domain-error type.
struct Diagnostic<K: Send + Sync + 'static, A: Send + 'static, E: Send + 'static> {
    entry: IndexEntry<K, A, E>,
    _permit: Option<SlotLease<'static, ()>>,
}
fn take_diagnostics<K: Send + Sync + 'static, A: Send + 'static, E: Send + 'static>(
    pending: &mut CacheList<Diagnostic<K, A, E>>,
    selected: &CacheIndex<K, A, E>,
    removed: &mut CacheList<Diagnostic<K, A, E>>,
) {
    let mut index = 0;
    while let Some(diagnostic) = pending.get(index) {
        if selected.iter().any(|entry| entry.same_entry(&diagnostic.entry))
            && let Some(diagnostic) = pending.remove(index)
        {
            removed.push_reserved(diagnostic);
        } else {
            index += 1;
        }
    }
}
/// Variants of submission used by cache.
enum Submission {
    Pending(Option<Defect>),
    Accepted,
    Finished,
}
/// Type alias for start load in cache.
///
/// # Type parameters
/// - `K`: Key type used for identity or lookup.
/// - `A`: Application value type.
/// - `E`: Domain-error type.
type StartLoad<K, A, E> = Arc<
    dyn Fn(IndexEntry<K, A, E>, Weak<Inner<K, A, E>>, Context, Option<ScheduledTask>) + Send + Sync,
>;
type LoadEffect<K, A, E> = Arc<dyn Fn(&EntryKey<K>, &Context, Finish<A, E>) -> Step + Send + Sync>;

struct PreparedLoader<K: Send + Sync + 'static, A: Send + 'static, E: Send + 'static> {
    entry: IndexEntry<K, A, E>,
    context: Context,
    evaluate: runtime::PreparedNestedReservation<A, E>,
    origin: runtime::ParallelContextReservation,
    load: LoadEffect<K, A, E>,
}

impl<K: Send + Sync + 'static, A: Send + 'static, E: Send + 'static> PreparedLoader<K, A, E> {
    async fn run(self) {
        let context = self
            .context
            .parallel_prepared(self.entry.cancellation.clone(), self.origin);
        let outcome = AssertUnwindSafe(runtime::evaluate_context_reserved(
            &context,
            self.evaluate,
            |finish| (self.load)(&self.entry.key, &context, finish),
        ))
        .catch_unwind()
        .await
        .unwrap_or_else(|panic| Exit::Die(Defect::from_panic(panic)));
        *self
            .entry
            .produced
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(outcome);
    }
}

struct LoaderNotice<K: Send + Sync + 'static, A: Send + 'static, E: Send + 'static> {
    entry: IndexEntry<K, A, E>,
    owner: Weak<Inner<K, A, E>>,
}

struct LoaderInput<K: Send + Sync + 'static, A: Send + 'static, E: Send + 'static> {
    entry: IndexEntry<K, A, E>,
    owner: Weak<Inner<K, A, E>>,
    context: Context,
}

impl<K: Send + Sync + 'static, A: Send + 'static, E: Send + 'static> LoaderNotice<K, A, E> {
    fn complete(self, event: PreparedTaskCompletion) {
        let outcome = self
            .entry
            .produced
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        match event {
            PreparedTaskCompletion::Ready => {
                let outcome = outcome.unwrap_or_else(|| {
                    Exit::Die(Defect::from(LibraryDefect::MissingInterpreterOutcome))
                });
                settle(&self.entry, &self.owner, outcome);
            }
            PreparedTaskCompletion::Lost(defect) => {
                self.lost(defect);
                drop(outcome);
            }
        }
    }

    fn lost(&self, defect: Defect) {
        let publish = {
            let mut admission = self
                .entry
                .submission
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            match &mut *admission {
                Submission::Pending(pending) => {
                    *pending = Some(defect);
                    None
                }
                Submission::Accepted => {
                    *admission = Submission::Finished;
                    Some(defect)
                }
                Submission::Finished => None,
            }
        };
        if let Some(defect) = publish {
            settle(&self.entry, &self.owner, Exit::Die(defect));
        }
    }
}

fn loader_layout<K: Send + Sync + 'static, A: Send + 'static, E: Send + 'static>()
-> Result<Layout, MemoryError> {
    fn future_layout<I, F: Future<Output = ()>>(_: fn(I) -> F) -> Layout {
        Layout::new::<F>()
    }
    let (result, finish) = runtime::PreparedNestedReservation::<A, E>::layouts();
    let origin = runtime::ParallelContextReservation::layout();
    let future = future_layout(PreparedLoader::<K, A, E>::run);
    let notification =
        ReservedOnce::<PreparedTaskCompletion, ()>::layout::<LoaderNotice<K, A, E>>();
    let size = result
        .size()
        .max(finish.size())
        .max(origin.size())
        .max(future.size())
        .max(notification.size());
    let alignment = result
        .align()
        .max(finish.align())
        .max(origin.align())
        .max(future.align())
        .max(notification.align());
    Layout::from_size_align(size, alignment).map_err(|_| MemoryError::InvalidLayout)
}

fn admit_loader<K: Send + Sync + 'static, A: Send + 'static, E: Send + 'static>(
    frames: &FramePool<'static>,
    load: LoadEffect<K, A, E>,
) -> Result<impl FnOnce(LoaderInput<K, A, E>) -> ScheduledTask + use<K, A, E>, MemoryError> {
    fn reserve_future<I, F: Future<Output = ()> + Send + 'static>(
        frames: &FramePool<'static>,
        _: fn(I) -> F,
    ) -> Result<crate::memory::FrameReservation<'static, F>, MemoryError> {
        frames.reserve()
    }
    let future = reserve_future(frames, PreparedLoader::<K, A, E>::run)?;
    let notice =
        ReservedOnce::<PreparedTaskCompletion, ()>::reserve::<LoaderNotice<K, A, E>>(frames)?;
    let evaluate = runtime::PreparedNestedReservation::reserve(frames)?;
    let origin = runtime::ParallelContextReservation::reserve(frames)?;
    Ok(
        move |LoaderInput {
                  entry,
                  owner,
                  context,
              }| {
            let runner = PreparedLoader {
                entry: entry.clone(),
                context,
                evaluate,
                origin,
                load,
            };
            ScheduledTask::prepared(
                PreparedUnitFuture::initialize(future, runner.run()),
                notice.initialize(LoaderNotice { entry, owner }, LoaderNotice::complete),
            )
        },
    )
}
/// Typed cache state used by cache.
///
/// # Type parameters
/// - `K`: Key type used for identity or lookup. Bound or type: `Send + Sync + 'static`.
/// - `A`: Application value type. Bound or type: `Send + 'static`.
/// - `E`: Domain-error type. Bound or type: `Send + 'static`.
struct CacheState<K: Send + Sync + 'static, A: Send + 'static, E: Send + 'static> {
    closed: bool,
    revision: CacheRevision,
    retained: CacheIndex<K, A, E>,
    active: CacheIndex<K, A, E>,
    diagnostics: CacheList<Diagnostic<K, A, E>>,
    shutdown: CacheShutdown<E>,
    shutdown_index: CacheIndex<K, A, E>,
    shutdown_retirement: Option<RetirementReservation<E>>,
    owner_loss: Option<Defect>,
}
impl<K: Send + Sync + 'static, A: Send + 'static, E: Send + 'static> CacheState<K, A, E> {
    fn advance_revision(&mut self) {
        self.revision = self.revision.next();
        if matches!(self.revision, CacheRevision::Exhausted) {
            self.closed = true;
        }
    }
}
/// Variants of cache shutdown used by cache.
///
/// # Type parameters
/// - `E`: Domain-error type. Bound or type: `Send + 'static`.
enum CacheShutdown<E: Send + 'static> {
    Open,
    Closing,
    Closed(Option<Exit<(), SharedValue<E>>>),
}
/// Typed inner used by cache.
///
/// # Type parameters
/// - `K`: Key type used for identity or lookup. Bound or type: `Send + Sync + 'static`.
/// - `A`: Application value type. Bound or type: `Send + 'static`.
/// - `E`: Domain-error type. Bound or type: `Send + 'static`.
struct Inner<K: Send + Sync + 'static, A: Send + 'static, E: Send + 'static> {
    state: Mutex<CacheState<K, A, E>>,
    changed: Option<Event>,
    parent: LoaderCancellation,
    config: CacheConfig<K, E>,
    options: CacheOptions<A, E>,
    now: Arc<dyn Fn() -> Duration + Send + Sync>,
    start: StartLoad<K, A, E>,
    load_effect: LoadEffect<K, A, E>,
    prepared_index: Option<Arc<PreparedIndexSelection<K, A, E>>>,
}
/// A scoped cache that shares in-flight loads and retains their outcomes under an explicit policy.
///
/// # Type parameters
/// - `K`: Key type used for identity or lookup. Bound or type: `Send + Sync + 'static`.
/// - `A`: Application value type. Bound or type: `Send + 'static`.
/// - `E`: Domain-error type. Bound or type: `Send + 'static`.
pub struct Cache<K: Send + Sync + 'static, A: Send + 'static, E: Send + 'static> {
    inner: Arc<Inner<K, A, E>>,
}
impl<K: Send + Sync + 'static, A: Send + 'static, E: Send + 'static> Clone for Cache<K, A, E> {
    ///
    /// # Returns
    /// Type: `Self`.
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}
///
/// # Type parameters
/// - `A`: Bounds: `Send + 'static`.
/// - `E`: Bounds: `Send + 'static`.
///
/// # Parameters
/// - `outcome` (`Exit<A, E>`): Complete terminal outcome, including expected errors and cleanup failures.
///
/// # Returns
/// Type: `SharedOutcome<A, E>`.
/// Returns `SharedOutcome<A, E>`.
fn share<A: Send + 'static, E: Send + 'static>(outcome: Exit<A, E>) -> SharedOutcome<A, E> {
    let outcome = outcome.map_error(SharedValue::new);
    match outcome {
        Exit::Success(value) => Exit::Success(SharedValue::new(value)),
        Exit::Fail(error) => Exit::Fail(error),
        Exit::Die(defect) => Exit::Die(defect),
        Exit::Interrupted => Exit::Interrupted,
        Exit::FinalizationFailed(details) => Exit::FinalizationFailed(details),
    }
}
///
/// # Type parameters
/// - `K`: Bounds: `Send + Sync + 'static`.
/// - `A`: Bounds: `Send + 'static`.
/// - `E`: Bounds: `Send + 'static`.
///
/// # Parameters
/// - `entry` (`&IndexEntry<K, A, E>`): Stored record or event selected for this transition.
/// - `owner` (`&Weak<Inner<K, A, E>>`): Worker or scope identity used to fence stale operations.
/// - `original` (`Exit<A, E>`): The original supplied to this operation.
///
/// # Returns
/// Type: `()`.
fn settle<K: Send + Sync + 'static, A: Send + 'static, E: Send + 'static>(
    entry: &IndexEntry<K, A, E>,
    owner: &Weak<Inner<K, A, E>>,
    original: Exit<A, E>,
) {
    let result = catch_unwind(AssertUnwindSafe(|| {
        let keep = match &original {
            Exit::Success(_) => true,
            Exit::Fail(error) => owner
                .upgrade()
                .is_some_and(|owner| (owner.config.cache_failure)(error)),
            Exit::Die(_) | Exit::Interrupted | Exit::FinalizationFailed(_) => false,
        };
        let expires = match owner.upgrade() {
            Some(owner) if keep => {
                let ttl = match &owner.options.ttl {
                    CacheTtl::Fixed(ttl) => *ttl,
                    CacheTtl::Outcome(callback) => callback(&original)?,
                };
                ttl.map(|ttl| {
                    (owner.now)()
                        .checked_add(ttl)
                        .ok_or_else(|| Defect::from(LibraryDefect::ClockDeadlineOverflow))
                })
                .transpose()?
            }
            Some(_) | None => None,
        };
        Ok::<_, Defect>((keep, expires))
    }));
    let (ttl_failure, mut keep, expires) = match result {
        Ok(Ok((keep, expires))) => (None, keep, expires),
        Ok(Err(defect)) => (Some(defect), false, None),
        Err(panic) => (Some(Defect::from_panic(panic)), false, None),
    };
    let storage = entry.outcome_storage.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner).take();
    let Some(storage) = storage else { return };
    let outcome = catch_unwind(AssertUnwindSafe(|| storage.publish(original, ttl_failure)))
        .unwrap_or_else(|panic| {
            keep = false;
            Exit::Die(Defect::from_panic(panic))
        });
    let independent = owner.upgrade().is_some_and(|owner| owner.prepared_index.as_ref().is_some_and(|selection| selection.independent));
    let publish = || {
        let mut state = entry.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.outcome.is_some() {
            return Err(outcome);
        }
        let diagnostic = !independent && !state.retiring && !state.observed
            && matches!(outcome, Exit::FinalizationFailed(_));
        state.outcome = Some(outcome);
        state.expires = expires;
        Ok((state.invalidated, diagnostic, state.diagnostic.take()))
    };
    if let Some(owner) = owner.upgrade() {
        // Serialize publication against eviction. Each transition can add only one
        // completed retained element, so at most one element must retire.
        let mut state = owner.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let (invalidated, diagnostic, mut permit) = match publish() {
            Ok(publication) => publication,
            Err(discarded) => {
                drop(state);
                drop(discarded);
                return;
            }
        };
        state.active.retain(|candidate| !candidate.same_entry(entry));
        if diagnostic {
            state.diagnostics.push_reserved(Diagnostic {
                entry: entry.clone(),
                _permit: permit.take(),
            });
        }
        let drop_current = !keep || invalidated || state.closed
            || matches!(owner.config.capacity, CacheCapacity::NoRetention);
        let discarded_position = if drop_current {
            state.retained.iter().position(|candidate| candidate.same_entry(entry))
        } else if let CacheCapacity::Bounded(capacity) = owner.config.capacity {
            let mut completed = state.retained.iter().enumerate().filter_map(|(index, candidate)| {
                candidate.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
                    .outcome.is_some().then_some(index)
            });
            let oldest = completed.next();
            if completed.count() >= capacity.get() { oldest } else { None }
        } else {
            None
        };
        let discarded = discarded_position.and_then(|position| state.retained.remove(position));
        state.advance_revision();
        drop(state);
        let notify = std::mem::DropGuard::new((), |_| {
            notify_entry(entry);
            if let Some(payloads) = owner.prepared_index.as_ref().and_then(|selection| selection.payloads.as_ref()) {
                payloads.changed.notify_all();
            }
            if let Some(changed) = &owner.changed {
                notify_all(changed);
            }
        });
        drop(discarded);
        drop(permit);
        drop(notify);
    } else if publish().is_ok() {
        notify_entry(entry);
    }
}

fn notify_entry<K: Send + Sync + 'static, A: Send + 'static, E: Send + 'static>(
    entry: &IndexEntry<K, A, E>,
) {
    entry.cancellation.unlink();
    if let Some(changed) = &entry.changed {
        notify_all(changed);
    }
}
impl<K: Send + Sync + 'static, A: Send + 'static, E: Send + 'static> Cache<K, A, E> {
    /// Lazily construct a scoped cache with legacy fixed TTL and unbounded active admission.
    ///
    /// R supplies clock and task spawner. Retention still follows config.capacity and
    /// cache_failure. Use make_with_options for explicit active admission and outcome
    /// TTL. Scope closure cancels and joins owned producers.
    pub fn make<R: HasClock + HasTaskSpawner + Send + Sync + 'static>(
        scope: Scope<SharedValue<E>>,
        config: CacheConfig<K, E>,
        load: impl Fn(&K) -> Effect<A, E, R> + Send + Sync + 'static,
    ) -> Effect<Self, SharedValue<E>, R> {
        let options = CacheOptions {
            ttl: CacheTtl::Fixed(config.ttl),
            admission: CacheAdmission::Unbounded,
        };
        cache! {
            scope: scope,
            config: config,
            options: options,
            load: load,
        }
    }

    /// Lazily create a scoped cache with explicit retention, expiry and active admission.
    ///
    /// `R` supplies clock and task-spawner capabilities and is retained by the engine.
    /// Equivalent keys share one producer. Active capacity is checked before dispatch;
    /// outcome TTL and failure-retention callbacks run outside the state gate. The
    /// scope owns shutdown and joins producer cleanup, including abandonment.
    pub fn make_with_options<R: HasClock + HasTaskSpawner + Send + Sync + 'static>(
        scope: Scope<SharedValue<E>>,
        config: CacheConfig<K, E>,
        options: CacheOptions<A, E>,
        load: impl Fn(&K) -> Effect<A, E, R> + Send + Sync + 'static,
    ) -> Effect<Self, SharedValue<E>, R> {
        Self::make_with_shared_key(scope, config, options, move |key| load(&key))
    }

    /// Create the same scoped cache using a shared owned key for a deferred loader body.
    /// Sharing the engine's existing Arc key avoids a hidden key Clone requirement and
    /// permits the loader to retain it across asynchronous Effect binds. Policies and
    /// producer ownership are identical to `make_with_options`; no payload is copied.
    pub fn make_with_shared_key<R: HasClock + HasTaskSpawner + Send + Sync + 'static>(
        scope: Scope<SharedValue<E>>,
        config: CacheConfig<K, E>,
        options: CacheOptions<A, E>,
        load: impl Fn(Arc<K>) -> Effect<A, E, R> + Send + Sync + 'static,
    ) -> Effect<Self, SharedValue<E>, R> {
        Self::make_with_index(scope, config, options, None, Self::ordinary_loader(load))
    }

    /// Select a physically prepared membership index and fixed lookup snapshots
    /// with an Arc-key loader. [`Self::make_prepared`] also selects inline
    /// key and outcome storage.
    pub fn make_with_prepared_index<R: HasClock + HasTaskSpawner + Send + Sync + 'static>(
        scope: Scope<SharedValue<E>>,
        config: CacheConfig<K, E>,
        options: CacheOptions<A, E>,
        index: PreparedCacheIndex<K, A, E>,
        on_error: impl Fn(MemoryError) -> E + Send + Sync + 'static,
        load: impl Fn(Arc<K>) -> Effect<A, E, R> + Send + Sync + 'static,
    ) -> Effect<Self, SharedValue<E>, R> {
        Self::make_with_index(
            scope,
            config,
            options,
            Some(Arc::new(PreparedIndexSelection {
                index,
                payloads: None,
                on_error: Arc::new(move |error| SharedValue::new(on_error(error))),
                on_active: None,
                active_limit: None,
                independent: false,
            })),
            Self::ordinary_loader(load),
        )
    }

    /// Create a cache with region-backed indexes and inline key and outcome storage.
    /// The loader receives a shared key handle it may retain across asynchronous work.
    pub fn make_prepared<R: HasClock + HasTaskSpawner + Send + Sync + 'static>(
        scope: Scope<SharedValue<E>>,
        config: CacheConfig<K, E>,
        options: CacheOptions<A, E>,
        storage: PreparedCacheStorage<K, A, E>,
        on_capacity: impl Fn(MemoryError) -> E + Send + Sync + 'static,
        load: impl Fn(CacheKey<K>) -> Effect<A, E, R> + Send + Sync + 'static,
    ) -> Effect<Self, SharedValue<E>, R> {
        Self::make_prepared_inner(
            scope,
            config,
            options,
            PreparedIndexSelection {
                index: storage.index,
                payloads: Some(storage.payloads),
                on_error: Arc::new(move |error| SharedValue::new(on_capacity(error))),
                on_active: None,
                active_limit: None,
                independent: false,
            },
            load,
        )
    }

    /// Create a prepared cache with shared results for memory and active-limit refusals.
    pub fn make_prepared_with_refusals<R: HasClock + HasTaskSpawner + Send + Sync + 'static>(
        scope: Scope<SharedValue<E>>,
        config: CacheConfig<K, E>,
        options: CacheOptions<A, E>,
        storage: PreparedCacheStorage<K, A, E>,
        refusals: PreparedCacheRefusals<E>,
        load: impl Fn(CacheKey<K>) -> Effect<A, E, R> + Send + Sync + 'static,
    ) -> Effect<Self, SharedValue<E>, R> {
        Self::make_prepared_inner(
            scope,
            config,
            options,
            PreparedIndexSelection {
                index: storage.index,
                payloads: Some(storage.payloads),
                on_error: refusals.memory,
                on_active: Some(refusals.active),
                active_limit: None,
                independent: false,
            },
            load,
        )
    }

    /// Prepare independent calls with no retained membership. Closing drains
    /// accepted loaders and caller cancellation still joins their owned cleanup.
    pub fn make_prepared_independent<R: HasClock + HasTaskSpawner + Send + Sync + 'static>(
        scope: Scope<SharedValue<E>>,
        admission: CacheAdmission<E>,
        storage: PreparedCacheStorage<K, A, E>,
        refusals: PreparedCacheRefusals<E>,
        load: impl Fn(CacheKey<K>) -> Effect<A, E, R> + Send + Sync + 'static,
    ) -> Effect<Self, SharedValue<E>, R> {
        Self::make_prepared_inner(
            scope,
            CacheConfig { equal: Arc::new(|_, _| false), ttl: None, capacity: CacheCapacity::NoRetention, cache_failure: Arc::new(|_| false) },
            CacheOptions { ttl: CacheTtl::Fixed(None), admission },
            PreparedIndexSelection {
                index: storage.index, payloads: Some(storage.payloads), on_error: refusals.memory,
                on_active: Some(refusals.active), active_limit: None, independent: true,
            },
            load,
        )
    }

    pub(crate) fn make_prepared_with_active_refusals<
        R: HasClock + HasTaskSpawner + Send + Sync + 'static,
    >(
        scope: Scope<SharedValue<E>>,
        config: CacheConfig<K, E>,
        ttl: CacheTtl<A, E>,
        active: NonZeroUsize,
        storage: PreparedCacheStorage<K, A, E>,
        refusals: PreparedCacheRefusals<E>,
        load: impl Fn(CacheKey<K>) -> Effect<A, E, R> + Send + Sync + 'static,
    ) -> Effect<Self, SharedValue<E>, R> {
        Self::make_prepared_inner(
            scope,
            config,
            CacheOptions {
                ttl,
                admission: CacheAdmission::Unbounded,
            },
            PreparedIndexSelection {
                index: storage.index,
                payloads: Some(storage.payloads),
                on_error: refusals.memory,
                on_active: Some(refusals.active),
                active_limit: Some(active),
                independent: false,
            },
            load,
        )
    }

    fn make_prepared_inner<R: HasClock + HasTaskSpawner + Send + Sync + 'static>(
        scope: Scope<SharedValue<E>>,
        config: CacheConfig<K, E>,
        options: CacheOptions<A, E>,
        selection: PreparedIndexSelection<K, A, E>,
        load: impl Fn(CacheKey<K>) -> Effect<A, E, R> + Send + Sync + 'static,
    ) -> Effect<Self, SharedValue<E>, R> {
        Self::make_with_index(
            scope,
            config,
            options,
            Some(Arc::new(selection)),
            move |key| match key {
                EntryKey::Prepared(key) => load(key.clone()),
                EntryKey::Ordinary(_) => {
                    crate::effect_die! { defect: Defect::from(LibraryDefect::MissingInterpreterOutcome), }
                }
            },
        )
    }

    fn ordinary_loader<R: Send + Sync + 'static>(
        load: impl Fn(Arc<K>) -> Effect<A, E, R> + Send + Sync + 'static,
    ) -> impl Fn(&EntryKey<K>) -> Effect<A, E, R> + Send + Sync + 'static {
        move |key| match key {
            EntryKey::Ordinary(key) => load(Arc::clone(key)),
            EntryKey::Prepared(_) => {
                crate::effect_die! { defect: Defect::from(LibraryDefect::MissingInterpreterOutcome), }
            }
        }
    }

    fn make_with_index<R: HasClock + HasTaskSpawner + Send + Sync + 'static>(
        scope: Scope<SharedValue<E>>,
        config: CacheConfig<K, E>,
        options: CacheOptions<A, E>,
        prepared_index: Option<Arc<PreparedIndexSelection<K, A, E>>>,
        load: impl Fn(&EntryKey<K>) -> Effect<A, E, R> + Send + Sync + 'static,
    ) -> Effect<Self, SharedValue<E>, R> {
        let load = Arc::new(load);
        Effect::new(move |env: Arc<R>, context, finish| {
            let (retained, active, diagnostics, shutdown_index, shutdown_retirement, shutdown_wait) = match &prepared_index {
                Some(selection) => {
                    let required = match selection.maximum(&options.admission) {
                        Some(maximum) if maximum <= selection.index.maximum_active => {
                            prepared_index_width(config.capacity, maximum)
                        }
                        Some(_) => Err(MemoryError::NoSlot),
                        None => Err(MemoryError::UnsupportedRunContract),
                    };
                    match required.and_then(|required| {
                        if required > selection.index.width {
                            return Err(MemoryError::NoSlot);
                        }
                        Ok((selection.index.reserve()?, selection.index.reserve_active()?,
                            selection.index.reserve_diagnostics()?, selection.index.reserve_shutdown()?,
                            selection.payloads.as_ref().map(RetirementReservation::reserve).transpose()?,
                            selection.payloads.as_ref().map(|payloads| payloads.closed.listen()).transpose()?))
                    }) {
                        Ok(retained) => retained,
                        Err(error) => return runtime::finish(finish, Exit::Fail((selection.on_error)(error))),
                    }
                }
                None => Default::default(),
            };
            let changed = shutdown_wait.is_none().then(Event::new);
            let parent = match &prepared_index {
                Some(selection) if let Some(payloads) = &selection.payloads => match payloads.cancellation.bind(&context.cancellation) {
                    Ok(parent) => parent,
                    Err(error) => return runtime::finish(finish, Exit::Fail((selection.on_error)(error))),
                },
                Some(_) | None => LoaderCancellation::ordinary(&context.cancellation),
            };
            let load_env = Arc::clone(&env);
            let prepared_env = Arc::clone(&env);
            let now_env = env;
            let load = Arc::clone(&load);
            let prepared_load = Arc::clone(&load);
            let cache = Self {
                inner: Arc::new(Inner {
                    state: Mutex::new(CacheState {
                        closed: false,
                        revision: if prepared_index.is_some() { CacheRevision::Prepared(0) } else { CacheRevision::Ordinary(Arc::new(())) },
                        retained,
                        active,
                        diagnostics,
                        shutdown: CacheShutdown::Open,
                        shutdown_index,
                        shutdown_retirement,
                        owner_loss: None,
                    }),
                    changed,
                    parent,
                    config: config.clone(),
                    options: options.clone(),
                    prepared_index: prepared_index.clone(),
                    now: Arc::new(move || now_env.clock().now()),
                    load_effect: Arc::new(move |key, context, finish| {
                        let program = prepared_load(key);
                        program.execute(Arc::clone(&prepared_env), context.clone(), finish)
                    }),
                    start: Arc::new(move |entry, owner, context, prepared| {
                        let submission_owner = owner.clone();
                        let task = prepared.unwrap_or_else(|| {
                            let env = Arc::clone(&load_env);
                            let load = Arc::clone(&load);
                            let completed = entry.clone();
                            let completed_owner = owner.clone();
                            let lost = LoaderNotice { entry: entry.clone(), owner };
                            ScheduledTask::new(
                                move |settlement| {
                                    Box::pin(async move {
                                        let outcome = AssertUnwindSafe(async {
                                            let program = load(&completed.key);
                                            runtime::evaluate_context(program, env, &context).await
                                        })
                                        .catch_unwind()
                                        .await
                                        .unwrap_or_else(|panic| Exit::Die(Defect::from_panic(panic)));
                                        settlement.release();
                                        settle(&completed, &completed_owner, outcome);
                                    })
                                },
                                move |event| {
                                    if let TaskEvent::Lost(defect) = event {
                                        lost.lost(defect);
                                    }
                                },
                            )
                        });
                        let submitted =
                            catch_unwind(AssertUnwindSafe(|| load_env.task_spawner().spawn(task)));
                        let failure = {
                            let mut admission = entry.submission
                                .lock()
                                .unwrap_or_else(std::sync::PoisonError::into_inner);
                            match submitted {
                                Ok(Ok(())) => {
                                    match std::mem::replace(&mut *admission, Submission::Accepted) {
                                        Submission::Pending(pending) => pending,
                                        Submission::Accepted | Submission::Finished => None,
                                    }
                                }
                                Ok(Err(defect)) => {
                                    *admission = Submission::Finished;
                                    Some(defect)
                                }
                                Err(panic) => {
                                    *admission = Submission::Finished;
                                    Some(Defect::from_panic(panic))
                                }
                            }
                        };
                        if let Some(defect) = failure {
                            settle(&entry, &submission_owner, Exit::Die(defect));
                        }
                    }),
                }),
            };
            let close = cache.clone();
            let close_context = context.protected();
            let abandoned = cache.clone();
            let finalizer = Registered::with_abandon(
                move |finish| {
                    Step::suspended(Box::pin(async move {
                        let cleanup = close.shutdown_owned(close_context, shutdown_wait).await;
                        runtime::finish(finish, cleanup)
                    }))
                },
                move || abandoned.abandon(),
            );
            scope.register_or_release(finalizer, cache, finish)
        })
    }
    /// # Type parameters
    /// - `K`: Key type used for identity or lookup. Bound: `Send + Sync + 'static`.
    /// - `A`: Application value type. Bound: `Send + 'static`.
    /// - `E`: Domain-error type. Bound: `Send + 'static`.
    ///
    /// # Returns
    /// Type: `bool`.
    /// Returns whether the requested condition holds.
    pub(crate) fn is_closed(&self) -> bool {
        self.inner
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .closed
    }
    ///
    /// # Returns
    /// Type: `()`.
    fn abandon(&self) {
        {
            let mut state = self
                .inner
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            state.closed = true;
            state.owner_loss = Some(Defect::from(LibraryDefect::ScopeOwnerDropped));
            state.advance_revision();
            for entry in state.active.iter().chain(state.retained.iter()) {
                entry
                    .state
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .invalidated = true;
            }
        }
        let _ = self.inner.parent.request();
        self.notify_shutdown();
    }
    /// # Type parameters
    /// - `A`: Success or application-value type.
    /// - `E`: Domain-error type.
    /// - `K`: Type supplied by the enclosing impl or trait.
    ///
    /// # Parameters
    /// - `key` (`EntryKey<K>`): Stable key used to locate or identify the selected value.
    /// - `context` (`&Context`): Execution context carrying cancellation, scope, and trace state.
    ///
    /// # Returns
    /// Type: `Result<Option<SelectedCacheEntry<K, A, E>>, CacheSelectionError<E>>`.
    pub(crate) async fn select_entry(
        &self,
        key: EntryKey<K>,
        context: &Context,
    ) -> Result<Option<SelectedCacheEntry<K, A, E>>, CacheSelectionError<E>> {
        self.select_observed_entry(key, context).await
    }

    fn payloads(&self) -> Option<&Payloads<K, A, E>> {
        self.inner.prepared_index.as_ref()?.payloads.as_ref()
    }

    /// Admit a key into this cache's selected memory. A refusal returns the
    /// original key. The returned handle may be reused with [`Self::get_prepared`].
    pub fn prepare_key(&self, key: K) -> Result<CacheKey<K>, (MemoryError, K)> {
        if self.is_closed() {
            return Err((MemoryError::Closed, key));
        }
        match self
            .payloads()
            .ok_or(MemoryError::UnsupportedRunContract)
            .and_then(|payloads| payloads.reserve_key().map(|lease| (payloads, lease)))
        {
            Ok((payloads, lease)) => Ok(payloads.publish_key(lease, key)),
            Err(error) => Err((error, key)),
        }
    }

    fn lookup_key(&self, key: K) -> LookupKey<K> {
        self.lookup_key_for(key, KeyPurpose::Lookup)
    }

    fn lookup_key_for(&self, key: K, purpose: KeyPurpose) -> LookupKey<K> {
        if self.payloads().is_some() {
            LookupKey::Pending(Arc::new(Mutex::new(RecipeKey::Pending(Some(key)))), purpose)
        } else {
            LookupKey::Owned(EntryKey::Ordinary(Arc::new(key)))
        }
    }

    fn admit_key(&self, key: LookupKey<K>) -> Result<EntryKey<K>, CacheSelectionError<E>> {
        self.admit_lookup_key(key, false)
    }

    fn admit_lookup_key(&self, key: LookupKey<K>, allow_closed: bool) -> Result<EntryKey<K>, CacheSelectionError<E>> {
        let key = match key {
            LookupKey::Owned(key) => key,
            LookupKey::Pending(key, purpose) => {
                if !allow_closed && self.is_closed() {
                    return Err(self.closed_error());
                }
                self.admit_recipe_key(&key, purpose).map_err(|error| self.index_error(error))?
            }
        };
        self.validate_key(&key)?;
        Ok(key)
    }

    fn admit_recipe_key(&self, key: &Mutex<RecipeKey<K>>, purpose: KeyPurpose) -> Result<EntryKey<K>, MemoryError> {
        let payloads = self.payloads().ok_or(MemoryError::UnsupportedRunContract)?;
        let mut recipe = key.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if let RecipeKey::Pending(pending) = &mut *recipe {
            let lease = match purpose {
                KeyPurpose::Lookup => payloads.reserve_key()?,
                KeyPurpose::Maintenance => payloads.reserve_maintenance_key()?,
            };
            let value = pending.take().ok_or(MemoryError::UnsupportedRunContract)?;
            *recipe = RecipeKey::Admitted(payloads.publish_key(lease, value));
        }
        match &*recipe {
            RecipeKey::Admitted(key) => Ok(EntryKey::Prepared(key.clone())),
            RecipeKey::Pending(_) => Err(MemoryError::UnsupportedRunContract),
        }
    }

    fn validate_key(&self, key: &EntryKey<K>) -> Result<(), CacheSelectionError<E>> {
        match (key, self.payloads()) {
            (EntryKey::Ordinary(_), None) => Ok(()),
            (EntryKey::Prepared(key), Some(payloads)) if payloads.owns_key(key) => Ok(()),
            _ => Err(self.index_error(MemoryError::UnsupportedRunContract)),
        }
    }

    fn reserve_entry(&self) -> Result<Option<EntryLease<K, A, E>>, MemoryError> {
        self.payloads().map(Payloads::reserve_entry).transpose()
    }

    fn reserve_outcome(&self) -> Result<OutcomeStorage<A, E>, MemoryError> {
        self.payloads()
            .map_or(Ok(OutcomeStorage::Ordinary), Payloads::reserve_outcome)
    }

    pub(crate) fn share_expected_error(&self, error: E) -> Result<SharedValue<E>, MemoryError> {
        if let Some(payloads) = self.payloads() {
            Ok(SharedValue::from_reserved(payloads.reserve_error()?, error))
        } else {
            Ok(SharedValue::new(error))
        }
    }

    fn reserve_cleanup(&self) -> Result<Option<Cleanup<E>>, MemoryError> {
        self.payloads().map(Payloads::reserve_cleanup).transpose()
    }
    fn try_observer(&self) -> Result<Option<SlotLease<'static, ()>>, MemoryError> {
        self.inner
            .prepared_index
            .as_ref()
            .map(|selection| selection.index.observers.try_checkout())
            .transpose()
    }

    pub(crate) fn select_existing(
        &self,
        entry: IndexEntry<K, A, E>,
        cancellation: &Cancellation,
    ) -> Result<SelectedCacheEntry<K, A, E>, CacheSelectionError<E>> {
        let (pending, compound) = {
            let state = entry
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            (
                state.outcome.is_none(),
                matches!(state.outcome, Some(Exit::FinalizationFailed(_))),
            )
        };
        let reservation = self
            .reserve_observation(pending, compound, cancellation)
            .map_err(|error| self.index_error(error))?;
        let observation = if pending {
            CacheObservation::Shared
        } else {
            CacheObservation::Retained
        };
        Ok(SelectedCacheEntry {
            entry,
            observation,
            reservation,
            independent_task: None,
        })
    }

    fn reserve_observation(
        &self,
        pending: bool,
        compound: bool,
        cancellation: &Cancellation,
    ) -> Result<ObservationReservation<E>, MemoryError> {
        let observer = if pending { self.try_observer()? } else { None };
        let cleanup = if pending || compound {
            self.reserve_cleanup()?
        } else {
            None
        };
        let notification = if pending {
            self.payloads()
                .map(|payloads| payloads.changed.listen())
                .transpose()?
        } else {
            None
        };
        let cancelled = if pending && self.payloads().is_some() {
            Some(cancellation.reserve_wait_owned()?)
        } else {
            None
        };
        Ok(ObservationReservation {
            observer,
            cleanup,
            notification,
            cancelled,
        })
    }

    fn independent(&self) -> bool {
        self.inner.prepared_index.as_ref().is_some_and(|selection| selection.independent)
    }

    fn closed_error(&self) -> CacheSelectionError<E> {
        if self.independent() {
            self.index_error(MemoryError::Closed)
        } else {
            CacheSelectionError::Defect(Defect::from(LibraryDefect::CacheClosed))
        }
    }

    fn active_limit(&self) -> Option<NonZeroUsize> {
        let admission = &self.inner.options.admission;
        self.inner.prepared_index.as_ref().map_or_else(
            || admission.maximum(),
            |selection| selection.maximum(admission),
        )
    }

    fn active_refusal(&self) -> CacheSelectionError<E> {
        if let Some(shared) = self
            .inner
            .prepared_index
            .as_ref()
            .and_then(|selection| selection.on_active.as_ref())
        {
            return CacheSelectionError::Capacity(shared());
        }
        match &self.inner.options.admission {
            CacheAdmission::Bounded { capacity_error, .. } => {
                CacheSelectionError::Capacity(SharedValue::new(capacity_error()))
            }
            CacheAdmission::Unbounded => {
                CacheSelectionError::Defect(Defect::from(LibraryDefect::MissingInterpreterOutcome))
            }
        }
    }

    fn index_error(&self, error: MemoryError) -> CacheSelectionError<E> {
        match &self.inner.prepared_index {
            Some(selection) => CacheSelectionError::Capacity((selection.on_error)(error)),
            None => CacheSelectionError::Defect(Defect::from(
                LibraryDefect::PreparedConstructionUnbound,
            )),
        }
    }

    fn reserve_retired_diagnostics(&self) -> Result<CacheList<Diagnostic<K, A, E>>, CacheSelectionError<E>> {
        match &self.inner.prepared_index {
            Some(selection) => selection.index.reserve_diagnostics().map_err(|error| self.index_error(error)),
            None => Ok(CacheList::default()),
        }
    }

    fn snapshot_index(&self, kind: SnapshotKind) -> Result<CacheSnapshot<K, A, E>, CacheSelectionError<E>> {
        let (maintenance, allow_closed) = match kind {
            SnapshotKind::Lookup => (false, false),
            SnapshotKind::Maintenance { allow_closed } => (true, allow_closed),
        };
        let mut fresh = match &self.inner.prepared_index {
            Some(selection) => match if maintenance {
                selection.index.reserve_maintenance()
            } else {
                selection.index.reserve()
            } {
                Ok(fresh) => fresh,
                Err(_) if !allow_closed && self.is_closed() => {
                    return Err(self.closed_error());
                }
                Err(error) => return Err(self.index_error(error)),
            },
            None => CacheIndex::default(),
        };
        let state = self.inner.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if (state.closed && !allow_closed) || matches!(state.revision, CacheRevision::Exhausted) {
            drop(state);
            return Err(self.closed_error());
        }
        let required = if maintenance {
            state.retained.len().checked_add(state.active.len())
                .and_then(|count| count.checked_add(state.diagnostics.len()))
        } else {
            Some(state.retained.len())
        };
        if required.is_none_or(|required| required > fresh.remaining()) {
            drop(state);
            drop(fresh);
            return Err(self.index_error(MemoryError::NoSlot));
        }
        for entry in state.retained.iter() {
            fresh.push_reserved(entry.clone());
        }
        if maintenance {
            for entry in state.active.iter() {
                fresh.push_reserved(entry.clone());
            }
            for diagnostic in state.diagnostics.iter() {
                fresh.push_reserved(diagnostic.entry.clone());
            }
        }
        Ok(CacheSnapshot { revision: state.revision.clone(), entries: fresh })
    }

    async fn select_observed_entry(
        &self,
        key: EntryKey<K>,
        context: &Context,
    ) -> Result<Option<SelectedCacheEntry<K, A, E>>, CacheSelectionError<E>> {
        self.validate_key(&key)?;
        loop {
            if context.cancelled() {
                return Ok(None);
            }
            let CacheSnapshot { revision, entries: mut fresh } = self.snapshot_index(SnapshotKind::Lookup)?;
            let now = (self.inner.now)();
            fresh.retain(|entry| {
                let state = entry.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                !state.invalidated && state.expires.is_none_or(|expires| now < expires)
            });
            let found = fresh.iter()
                .find(|entry| (self.inner.config.equal)(&entry.key, &key))
                .cloned();
            let (selection, stale) = {
                let mut state = self.inner.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                if state.closed {
                    drop(state);
                    return Err(self.closed_error());
                }
                if !state.revision.matches(&revision) {
                    (None, Some(fresh))
                } else {
                    if found.is_none() {
                        if self.active_limit().is_some_and(|maximum| state.active.len() >= maximum.get()) {
                            drop(state);
                            drop(fresh);
                            return Err(self.active_refusal());
                        }
                        if fresh.remaining() == 0 || state.active.remaining() == 0 {
                            drop(state);
                            drop(fresh);
                            return Err(self.index_error(MemoryError::NoSlot));
                        }
                    }
                    let pending = found.as_ref().is_none_or(|entry| {
                        entry.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner).outcome.is_none()
                    });
                    let reservation = match self.reserve_observation(pending, false, &context.cancellation) {
                        Ok(reservation) => reservation,
                        Err(error) => {
                            drop(state);
                            drop(fresh);
                            return Err(self.index_error(error));
                        }
                    };
                    let (entry, start, prepared) = match found {
                        Some(entry) => {
                            fresh.retain(|candidate| !candidate.same_entry(&entry));
                            (entry, false, None)
                        }
                        None => {
                            let diagnostic = match &self.inner.prepared_index {
                                Some(selection) if !selection.independent => match selection.index.diagnostic_permits.try_checkout() {
                                    Ok(permit) => Some(permit),
                                    Err(error) => {
                                        drop(state);
                                        drop(fresh);
                                        drop(reservation);
                                        return Err(self.index_error(error));
                                    }
                                },
                                Some(_) | None => None,
                            };
                            let outcome_storage = match self.reserve_outcome() {
                                Ok(storage) => storage,
                                Err(error) => {
                                    drop(state);
                                    drop(fresh);
                                    drop(reservation);
                                    drop(diagnostic);
                                    return Err(self.index_error(error));
                                }
                            };
                            let lease = match self.reserve_entry() {
                                Ok(lease) => lease,
                                Err(error) => {
                                    drop(state);
                                    drop(fresh);
                                    drop(reservation);
                                    drop(diagnostic);
                                    drop(outcome_storage);
                                    return Err(self.index_error(error));
                                }
                            };
                            let loader = match self.payloads().map(|payloads| {
                                admit_loader(&payloads.loader, self.inner.load_effect.clone())
                            }).transpose() {
                                Ok(loader) => loader,
                                Err(error) => {
                                    drop(state);
                                    drop(fresh);
                                    drop(reservation);
                                    drop(diagnostic);
                                    drop(outcome_storage);
                                    drop(lease);
                                    return Err(self.index_error(error));
                                }
                            };
                            let cancellation = match self.inner.parent.child() {
                                Ok(cancellation) => cancellation,
                                Err(error) => {
                                    drop(state);
                                    drop(fresh);
                                    drop(reservation);
                                    drop(diagnostic);
                                    drop(outcome_storage);
                                    drop(lease);
                                    drop(loader);
                                    return Err(self.index_error(error));
                                }
                            };
                            let changed = lease.is_none().then(Event::new);
                            let entry = IndexEntry::publish(lease, Entry {
                                key: key.clone(),
                                outcome_storage: Mutex::new(Some(outcome_storage)),
                                produced: Mutex::new(None),
                                submission: Mutex::new(Submission::Pending(None)),
                                state: Mutex::new(EntryState {
                                    outcome: None,
                                    expires: None,
                                    invalidated: false,
                                    retiring: false,
                                    observed: false,
                                    diagnostic,
                                }),
                                changed,
                                cancellation,
                            });
                            let prepared = loader.map(|loader| loader(LoaderInput {
                                entry: entry.clone(), owner: Arc::downgrade(&self.inner), context: context.clone(),
                            }));
                            state.active.push_reserved(entry.clone());
                            (entry, true, prepared)
                        }
                    };
                    fresh.push_reserved(entry.clone());
                    let old = std::mem::replace(&mut state.retained, fresh);
                    state.advance_revision();
                    let observation = if start {
                        CacheObservation::Loaded
                    } else if entry.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner).outcome.is_some() {
                        CacheObservation::Retained
                    } else {
                        CacheObservation::Shared
                    };
                    (Some((SelectedCacheEntry { entry, observation, reservation, independent_task: None }, start, old, prepared)), None)
                }
            };
            if let Some((mut selected, start, old, prepared)) = selection {
                let old = if prepared.is_none() {
                    drop(old);
                    None
                } else {
                    Some(old)
                };
                if start && self.independent() {
                    *selected.entry.submission.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Submission::Accepted;
                    selected.independent_task = prepared;
                } else if start {
                    let dispatch = if prepared.is_some() {
                        context.clone()
                    } else {
                        context.parallel(selected.entry.cancellation.clone())
                    };
                    (self.inner.start)(selected.entry.clone(), Arc::downgrade(&self.inner), dispatch, prepared);
                }
                drop(old);
                return Ok(Some(selected));
            }
            drop(stale);
            cooperative_retry().await;
        }
    }

    /// # Type parameters
    /// - `A`: Success or application-value type.
    /// - `E`: Domain-error type.
    /// - `K`: Type supplied by the enclosing impl or trait.
    ///
    /// # Parameters
    /// - `entry` (`&IndexEntry<K, A, E>`): Stored record or event selected for this transition.
    /// - `cancellation` (`Cancellation`): The cancellation token for this operation.
    ///
    /// # Returns
    /// Type: `SharedOutcome<A, E>`.
    async fn observe_with_cleanup(
        entry: &IndexEntry<K, A, E>,
        cancellation: Cancellation,
        reservation: ObservationReservation<E>,
        independent: bool,
    ) -> SharedOutcome<A, E> {
        let ObservationReservation {
            observer: _observer,
            mut cleanup,
            notification,
            cancelled,
        } = reservation;
        let mut ready = || {
            let state = entry
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if !independent && cancellation.is_requested() {
                return Some(Exit::Interrupted);
            }
            state
                .outcome
                .as_ref()
                .map(|outcome| clone_observed(outcome, state.invalidated, &mut cleanup))
        };
        if let Some(result) = ready() {
            return result;
        }
        let mut cancelled = std::pin::pin!(async {
            if let Some(cancelled) = cancelled {
                cancelled.wait().await;
            } else {
                cancellation.cancelled().await;
            }
        });
        let mut cancellation_forwarded = false;
        loop {
            if independent && !cancellation_forwarded && cancellation.is_requested() {
                let _ = entry.cancellation.request();
                cancellation_forwarded = true;
            }
            let ordinary = if let Some(notification) = &notification {
                notification.reset();
                None
            } else {
                entry.changed.as_ref().map(Event::listen)
            };
            if let Some(result) = ready() {
                return result;
            }
            let changed = async {
                if let Some(notification) = &notification {
                    std::future::poll_fn(|context| notification.poll(context)).await;
                } else if let Some(ordinary) = ordinary {
                    ordinary.await;
                }
            };
            let changed = std::pin::pin!(changed);
            if cancellation_forwarded {
                changed.await;
                continue;
            }
            match select(changed, cancelled.as_mut()).await {
                Either::Left(((), _)) => {}
                Either::Right(((), _)) if independent => {
                    let _ = entry.cancellation.request();
                    cancellation_forwarded = true;
                }
                Either::Right(((), _)) => return Exit::Interrupted,
            }
        }
    }
    /// # Type parameters
    /// - `A`: Success or application-value type.
    /// - `E`: Domain-error type.
    /// - `K`: Type supplied by the enclosing impl or trait.
    ///
    /// # Parameters
    /// - `entry` (`&IndexEntry<K, A, E>`): Stored record or event selected for this transition.
    /// - `cancellation` (`Cancellation`): The cancellation token for this operation.
    ///
    /// # Returns
    /// Type: `SharedOutcome<A, E>`.
    pub(crate) async fn observe_selected(
        &self,
        selected: SelectedCacheEntry<K, A, E>,
        cancellation: Cancellation,
    ) -> SharedOutcome<A, E> {
        let SelectedCacheEntry { entry, reservation, independent_task, .. } = selected;
        if independent_task.is_some() && cancellation.is_requested() {
            let _ = entry.cancellation.request();
        }
        let observe = self.observe_value_with_cleanup(&entry, cancellation, reservation);
        if let Some(task) = independent_task {
            // Disabled Web loads stay in their caller. The cache tracks the
            // settlement for drain, while this caller drives the prepared work.
            let (outcome, ()) = futures_util::future::join(observe, task).await;
            outcome
        } else {
            observe.await
        }
    }

    async fn observe_value_with_cleanup(
        &self,
        entry: &IndexEntry<K, A, E>,
        cancellation: Cancellation,
        reservation: ObservationReservation<E>,
    ) -> SharedOutcome<A, E> {
        let independent = self.independent();
        let cleanup = std::mem::DropGuard::new(entry, |entry| {
            if independent {
                let _ = entry.cancellation.request();
            }
        });
        let result = Self::observe_with_cleanup(entry, cancellation, reservation, independent).await;
        let _ = std::mem::DropGuard::dismiss(cleanup);
        if matches!(result, Exit::FinalizationFailed(_)) {
            entry
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .observed = true;
            let discarded = {
                let mut state = self
                    .inner
                    .state
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                let position = state
                    .diagnostics
                    .iter()
                    .position(|diagnostic| diagnostic.entry.same_entry(entry));
                let discarded = position.and_then(|position| state.diagnostics.remove(position));
                if discarded.is_some() {
                    state.advance_revision();
                }
                discarded
            };
            drop(discarded);
        }
        result
    }
    /// Lazily get or load one key, sharing producer work and successful/error ownership for equivalent keys; use get_observed for acquisition classification.
    /// A recipe for a prepared cache retains its admitted key for subsequent runs.
    pub fn get<R: Send + Sync + 'static>(
        &self,
        key: K,
    ) -> Effect<SharedValue<A>, SharedValue<E>, R> {
        self.get_key(self.lookup_key(key))
    }

    /// Lazily get or load a key admitted by [`Self::prepare_key`]. Reusing this
    /// handle consumes no additional key slot, including when the lookup hits.
    pub fn get_prepared<R: Send + Sync + 'static>(
        &self,
        key: CacheKey<K>,
    ) -> Effect<SharedValue<A>, SharedValue<E>, R> {
        self.get_key(LookupKey::Owned(EntryKey::Prepared(key)))
    }

    /// Lazily acquire a key and report Retained, Shared or Loaded with its shared success.
    ///
    /// Classification is selected with the cache entry; all equivalent waiters observe
    /// one producer outcome. Cancelling one observer does not cancel work still owned
    /// by other observers or retention. Typed loader errors are shared, not cloned.
    #[doc(alias = "getObserved")]
    pub fn get_observed<R: Send + Sync + 'static>(
        &self,
        key: K,
    ) -> Effect<CacheLookup<A>, SharedValue<E>, R> {
        let key = self.lookup_key(key);
        let cache = self.clone();
        Effect::new(move |_, context, finish| {
            let cache = cache.clone();
            let key = key.clone();
            Step::suspended(Box::pin(async move {
                let work = async {
                    if context.cancelled() {
                        return Exit::Interrupted;
                    }
                    let key = match cache.admit_key(key) {
                        Ok(key) => key,
                        Err(error) => return error.exit(),
                    };
                    match cache.select_observed_entry(key, &context).await {
                        Ok(Some(selected)) => {
                            let observation = selected.observation;
                            match cache.observe_selected(selected, context.cancellation.clone()).await {
                                Exit::Success(value) => Exit::Success(CacheLookup { value, observation }),
                                Exit::Fail(error) => Exit::Fail(error),
                                Exit::Die(defect) => Exit::Die(defect),
                                Exit::Interrupted => Exit::Interrupted,
                                Exit::FinalizationFailed(details) => Exit::FinalizationFailed(details),
                            }
                        }
                        Ok(None) => Exit::Interrupted,
                        Err(error) => error.exit(),
                    }
                };
                let result = AssertUnwindSafe(work)
                    .catch_unwind()
                    .await
                    .unwrap_or_else(|panic| Exit::Die(Defect::from_panic(panic)));
                runtime::finish(finish, result)
            }))
        })
    }

    /// Lazily inspect only a fresh retained success without registering or dispatching work.
    ///
    /// Returns None for absent, expired, active or failed entries. Payload handles share
    /// ownership; this call never joins an active producer.
    pub fn try_get<R: Send + Sync + 'static>(
        &self,
        key: K,
    ) -> Effect<Option<SharedValue<A>>, SharedValue<E>, R> {
        self.try_get_key(self.lookup_key(key))
    }

    /// Inspect a fresh retained success using an already admitted lookup key.
    pub fn try_get_prepared<R: Send + Sync + 'static>(
        &self,
        key: CacheKey<K>,
    ) -> Effect<Option<SharedValue<A>>, SharedValue<E>, R> {
        self.try_get_key(LookupKey::Owned(EntryKey::Prepared(key)))
    }

    fn try_get_key<R: Send + Sync + 'static>(
        &self,
        key: LookupKey<K>,
    ) -> Effect<Option<SharedValue<A>>, SharedValue<E>, R> {
        let cache = self.clone();
        Effect::new(move |_, context, finish| {
            let cache = cache.clone();
            let key = key.clone();
            Step::suspended(Box::pin(async move {
                let work = async {
                    if context.cancelled() {
                        return Exit::Interrupted;
                    }
                    let key = match cache.admit_key(key) {
                        Ok(key) => key,
                        Err(error) => return error.exit(),
                    };
                    loop {
                        if context.cancelled() {
                            return Exit::Interrupted;
                        }
                        let CacheSnapshot { revision, entries } = match cache.snapshot_index(SnapshotKind::Lookup) {
                            Ok(snapshot) => snapshot,
                            Err(error) => return error.exit(),
                        };
                        let now = (cache.inner.now)();
                        let found = entries
                            .iter()
                            .find(|entry| (cache.inner.config.equal)(&entry.key, &key));
                        let value = found.and_then(|entry| {
                            let state = entry
                                .state
                                .lock()
                                .unwrap_or_else(std::sync::PoisonError::into_inner);
                            if state.invalidated
                                || state.expires.is_some_and(|expires| now >= expires)
                            {
                                None
                            } else {
                                match &state.outcome {
                                    Some(Exit::Success(value)) => Some(value.clone()),
                                    _ => None,
                                }
                            }
                        });
                        let current = {
                            let state = cache
                                .inner
                                .state
                                .lock()
                                .unwrap_or_else(std::sync::PoisonError::into_inner);
                            state.revision.matches(&revision)
                        };
                        if current {
                            return Exit::Success(value);
                        }
                        drop(entries);
                        cooperative_retry().await;
                    }
                };
                let result = AssertUnwindSafe(work)
                    .catch_unwind()
                    .await
                    .unwrap_or_else(|panic| Exit::Die(Defect::from_panic(panic)));
                runtime::finish(finish, result)
            }))
        })
    }

    pub(crate) fn get_shared<R: Send + Sync + 'static>(
        &self,
        key: Arc<K>,
    ) -> Effect<SharedValue<A>, SharedValue<E>, R> {
        self.get_key(LookupKey::Owned(EntryKey::Ordinary(key)))
    }

    fn get_key<R: Send + Sync + 'static>(&self, key: LookupKey<K>) -> Effect<SharedValue<A>, SharedValue<E>, R> {
        let cache = self.clone();
        Effect::new(move |_, context, finish| {
            let cache = cache.clone();
            let key = key.clone();
            Step::suspended(Box::pin(async move {
                let work = async {
                    if context.cancelled() {
                        return Exit::Interrupted;
                    }
                    let key = match cache.admit_key(key) {
                        Ok(key) => key,
                        Err(error) => return error.exit(),
                    };
                    match cache.select_observed_entry(key, &context).await {
                        Ok(Some(selected)) => cache.observe_selected(selected, context.cancellation.clone()).await,
                        Ok(None) => Exit::Interrupted,
                        Err(error) => error.exit(),
                    }
                };
                let outcome = AssertUnwindSafe(work)
                    .catch_unwind()
                    .await
                    .unwrap_or_else(|panic| Exit::Die(Defect::from_panic(panic)));
                runtime::finish(finish, outcome)
            }))
        })
    }
    ///
    /// # Type parameters
    /// - `A`: Success or application-value type.
    /// - `E`: Domain-error type.
    /// - `K`: Type supplied by the enclosing impl or trait.
    ///
    /// # Parameters
    /// - `selected` (`Vec<Arc<Entry<K, A, E>>>`): The selected supplied to this operation.
    /// - `earlier` (`Vec<SharedOutcome<A, E>>`): The earlier supplied to this operation.
    ///
    /// # Returns
    /// Type: `Exit<(), SharedValue<E>>`.
    /// Returns `Exit<(), SharedValue<E>>`.
    async fn retire(
        &self,
        mut selected: CacheIndex<K, A, E>,
        earlier: impl Iterator<Item = Diagnostic<K, A, E>> + Send,
        ignore_selected_diagnostics: bool,
        cancel: bool,
        mut cleanup: Retirement<E>,
    ) -> Retirement<E> {
        if cancel {
            for entry in selected.iter() {
                let _ = entry.cancellation.request();
            }
        }
        for diagnostic in earlier {
            if !ignore_selected_diagnostics
                || !selected
                    .iter()
                    .any(|entry| entry.same_entry(&diagnostic.entry))
            {
                let state = diagnostic
                    .entry
                    .state
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                if let Some(outcome) = &state.outcome {
                    cleanup.append_exit(outcome);
                }
            }
        }
        while let Some(entry) = selected.pop_front() {
            loop {
                let ordinary = if let Some(notification) = &cleanup.notification {
                    notification.reset();
                    None
                } else {
                    entry.changed.as_ref().map(Event::listen)
                };
                {
                    let state = entry
                        .state
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    if let Some(outcome) = &state.outcome {
                        if cancel {
                            cleanup.append_exit(outcome);
                        }
                        break;
                    }
                }
                if let Some(notification) = &cleanup.notification {
                    std::future::poll_fn(|context| notification.poll(context)).await;
                } else if let Some(ordinary) = ordinary {
                    ordinary.await;
                }
            }
        }
        cleanup
    }

    fn reserve_retirement(&self) -> Result<Option<RetirementReservation<E>>, CacheSelectionError<E>> {
        self.payloads()
            .map(RetirementReservation::reserve)
            .transpose()
            .map_err(|error| self.index_error(error))
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
    /// - `key` (`K`): Stable key used to locate or identify the selected value.
    ///
    /// # Returns
    /// Type: `Effect<(), SharedValue<E>, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    pub fn invalidate<R: Send + Sync + 'static>(&self, key: K) -> Effect<(), SharedValue<E>, R> {
        self.invalidate_selected(Invalidation::Key(self.lookup_key_for(key, KeyPurpose::Maintenance)), || {})
    }

    /// Invalidate one key using its already admitted region handle.
    pub fn invalidate_prepared<R: Send + Sync + 'static>(
        &self,
        key: CacheKey<K>,
    ) -> Effect<(), SharedValue<E>, R> {
        self.invalidate_selected(Invalidation::Key(LookupKey::Owned(EntryKey::Prepared(key))), || {})
    }

    /// # Type parameters
    /// - `A`: Application value type. Bound: `Send + 'static`.
    /// - `R`: Environment type. Bounds: `Send + Sync + 'static`.
    /// - `E`: Domain-error type.
    /// - `K`: Type supplied by the enclosing impl or trait.
    ///
    /// # Parameters
    /// - `key` (`Arc<K>`): Stable key used to locate or identify the selected value.
    ///
    /// # Returns
    /// Type: `Effect<(), SharedValue<E>, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    pub(crate) fn invalidate_shared<R: Send + Sync + 'static>(
        &self,
        key: Arc<K>,
    ) -> Effect<(), SharedValue<E>, R> {
        let equal = Arc::clone(&self.inner.config.equal);
        self.invalidate_matching(move |candidate, _, _| equal(candidate, &key))
    }

    pub(crate) fn invalidate_matching<R: Send + Sync + 'static>(
        &self,
        predicate: impl Fn(&K, Option<SharedValue<A>>, bool) -> bool + Send + Sync + 'static,
    ) -> Effect<(), SharedValue<E>, R> {
        self.invalidate_matching_committed(predicate, || {})
    }

    pub(crate) fn invalidate_matching_committed<R: Send + Sync + 'static>(
        &self,
        predicate: impl Fn(&K, Option<SharedValue<A>>, bool) -> bool + Send + Sync + 'static,
        on_commit: impl Fn() + Send + Sync + 'static,
    ) -> Effect<(), SharedValue<E>, R> {
        self.invalidate_selected(Invalidation::Predicate(Arc::new(predicate)), on_commit)
    }

    fn invalidate_selected<R: Send + Sync + 'static>(
        &self,
        query: Invalidation<K, A, LookupKey<K>>,
        on_commit: impl Fn() + Send + Sync + 'static,
    ) -> Effect<(), SharedValue<E>, R> {
        let on_commit = Arc::new(on_commit);
        let cache = self.clone();
        let program = Effect::new(move |_, _, finish| {
            let cache = cache.clone();
            let query = query.clone();
            let on_commit = Arc::clone(&on_commit);
            Step::suspended(Box::pin(async move {
                let work = async {
                    let query = match query {
                        Invalidation::Predicate(predicate) => Invalidation::Predicate(predicate),
                        Invalidation::Key(key) => match cache.admit_lookup_key(key, true) {
                            Ok(key) => Invalidation::Key(key),
                            Err(error) => return error.exit(),
                        },
                    };
                    loop {
                        let CacheSnapshot {
                            revision,
                            entries: mut selected,
                        } = match cache
                            .snapshot_index(SnapshotKind::Maintenance { allow_closed: true })
                        {
                            Ok(snapshot) => snapshot,
                            Err(error) => return error.exit(),
                        };
                        let mut discarded = match cache.reserve_retired_diagnostics() {
                            Ok(discarded) => discarded,
                            Err(error) => return error.exit(),
                        };
                        retain_selected(&mut selected, |entry| match &query {
                            Invalidation::Key(key) => (cache.inner.config.equal)(&entry.key, key),
                            Invalidation::Predicate(predicate) => {
                                let (value, pending) = {
                                    let state = entry.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                                    let value = match &state.outcome {
                                        Some(Exit::Success(value)) => Some(value.clone()),
                                        _ => None,
                                    };
                                    (value, state.outcome.is_none())
                                };
                                predicate(&entry.key, value, pending)
                            }
                        });
                        let retirement = if selected.len() == 0 {
                            Retirement::new(None)
                        } else {
                            match cache.reserve_retirement() {
                                Ok(cleanup) => Retirement::new(cleanup),
                                Err(error) => return error.exit(),
                            }
                        };
                        let committed = {
                            let mut state = cache
                                .inner
                                .state
                                .lock()
                                .unwrap_or_else(std::sync::PoisonError::into_inner);
                            if !state.revision.matches(&revision) {
                                false
                            } else {
                                for entry in selected.iter() {
                                    let mut entry_state = entry
                                        .state
                                        .lock()
                                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                                    entry_state.invalidated = true;
                                    entry_state.retiring = true;
                                }
                                state.retained.retain(|entry| {
                                    !selected
                                        .iter()
                                        .any(|candidate| candidate.same_entry(entry))
                                });
                                take_diagnostics(&mut state.diagnostics, &selected, &mut discarded);
                                state.advance_revision();
                                true
                            }
                        };
                        // The selected canonical outcomes are joined below.
                        drop(discarded);
                        if committed {
                            let committed = catch_unwind(AssertUnwindSafe(|| on_commit()));
                            let mut retirement = cache
                                .retire(selected, std::iter::empty(), false, true, retirement)
                                .await;
                            if let Err(panic) = committed {
                                retirement.outcome.append_cleanup(Exit::Die(Defect::from_panic(panic)));
                            }
                            return retirement.finish();
                        }
                        drop(selected);
                        cooperative_retry().await;
                    }
                };
                let outcome = AssertUnwindSafe(work)
                    .catch_unwind()
                    .await
                    .unwrap_or_else(|panic| Exit::Die(Defect::from_panic(panic)));
                runtime::finish(finish, outcome)
            }))
        });
        effect_uninterruptible! { program: program, }
    }
    /// Lazily evict all entries, fence active generations, and join their owned cleanup.
    ///
    /// Eviction commits before cancellation/destruction outside the gate. Cleanup
    /// failures remain in the effect outcome; the cache remains open for later loads.
    pub fn clear<R: Send + Sync + 'static>(&self) -> Effect<(), SharedValue<E>, R> {
        self.invalidate_matching(|_, _, _| true)
    }

    /// Lazily replace a key with one owned success, fencing older producer generations.
    ///
    /// Publishes the replacement under the gate, then cancels and joins old work
    /// outside it. A late producer cannot overwrite the new value. Retention/TTL rules
    /// still apply, and the stored input is consumed once without cloning.
    pub fn set<R: Send + Sync + 'static>(&self, key: K, value: A) -> Effect<(), SharedValue<E>, R> {
        let cache = self.clone();
        let input = effect_sync! { once; body: move || (key, value), };
        effect! {
            let cache = cache.clone();
            let input = input.clone();
            let input = input?;
            let input = Arc::new(Mutex::new(Some(input)));
            let replacement = Effect::new(move |_, _, finish| {
                let cache = cache.clone();
                let input = Arc::clone(&input);
                Step::suspended(Box::pin(async move {
                    let work = async {
                        let input = input
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner)
                            .take();
                        let Some((key, value)) = input else {
                            return Exit::Die(Defect::from(LibraryDefect::CacheClosed));
                        };
                        let key = if cache.payloads().is_some() {
                            match cache.prepare_key(key) {
                                Ok(key) => EntryKey::Prepared(key),
                                Err((MemoryError::Closed, _key)) => return Exit::Die(Defect::from(LibraryDefect::CacheClosed)),
                                Err((error, _key)) => return cache.index_error(error).exit(),
                            }
                        } else {
                            EntryKey::Ordinary(Arc::new(key))
                        };
                        let mut entry_lease = match cache.reserve_entry() {
                            Ok(lease) => lease,
                            Err(error) => return cache.index_error(error).exit(),
                        };
                        let mut outcome_storage = match cache.payloads().map_or(Ok(OutcomeStorage::Ordinary), Payloads::reserve_success) {
                            Ok(storage) => Some(storage),
                            Err(error) => return cache.index_error(error).exit(),
                        };
                        let retirement_cleanup = match cache.reserve_retirement() {
                            Ok(cleanup) => cleanup,
                            Err(error) => return error.exit(),
                        };
                        loop {
                            let CacheSnapshot { revision, entries: mut selected } = match
                                cache.snapshot_index(SnapshotKind::Maintenance { allow_closed: false })
                            {
                                Ok(snapshot) => snapshot,
                                Err(error) => return error.exit(),
                            };
                            let mut diagnostics = match cache.reserve_retired_diagnostics() {
                                Ok(diagnostics) => diagnostics,
                                Err(error) => return error.exit(),
                            };
                            retain_selected(&mut selected, |entry| (cache.inner.config.equal)(&entry.key, &key));
                            let cancellation = match cache.inner.parent.child() {
                                Ok(cancellation) => cancellation,
                                Err(error) => return cache.index_error(error).exit(),
                            };
                            let installed = {
                                let mut state = cache
                                    .inner
                                    .state
                                    .lock()
                                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                                if !state.revision.matches(&revision) {
                                    None
                                } else {
                                    if state.active.remaining() == 0 || (state.retained.remaining() == 0
                                        && !state.retained.iter().any(|candidate| {
                                            selected.iter().any(|entry| entry.same_entry(candidate))
                                        }))
                                    {
                                        drop(state);
                                        return cache.index_error(MemoryError::NoSlot).exit();
                                    }
                                    for entry in selected.iter() {
                                        let mut state = entry
                                            .state
                                            .lock()
                                            .unwrap_or_else(std::sync::PoisonError::into_inner);
                                        state.invalidated = true;
                                        state.retiring = true;
                                    }
                                    take_diagnostics(&mut state.diagnostics, &selected, &mut diagnostics);
                                    state.retained.retain(|entry| {
                                        !selected
                                            .iter()
                                            .any(|candidate| candidate.same_entry(entry))
                                    });
                                    let changed = entry_lease.is_none().then(Event::new);
                                    let entry = IndexEntry::publish(entry_lease.take(), Entry {
                                        key: key.clone(),
                                        outcome_storage: Mutex::new(outcome_storage.take()),
                                        produced: Mutex::new(None),
                                        submission: Mutex::new(Submission::Finished),
                                        state: Mutex::new(EntryState {
                                            outcome: None,
                                            expires: None,
                                            invalidated: false,
                                            retiring: false,
                                            observed: true,
                                            diagnostic: None,
                                        }),
                                        changed,
                                        cancellation,
                                    });
                                    state.retained.push_reserved(entry.clone());
                                    state.active.push_reserved(entry.clone());
                                    state.advance_revision();
                                    Some(entry)
                                }
                            };
                            if let Some(entry) = installed {
                                settle(&entry, &Arc::downgrade(&cache.inner), Exit::Success(value));
                                let retirement = {
                                    let entry = entry.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                                    let missing = Exit::Die(Defect::from(LibraryDefect::MissingInterpreterOutcome));
                                    Retirement::with_primary(retirement_cleanup, entry.outcome.as_ref().unwrap_or(&missing))
                                };
                                let earlier = std::iter::from_fn(move || diagnostics.pop_front());
                                return cache.retire(selected, earlier, false, true, retirement).await.finish();
                            }
                            drop(diagnostics);
                            drop(selected);
                            cooperative_retry().await;
                        }
                    };
                    let result = AssertUnwindSafe(work)
                        .catch_unwind()
                        .await
                        .unwrap_or_else(|panic| Exit::Die(Defect::from_panic(panic)));
                    runtime::finish(finish, result)
                }))
            });
            effect_uninterruptible! { program: replacement, }?
        }
    }

    async fn shutdown_start(&self, notification: Option<prepared_notification::Registration>) {
        let cancel = !self.independent();
        let selected = {
            let mut state = self
                .inner
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            match state.shutdown {
                CacheShutdown::Open => {
                    state.closed = true;
                    state.shutdown = CacheShutdown::Closing;
                    state.advance_revision();
                    let mut selected = std::mem::take(&mut state.shutdown_index);
                    for entry in state.active.iter() {
                        selected.push_reserved(entry.clone());
                    }
                    for entry in selected.iter() {
                        let mut entry_state = entry
                            .state
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner);
                        entry_state.invalidated = cancel;
                        entry_state.retiring = true;
                    }
                    let diagnostics = std::mem::take(&mut state.diagnostics);
                    let retained = std::mem::take(&mut state.retained);
                    Some((
                        selected,
                        diagnostics,
                        retained,
                        state.shutdown_retirement.take(),
                    ))
                }
                CacheShutdown::Closing | CacheShutdown::Closed(_) => None,
            }
        };
        if let Some((selected, mut diagnostics, retained, cleanup)) = selected {
            let mut guard = CacheShutdownDriver {
                cache: self.clone(),
                settled: false,
            };
            if cancel {
                for entry in retained.iter() {
                    entry.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner).invalidated = true;
                }
            }
            let cancel_on_drop = std::mem::DropGuard::new(&self.inner.parent, |parent| {
                if cancel {
                    let _ = parent.request();
                }
            });
            drop(retained);
            drop(cancel_on_drop);
            let earlier = std::iter::from_fn(move || diagnostics.pop_front());
            let mut retirement = self
                .retire(selected, earlier, true, cancel, Retirement::new(cleanup))
                .await;
            drop(retirement.notification.take());
            let mut state = self
                .inner
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if let Some(defect) = state.owner_loss.take() {
                retirement.outcome.append_cleanup(Exit::Die(defect));
            }
            state.shutdown = CacheShutdown::Closed(Some(retirement.finish()));
            drop(state);
            guard.settled = true;
            let notify = std::mem::DropGuard::new(self, |cache| cache.notify_shutdown());
            self.inner.parent.unlink();
            drop(notify);
        } else {
            loop {
                let ordinary = if let Some(notification) = &notification {
                    notification.reset();
                    None
                } else {
                    self.inner.changed.as_ref().map(Event::listen)
                };
                if matches!(
                    self.inner
                        .state
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .shutdown,
                    CacheShutdown::Closed(_)
                ) {
                    break;
                }
                if let Some(notification) = &notification {
                    std::future::poll_fn(|context| notification.poll(context)).await;
                } else if let Some(ordinary) = ordinary {
                    ordinary.await;
                }
            }
        }
    }

    fn notify_shutdown(&self) {
        if let Some(payloads) = self.payloads() {
            payloads.closed.notify_all();
        }
        if let Some(changed) = &self.inner.changed {
            notify_all(changed);
        }
    }

    fn reserve_close_wait(
        &self,
    ) -> Result<Option<prepared_notification::Registration>, MemoryError> {
        let Some(payloads) = self.payloads() else {
            return Ok(None);
        };
        let closed = matches!(
            self.inner
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .shutdown,
            CacheShutdown::Closed(_)
        );
        if closed {
            return Ok(None);
        }
        payloads.closed.listen().map(Some)
    }

    async fn shutdown_owned(
        &self,
        _: Context,
        notification: Option<prepared_notification::Registration>,
    ) -> Exit<(), SharedValue<E>> {
        self.shutdown_start(notification).await;
        let mut state = self
            .inner
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        match &mut state.shutdown {
            CacheShutdown::Closed(outcome) => outcome.take().unwrap_or(Exit::Success(())),
            CacheShutdown::Open | CacheShutdown::Closing => {
                Exit::Die(Defect::from(LibraryDefect::CacheShutdownDriverDropped))
            }
        }
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
        effect_uninterruptible! { program: Effect::new(move |_, _, finish| {
            let cache = cache.clone();
            Step::suspended(Box::pin(async move {
                let notification = match cache.reserve_close_wait() {
                    Ok(notification) => notification,
                    Err(error) => return runtime::finish(finish, cache.index_error(error).exit()),
                };
                cache.shutdown_start(notification).await;
                let status = {
                    let state = cache
                        .inner
                        .state
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    match &state.shutdown {
                        CacheShutdown::Closed(Some(Exit::Success(())))
                        | CacheShutdown::Closed(None) => crate::PoolEndStatus::Clean,
                        CacheShutdown::Open
                        | CacheShutdown::Closing
                        | CacheShutdown::Closed(Some(
                            Exit::Fail(_)
                            | Exit::Die(_)
                            | Exit::Interrupted
                            | Exit::FinalizationFailed(_),
                        )) => crate::PoolEndStatus::Failed,
                    }
                };
                runtime::finish(finish, Exit::Success(status))
            }))
        }), }
    }
}
/// Typed cache shutdown driver used by cache.
///
/// # Type parameters
/// - `K`: Key type used for identity or lookup. Bound or type: `Send + Sync + 'static`.
/// - `A`: Application value type. Bound or type: `Send + 'static`.
/// - `E`: Domain-error type. Bound or type: `Send + 'static`.
struct CacheShutdownDriver<K: Send + Sync + 'static, A: Send + 'static, E: Send + 'static> {
    cache: Cache<K, A, E>,
    settled: bool,
}
impl<K: Send + Sync + 'static, A: Send + 'static, E: Send + 'static> Drop
    for CacheShutdownDriver<K, A, E>
{
    ///
    /// # Returns
    /// Type: `()`.
    fn drop(&mut self) {
        if !self.settled {
            self.cache
                .inner
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .shutdown = CacheShutdown::Closed(Some(Exit::Die(Defect::from(
                LibraryDefect::CacheShutdownDriverDropped,
            ))));
            self.cache.notify_shutdown();
        }
    }
}

impl<K: Send + Sync + 'static, A: Send + 'static, E: Send + 'static> Entry<K, A, E> {
    /// # Type parameters
    /// - `K`: Key type used for identity or lookup. Bound: `Send + Sync + 'static`.
    /// - `A`: Application value type. Bound: `Send + 'static`.
    /// - `E`: Domain-error type. Bound: `Send + 'static`.
    ///
    /// # Returns
    /// Type: `()`.
    pub(crate) fn invalidate_association(&self) {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .invalidated = true;
    }
    /// # Type parameters
    /// - `K`: Key type used for identity or lookup. Bound: `Send + Sync + 'static`.
    /// - `A`: Application value type. Bound: `Send + 'static`.
    /// - `E`: Domain-error type. Bound: `Send + 'static`.
    ///
    /// # Returns
    /// Type: `bool`.
    /// Returns whether the requested condition holds.
    pub(crate) fn is_invalidated(&self) -> bool {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .invalidated
    }
}

// One handoff per failed optimistic selection; no arbitrary retry batch.
///
/// # Returns
/// Type: `()`.
async fn cooperative_retry() {
    let mut yielded = false;
    std::future::poll_fn(|cx| {
        if yielded {
            std::task::Poll::Ready(())
        } else {
            yielded = true;
            cx.waker().wake_by_ref();
            std::task::Poll::Pending
        }
    })
    .await;
}
