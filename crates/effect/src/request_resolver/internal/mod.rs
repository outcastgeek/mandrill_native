//! Private coordination of shared pending request loads.

use super::{
    BatchRunner, HasRequestScope, RequestBatchInput, RequestResolverError, RequestScope,
    ResolverRunner,
};
use crate::fiber::{PreparedForkDispatch, PreparedUnitFork, PreparedUnitForks};
use crate::internal::notification::notify_all;
use crate::internal::outcome;
use crate::internal::runtime::{self, Context, Step};
use crate::memory::prepared_notification;
use crate::memory::{
    Bytes, FixedVector, FramePool, MemoryError, PreparedMemory, RegionId, SharedSlot, SlotLease,
    SlotPool,
};
use crate::{
    Defect, Effect, Exit, Fiber, HasTaskSpawner, LibraryDefect, Request, RequestGroup, RequestKey,
    Scope, SharedValue,
};
use event_listener::Event;
use futures_util::{
    FutureExt,
    future::{Either, select},
};
use std::collections::{BTreeMap, HashMap};
use std::future::{Future, poll_fn};
use std::panic::AssertUnwindSafe;
use std::pin::{Pin, pin};
use std::sync::{Arc, Mutex};

mod direct_batch;
mod prepared_batch;
mod results;
use direct_batch::{BatchTracking, PreparedBatchTask, PreparedInvocation};
pub use prepared_batch::PreparedRequestBatch;
use prepared_batch::{
    BatchValues, CompletionPools, CompletionStorage, Diagnostics, Position, ValueTable,
    clone_with_diagnostics,
};
pub use results::PreparedResolvedBatch;
pub(super) use results::ResultCollector;
use results::ResultPool;

/// Type alias for shared result in request resolver internal mod.
///
/// # Type parameters
/// - `A`: Application value type.
/// - `E`: Domain-error type.
type SharedResult<A, E> = Exit<SharedValue<A>, RequestResolverError<E>>;
/// Type alias for grouped requests in request resolver internal mod.
///
/// # Type parameters
/// - `Q`: Request type.
/// - `A`: Application value type.
/// - `E`: Domain-error type.
type GroupedRequests<Q, A, E> = BTreeMap<RequestGroup, Vec<(Q, PendingRef<A, E>)>>;
type PendingEntries<A, E> = FixedVector<'static, PendingRef<A, E>>;
type PendingRecords<A, E> = SlotPool<'static, Option<Pending<A, E>>>;
type ForkPermit = (SharedSlot<'static, ()>, SharedSlot<'static, ()>);
type GroupItem<Q, A, E> = (RequestGroup, usize, Option<Q>, PendingRef<A, E>);
type EntryRange<A, E> = (
    std::ops::Range<usize>,
    SharedSlot<'static, PendingEntries<A, E>>,
);
type CapacityMapper<E> = dyn Fn(MemoryError) -> RequestResolverError<E> + Send + Sync;
pub(super) type PreparedConfiguration<Q, A, E> =
    (PreparedResolverIndex<Q, A, E>, Arc<CapacityMapper<E>>);

/// Explicit limits for prepared request resolution.
#[derive(Clone, Copy, Debug)]
pub struct PreparedResolverCapacity {
    /// Distinct currently pending keys.
    pub pending: usize,
    /// Concurrent batches, including batches suspended on a shared result.
    pub invocations: usize,
    /// Input positions per batch, including duplicate requests.
    pub requests_per_batch: usize,
    /// Maximum encoded bytes in an admitted key identity.
    pub key_bytes: usize,
    /// Maximum encoded bytes in an admitted batch identity.
    pub group_bytes: usize,
    /// Each pool's maximum live success values or expected errors, including
    /// cells reserved for pending requests and values retained by callers.
    pub retained_values: usize,
    /// Total prepared cleanup destinations across admitted work and live results.
    pub cleanup_copies: usize,
    /// Secondary failures retained per cleanup destination beyond the first.
    /// Additional secondary failures are represented by the cleanup overflow diagnostic.
    pub cleanup_retained: usize,
    /// Simultaneous prepared cancellation listeners per selected loader group.
    pub child_cancellation_listeners: usize,
}

/// Actual pending membership and invocation admission for a scoped resolver.
pub struct PreparedResolverIndex<Q: Send + 'static, A: Send + 'static, E: Send + 'static> {
    capacity: PreparedResolverCapacity,
    entries: SlotPool<'static, PendingEntries<A, E>>,
    invocations: SlotPool<'static, PreparedWork<Q, A, E>>,
    batches: SlotPool<'static, PendingEntries<A, E>>,
    values: Option<SlotPool<'static, ValueTable<Q, A, E>>>,
    results: Option<ResultPool<A, E>>,
    completion: Option<CompletionPools<A, E>>,
    pending_records: PendingRecords<A, E>,
    changes: prepared_notification::Event,
    fork_permits: SlotPool<'static, ()>,
    prepared_forks: PreparedUnitForks<RequestResolverError<E>>,
    batch_evaluation: Option<FramePool<'static>>,
}
impl<Q: Send + 'static, A: Send + 'static, E: Send + 'static> PreparedResolverIndex<Q, A, E> {
    /// Payload bytes for membership, invocation scratch, and shared fork tracking.
    /// The region also needs allocation bookkeeping and alignment space.
    pub fn storage_bytes(capacity: PreparedResolverCapacity) -> Result<Bytes, MemoryError> {
        let entries = std::alloc::Layout::array::<PendingRef<A, E>>(capacity.pending)
            .map_err(|_| MemoryError::InvalidLayout)?;
        let width = capacity.requests_per_batch;
        let scratch = Bytes::new(
            std::alloc::Layout::array::<PendingRef<A, E>>(width)
                .map_err(|_| MemoryError::InvalidLayout)?
                .size(),
        )
        .checked_add(Bytes::new(
            std::alloc::Layout::array::<GroupItem<Q, A, E>>(width)
                .map_err(|_| MemoryError::InvalidLayout)?
                .size(),
        ))?
        .checked_add(Bytes::new(
            std::alloc::Layout::array::<Q>(width)
                .map_err(|_| MemoryError::InvalidLayout)?
                .size(),
        ))?
        .checked_add(Bytes::new(
            std::alloc::Layout::array::<PreparedLaunch<Q, A, E>>(width)
                .map_err(|_| MemoryError::InvalidLayout)?
                .size(),
        ))?
        .checked_add(Bytes::new(
            std::alloc::Layout::array::<Diagnostics<E>>(width)
                .map_err(|_| MemoryError::InvalidLayout)?
                .size(),
        ))?
        .checked_mul(capacity.invocations)?;
        let batch_slots = capacity.batch_slots()?;
        let batch_data = Bytes::new(
            std::alloc::Layout::array::<PendingRef<A, E>>(width)
                .map_err(|_| MemoryError::InvalidLayout)?
                .size(),
        )
        .checked_mul(batch_slots)?;
        SlotPool::<PendingEntries<A, E>>::storage_bytes(1)?
            .checked_add(Bytes::new(entries.size()))?
            .checked_add(SlotPool::<PreparedWork<Q, A, E>>::storage_bytes(
                capacity.invocations,
            )?)?
            .checked_add(scratch)?
            .checked_add(SlotPool::<PendingEntries<A, E>>::storage_bytes(
                batch_slots,
            )?)?
            .checked_add(batch_data)?
            .checked_add(PendingRecords::<A, E>::storage_bytes(
                capacity.record_slots()?,
            )?)?
            .checked_add(prepared_notification::Event::storage_bytes(
                capacity.invocations,
            )?)?
            .checked_add(SlotPool::<()>::storage_bytes(batch_slots)?)?
            .checked_add(PreparedUnitForks::<RequestResolverError<E>>::storage_bytes(
                batch_slots,
            )?)
    }

    /// Payload bytes for prepared membership and owned per-request input/output storage.
    pub fn storage_bytes_with_batches<
        R: HasTaskSpawner + HasRequestScope + Send + Sync + 'static,
    >(
        capacity: PreparedResolverCapacity,
    ) -> Result<Bytes, MemoryError> {
        let batches = capacity.batch_slots()?;
        let payload = std::alloc::Layout::array::<Position<Q, A, E>>(capacity.requests_per_batch)
            .map_err(|_| MemoryError::InvalidLayout)?;
        let outcomes = std::alloc::Layout::array::<SharedResult<A, E>>(capacity.requests_per_batch)
            .map_err(|_| MemoryError::InvalidLayout)?;
        Self::storage_bytes(capacity)?
            .checked_add(SlotPool::<ValueTable<Q, A, E>>::storage_bytes(batches)?)?
            .checked_add(Bytes::new(payload.size()).checked_mul(batches)?)?
            .checked_add(ResultPool::<A, E>::storage_bytes(capacity.invocations)?)?
            .checked_add(Bytes::new(outcomes.size()).checked_mul(capacity.invocations)?)?
            .checked_add(CompletionPools::<A, E>::storage_bytes(
                capacity.retained_values,
                capacity.cleanup_copies,
                capacity.cleanup_retained,
            )?)?
            .checked_add(PreparedInvocation::<Q, A, E>::storage_bytes(batches)?)?
            .checked_add(
                PreparedUnitForks::<RequestResolverError<E>>::task_storage_bytes(
                    batches,
                    capacity.child_cancellation_listeners,
                    PreparedBatchTask::<Q, A, E, R>::layout(),
                )?,
            )
    }

    /// Provision the fixed index and independent invocation admissions.
    pub fn prepare(
        memory: &PreparedMemory<'static>,
        region: RegionId,
        capacity: PreparedResolverCapacity,
    ) -> Result<Self, MemoryError> {
        let _ = Self::storage_bytes(capacity)?;
        let entries = memory.slots(region, 1, |_| {
            FixedVector::prepare(memory.allocator(region)?, capacity.pending)
        })?;
        let invocations = memory.slots(region, capacity.invocations, |_| {
            let width = capacity.requests_per_batch;
            Ok(PreparedWork {
                selected: FixedVector::prepare(memory.allocator(region)?, width)?,
                groups: FixedVector::prepare(memory.allocator(region)?, width)?,
                discard: FixedVector::prepare(memory.allocator(region)?, width)?,
                launches: FixedVector::prepare(memory.allocator(region)?, width)?,
                observations: FixedVector::prepare(memory.allocator(region)?, width)?,
                values: None,
            })
        })?;
        let batches = memory.slots(region, capacity.batch_slots()?, |_| {
            FixedVector::prepare(memory.allocator(region)?, capacity.requests_per_batch)
        })?;
        let pending_records = memory.slots(region, capacity.record_slots()?, |_| Ok(None))?;
        let changes = prepared_notification::Event::prepare(memory, region, capacity.invocations)?;
        let fork_permits = memory.slots(region, capacity.batch_slots()?, |_| Ok(()))?;
        let prepared_forks = PreparedUnitForks::prepare(memory, region, capacity.batch_slots()?)?;
        Ok(Self {
            capacity,
            entries,
            invocations,
            batches,
            values: None,
            results: None,
            completion: None,
            pending_records,
            changes,
            fork_permits,
            prepared_forks,
            batch_evaluation: None,
        })
    }

    /// Provision the existing index and fixed per-batch request/outcome ownership.
    pub fn prepare_with_batches<R: HasTaskSpawner + HasRequestScope + Send + Sync + 'static>(
        memory: &PreparedMemory<'static>,
        region: RegionId,
        capacity: PreparedResolverCapacity,
    ) -> Result<Self, MemoryError> {
        let _ = Self::storage_bytes_with_batches::<R>(capacity)?;
        let mut index = Self::prepare(memory, region, capacity)?;
        index.values = Some(memory.slots(region, capacity.batch_slots()?, |_| {
            Ok(Mutex::new(FixedVector::prepare(
                memory.allocator(region)?,
                capacity.requests_per_batch,
            )?))
        })?);
        index.results = Some(memory.slots(region, capacity.invocations, |_| {
            FixedVector::prepare(memory.allocator(region)?, capacity.requests_per_batch)
        })?);
        index.completion = Some(CompletionPools::prepare(
            memory,
            region,
            capacity.retained_values,
            capacity.cleanup_copies,
            capacity.cleanup_retained,
        )?);
        index.batch_evaluation = Some(PreparedInvocation::<Q, A, E>::prepare(
            memory,
            region,
            capacity.batch_slots()?,
        )?);
        index.prepared_forks.prepare_tasks(
            memory,
            region,
            capacity.batch_slots()?,
            capacity.child_cancellation_listeners,
            PreparedBatchTask::<Q, A, E, R>::layout(),
        )?;
        Ok(index)
    }

    /// Share the same physical storage without opening an independent budget.
    pub fn try_clone(&self) -> Result<Self, MemoryError> {
        Ok(Self {
            capacity: self.capacity,
            entries: self.entries.try_clone()?,
            invocations: self.invocations.try_clone()?,
            batches: self.batches.try_clone()?,
            values: self.values.as_ref().map(SlotPool::try_clone).transpose()?,
            results: self.results.as_ref().map(SlotPool::try_clone).transpose()?,
            completion: self
                .completion
                .as_ref()
                .map(CompletionPools::try_clone)
                .transpose()?,
            pending_records: self.pending_records.try_clone()?,
            changes: self.changes.try_clone()?,
            fork_permits: self.fork_permits.try_clone()?,
            prepared_forks: self.prepared_forks.try_clone()?,
            batch_evaluation: self
                .batch_evaluation
                .as_ref()
                .map(FramePool::try_clone)
                .transpose()?,
        })
    }

    pub(super) fn has_batch_storage(&self) -> bool {
        self.values.is_some()
    }
}

impl PreparedResolverCapacity {
    fn batch_slots(&self) -> Result<usize, MemoryError> {
        // A caller or fresh pending keys keep an outstanding dispatch active.
        // Physical admission also accounts for slots still being retired.
        self.pending
            .checked_add(self.invocations)
            .ok_or(MemoryError::ArithmeticOverflow)
    }

    fn record_slots(&self) -> Result<usize, MemoryError> {
        self.invocations
            .checked_mul(self.requests_per_batch)
            .and_then(|observers| self.pending.checked_add(observers))
            .ok_or(MemoryError::ArithmeticOverflow)
    }
}

struct PreparedEntries<A: Send + 'static, E: Send + 'static>(Option<EntryRange<A, E>>);

impl<A: Send + 'static, E: Send + 'static> Drop for PreparedEntries<A, E> {
    fn drop(&mut self) {
        if let Some((_, shared)) = self.0.take()
            && let Some(mut entries) = shared.into_last()
        {
            entries.clear();
        }
    }
}

impl<A: Send + 'static, E: Send + 'static> PreparedEntries<A, E> {
    fn range(&self, range: std::ops::Range<usize>) -> Result<Self, MemoryError> {
        let (_, shared) = self.0.as_ref().ok_or(MemoryError::MissingRuntimeStorage)?;
        shared
            .as_slice()
            .get(range.clone())
            .ok_or(MemoryError::InvalidRange)?;
        Ok(Self(Some((range, shared.try_clone()?))))
    }

    fn entries(&self) -> &[PendingRef<A, E>] {
        self.0
            .as_ref()
            .and_then(|(range, shared)| shared.as_slice().get(range.clone()))
            .unwrap_or_default()
    }
}

enum BatchEntries<A: Send + 'static, E: Send + 'static> {
    Ordinary(Vec<PendingRef<A, E>>),
    Prepared(PreparedEntries<A, E>),
}

impl<A: Send + 'static, E: Send + 'static> BatchEntries<A, E> {
    fn entries(&self) -> &[PendingRef<A, E>] {
        match self {
            Self::Ordinary(entries) => entries,
            Self::Prepared(entries) => entries.entries(),
        }
    }
}

struct PreparedLaunch<Q: Send + 'static, A: Send + 'static, E: Send + 'static> {
    width: usize,
    parent: PreparedEntries<A, E>,
    child: PreparedEntries<A, E>,
    values: Option<PreparedInvocation<Q, A, E>>,
    settlement: prepared_notification::Event,
    permits: ForkPermit,
    fork: PreparedUnitFork<RequestResolverError<E>>,
}

#[allow(clippy::large_enum_variant)] // The prepared invocation carries its admitted frame handles inline.
enum BatchPayload<Q: Send + 'static, A: Send + 'static, E: Send + 'static> {
    Ordinary(Vec<Q>),
    Prepared(PreparedInvocation<Q, A, E>),
}

#[allow(clippy::large_enum_variant)] // Keep the preheld group controls inline through dispatcher transfer.
enum BatchParent<A: Send + 'static, E: Send + 'static> {
    Ordinary(Vec<PendingRef<A, E>>),
    Prepared {
        entries: PreparedEntries<A, E>,
        settlement: prepared_notification::Event,
        permits: ForkPermit,
        fork: PreparedUnitFork<RequestResolverError<E>>,
    },
}

struct Batch<Q: Send + 'static, A: Send + 'static, E: Send + 'static> {
    group: RequestGroup,
    payloads: BatchPayload<Q, A, E>,
    parent: BatchParent<A, E>,
    child: BatchEntries<A, E>,
}

struct PreparedWork<Q: Send + 'static, A: Send + 'static, E: Send + 'static> {
    selected: PendingEntries<A, E>,
    groups: FixedVector<'static, GroupItem<Q, A, E>>,
    discard: FixedVector<'static, Q>,
    launches: FixedVector<'static, PreparedLaunch<Q, A, E>>,
    values: Option<BatchValues<Q, A, E>>,
    observations: FixedVector<'static, Diagnostics<E>>,
}

struct Invocation<Q: Send + 'static, A: Send + 'static, E: Send + 'static>(
    SlotLease<'static, PreparedWork<Q, A, E>>,
    prepared_notification::Registration,
);

impl<Q: Send + 'static, A: Send + 'static, E: Send + 'static> Drop for Invocation<Q, A, E> {
    fn drop(&mut self) {
        let work = &mut *self.0;
        let _values = work.values.take();
        let _selected = std::mem::DropGuard::new(&mut work.selected, FixedVector::clear);
        let _groups = std::mem::DropGuard::new(&mut work.groups, FixedVector::clear);
        let _discard = std::mem::DropGuard::new(&mut work.discard, FixedVector::clear);
        let _observations = std::mem::DropGuard::new(&mut work.observations, FixedVector::clear);
        work.launches.clear();
    }
}

enum Work<Q: Send + 'static, A: Send + 'static, E: Send + 'static> {
    Ordinary {
        selected: Vec<PendingRef<A, E>>,
        groups: GroupedRequests<Q, A, E>,
        discard: Vec<Q>,
    },
    Prepared(Invocation<Q, A, E>),
}

impl<Q: Send + 'static, A: Send + 'static, E: Send + 'static> Work<Q, A, E> {
    fn observer(&self) -> Option<&prepared_notification::Registration> {
        match self {
            Self::Ordinary { .. } => None,
            Self::Prepared(work) => Some(&work.1),
        }
    }

    fn reserve_observations(
        &mut self,
        count: usize,
        completion: &CompletionPools<A, E>,
    ) -> Result<(), MemoryError> {
        let Self::Prepared(work) = self else {
            return Err(MemoryError::MissingRuntimeStorage);
        };
        for _ in 0..count {
            work.0
                .observations
                .push_reserved(completion.reserve_diagnostics()?);
        }
        Ok(())
    }

    fn next_observation(&mut self) -> Option<Diagnostics<E>> {
        match self {
            Self::Ordinary { .. } => None,
            Self::Prepared(work) => work.0.observations.pop(),
        }
    }

    fn select(&mut self, entry: PendingRef<A, E>) {
        match self {
            Self::Ordinary { selected, .. } => selected.push(entry),
            Self::Prepared(work) => work.0.selected.push_reserved(entry),
        }
    }

    fn load(&mut self, position: usize, group: RequestGroup, payload: Q, entry: PendingRef<A, E>) {
        match self {
            Self::Ordinary { groups, .. } => {
                groups.entry(group).or_default().push((payload, entry))
            }
            Self::Prepared(work) => {
                work.0
                    .groups
                    .push_reserved((group, position, Some(payload), entry))
            }
        }
    }

    fn discard(&mut self, payload: Q) {
        match self {
            Self::Ordinary { discard, .. } => discard.push(payload),
            Self::Prepared(work) => work.0.discard.push_reserved(payload),
        }
    }

    fn staged(&self, key: &RequestKey) -> Option<&PendingRef<A, E>> {
        match self {
            Self::Ordinary { .. } => None,
            Self::Prepared(work) => work
                .0
                .groups
                .as_slice()
                .iter()
                .find_map(|(_, _, _, entry)| (entry.key == *key).then_some(entry)),
        }
    }

    fn prepare_groups<R: HasTaskSpawner + HasRequestScope + Send + Sync + 'static>(
        &mut self,
        storage: &PreparedResolverIndex<Q, A, E>,
        pending: &mut PendingIndex<A, E>,
        prepare_values: bool,
    ) -> Result<(), MemoryError> {
        let Self::Prepared(work) = self else {
            return Err(MemoryError::MissingRuntimeStorage);
        };
        if work.0.groups.is_empty() {
            return Ok(());
        }
        // Reversed for pop(). Original input position breaks same-group ties.
        work.0
            .groups
            .as_mut_slice()
            .sort_unstable_by(|a, b| b.0.cmp(&a.0).then_with(|| b.1.cmp(&a.1)));
        let mut entries = storage.batches.try_checkout()?;
        for (_, _, _, entry) in work.0.groups.as_slice().iter().rev() {
            entries.push_reserved(entry.clone());
        }
        let width = entries.len();
        let shared = PreparedEntries(Some((0..width, entries.share())));
        if prepare_values {
            let pool = storage
                .values
                .as_ref()
                .ok_or(MemoryError::MissingRuntimeStorage)?;
            let completion = storage
                .completion
                .as_ref()
                .ok_or(MemoryError::MissingRuntimeStorage)?;
            let mut values = std::mem::DropGuard::new(
                pool.try_checkout()?,
                |mut values: SlotLease<'static, ValueTable<Q, A, E>>| {
                    values
                        .get_mut()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .clear();
                },
            );
            let positions = values
                .get_mut()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            for _ in 0..width {
                positions.push_reserved(Position::new(completion.reserve()?));
            }
            for (position, (_, _, payload, _)) in positions
                .as_mut_slice()
                .iter_mut()
                .zip(work.0.groups.as_mut_slice().iter_mut().rev())
            {
                position.request = payload.take();
            }
            work.0.values = Some(BatchValues::new(
                std::mem::DropGuard::dismiss(values).share(),
            ));
        }
        let mut start = 0;
        for batch in shared.entries().chunk_by(|a, b| a.group == b.group) {
            let width = batch.len();
            let end = start + width;
            let parent = shared.range(start..end)?;
            let child = shared.range(start..end)?;
            let values = work
                .0
                .values
                .as_ref()
                .map(|values| {
                    let completion = storage
                        .completion
                        .as_ref()
                        .ok_or(MemoryError::MissingRuntimeStorage)?;
                    let pool = storage
                        .batch_evaluation
                        .as_ref()
                        .ok_or(MemoryError::MissingRuntimeStorage)?;
                    let values = values.view(start..end, completion.reserve_diagnostics()?)?;
                    PreparedInvocation::reserve(pool, values)
                })
                .transpose()?;
            let finalizer = storage.fork_permits.try_checkout()?.share();
            let task = finalizer.try_clone()?;
            let task_layout = values
                .as_ref()
                .map(|_| PreparedBatchTask::<Q, A, E, R>::layout());
            work.0.launches.push_reserved(PreparedLaunch {
                width,
                parent,
                child,
                values,
                settlement: storage.changes.try_clone()?,
                permits: (finalizer, task),
                fork: storage.prepared_forks.reserve(task_layout)?,
            });
            start = end;
        }
        work.0.launches.as_mut_slice().reverse();

        // All regions and shared controls have been admitted. Publication
        // below cannot refuse the invocation partway through insertion.
        for entry in shared.entries() {
            pending.insert_reserved(entry.key.clone(), entry.clone());
        }
        Ok(())
    }

    fn next_group(&mut self) -> Option<Batch<Q, A, E>> {
        match self {
            Self::Ordinary { groups, .. } => {
                let (group, batch) = groups.pop_first()?;
                let parent = batch.iter().map(|(_, entry)| entry.clone()).collect();
                let (payloads, children) = batch.into_iter().unzip();
                Some(Batch {
                    group,
                    payloads: BatchPayload::Ordinary(payloads),
                    parent: BatchParent::Ordinary(parent),
                    child: BatchEntries::Ordinary(children),
                })
            }
            Self::Prepared(work) => {
                let launch = work.0.launches.pop()?;
                let (group, _, payload, _) = work.0.groups.pop()?;
                let payloads = if let Some(batch) = launch.values {
                    for _ in 1..launch.width {
                        work.0.groups.pop()?;
                    }
                    BatchPayload::Prepared(batch)
                } else {
                    // The compatibility callback takes an owned Vec.
                    let mut payloads = Vec::with_capacity(launch.width);
                    payloads.push(payload?);
                    for _ in 1..launch.width {
                        let (_, _, payload, _) = work.0.groups.pop()?;
                        payloads.push(payload?);
                    }
                    BatchPayload::Ordinary(payloads)
                };
                Some(Batch {
                    group,
                    payloads,
                    parent: BatchParent::Prepared {
                        entries: launch.parent,
                        settlement: launch.settlement,
                        permits: launch.permits,
                        fork: launch.fork,
                    },
                    child: BatchEntries::Prepared(launch.child),
                })
            }
        }
    }

    fn drop_duplicates(&mut self) {
        match self {
            Self::Ordinary { discard, .. } => discard.clear(),
            Self::Prepared(work) => work.0.discard.clear(),
        }
    }

    fn order_results(&mut self) -> usize {
        match self {
            Self::Ordinary { selected, .. } => {
                selected.reverse();
                selected.len()
            }
            Self::Prepared(work) => {
                work.0.selected.as_mut_slice().reverse();
                work.0.selected.len()
            }
        }
    }

    fn next_selected(&mut self) -> Option<PendingRef<A, E>> {
        match self {
            Self::Ordinary { selected, .. } => selected.pop(),
            Self::Prepared(work) => work.0.selected.pop(),
        }
    }
}

struct PreparedPendingEntries<A: Send + 'static, E: Send + 'static>(
    SlotLease<'static, PendingEntries<A, E>>,
);
impl<A: Send + 'static, E: Send + 'static> Drop for PreparedPendingEntries<A, E> {
    fn drop(&mut self) {
        self.0.clear();
    }
}

enum PendingIndex<A: Send + 'static, E: Send + 'static> {
    Ordinary(HashMap<RequestKey, PendingRef<A, E>>),
    Prepared(PreparedPendingEntries<A, E>),
}
impl<A: Send + 'static, E: Send + 'static> Default for PendingIndex<A, E> {
    fn default() -> Self {
        Self::Ordinary(HashMap::new())
    }
}
impl<A: Send + 'static, E: Send + 'static> PendingIndex<A, E> {
    fn get(&self, key: &RequestKey) -> Option<&PendingRef<A, E>> {
        match self {
            Self::Ordinary(entries) => entries.get(key),
            Self::Prepared(entries) => entries.0.as_slice().iter().find(|entry| entry.key == *key),
        }
    }

    fn remove(&mut self, key: &RequestKey) -> Option<PendingRef<A, E>> {
        match self {
            Self::Ordinary(entries) => entries.remove(key),
            Self::Prepared(entries) => {
                let position = entries
                    .0
                    .as_slice()
                    .iter()
                    .position(|entry| entry.key == *key)?;
                entries.0.remove_at(position)
            }
        }
    }

    fn insert_reserved(&mut self, key: RequestKey, entry: PendingRef<A, E>) {
        match self {
            Self::Ordinary(entries) => {
                entries.insert(key, entry);
            }
            Self::Prepared(entries) => entries.0.push_reserved(entry),
        }
    }
}

/// Typed registry used by request resolver internal mod.
///
/// # Type parameters
/// - `A`: Application value type. Bound or type: `Send + 'static`.
/// - `E`: Domain-error type. Bound or type: `Send + 'static`.
struct Registry<A: Send + 'static, E: Send + 'static> {
    closed: bool,
    pending: PendingIndex<A, E>,
}

fn validate_ordinary<Q, A: Send + 'static, E: Send + 'static>(
    pending: &PendingIndex<A, E>,
    requests: &[Request<Q, A, E>],
) -> Result<(), RequestResolverError<E>> {
    let mut seen = HashMap::new();
    for request in requests {
        if seen
            .insert(request.key(), request.group())
            .is_some_and(|group| group != request.group())
            || pending
                .get(request.key())
                .is_some_and(|entry| entry.group != *request.group())
        {
            return Err(RequestResolverError::ConflictingGroup(
                request.key().clone(),
            ));
        }
    }
    Ok(())
}

fn prepare_invocation<Q: Send + 'static, A: Send + 'static, E: Send + 'static>(
    pending: &PendingIndex<A, E>,
    requests: &[Request<Q, A, E>],
    index: &PreparedResolverIndex<Q, A, E>,
) -> Result<Invocation<Q, A, E>, RequestResolverError<E>> {
    let capacity = index.capacity;
    if requests.len() > capacity.requests_per_batch {
        return Err(RequestResolverError::Memory(
            MemoryError::CapacityExceeded {
                resource: 0,
                requested: requests.len(),
                available: capacity.requests_per_batch,
                control: true,
            },
        ));
    }
    let mut fresh = 0;
    for (position, request) in requests.iter().enumerate() {
        for (requested, capacity) in [
            (request.key().as_bytes().len(), capacity.key_bytes),
            (request.group().as_bytes().len(), capacity.group_bytes),
        ] {
            if requested > capacity {
                return Err(RequestResolverError::Memory(MemoryError::ValueTooLarge {
                    requested: Bytes::new(requested),
                    capacity: Bytes::new(capacity),
                }));
            }
        }
        let earlier = requests
            .iter()
            .take(position)
            .find(|prior| prior.key() == request.key());
        let active = pending.get(request.key());
        if earlier.is_some_and(|prior| prior.group() != request.group())
            || active.is_some_and(|entry| entry.group != *request.group())
        {
            return Err(RequestResolverError::ConflictingGroup(
                request.key().clone(),
            ));
        }
        fresh += usize::from(earlier.is_none() && active.is_none());
    }
    let PendingIndex::Prepared(entries) = pending else {
        return Err(RequestResolverError::Memory(
            MemoryError::MissingRuntimeStorage,
        ));
    };
    let available = entries.0.capacity() - entries.0.len();
    if fresh > available {
        return Err(RequestResolverError::Memory(
            MemoryError::CapacityExceeded {
                resource: 0,
                requested: fresh,
                available,
                control: true,
            },
        ));
    }
    let work = index
        .invocations
        .try_checkout()
        .map_err(RequestResolverError::Memory)?;
    let registered = index
        .changes
        .listen()
        .map_err(RequestResolverError::Memory)?;
    Ok(Invocation(work, registered))
}

/// Typed pending state used by request resolver internal mod.
///
/// # Type parameters
/// - `A`: Application value type. Bound or type: `Send + 'static`.
/// - `E`: Domain-error type. Bound or type: `Send + 'static`.
struct PendingState<A: Send + 'static, E: Send + 'static> {
    outcome: Option<SharedResult<A, E>>,
    fiber: Option<Fiber<(), RequestResolverError<E>>>,
    admission_done: bool,
}

enum PendingRef<A: Send + 'static, E: Send + 'static> {
    Ordinary(Arc<Pending<A, E>>),
    Prepared(Option<SharedSlot<'static, Option<Pending<A, E>>>>),
}

impl<A: Send + 'static, E: Send + 'static> PendingRef<A, E> {
    fn ordinary(key: RequestKey, group: RequestGroup) -> Self {
        Self::Ordinary(Arc::new(Pending::new(
            key,
            group,
            PendingEvent::Ordinary(Event::new()),
        )))
    }

    fn prepare(
        records: &PendingRecords<A, E>,
        changes: &prepared_notification::Event,
        key: RequestKey,
        group: RequestGroup,
    ) -> Result<Self, MemoryError> {
        let changes = changes.try_clone()?;
        let mut slot = records.try_checkout()?;
        *slot = Some(Pending::new(key, group, PendingEvent::Prepared(changes)));
        Ok(Self::Prepared(Some(slot.share())))
    }

    fn same(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Ordinary(left), Self::Ordinary(right)) => Arc::ptr_eq(left, right),
            (Self::Prepared(Some(left)), Self::Prepared(Some(right))) => left.same_slot(right),
            _ => false,
        }
    }
}

impl<A: Send + 'static, E: Send + 'static> Clone for PendingRef<A, E> {
    #[allow(clippy::expect_used)] // Bounded invocation widths cap live owners below the reference limit.
    fn clone(&self) -> Self {
        match self {
            Self::Ordinary(entry) => Self::Ordinary(Arc::clone(entry)),
            Self::Prepared(entry) => Self::Prepared(Some(
                entry
                    .as_ref()
                    .expect("live pending owner")
                    .try_clone()
                    .expect("pending owner limit"),
            )),
        }
    }
}

impl<A: Send + 'static, E: Send + 'static> std::ops::Deref for PendingRef<A, E> {
    type Target = Pending<A, E>;

    #[allow(clippy::expect_used)] // Publication initializes the record and final retirement consumes the owner.
    fn deref(&self) -> &Self::Target {
        match self {
            Self::Ordinary(entry) => entry,
            Self::Prepared(entry) => entry
                .as_deref()
                .and_then(Option::as_ref)
                .expect("live pending record"),
        }
    }
}

impl<A: Send + 'static, E: Send + 'static> Drop for PendingRef<A, E> {
    fn drop(&mut self) {
        if let Self::Prepared(entry) = self
            && let Some(mut slot) = entry.take().and_then(SharedSlot::into_last)
        {
            drop(slot.take());
        }
    }
}

enum PendingEvent {
    Ordinary(Event),
    Prepared(prepared_notification::Event),
}

impl PendingEvent {
    fn notify_all(&self) {
        match self {
            Self::Ordinary(signal) => notify_all(signal),
            Self::Prepared(signal) => signal.notify_all(),
        }
    }
}

/// Typed pending used by request resolver internal mod.
///
/// # Type parameters
/// - `A`: Application value type. Bound or type: `Send + 'static`.
/// - `E`: Domain-error type. Bound or type: `Send + 'static`.
struct Pending<A: Send + 'static, E: Send + 'static> {
    key: RequestKey,
    group: RequestGroup,
    state: Mutex<PendingState<A, E>>,
    changed: PendingEvent,
}

impl<A: Send + 'static, E: Send + 'static> Pending<A, E> {
    ///
    /// # Parameters
    /// - `key` (`RequestKey`): Stable key used to locate or identify the selected value.
    /// - `group` (`RequestGroup`): The group supplied to this operation.
    ///
    /// # Returns
    /// Type: `Self`.
    fn new(key: RequestKey, group: RequestGroup, changed: PendingEvent) -> Self {
        Self {
            key,
            group,
            state: Mutex::new(PendingState {
                outcome: None,
                fiber: None,
                admission_done: false,
            }),
            changed,
        }
    }

    ///
    /// # Type parameters
    /// - `A`: Success or application-value type.
    /// - `E`: Domain-error type.
    ///
    /// # Parameters
    /// - `outcome` (`SharedResult<A, E>`): Complete terminal outcome, including expected errors and cleanup failures.
    ///
    /// # Returns
    /// Type: `()`.
    fn complete(&self, outcome: SharedResult<A, E>) {
        let discarded = {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if state.outcome.is_none() {
                state.outcome = Some(outcome);
                None
            } else {
                Some(outcome)
            }
        };
        if let Some(discarded) = discarded {
            drop(discarded);
        } else {
            self.changed.notify_all();
        }
    }

    ///
    /// # Type parameters
    /// - `E`: Domain-error type.
    ///
    /// # Parameters
    /// - `fiber` (`Fiber<(), RequestResolverError<E>>`): The fiber supplied to this operation.
    ///
    /// # Returns
    /// Type: `()`.
    fn assign_fiber(&self, fiber: Fiber<(), RequestResolverError<E>>) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.fiber = Some(fiber);
        state.admission_done = true;
        drop(state);
        self.changed.notify_all();
    }

    ///
    /// # Type parameters
    /// - `A`: Success or application-value type.
    /// - `E`: Domain-error type.
    ///
    /// # Parameters
    /// - `outcome` (`SharedResult<A, E>`): Complete terminal outcome, including expected errors and cleanup failures.
    ///
    /// # Returns
    /// Type: `()`.
    fn reject(&self, outcome: SharedResult<A, E>) {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .admission_done = true;
        self.complete(outcome);
    }

    ///
    /// # Returns
    /// Type: `bool`.
    /// Returns whether the requested condition holds.
    fn completed(&self) -> bool {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .outcome
            .is_some()
    }

    ///
    /// # Type parameters
    /// - `R`: Bounds: `Send + Sync + 'static`.
    /// - `A`: Success or application-value type.
    /// - `E`: Domain-error type.
    ///
    /// # Parameters
    /// - `env` (`Arc<R>`): The environment supplying this operation's capabilities.
    /// - `context` (`&Context`): Execution context carrying cancellation, scope, and trace state.
    ///
    /// # Returns
    /// Type: `SharedResult<A, E>`.
    /// Returns `SharedResult<A, E>`.
    async fn observe<R: Send + Sync + 'static, C: Future<Output = ()> + Send>(
        &self,
        env: Arc<R>,
        context: &Context,
        observer: Option<&prepared_notification::Registration>,
        mut diagnostics: Option<Diagnostics<E>>,
        mut prepared_cancellation: Option<Pin<&mut C>>,
    ) -> SharedResult<A, E> {
        let mut listener = None;
        loop {
            if let Some(observer) = observer {
                observer.reset();
            }
            if context.cancelled() {
                return Exit::Interrupted;
            }
            let (outcome, fiber) = {
                let state = self
                    .state
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                let ready = state
                    .outcome
                    .as_ref()
                    .filter(|outcome| state.admission_done || !Self::needs_fiber(outcome));
                (
                    ready.map(|outcome| clone_with_diagnostics(outcome, &mut diagnostics)),
                    state.fiber.clone(),
                )
            };
            if let Some(outcome) = outcome {
                // A batch-level error is owned by its child until one observer
                // marks it observed. Request-level errors are data in a
                // successful batch and need no child failure observation.
                if Self::needs_fiber(&outcome)
                    && let Some(fiber) = &fiber
                {
                    if let Some(observer) = observer {
                        loop {
                            observer.reset();
                            match fiber.observe_ready(|_| ()) {
                                Ok(Some(())) => return outcome,
                                Err(defect) => {
                                    return outcome::append_cleanup(outcome, Exit::Die(defect));
                                }
                                Ok(None) => poll_fn(|context| observer.poll(context)).await,
                            }
                        }
                    }
                    let protected = context.protected();
                    let seen =
                        runtime::evaluate_context(fiber.await_exit_shared::<R>(), env, &protected)
                            .await;
                    return match seen.into_failure::<()>() {
                        Ok(_) => outcome,
                        Err(failure) => outcome::append_cleanup(outcome, failure),
                    };
                }
                return outcome;
            }
            if let Some(fiber) = &fiber
                && fiber.settled_status().is_some()
            {
                let result = if observer.is_some() {
                    match fiber
                        .observe_ready(|outcome| clone_with_diagnostics(outcome, &mut diagnostics))
                    {
                        Ok(Some(outcome)) => Self::settled_result(outcome),
                        Ok(None) => continue,
                        Err(defect) => Exit::Die(defect),
                    }
                } else {
                    let protected = context.protected();
                    let seen =
                        runtime::evaluate_context(fiber.await_exit_shared::<R>(), env, &protected)
                            .await;
                    let result = match seen.into_failure() {
                        Ok(shared) => match shared.with(Clone::clone) {
                            Ok(outcome) => Self::settled_result(outcome),
                            Err(defect) => Exit::Die(defect),
                        },
                        Err(failure) => failure,
                    };
                    self.complete(result.clone());
                    result
                };
                return result;
            }
            let cancelled = match (&self.changed, observer, prepared_cancellation.as_mut()) {
                (PendingEvent::Prepared(_), Some(observer), Some(cancelled)) => {
                    let wake = poll_fn(|context| observer.poll(context));
                    matches!(select(wake, cancelled.as_mut()).await, Either::Right(_))
                }
                (PendingEvent::Ordinary(changed), _, _) => {
                    let Some(changed) = listener.take() else {
                        listener = Some(changed.listen());
                        continue;
                    };
                    let cancelled = pin!(context.cancellation.cancelled());
                    if let Some(fiber) = fiber {
                        let finished = pin!(fiber.settled());
                        let wake = pin!(select(changed, finished));
                        matches!(select(wake, cancelled).await, Either::Right(_))
                    } else {
                        matches!(select(changed, cancelled).await, Either::Right(_))
                    }
                }
                (PendingEvent::Prepared(_), _, _) => {
                    return Exit::Fail(RequestResolverError::Memory(
                        MemoryError::MissingRuntimeStorage,
                    ));
                }
            };
            if cancelled {
                return Exit::Interrupted;
            }
        }
    }

    fn settled_result(outcome: Exit<(), RequestResolverError<E>>) -> SharedResult<A, E> {
        match outcome.into_failure() {
            Ok(()) => Exit::Fail(RequestResolverError::Uncompleted),
            Err(failure) => failure,
        }
    }

    fn needs_fiber(outcome: &SharedResult<A, E>) -> bool {
        !matches!(
            outcome,
            Exit::Success(_) | Exit::Fail(RequestResolverError::RequestFailed(_))
        )
    }
}

/// Typed resolver state used by request resolver internal mod.
///
/// # Type parameters
/// - `Q`: Request type. Bound or type: `Send + 'static`.
/// - `A`: Application value type. Bound or type: `Send + 'static`.
/// - `E`: Domain-error type. Bound or type: `Send + 'static`.
/// - `R`: Environment type. Bound or type: `HasTaskSpawner + HasRequestScope + Send + Sync + 'static`.
pub(super) struct ResolverState<
    Q: Send + 'static,
    A: Send + 'static,
    E: Send + 'static,
    R: HasTaskSpawner + HasRequestScope + Send + Sync + 'static,
> {
    scope: Scope<RequestResolverError<E>>,
    request_scope: RequestScope,
    run: ResolverRunner<Q, A, E, R>,
    registry: Mutex<Registry<A, E>>,
    prepared: Option<Arc<PreparedConfiguration<Q, A, E>>>,
}

impl<
    Q: Send + 'static,
    A: Send + 'static,
    E: Send + 'static,
    R: HasTaskSpawner + HasRequestScope + Send + Sync + 'static,
> ResolverState<Q, A, E, R>
{
    /// # Type parameters
    /// - `A`: Success or application-value type.
    /// - `E`: Domain-error type.
    /// - `Q`: Request type.
    /// - `R`: Environment type.
    ///
    /// # Parameters
    /// - `scope` (`Scope<RequestResolverError<E>>`): Owner that joins child work and releases resources at close.
    /// - `request_scope` (`RequestScope`): The request scope supplied to this operation.
    /// - `run`: Loader for the selected batch representation.
    /// - `capacity`: Error mapper for refused batch storage.
    ///
    /// # Returns
    /// Type: `Self`.
    pub(super) fn new(
        scope: Scope<RequestResolverError<E>>,
        request_scope: RequestScope,
        run: ResolverRunner<Q, A, E, R>,
        prepared: Option<Arc<PreparedConfiguration<Q, A, E>>>,
    ) -> Result<Self, MemoryError> {
        if run.is_prepared()
            && !prepared
                .as_ref()
                .is_some_and(|config| config.0.has_batch_storage())
        {
            return Err(MemoryError::MissingRuntimeStorage);
        }
        let pending = match &prepared {
            Some(prepared) => {
                PendingIndex::Prepared(PreparedPendingEntries(prepared.0.entries.try_checkout()?))
            }
            None => PendingIndex::default(),
        };
        Ok(Self {
            scope,
            request_scope,
            run,
            registry: Mutex::new(Registry {
                closed: false,
                pending,
            }),
            prepared,
        })
    }

    /// # Type parameters
    /// - `Q`: Request type. Bound: `Send + 'static`.
    /// - `A`: Application value type. Bound: `Send + 'static`.
    /// - `E`: Domain-error type. Bound: `Send + 'static`.
    /// - `R`: Environment type. Bound: `HasTaskSpawner + HasRequestScope + Send + Sync + 'static`.
    ///
    /// # Returns
    /// Type: `()`.
    pub(super) fn close(&self) {
        let pending = {
            let mut registry = self
                .registry
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            registry.closed = true;
            std::mem::take(&mut registry.pending)
        };
        match pending {
            PendingIndex::Ordinary(pending) => {
                for entry in pending.into_values() {
                    entry.reject(Exit::Fail(RequestResolverError::Closed));
                }
            }
            PendingIndex::Prepared(mut pending) => {
                while let Some(entry) = pending.0.pop() {
                    entry.reject(Exit::Fail(RequestResolverError::Closed));
                }
            }
        }
    }

    ///
    /// # Type parameters
    /// - `A`: Success or application-value type.
    /// - `E`: Domain-error type.
    ///
    /// # Parameters
    /// - `entry` (`&PendingRef<A, E>`): Stored record or event selected for this transition.
    ///
    /// # Returns
    /// Type: `()`.
    fn forget(&self, entry: &PendingRef<A, E>) {
        let removed = {
            let mut registry = self
                .registry
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if registry
                .pending
                .get(&entry.key)
                .is_some_and(|current| current.same(entry))
            {
                registry.pending.remove(&entry.key)
            } else {
                None
            }
        };
        drop(removed);
    }

    /// # Type parameters
    /// - `A`: Success or application-value type.
    /// - `E`: Domain-error type.
    /// - `Q`: Request type.
    /// - `R`: Environment type.
    ///
    /// # Parameters
    /// - `requests` (`Vec<Request<Q, A, E>>`): The requests supplied to this operation.
    /// - `env` (`Arc<R>`): The environment supplying this operation's capabilities.
    /// - `context` (`Context`): Execution context carrying cancellation, scope, and trace state.
    ///
    /// # Returns
    /// Type: `Exit<Vec<SharedResult<A, E>>, RequestResolverError<E>>`.
    pub(super) async fn resolve<I, Out>(
        self: Arc<Self>,
        requests: I,
        env: Arc<R>,
        context: Context,
    ) -> Exit<Out, RequestResolverError<E>>
    where
        I: RequestBatchInput<Q, A, E>,
        Out: ResultCollector<A, E>,
    {
        if !self.request_scope.same(env.request_scope()) {
            return Exit::Fail(RequestResolverError::WrongEnvironment);
        }
        if context.cancelled() {
            return Exit::Interrupted;
        }
        let (work, mut results, prepared_cancellation) = {
            let mut registry = self
                .registry
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if registry.closed {
                return Exit::Fail(RequestResolverError::Closed);
            }
            let count = requests.as_ref().len();
            let pool = self
                .prepared
                .as_ref()
                .and_then(|config| config.0.results.as_ref());
            if count == 0 {
                drop(registry);
                return match Out::reserve(pool, 0) {
                    Ok(result) => Exit::Success(result.finish()),
                    Err(error) => Exit::Fail(self.capacity_error(error)),
                };
            }
            let mut work = match &self.prepared {
                Some(prepared) => {
                    match prepare_invocation(&registry.pending, requests.as_ref(), &prepared.0) {
                        Ok(invocation) => Work::Prepared(invocation),
                        Err(error) => {
                            drop(registry);
                            return Exit::Fail(
                                if let RequestResolverError::Memory(reason) = error {
                                    (prepared.1)(reason)
                                } else {
                                    error
                                },
                            );
                        }
                    }
                }
                None => {
                    if let Err(error) = validate_ordinary(&registry.pending, requests.as_ref()) {
                        return Exit::Fail(error);
                    }
                    Work::Ordinary {
                        selected: Vec::with_capacity(count),
                        groups: BTreeMap::new(),
                        discard: Vec::new(),
                    }
                }
            };
            let results = match Out::reserve(pool, count) {
                Ok(results) => results,
                Err(error) => {
                    drop(registry);
                    drop(work);
                    return Exit::Fail(self.capacity_error(error));
                }
            };
            let prepared_cancellation = if self.prepared.is_some() {
                match context.cancellation.try_cancelled() {
                    Ok(wait) => Some(wait),
                    Err(error) => {
                        drop(registry);
                        drop(work);
                        drop(results);
                        return Exit::Fail(self.capacity_error(error));
                    }
                }
            } else {
                None
            };
            if self.run.is_prepared() {
                let admission = self
                    .prepared
                    .as_ref()
                    .and_then(|config| config.0.completion.as_ref())
                    .ok_or(MemoryError::MissingRuntimeStorage)
                    .and_then(|completion| work.reserve_observations(count, completion));
                if let Err(error) = admission {
                    drop(registry);
                    drop(work);
                    drop(results);
                    drop(prepared_cancellation);
                    return Exit::Fail(self.capacity_error(error));
                }
            }
            for (position, request) in requests.into_iter().enumerate() {
                let (key, group, payload) = request.into_parts();
                let entry = match registry
                    .pending
                    .get(&key)
                    .or_else(|| work.staged(&key))
                    .cloned()
                {
                    Some(entry) => {
                        work.discard(payload);
                        entry
                    }
                    None => {
                        let entry = match &self.prepared {
                            Some(prepared) => match PendingRef::prepare(
                                &prepared.0.pending_records,
                                &prepared.0.changes,
                                key,
                                group.clone(),
                            ) {
                                Ok(entry) => entry,
                                Err(error) => {
                                    drop(registry);
                                    drop(work);
                                    drop(results);
                                    drop(prepared_cancellation);
                                    return Exit::Fail((prepared.1)(error));
                                }
                            },
                            None => {
                                let entry = PendingRef::ordinary(key.clone(), group.clone());
                                registry.pending.insert_reserved(key, entry.clone());
                                entry
                            }
                        };
                        work.load(position, group, payload, entry.clone());
                        entry
                    }
                };
                work.select(entry);
            }
            if let Some(prepared) = &self.prepared
                && let Err(error) = work.prepare_groups::<R>(
                    &prepared.0,
                    &mut registry.pending,
                    self.run.is_prepared(),
                )
            {
                drop(registry);
                drop(work);
                drop(results);
                drop(prepared_cancellation);
                return Exit::Fail((prepared.1)(error));
            }
            (work, results, prepared_cancellation)
        };

        let mut work = std::mem::DropGuard::new(work, |work| {
            if let Work::Prepared(work) = work {
                for launch in work.0.launches.as_slice() {
                    self.withdraw(launch.parent.entries());
                }
            }
        });
        while let Some(Batch {
            group,
            payloads,
            parent,
            child,
        }) = work.next_group()
        {
            let tracking = BatchTracking {
                entries: child,
                owner: Arc::downgrade(&self),
            };
            let payloads = match payloads {
                BatchPayload::Ordinary(payloads) => payloads,
                BatchPayload::Prepared(invocation) => {
                    let (
                        BatchParent::Prepared {
                            entries,
                            settlement,
                            permits,
                            fork,
                        },
                        ResolverRunner::Prepared(run),
                        Some(configuration),
                    ) = (parent, &self.run, &self.prepared)
                    else {
                        self.withdraw(tracking.entries.entries());
                        return Exit::Fail(
                            self.capacity_error(MemoryError::UnsupportedRunContract),
                        );
                    };
                    let dispatch = PreparedForkDispatch {
                        fork,
                        observers: settlement,
                        permits,
                    };
                    let run = Arc::clone(run);
                    let capacity = Arc::clone(&configuration.1);
                    let spawned = self.scope.start_prepared_fork(
                        &env,
                        &context,
                        dispatch,
                        move |env, context| {
                            PreparedBatchTask {
                                group,
                                invocation,
                                tracking,
                                run,
                                capacity,
                                env,
                                context,
                            }
                            .run()
                        },
                    );
                    self.record_batch_admission(entries.entries(), spawned);
                    // Admission is recorded before yielding. A dropped caller
                    // can withdraw only the still-unsubmitted groups.
                    let mut yielded = false;
                    poll_fn(|context| {
                        if std::mem::replace(&mut yielded, true) {
                            std::task::Poll::Ready(())
                        } else {
                            context.waker().wake_by_ref();
                            std::task::Poll::Pending
                        }
                    })
                    .await;
                    continue;
                }
            };
            let ResolverRunner::Ordinary(run) = &self.run else {
                self.withdraw(tracking.entries.entries());
                return Exit::Fail(self.capacity_error(MemoryError::UnsupportedRunContract));
            };
            let program = batch_effect(group, payloads, tracking, Arc::clone(run));
            let (fork, ordinary) = match parent {
                BatchParent::Ordinary(parent) => (
                    crate::effect_scope! { fork; scope: self.scope, program: program, },
                    Some(parent),
                ),
                BatchParent::Prepared {
                    entries: parent,
                    settlement,
                    permits,
                    fork,
                } => {
                    let owner = Arc::clone(&self);
                    let withdrawing = Arc::clone(&self);
                    let pending =
                        std::mem::DropGuard::new(parent, move |entries: PreparedEntries<A, E>| {
                            withdrawing.withdraw(entries.entries());
                        });
                    let fork = self.scope.fork_recording_admission(
                        program,
                        settlement,
                        permits,
                        fork,
                        move |outcome| {
                            let entries = std::mem::DropGuard::dismiss(pending);
                            owner.record_batch_admission(entries.entries(), outcome.clone());
                        },
                    );
                    (fork, None)
                }
            };
            let protected = context.protected();
            let spawned = runtime::evaluate_context(fork, Arc::clone(&env), &protected).await;
            if let Some(parent) = ordinary {
                self.record_batch_admission(&parent, spawned);
            }
        }
        work.drop_duplicates();

        results.ready(work.order_results());
        let mut prepared_cancellation = pin!(prepared_cancellation);
        while let Some(entry) = work.next_selected() {
            let diagnostics = work.next_observation();
            let result = entry
                .observe(
                    Arc::clone(&env),
                    &context,
                    work.observer(),
                    diagnostics,
                    prepared_cancellation.as_mut().as_pin_mut(),
                )
                .await;
            if entry.completed() {
                self.forget(&entry);
            }
            results.push(result);
        }
        Exit::Success(results.finish())
    }

    fn capacity_error(&self, error: MemoryError) -> RequestResolverError<E> {
        self.prepared
            .as_ref()
            .map_or(RequestResolverError::Memory(error), |config| {
                (config.1)(error)
            })
    }

    fn withdraw(&self, entries: &[PendingRef<A, E>]) {
        for entry in entries {
            entry.reject(Exit::Interrupted);
            self.forget(entry);
        }
    }

    fn record_batch_admission(
        &self,
        entries: &[PendingRef<A, E>],
        spawned: Exit<Fiber<(), RequestResolverError<E>>, RequestResolverError<E>>,
    ) {
        match spawned.into_failure::<SharedValue<A>>() {
            Ok(fiber) => {
                for entry in entries {
                    entry.assign_fiber(fiber.clone());
                }
            }
            Err(failure) => {
                for entry in entries {
                    entry.reject(failure.clone());
                    self.forget(entry);
                }
            }
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
/// Type: `SharedResult<A, E>`.
/// Returns `SharedResult<A, E>`.
fn share_request<A: Send + 'static, E: Send + 'static>(outcome: Exit<A, E>) -> SharedResult<A, E> {
    match outcome.map_error(|error| RequestResolverError::RequestFailed(SharedValue::new(error))) {
        Exit::Success(value) => Exit::Success(SharedValue::new(value)),
        Exit::Fail(error) => Exit::Fail(error),
        Exit::Die(defect) => Exit::Die(defect),
        Exit::Interrupted => Exit::Interrupted,
        Exit::FinalizationFailed(details) => Exit::FinalizationFailed(details),
    }
}

fn share_prepared_request<A: Send + 'static, E: Send + 'static>(
    outcome: Exit<A, E>,
    completion: CompletionStorage<A, E>,
) -> SharedResult<A, E> {
    let (outcome, value_slot) = completion.share(outcome, RequestResolverError::RequestFailed);
    match outcome.into_failure() {
        Ok(value) => Exit::Success(SharedValue::from_reserved(value_slot, value)),
        Err(failure) => failure,
    }
}

///
/// # Type parameters
/// - `B`: Transformed success-value type.
/// - `C`: Stream chunk or item type.
/// - `E`: Expected application-error type.
///
/// # Parameters
/// - `outcome` (`Exit<B, E>`): Complete terminal outcome, including expected errors and cleanup failures.
///
/// # Returns
/// Type: `Exit<C, E>`.
/// Returns `Exit<C, E>`.
fn failed<B, C, E>(outcome: Exit<B, E>) -> Exit<C, E> {
    match outcome.into_failure() {
        Ok(_) => Exit::Die(Defect::from(
            LibraryDefect::RequestResolverUnexpectedSuccess,
        )),
        Err(failure) => failure,
    }
}

fn settle_batch<A: Send + 'static, E: Send + 'static>(
    entries: &[PendingRef<A, E>],
    count: usize,
    outcomes: impl Iterator<Item = Exit<A, E>>,
) -> Exit<(), RequestResolverError<E>> {
    if count != entries.len() {
        let error = RequestResolverError::CardinalityMismatch {
            expected: entries.len(),
            actual: count,
        };
        return finish_failed_batch(entries, Exit::Fail(error));
    }
    for (entry, result) in entries.iter().zip(outcomes) {
        entry.complete(share_request(result));
    }
    Exit::Success(())
}

fn finish_failed_batch<A: Send + 'static, E: Send + 'static>(
    entries: &[PendingRef<A, E>],
    failure: Exit<(), RequestResolverError<E>>,
) -> Exit<(), RequestResolverError<E>> {
    for entry in entries {
        entry.complete(failed::<_, SharedValue<A>, _>(failure.clone()));
    }
    failure
}

// The compatibility runner retains its reusable ordinary Effect and owned Vec API.
fn batch_effect<Q, A, E, R>(
    group: RequestGroup,
    payloads: Vec<Q>,
    tracking: BatchTracking<Q, A, E, R>,
    run: Arc<BatchRunner<Q, A, E, R>>,
) -> Effect<(), RequestResolverError<E>, R>
where
    Q: Send + 'static,
    A: Send + 'static,
    E: Send + 'static,
    R: HasTaskSpawner + HasRequestScope + Send + Sync + 'static,
{
    let batch = Mutex::new(Some((payloads, tracking)));
    Effect::new(move |env, context, finish| {
        let batch = batch
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        let Some((payloads, tracking)) = batch else {
            return runtime::finish(finish, Exit::Fail(RequestResolverError::InputConsumed));
        };
        let group = group.clone();
        let run = Arc::clone(&run);
        Step::suspended(Box::pin(async move {
            let entries = tracking.entries.entries();
            let work = async {
                let program = run(group, payloads);
                let outcome = runtime::evaluate_context(program, env, &context).await;
                match outcome.into_failure() {
                    Ok(outcomes) => settle_batch(entries, outcomes.len(), outcomes.into_iter()),
                    Err(failure) => finish_failed_batch(
                        entries,
                        failure.map_error(|error| {
                            RequestResolverError::BatchFailed(SharedValue::new(error))
                        }),
                    ),
                }
            };
            let outcome = AssertUnwindSafe(work)
                .catch_unwind()
                .await
                .unwrap_or_else(|panic| {
                    finish_failed_batch(entries, Exit::Die(Defect::from_panic(panic)))
                });
            drop(tracking);
            runtime::finish(finish, outcome)
        }))
    })
}
