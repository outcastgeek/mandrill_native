//! Region-backed encoded records for the volatile key-value store contract.

use super::{CompareExchange, StoreRevision, UtcMillis};
use crate::memory::{
    Buffer, BufferView, Bytes, FixedVector, MemoryError, PreparedMemory, RegionId, SharedSlot,
    SlotPool,
};
use crate::{PreparedChunkPool, PreparedChunkValues};
use std::alloc::Layout;
use std::num::NonZeroUsize;
use std::sync::Mutex;

/// Separate bounds for live records, retained byte blocks and ordered reads.
#[derive(Clone, Copy, Debug)]
pub struct PreparedStoreCapacity {
    pub entries: NonZeroUsize,
    /// Includes resident keys and key copies retained by completed batches.
    pub key_slots: NonZeroUsize,
    pub key_bytes: Bytes,
    /// Includes resident values, replacements and values retained by readers.
    pub value_slots: NonZeroUsize,
    pub value_bytes: Bytes,
    pub batches: NonZeroUsize,
    pub keys_per_batch: NonZeroUsize,
}

/// The result retains the store's real encoded bytes after removal or expiry.
pub struct PreparedStoredValue {
    bytes: BufferView<'static>,
    pub expires_at: Option<UtcMillis>,
}

impl PreparedStoredValue {
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        self.bytes.as_ref()
    }

    pub fn try_clone(&self) -> Result<Self, MemoryError> {
        Ok(Self {
            bytes: self.bytes.try_clone()?,
            expires_at: self.expires_at,
        })
    }
}

pub struct PreparedStoreEntry {
    pub value: PreparedStoredValue,
    pub revision: StoreRevision,
}

impl PreparedStoreEntry {
    fn try_clone(&self) -> Result<Self, MemoryError> {
        Ok(Self {
            value: self.value.try_clone()?,
            revision: self.revision,
        })
    }
}

/// Expiry is observable without removing or releasing authoritative bytes.
pub enum PreparedStoreRead {
    Missing,
    Expired(PreparedStoreEntry),
    Found(PreparedStoreEntry),
}

pub struct PreparedBatchReadItem {
    key: BufferView<'static>,
    pub result: PreparedStoreRead,
}

impl PreparedBatchReadItem {
    #[must_use]
    pub fn key(&self) -> &[u8] {
        self.key.as_ref()
    }
}

/// Ordered, owned batch storage. Duplicated input keys retain their positions.
pub struct PreparedStoreBatch(PreparedChunkValues<PreparedBatchReadItem>);

impl PreparedStoreBatch {
    #[must_use]
    pub fn items(&self) -> &[PreparedBatchReadItem] {
        self.0.as_slice()
    }
}

/// The selected memory store reports storage refusal separately from revision exhaustion.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PreparedStoreError {
    Memory(MemoryError),
    RevisionOverflow,
}

impl From<MemoryError> for PreparedStoreError {
    fn from(error: MemoryError) -> Self {
        Self::Memory(error)
    }
}

impl std::fmt::Display for PreparedStoreError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Memory(error) => error.fmt(formatter),
            Self::RevisionOverflow => formatter.write_str("store revision exhausted"),
        }
    }
}

impl std::error::Error for PreparedStoreError {}

struct Record {
    key: BufferView<'static>,
    entry: PreparedStoreEntry,
}

struct State {
    next_revision: u64,
    records: FixedVector<'static, Record>,
}

impl State {
    fn find(&self, key: &[u8]) -> Option<usize> {
        self.records
            .as_slice()
            .iter()
            .position(|record| record.key.as_ref() == key)
    }

    fn read(&self, key: &[u8], now: UtcMillis) -> Result<PreparedStoreRead, MemoryError> {
        let Some(record) = self
            .find(key)
            .and_then(|index| self.records.as_slice().get(index))
        else {
            return Ok(PreparedStoreRead::Missing);
        };
        let entry = record.entry.try_clone()?;
        if expired(&entry, now) {
            Ok(PreparedStoreRead::Expired(entry))
        } else {
            Ok(PreparedStoreRead::Found(entry))
        }
    }
}

fn expired(entry: &PreparedStoreEntry, now: UtcMillis) -> bool {
    entry
        .value
        .expires_at
        .is_some_and(|deadline| now >= deadline)
}

/// Prepared, process-local encoded store. Operations use the actual preallocated
/// key/value and batch backing and can run inside an existing prepared native
/// operation. The store does not claim restart recovery.
pub struct PreparedMemoryKeyValueStore {
    state: SharedSlot<'static, Mutex<State>>,
    keys: SlotPool<'static, Buffer<'static>>,
    values: SlotPool<'static, Buffer<'static>>,
    batches: PreparedChunkPool<PreparedBatchReadItem>,
}

impl PreparedMemoryKeyValueStore {
    /// Payload and pool sizes, excluding partition allocator alignment and
    /// allocation headers.
    pub fn storage_bytes(capacity: PreparedStoreCapacity) -> Result<Bytes, MemoryError> {
        let records = Layout::array::<Record>(capacity.entries.get())
            .map_err(|_| MemoryError::InvalidLayout)?;
        let buffers = |slots: NonZeroUsize, bytes: Bytes| {
            let layout =
                Layout::array::<u8>(bytes.get()).map_err(|_| MemoryError::InvalidLayout)?;
            SlotPool::<Buffer<'static>>::storage_bytes(slots.get())?
                .checked_add(Bytes::new(layout.size()).checked_mul(slots.get())?)
        };
        SlotPool::<Mutex<State>>::storage_bytes(1)?
            .checked_add(Bytes::new(records.size()))?
            .checked_add(buffers(capacity.key_slots, capacity.key_bytes)?)?
            .checked_add(buffers(capacity.value_slots, capacity.value_bytes)?)?
            .checked_add(PreparedChunkPool::<PreparedBatchReadItem>::storage_bytes(
                capacity.batches,
                capacity.keys_per_batch,
            )?)
    }

    pub fn prepare(
        memory: &PreparedMemory<'static>,
        region: RegionId,
        capacity: PreparedStoreCapacity,
    ) -> Result<Self, MemoryError> {
        Self::storage_bytes(capacity)?;
        let states = memory.slots(region, 1, |_| {
            Ok(Mutex::new(State {
                next_revision: 0,
                records: FixedVector::prepare(memory.allocator(region)?, capacity.entries.get())?,
            }))
        })?;
        Ok(Self {
            state: states.try_checkout()?.share(),
            keys: memory.buffers(region, capacity.key_slots.get(), capacity.key_bytes)?,
            values: memory.buffers(region, capacity.value_slots.get(), capacity.value_bytes)?,
            batches: PreparedChunkPool::prepare(
                memory,
                region,
                capacity.batches,
                capacity.keys_per_batch,
            )?,
        })
    }

    pub fn try_clone(&self) -> Result<Self, MemoryError> {
        Ok(Self {
            state: self.state.try_clone()?,
            keys: self.keys.try_clone()?,
            values: self.values.try_clone()?,
            batches: self.batches.try_clone()?,
        })
    }

    /// Borrow the lookup key. Only the returned value retains storage.
    pub fn get(&self, key: &[u8], now: UtcMillis) -> Result<PreparedStoreRead, MemoryError> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .read(key, now)
    }

    /// Read one consistent snapshot. The batch structure, copied request keys
    /// and retained values all have physical owners. Refusal publishes no batch.
    pub fn get_many(
        &self,
        keys: &[&[u8]],
        now: UtcMillis,
    ) -> Result<PreparedStoreBatch, MemoryError> {
        let mut output = self.batches.try_checkout()?;
        if keys.len() > output.capacity() {
            return Err(MemoryError::CollectionReservationFailed);
        }
        let state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        for &key in keys {
            output.push_reserved(PreparedBatchReadItem {
                key: copy_bytes(&self.keys, key)?,
                result: state.read(key, now)?,
            });
        }
        Ok(PreparedStoreBatch(output))
    }

    /// Commit a copied encoded record. On refusal, the entry and revision
    /// remain unchanged. Replacing an existing value reuses its original key.
    pub fn put(
        &self,
        key: &[u8],
        bytes: &[u8],
        expires_at: Option<UtcMillis>,
    ) -> Result<StoreRevision, PreparedStoreError> {
        let (revision, retired) = {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let index = state.find(key);
            self.commit(&mut state, index, key, Some((bytes, expires_at)))?
        };
        drop(retired);
        Ok(revision)
    }

    /// Match the revision before reserving replacement bytes. A mismatch is a
    /// conflict even when the store has no replacement capacity.
    pub fn compare_exchange(
        &self,
        key: &[u8],
        expected: Option<StoreRevision>,
        replacement: Option<(&[u8], Option<UtcMillis>)>,
    ) -> Result<CompareExchange, PreparedStoreError> {
        let (revision, retired) = {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let index = state.find(key);
            let actual = index
                .and_then(|index| state.records.as_slice().get(index))
                .map(|record| record.entry.revision);
            if actual != expected {
                return Ok(CompareExchange::Conflict { actual });
            }
            self.commit(&mut state, index, key, replacement)?
        };
        drop(retired);
        Ok(CompareExchange::Written(revision))
    }

    /// Remove without incrementing the revision. Returned data keeps its bytes.
    #[must_use]
    pub fn remove(&self, key: &[u8]) -> Option<PreparedStoreEntry> {
        let record = {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            state
                .find(key)
                .and_then(|index| state.records.remove_at(index))
        };
        record.map(|record| record.entry)
    }

    /// Remove all expired rows under one store gate. Record destruction only
    /// releases this store's own byte handles and has no application callback.
    #[must_use]
    pub fn cleanup_expired(&self, now: UtcMillis) -> usize {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let previous = state.records.len();
        state.records.retain(|record| !expired(&record.entry, now));
        previous - state.records.len()
    }

    fn commit(
        &self,
        state: &mut State,
        index: Option<usize>,
        key: &[u8],
        replacement: Option<(&[u8], Option<UtcMillis>)>,
    ) -> Result<(StoreRevision, Option<PreparedStoreEntry>), PreparedStoreError> {
        if index.is_none()
            && replacement.is_some()
            && state.records.len() == state.records.capacity()
        {
            return Err(MemoryError::NoSlot.into());
        }
        let revision = StoreRevision(
            state
                .next_revision
                .checked_add(1)
                .ok_or(PreparedStoreError::RevisionOverflow)?,
        );
        let retired = if let Some((bytes, expires_at)) = replacement {
            let entry = PreparedStoreEntry {
                value: PreparedStoredValue {
                    bytes: copy_bytes(&self.values, bytes)?,
                    expires_at,
                },
                revision,
            };
            if let Some(record) =
                index.and_then(|index| state.records.as_mut_slice().get_mut(index))
            {
                Some(std::mem::replace(&mut record.entry, entry))
            } else {
                let key = copy_bytes(&self.keys, key)?;
                state.records.push_reserved(Record { key, entry });
                None
            }
        } else {
            index
                .and_then(|index| state.records.remove_at(index))
                .map(|record| record.entry)
        };
        state.next_revision = revision.value();
        Ok((revision, retired))
    }
}

fn copy_bytes(
    pool: &SlotPool<'static, Buffer<'static>>,
    input: &[u8],
) -> Result<BufferView<'static>, MemoryError> {
    let mut output = pool.try_checkout()?;
    output.clear();
    output.extend_from_slice(input)?;
    Ok(BufferView::new(output.share()))
}
