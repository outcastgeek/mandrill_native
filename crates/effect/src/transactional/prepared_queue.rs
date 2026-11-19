//! Bounded FIFO snapshots in the existing optimistic journal.

use super::{
    PreparedTxRef, PreparedTxValue, PreparedTxValuePool, Transaction, TransactionDomain,
    TransactionDomainError, TransactionError, TxStep, TxValue,
};
use crate::memory::{Bytes, MemoryError, PreparedMemory, RegionId};
use crate::{PreparedChunkPool, PreparedChunkValues, transaction};
use std::num::NonZeroUsize;

/// Retained item payloads, simultaneous queue versions, and the FIFO limit.
#[derive(Clone, Copy, Debug)]
pub struct PreparedTxQueueCapacity {
    pub items: NonZeroUsize,
    pub snapshots: NonZeroUsize,
    pub items_per_snapshot: NonZeroUsize,
}

struct Snapshot<T: TxValue>(PreparedChunkValues<PreparedTxValue<T>>);

// Published versions expose only immutable item handles. A replacement uses a
// different snapshot lease; there is no mutable alias to a committed version.
impl<T: TxValue> TxValue for Snapshot<T> {}

/// Prepared storage for one FIFO. Snapshot demand includes the committed
/// version, pending replacements and versions retained by branches or readers.
pub struct PreparedTxQueueStorage<T: TxValue> {
    items: PreparedTxValuePool<T>,
    versions: PreparedTxValuePool<Snapshot<T>>,
    snapshots: PreparedChunkPool<PreparedTxValue<T>>,
}

impl<T: TxValue> PreparedTxQueueStorage<T> {
    pub fn storage_bytes(capacity: PreparedTxQueueCapacity) -> Result<Bytes, MemoryError> {
        PreparedTxValuePool::<T>::storage_bytes(capacity.items)?
            .checked_add(PreparedTxValuePool::<Snapshot<T>>::storage_bytes(
                capacity.snapshots,
            )?)?
            .checked_add(PreparedChunkPool::<PreparedTxValue<T>>::storage_bytes(
                capacity.snapshots,
                capacity.items_per_snapshot,
            )?)
    }

    pub fn prepare(
        memory: &PreparedMemory<'static>,
        region: RegionId,
        capacity: PreparedTxQueueCapacity,
    ) -> Result<Self, MemoryError> {
        Self::storage_bytes(capacity)?;
        Ok(Self {
            items: PreparedTxValuePool::prepare(memory, region, capacity.items)?,
            versions: PreparedTxValuePool::prepare(memory, region, capacity.snapshots)?,
            snapshots: PreparedChunkPool::prepare(
                memory,
                region,
                capacity.snapshots,
                capacity.items_per_snapshot,
            )?,
        })
    }
}

/// Region-backed FIFO with ordinary transaction conflict and empty-retry
/// semantics. Checked capacity refusal leaves the committed queue unchanged.
pub struct PreparedTxQueue<T: TxValue> {
    cell: PreparedTxRef<Snapshot<T>>,
    items: PreparedTxValuePool<T>,
    snapshots: PreparedChunkPool<PreparedTxValue<T>>,
}

impl<T: TxValue> PreparedTxQueue<T> {
    pub fn new(
        domain: &TransactionDomain,
        storage: PreparedTxQueueStorage<T>,
    ) -> Result<Self, TransactionDomainError> {
        let initial = storage
            .snapshots
            .try_checkout()
            .map_err(TransactionDomainError::Memory)?;
        Ok(Self {
            cell: domain.reference_prepared(Snapshot(initial), storage.versions)?,
            items: storage.items,
            snapshots: storage.snapshots,
        })
    }

    /// Initialize one actual item slot. Refusal returns the untouched item.
    pub fn prepare_item(&self, item: T) -> Result<PreparedTxValue<T>, (MemoryError, T)> {
        self.items.try_store(item)
    }

    /// Describe an append of the admitted value. Old and new versions share
    /// item owners without copying T or allocating a new item backing.
    pub fn offer<E: Send + 'static>(
        &self,
        item: PreparedTxValue<T>,
    ) -> Result<Transaction<(), E>, MemoryError> {
        if !self.items.owns(&item) {
            return Err(MemoryError::UnsupportedRunContract);
        }
        let snapshots = self.snapshots.try_clone()?;
        self.cell.update_step(move |current| {
            TxStep::from_memory((|| {
                if current.0.len() == current.0.capacity() {
                    return Err(MemoryError::CollectionReservationFailed);
                }
                let mut next = copy_items(&snapshots, current.0.as_slice())?;
                next.0.push_reserved(item.try_clone()?);
                Ok((next, ()))
            })())
        })
    }

    /// Describe dequeue. An empty queue observes its version and retries
    /// without reserving a replacement. A returned item retains its own slot.
    pub fn take<E: Send + 'static>(
        &self,
    ) -> Result<Transaction<PreparedTxValue<T>, E>, MemoryError> {
        let snapshots = self.snapshots.try_clone()?;
        let read = self.cell.read();
        let retry = Transaction::retry();
        let dequeue = self.cell.update_step(move |current| {
            let Some((first, rest)) = current.0.as_slice().split_first() else {
                return TxStep::Retry;
            };
            TxStep::from_memory((|| {
                let next = copy_items(&snapshots, rest)?;
                Ok((next, first.try_clone()?))
            })())
        })?;
        Ok(transaction! {
            let current = read.clone()?;
            if current.0.is_empty() {
                retry.clone()?
            } else {
                dequeue.clone()?
            }
        })
    }

    #[must_use]
    pub fn len<E: Send + 'static>(&self) -> Transaction<usize, E> {
        let read = self.cell.read();
        transaction! {
            let current = read.clone()?;
            current.0.len()
        }
    }
}

fn copy_items<T: TxValue>(
    snapshots: &PreparedChunkPool<PreparedTxValue<T>>,
    items: &[PreparedTxValue<T>],
) -> Result<Snapshot<T>, MemoryError> {
    let mut next = snapshots.try_checkout()?;
    // All versions draw from the same snapshot capacity. The caller either
    // copies the whole non-full version or removes its first item.
    for item in items {
        next.push_reserved(item.try_clone()?);
    }
    Ok(Snapshot(next))
}

impl<A, E> TxStep<A, E> {
    fn from_memory(result: Result<A, MemoryError>) -> Self {
        match result {
            Ok(value) => Self::Success(value),
            Err(error) => Self::Invalid(TransactionError::Memory(error)),
        }
    }
}
