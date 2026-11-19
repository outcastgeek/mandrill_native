//! Byte-owned state and checkpoint publication for selected projections.

use super::MemoryProjectionError;
use crate::eventlog::{JournalCursor, ProjectionKey, ProjectionVersion};
use crate::memory::{
    Buffer, BufferView, Bytes, FixedIndex, MemoryError, PreparedMemory, RegionId, SharedSlot,
    SlotLease, SlotPool,
};
use std::alloc::Layout;
use std::num::NonZeroUsize;
use std::sync::{Mutex, PoisonError};

/// Startup demand for the selected projection store.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PreparedProjectionCapacity {
    /// Projection keys with retained state and checkpoints.
    pub projections: NonZeroUsize,
    /// Concurrent resident, retained and in-progress state byte owners.
    pub snapshots: NonZeroUsize,
    /// Fixed size of each state byte buffer.
    pub bytes_per_snapshot: Bytes,
    /// Combined logical bytes of each caller-owned projection and stream key.
    pub identity_bytes_per_key: Bytes,
}

impl PreparedProjectionCapacity {
    /// Inline key/record table, slot controls and physical snapshot demand.
    /// The enclosing memory plan accounts for allocation headers and alignment.
    pub fn storage_bytes(self) -> Result<Bytes, MemoryError> {
        let records = Layout::array::<(ProjectionKey, ProjectionRecord)>(self.projections.get())
            .map_err(|_| MemoryError::InvalidLayout)?;
        let snapshot = Layout::array::<u8>(self.bytes_per_snapshot.get())
            .map_err(|_| MemoryError::InvalidLayout)?;
        Bytes::new(records.size())
            .checked_add(SlotPool::<Buffer<'static>>::storage_bytes(
                self.snapshots.get(),
            )?)?
            .checked_add(Bytes::new(snapshot.size()).checked_mul(self.snapshots.get())?)
    }
}

/// An exclusive state writer. Dropping it returns backing to its original store.
pub struct PreparedProjectionReservation(SlotLease<'static, Buffer<'static>>);

impl PreparedProjectionReservation {
    /// Append bytes to the preadmitted state backing.
    pub fn extend_from_slice(&mut self, bytes: &[u8]) -> Result<(), MemoryError> {
        self.0.extend_from_slice(bytes)
    }

    /// Discard the current contents while retaining this writer.
    pub fn clear(&mut self) {
        self.0.clear();
    }

    /// Borrow the state written so far.
    #[must_use]
    pub fn as_slice(&self) -> &[u8] {
        self.0.as_slice()
    }

    /// Capacity of this writer's actual byte buffer.
    #[must_use]
    pub fn capacity(&self) -> Bytes {
        self.0.capacity()
    }
}

/// State bytes and their checkpoint as observed together under the store gate.
pub struct PreparedProjectionSnapshot {
    /// Projection and stream identity.
    pub key: ProjectionKey,
    /// Version of the projection definition.
    pub version: ProjectionVersion,
    /// Journal cursor reflected in these bytes.
    pub checkpoint: JournalCursor,
    /// The state backing stays checked out until its last reader releases it.
    pub state: BufferView<'static>,
}

/// Decision from publishing one prepared state and checkpoint.
pub enum PreparedProjectionWrite {
    /// The new state and checkpoint are authoritative.
    Committed(PreparedProjectionSnapshot),
    /// Another checkpoint is authoritative.
    Conflict(Option<PreparedProjectionSnapshot>),
    /// A different projection version is already registered for this key.
    VersionConflict(ProjectionVersion),
}

/// Refusal from the selected projection store.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PreparedProjectionError {
    /// The projection transition violates its existing checkpoint contract.
    Store(MemoryProjectionError),
    /// Configured byte backing or a counted owner could not admit the operation.
    Memory(MemoryError),
}

struct ProjectionRecord {
    version: ProjectionVersion,
    checkpoint: JournalCursor,
    state: SharedSlot<'static, Buffer<'static>>,
}

impl ProjectionRecord {
    fn snapshot(
        &self,
        key: &ProjectionKey,
    ) -> Result<PreparedProjectionSnapshot, PreparedProjectionError> {
        Ok(PreparedProjectionSnapshot {
            key: key.clone(),
            version: self.version,
            checkpoint: self.checkpoint,
            state: BufferView::new(
                self.state
                    .try_clone()
                    .map_err(PreparedProjectionError::Memory)?,
            ),
        })
    }
}

/// Process-local projection publication with fixed regional index and byte owners.
/// Callbacks and encoding use the reserved writer outside the publication gate.
pub struct PreparedMemoryProjectionStore {
    records: Mutex<FixedIndex<'static, ProjectionKey, ProjectionRecord>>,
    snapshots: SlotPool<'static, Buffer<'static>>,
    identity_bytes_per_key: Bytes,
}

impl PreparedMemoryProjectionStore {
    /// Provision every index entry, snapshot buffer and lease control before sealing.
    pub fn prepare(
        memory: &PreparedMemory<'static>,
        region: RegionId,
        capacity: PreparedProjectionCapacity,
    ) -> Result<Self, MemoryError> {
        capacity.storage_bytes()?;
        Ok(Self {
            records: Mutex::new(FixedIndex::prepare(
                memory.allocator(region)?,
                capacity.projections.get(),
            )?),
            snapshots: memory.buffers(
                region,
                capacity.snapshots.get(),
                capacity.bytes_per_snapshot,
            )?,
            identity_bytes_per_key: capacity.identity_bytes_per_key,
        })
    }

    /// Retain the current state and checkpoint, if present.
    pub fn load(
        &self,
        key: &ProjectionKey,
    ) -> Result<Option<PreparedProjectionSnapshot>, PreparedProjectionError> {
        let records = self.records.lock().unwrap_or_else(PoisonError::into_inner);
        records
            .get(key)
            .map(|record| record.snapshot(key))
            .transpose()
    }

    /// Claim one real byte slot before folding or encoding a replacement.
    pub fn reserve_snapshot(&self) -> Result<PreparedProjectionReservation, MemoryError> {
        let mut buffer = self.snapshots.try_checkout()?;
        buffer.clear();
        Ok(PreparedProjectionReservation(buffer))
    }

    /// Retain initialized state without inserting a checkpoint into the store.
    pub fn finish_snapshot(
        &self,
        key: ProjectionKey,
        version: ProjectionVersion,
        checkpoint: JournalCursor,
        next: PreparedProjectionReservation,
    ) -> Result<PreparedProjectionSnapshot, PreparedProjectionError> {
        self.check_key_bytes(&key)
            .map_err(PreparedProjectionError::Memory)?;
        Ok(PreparedProjectionSnapshot {
            key,
            version,
            checkpoint,
            state: BufferView::new(self.take_reserved(next)?),
        })
    }

    /// Transfer a writer from this store into the atomic checkpoint transition.
    pub fn compare_exchange(
        &self,
        key: ProjectionKey,
        expected: Option<JournalCursor>,
        version: ProjectionVersion,
        checkpoint: JournalCursor,
        next: PreparedProjectionReservation,
    ) -> Result<PreparedProjectionWrite, PreparedProjectionError> {
        let state = self.take_reserved(next)?;
        let mut records = self.records.lock().unwrap_or_else(PoisonError::into_inner);
        let existing = records.get(&key);
        if let Some(existing) = existing
            && existing.version != version
        {
            return Ok(PreparedProjectionWrite::VersionConflict(existing.version));
        }
        if existing.map(|record| record.checkpoint) != expected {
            let conflict = existing.map(|record| record.snapshot(&key)).transpose()?;
            return Ok(PreparedProjectionWrite::Conflict(conflict));
        }
        if existing.is_none() {
            if records.len() == records.capacity() {
                return Err(PreparedProjectionError::Memory(MemoryError::NoSlot));
            }
            self.check_key_bytes(&key)
                .map_err(PreparedProjectionError::Memory)?;
        }
        if expected.is_some_and(|current| checkpoint < current) {
            return Err(PreparedProjectionError::Store(
                MemoryProjectionError::CheckpointRegression,
            ));
        }

        let record = ProjectionRecord {
            version,
            checkpoint,
            state,
        };
        let result = record.snapshot(&key)?;
        let retired = records
            .insert(key, record)
            .map_err(|_| PreparedProjectionError::Memory(MemoryError::NoSlot))?;
        drop(records);
        drop(retired);
        Ok(PreparedProjectionWrite::Committed(result))
    }

    fn take_reserved(
        &self,
        reservation: PreparedProjectionReservation,
    ) -> Result<SharedSlot<'static, Buffer<'static>>, PreparedProjectionError> {
        if !reservation.0.belongs_to(&self.snapshots) {
            return Err(PreparedProjectionError::Memory(
                MemoryError::UnsupportedRunContract,
            ));
        }
        Ok(reservation.0.share())
    }

    fn check_key_bytes(&self, key: &ProjectionKey) -> Result<(), MemoryError> {
        let requested = Bytes::new(key.id.as_bytes().len())
            .checked_add(Bytes::new(key.stream.journal.as_bytes().len()))?
            .checked_add(Bytes::new(key.stream.aggregate.as_bytes().len()))?;
        if requested > self.identity_bytes_per_key {
            return Err(MemoryError::ValueTooLarge {
                requested,
                capacity: self.identity_bytes_per_key,
            });
        }
        Ok(())
    }
}
