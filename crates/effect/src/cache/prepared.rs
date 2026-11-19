use super::{CacheCapacity, Entry, EntryLease, PreparedCacheIndex, SharedValue, loader_layout};
use crate::cancellation::PreparedCancellationBank;
use crate::internal::work_list::WorkList;
use crate::memory::{
    Bytes, FixedVector, FramePool, MemoryError, PreparedMemory, RegionId, SharedSlot, SlotLease,
    SlotPool, WeakSlot, prepared_notification,
};
use crate::{
    CleanupFailure, CleanupFailures, Defect, DefectKind, Exit, Finalization, LibraryDefect,
    PreparedCleanupAccumulator, PreparedCleanupConfig, PreparedCleanupPool, PrimaryOutcome,
};
use std::num::NonZeroUsize;
use std::ops::Deref;
use std::sync::{Arc, Mutex};

type CellPool<T> = SlotPool<'static, Option<T>>;
pub(super) type CellLease<T> = SlotLease<'static, Option<T>>;
type ErrorLease<E> = CellLease<Mutex<E>>;
type ErrorRows<E> = FixedVector<'static, ErrorLease<E>>;
pub(super) type Cleanup<E> = CleanupFailures<SharedValue<E>>;

pub(crate) struct Cell<T: 'static>(Option<SharedSlot<'static, Option<T>>>);

impl<T> Cell<T> {
    pub(super) fn from_shared(slot: SharedSlot<'static, Option<T>>) -> Self {
        Self(Some(slot))
    }

    pub(super) fn publish(mut slot: CellLease<T>, value: T) -> Self {
        debug_assert!(slot.is_none());
        *slot = Some(value);
        Self::from_shared(slot.share())
    }

    pub(super) fn same_cell(&self, other: &Self) -> bool {
        self.slot().same_slot(other.slot())
    }

    pub(super) fn downgrade(&self) -> Result<WeakSlot<'static, Option<T>>, MemoryError> {
        self.slot().downgrade()
    }

    pub(super) fn matches_weak(&self, other: &WeakSlot<'static, Option<T>>) -> bool {
        other.matches_shared(self.slot())
    }

    #[allow(clippy::expect_used)] // A live cell always has a slot; retirement consumes the cell.
    fn slot(&self) -> &SharedSlot<'static, Option<T>> {
        self.0.as_ref().expect("live cache cell")
    }

    #[allow(clippy::expect_used)] // Exclusivity cannot be lost without another live handle.
    pub(super) fn try_into_inner(mut self) -> Result<T, Self> {
        if !self.slot().is_unique() {
            return Err(self);
        }
        let mut slot = self
            .0
            .take()
            .and_then(SharedSlot::into_last)
            .expect("unique cache cell");
        Ok(slot.take().expect("published cache cell"))
    }
}

impl<T> Deref for Cell<T> {
    type Target = T;

    #[allow(clippy::expect_used)] // Publication writes a value before sharing the cell.
    fn deref(&self) -> &T {
        (**self.slot()).as_ref().expect("published cache cell")
    }
}

impl<T> Clone for Cell<T> {
    #[allow(clippy::expect_used)] // Clone is infallible; only an unrepresentable reference count can fail.
    fn clone(&self) -> Self {
        // Cloning neither waits for capacity nor allocates another slot.
        Self(Some(self.slot().try_clone().expect(
            "cache cell reference count exceeds its representation",
        )))
    }
}

impl<T> Drop for Cell<T> {
    fn drop(&mut self) {
        if let Some(mut slot) = self.0.take().and_then(SharedSlot::into_last) {
            // Clear the cell before user destruction, with the unique lease
            // retaining its backing until destruction finishes.
            drop(slot.take());
        }
    }
}

/// An immutable, region-backed cache key. Clones retain the same physical slot.
pub struct CacheKey<K: Send + Sync + 'static> {
    cell: Cell<K>,
    origin: Arc<()>,
}

impl<K: Send + Sync + 'static> Clone for CacheKey<K> {
    fn clone(&self) -> Self {
        Self {
            cell: self.cell.clone(),
            origin: self.origin.clone(),
        }
    }
}

impl<K: Send + Sync + 'static> Deref for CacheKey<K> {
    type Target = K;

    fn deref(&self) -> &K {
        &self.cell
    }
}

/// Selected bounds for a cache's indexes, pending work, and retained payloads.
#[derive(Clone, Copy)]
pub struct PreparedCacheCapacity {
    /// Settled membership limit, excluding values or keys held by external owners.
    pub retention: CacheCapacity,
    /// Concurrent independent loader count.
    pub active: NonZeroUsize,
    /// Concurrent lookup snapshots.
    pub lookups: NonZeroUsize,
    /// Pending or retained unobserved finalization diagnostics.
    pub diagnostics: NonZeroUsize,
    /// Concurrent suspended observations.
    pub observers: NonZeroUsize,
    /// Live key slots across lookup recipes, loader views, and cache entries.
    pub keys: NonZeroUsize,
    /// Live cache records across membership, pending work, and external observers.
    pub entries: NonZeroUsize,
    /// Success slots, including pending producers and values held after eviction.
    pub values: NonZeroUsize,
    /// Expected error slots, including pending producers and escaped errors.
    pub errors: NonZeroUsize,
    /// Concurrent pending, published, and observed compound outcomes.
    pub cleanup_outcomes: NonZeroUsize,
    /// Concurrent maintenance outcomes and separately reserved keys for
    /// maintenance recipes. Shutdown has an independent outcome carrier.
    pub maintenance_outcomes: NonZeroUsize,
    /// Retained compound failures after the first, with a separate overflow marker.
    pub cleanup_entries: usize,
}

/// Cache indexes and cells prepared in the caller's memory region.
///
/// The region holds cache entries, inline keys and outcomes, observer records,
/// and the driver, context, and completion records for selected loads.
/// Pointees owned by caller values, arbitrary loader work, and native provider
/// internals retain their existing allocation domains.
pub struct PreparedCacheStorage<K: Send + Sync + 'static, A: Send + 'static, E: Send + 'static> {
    pub(super) index: PreparedCacheIndex<K, A, E>,
    pub(super) payloads: Payloads<K, A, E>,
}

impl<K: Send + Sync + 'static, A: Send + 'static, E: Send + 'static> PreparedCacheStorage<K, A, E> {
    /// Layout bytes for indexes, inline payload cells and their pool metadata.
    /// The memory report accounts separately for allocator overhead and padding.
    pub fn storage_bytes(capacity: PreparedCacheCapacity) -> Result<Bytes, MemoryError> {
        let error_width = error_width(capacity)?;
        let rows = std::alloc::Layout::array::<ErrorLease<E>>(error_width)
            .map_err(|_| MemoryError::InvalidLayout)?;
        PreparedCacheIndex::<K, A, E>::storage_bytes(
            capacity.retention,
            capacity.active,
            capacity.lookups,
            capacity.diagnostics,
            capacity.observers,
        )?
        .checked_add(CellPool::<K>::storage_bytes(capacity.keys.get())?)?
        .checked_add(CellPool::<K>::storage_bytes(
            capacity.maintenance_outcomes.get(),
        )?)?
        .checked_add(CellPool::<Entry<K, A, E>>::storage_bytes(
            capacity.entries.get(),
        )?)?
        .checked_add(CellPool::<Mutex<A>>::storage_bytes(capacity.values.get())?)?
        .checked_add(CellPool::<Mutex<E>>::storage_bytes(capacity.errors.get())?)?
        .checked_add(PreparedCancellationBank::storage_bytes(
            capacity.entries.get(),
            capacity.observers.get(),
        )?)?
        .checked_add(FramePool::storage_bytes(
            loader_frames(capacity)?,
            loader_layout::<K, A, E>()?,
        )?)?
        .checked_add(SlotPool::<ErrorRows<E>>::storage_bytes(
            capacity.active.get(),
        )?)?
        .checked_add(Bytes::new(rows.size()).checked_mul(capacity.active.get())?)?
        .checked_add(cleanup_bytes::<E>(
            capacity.cleanup_outcomes.get(),
            capacity.cleanup_entries,
        )?)?
        .checked_add(prepared_notification::Event::storage_bytes(
            settlement_listeners(capacity)?,
        )?)?
        .checked_add(prepared_notification::Event::storage_bytes(
            close_listeners(capacity)?,
        )?)?
        .checked_add(cleanup_bytes::<E>(
            retirement_count(capacity)?,
            capacity.cleanup_entries,
        )?)
    }

    /// Provision the actual buffers and cells before accepting cache work.
    pub fn prepare(
        memory: &PreparedMemory<'static>,
        region: RegionId,
        capacity: PreparedCacheCapacity,
    ) -> Result<Self, MemoryError> {
        let _ = Self::storage_bytes(capacity)?;
        let index = PreparedCacheIndex::prepare(
            memory,
            region,
            capacity.retention,
            capacity.active,
            capacity.lookups,
            capacity.diagnostics,
            capacity.observers,
        )?;
        let keys = memory.slots(region, capacity.keys.get(), |_| Ok(None))?;
        let maintenance_keys =
            memory.slots(region, capacity.maintenance_outcomes.get(), |_| Ok(None))?;
        let entries = memory.slots(region, capacity.entries.get(), |_| Ok(None))?;
        let values = memory.slots(region, capacity.values.get(), |_| Ok(None))?;
        let errors = memory.slots(region, capacity.errors.get(), |_| Ok(None))?;
        let loader = memory.frames(
            region,
            loader_frames(capacity)?,
            loader_layout::<K, A, E>()?,
        )?;
        let cancellation = PreparedCancellationBank::prepare(
            memory,
            region,
            capacity.entries.get(),
            capacity.observers.get(),
        )?;
        let error_width = error_width(capacity)?;
        let error_rows = memory.slots(region, capacity.active.get(), |_| {
            FixedVector::prepare(memory.allocator(region)?, error_width)
        })?;
        let cleanup = prepare_cleanup(
            memory,
            region,
            capacity.cleanup_outcomes.get(),
            capacity.cleanup_entries,
        )?;
        let retirement = prepare_cleanup(
            memory,
            region,
            retirement_count(capacity)?,
            capacity.cleanup_entries,
        )?;
        let changed =
            prepared_notification::Event::prepare(memory, region, settlement_listeners(capacity)?)?;
        let closed =
            prepared_notification::Event::prepare(memory, region, close_listeners(capacity)?)?;
        Ok(Self {
            index,
            payloads: Payloads {
                keys,
                maintenance_keys,
                entries,
                values,
                errors,
                loader,
                cancellation,
                error_rows,
                cleanup,
                retirement,
                changed,
                closed,
                error_width,
                cleanup_entries: capacity.cleanup_entries,
                origin: Arc::new(()),
            },
        })
    }
}

fn settlement_listeners(capacity: PreparedCacheCapacity) -> Result<usize, MemoryError> {
    // Shutdown owns a preheld registration for its separate snapshot.
    capacity
        .observers
        .get()
        .checked_add(retirement_count(capacity)?)
        .ok_or(MemoryError::ArithmeticOverflow)
}

fn close_listeners(capacity: PreparedCacheCapacity) -> Result<usize, MemoryError> {
    capacity
        .observers
        .get()
        .checked_add(1)
        .ok_or(MemoryError::ArithmeticOverflow)
}

fn loader_frames(capacity: PreparedCacheCapacity) -> Result<usize, MemoryError> {
    // Driver, completion notice, parallel context, and two interpreter frames.
    capacity
        .active
        .get()
        .checked_mul(5)
        .ok_or(MemoryError::ArithmeticOverflow)
}

fn retirement_count(capacity: PreparedCacheCapacity) -> Result<usize, MemoryError> {
    capacity
        .maintenance_outcomes
        .get()
        .checked_add(1)
        .ok_or(MemoryError::ArithmeticOverflow)
}

fn cleanup_bytes<E: Send + 'static>(count: usize, retained: usize) -> Result<Bytes, MemoryError> {
    PreparedCleanupPool::<SharedValue<E>>::storage_bytes(count, retained)?.checked_add(
        FramePool::storage_bytes(count, PreparedCleanupPool::<SharedValue<E>>::frame_layout())?,
    )
}

fn prepare_cleanup<E: Send + 'static>(
    memory: &PreparedMemory<'static>,
    region: RegionId,
    count: usize,
    retained: usize,
) -> Result<PreparedCleanupConfig<SharedValue<E>>, MemoryError> {
    Ok(PreparedCleanupConfig::new(
        PreparedCleanupPool::prepare(memory, region, count, retained)?,
        memory.frames(
            region,
            count,
            PreparedCleanupPool::<SharedValue<E>>::frame_layout(),
        )?,
    ))
}

fn error_width(capacity: PreparedCacheCapacity) -> Result<usize, MemoryError> {
    // Include the primary error and first cleanup failure as well as the rest.
    let width = capacity
        .cleanup_entries
        .checked_add(2)
        .ok_or(MemoryError::ArithmeticOverflow)?;
    if width > capacity.errors.get() {
        return Err(MemoryError::NoSlot);
    }
    Ok(width)
}

pub(super) struct Payloads<K: Send + Sync + 'static, A: Send + 'static, E: Send + 'static> {
    keys: CellPool<K>,
    maintenance_keys: CellPool<K>,
    entries: CellPool<Entry<K, A, E>>,
    values: CellPool<Mutex<A>>,
    errors: CellPool<Mutex<E>>,
    pub(super) loader: FramePool<'static>,
    pub(super) cancellation: PreparedCancellationBank,
    error_rows: SlotPool<'static, ErrorRows<E>>,
    cleanup: PreparedCleanupConfig<SharedValue<E>>,
    retirement: PreparedCleanupConfig<SharedValue<E>>,
    pub(super) changed: prepared_notification::Event,
    pub(super) closed: prepared_notification::Event,
    error_width: usize,
    cleanup_entries: usize,
    origin: Arc<()>,
}

impl<K: Send + Sync + 'static, A: Send + 'static, E: Send + 'static> Payloads<K, A, E> {
    pub(super) fn reserve_key(&self) -> Result<CellLease<K>, MemoryError> {
        self.keys.try_checkout()
    }

    pub(super) fn reserve_maintenance_key(&self) -> Result<CellLease<K>, MemoryError> {
        self.maintenance_keys.try_checkout()
    }

    pub(super) fn reserve_error(&self) -> Result<CellLease<Mutex<E>>, MemoryError> {
        self.errors.try_checkout()
    }

    pub(super) fn reserve_entry(&self) -> Result<EntryLease<K, A, E>, MemoryError> {
        self.entries.try_checkout()
    }

    pub(super) fn publish_key(&self, lease: CellLease<K>, key: K) -> CacheKey<K> {
        CacheKey {
            cell: Cell::publish(lease, key),
            origin: self.origin.clone(),
        }
    }

    pub(super) fn owns_key(&self, key: &CacheKey<K>) -> bool {
        Arc::ptr_eq(&self.origin, &key.origin)
    }

    pub(super) fn reserve_outcome(&self) -> Result<OutcomeStorage<A, E>, MemoryError> {
        let value = self.values.try_checkout()?;
        let rest = self.reserve_cleanup()?;
        let mut errors = WorkList::checkout(&self.error_rows)?;
        for _ in 0..self.error_width {
            errors.push_reserved(self.errors.try_checkout()?);
        }
        Ok(OutcomeStorage::Prepared(PreparedOutcome {
            value,
            errors,
            rest,
            retained: self.cleanup_entries,
        }))
    }

    pub(super) fn reserve_success(&self) -> Result<OutcomeStorage<A, E>, MemoryError> {
        Ok(OutcomeStorage::Prepared(PreparedOutcome {
            value: self.values.try_checkout()?,
            errors: WorkList::default(),
            rest: self.reserve_cleanup()?,
            retained: self.cleanup_entries,
        }))
    }

    pub(super) fn reserve_cleanup(&self) -> Result<Cleanup<E>, MemoryError> {
        self.cleanup.reserve_failures()
    }

    pub(super) fn reserve_retirement(&self) -> Result<Cleanup<E>, MemoryError> {
        self.retirement.reserve_failures()
    }
}

pub(super) enum OutcomeStorage<A: Send + 'static, E: Send + 'static> {
    Ordinary,
    Prepared(PreparedOutcome<A, E>),
}

impl<A: Send + 'static, E: Send + 'static> OutcomeStorage<A, E> {
    pub(super) fn publish(
        self,
        outcome: Exit<A, E>,
        ttl_failure: Option<Defect>,
    ) -> Exit<SharedValue<A>, SharedValue<E>> {
        match self {
            Self::Ordinary => {
                let outcome = if let Some(defect) = ttl_failure {
                    crate::internal::outcome::append_cleanup(outcome, Exit::Die(defect))
                } else {
                    outcome
                };
                super::share(outcome)
            }
            Self::Prepared(prepared) => prepared.publish(outcome, ttl_failure),
        }
    }
}

pub(super) struct PreparedOutcome<A: Send + 'static, E: Send + 'static> {
    value: CellLease<Mutex<A>>,
    errors: WorkList<ErrorLease<E>>,
    rest: Cleanup<E>,
    retained: usize,
}

impl<A: Send + 'static, E: Send + 'static> PreparedOutcome<A, E> {
    fn publish(
        self,
        outcome: Exit<A, E>,
        ttl_failure: Option<Defect>,
    ) -> Exit<SharedValue<A>, SharedValue<E>> {
        let Self {
            value,
            mut errors,
            mut rest,
            retained,
        } = self;
        let mut slots = errors.drain();
        #[allow(clippy::expect_used)]
        // Admission covers the primary, first, and retained subsequent failures.
        let mut share_error = |error| {
            SharedValue::from_reserved(
                slots.next().expect("preadmitted cache outcome error cell"),
                error,
            )
        };
        let outcome = match outcome {
            Exit::Success(payload) => Exit::Success(SharedValue::from_reserved(value, payload)),
            Exit::Fail(error) => Exit::Fail(share_error(error)),
            Exit::Die(defect) => Exit::Die(defect),
            Exit::Interrupted => Exit::Interrupted,
            Exit::FinalizationFailed(details) => {
                let primary = match details.primary {
                    PrimaryOutcome::Succeeded => PrimaryOutcome::Succeeded,
                    PrimaryOutcome::Failed(error) => PrimaryOutcome::Failed(share_error(error)),
                    PrimaryOutcome::Defected(defect) => PrimaryOutcome::Defected(defect),
                    PrimaryOutcome::WasInterrupted => PrimaryOutcome::WasInterrupted,
                };
                let first = map_failure(details.first, &mut share_error);
                let mut retained_count = 0;
                for failure in details.rest {
                    let is_summary = matches!(&failure, CleanupFailure::ReleaseDefect(defect)
                        if matches!(defect.kind(), DefectKind::Library(LibraryDefect::CleanupOverflow { .. })));
                    if is_summary || retained_count < retained {
                        rest.push(map_failure(failure, &mut share_error));
                        retained_count += usize::from(!is_summary);
                    } else {
                        rest.push(CleanupFailure::ReleaseDefect(Defect::library(
                            LibraryDefect::CleanupOverflow {
                                omitted: 1,
                                saturated: false,
                            },
                        )));
                        drop(failure);
                    }
                }
                return Exit::FinalizationFailed(Finalization {
                    primary,
                    first,
                    rest,
                });
            }
        };
        if let Some(defect) = ttl_failure {
            let mut failed = PreparedCleanupAccumulator::from_reserved(outcome, rest);
            failed.append_cleanup(Exit::Die(defect));
            return failed.finish();
        }
        outcome
    }
}

fn map_failure<E: Send + 'static>(
    failure: CleanupFailure<E>,
    share_error: &mut impl FnMut(E) -> SharedValue<E>,
) -> CleanupFailure<SharedValue<E>> {
    match failure {
        CleanupFailure::ReleaseFailure(error) => CleanupFailure::ReleaseFailure(share_error(error)),
        CleanupFailure::ReleaseDefect(defect) => CleanupFailure::ReleaseDefect(defect),
        CleanupFailure::ReleaseInterrupted => CleanupFailure::ReleaseInterrupted,
    }
}
