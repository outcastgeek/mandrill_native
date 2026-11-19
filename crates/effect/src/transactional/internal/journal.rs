//! Read and write journals for atomic transactions.

use super::records::{RecordPool, Records};
use super::value::ErasedValue;
use crate::memory::prepared_notification;
use crate::memory::{Bytes, MemoryError, PreparedMemory, RegionId, SlotLease, SlotPool};
use crate::transactional::tx_ref::{PreparedTransactionCapacity, ReferenceHandle};
use event_listener::Event;
use std::any::Any;
use std::num::NonZeroU64;
use std::sync::{Arc, Mutex};

/// Typed cell used by transactional internal journal.
pub(crate) struct Cell {
    pub version: u64,
    pub value: ErasedValue,
}

#[derive(Default)]
/// Typed domain state used by transactional internal journal.
pub(crate) struct DomainState {
    pub epoch: u64,
    pub next_id: u64,
    cells: Records<Cell>,
}

/// Typed domain inner used by transactional internal journal.
pub(crate) struct DomainInner {
    pub state: Mutex<DomainState>,
    pub changed: Event,
    prepared: Option<PreparedJournals>,
}

impl Default for DomainInner {
    ///
    /// # Returns
    /// Type: `Self`.
    fn default() -> Self {
        Self {
            state: Mutex::new(DomainState::default()),
            changed: Event::new(),
            prepared: None,
        }
    }
}

struct PreparedJournals {
    reads: RecordPool<ReadRecord>,
    writes: RecordPool<ErasedValue>,
    retry_reads: RecordPool<ReadRecord>,
    changed: prepared_notification::Event,
    cleanup: SlotPool<'static, ReleaseSlot>,
}

type ReleaseSlot = Mutex<Option<JournalBranch>>;

/// An affine prepared cleanup control and the journal awaiting protected release.
pub(crate) struct ReleaseJournal(SlotLease<'static, ReleaseSlot>);

impl ReleaseJournal {
    pub fn take(&self) -> Result<JournalBranch, MemoryError> {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
            .ok_or(MemoryError::Closed)
    }
}

impl Drop for ReleaseJournal {
    fn drop(&mut self) {
        drop(
            self.0
                .get_mut()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take(),
        );
    }
}

impl DomainInner {
    pub fn is_prepared(&self) -> bool {
        self.prepared.is_some()
    }
    pub fn storage_bytes(capacity: PreparedTransactionCapacity) -> Result<Bytes, MemoryError> {
        RecordPool::<Cell>::storage_bytes(1, capacity.cells.get())?
            .checked_add(RecordPool::<ReadRecord>::storage_bytes(
                capacity.journals.get(),
                capacity.reads.get(),
            )?)?
            .checked_add(RecordPool::<ErasedValue>::storage_bytes(
                capacity.journals.get(),
                capacity.writes.get(),
            )?)?
            .checked_add(RecordPool::<ReadRecord>::storage_bytes(
                capacity.retry_waiters.get(),
                capacity.reads.get(),
            )?)?
            .checked_add(prepared_notification::Event::storage_bytes(
                capacity.retry_waiters.get(),
            )?)?
            .checked_add(SlotPool::<ReleaseSlot>::storage_bytes(
                capacity.journals.get(),
            )?)
    }

    pub fn prepare(
        memory: &PreparedMemory<'static>,
        region: RegionId,
        capacity: PreparedTransactionCapacity,
    ) -> Result<Self, MemoryError> {
        Self::storage_bytes(capacity)?;
        let cells = RecordPool::prepare(memory, region, 1, capacity.cells.get())?.reserve()?;
        Ok(Self {
            state: Mutex::new(DomainState {
                cells,
                ..DomainState::default()
            }),
            changed: Event::new(),
            prepared: Some(PreparedJournals {
                reads: RecordPool::prepare(
                    memory,
                    region,
                    capacity.journals.get(),
                    capacity.reads.get(),
                )?,
                writes: RecordPool::prepare(
                    memory,
                    region,
                    capacity.journals.get(),
                    capacity.writes.get(),
                )?,
                retry_reads: RecordPool::prepare(
                    memory,
                    region,
                    capacity.retry_waiters.get(),
                    capacity.reads.get(),
                )?,
                changed: prepared_notification::Event::prepare(
                    memory,
                    region,
                    capacity.retry_waiters.get(),
                )?,
                cleanup: memory.slots(region, capacity.journals.get(), |_| Ok(Mutex::new(None)))?,
            }),
        })
    }

    fn reserve_journals(&self) -> Result<JournalBranch, MemoryError> {
        Ok(match &self.prepared {
            Some(prepared) => JournalBranch {
                reads: prepared.reads.reserve()?,
                writes: prepared.writes.reserve()?,
            },
            None => JournalBranch {
                reads: Records::default(),
                writes: Records::default(),
            },
        })
    }

    pub fn reserve_cleanup(&self) -> Result<Option<ReleaseJournal>, MemoryError> {
        let Some(prepared) = &self.prepared else {
            return Ok(None);
        };
        let mut control = ReleaseJournal(prepared.cleanup.try_checkout()?);
        let records = self.reserve_journals()?;
        *control
            .0
            .get_mut()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(records);
        Ok(Some(control))
    }

    pub fn reserve_retry(&self) -> Result<impl Future<Output = ()> + Send + use<>, MemoryError> {
        let listener = match &self.prepared {
            Some(prepared) => futures_util::future::Either::Left(prepared.changed.listen()?),
            None => futures_util::future::Either::Right(self.changed.listen()),
        };
        Ok(async move {
            match listener {
                futures_util::future::Either::Left(registration) => {
                    std::future::poll_fn(|cx| registration.poll(cx)).await;
                }
                futures_util::future::Either::Right(listener) => listener.await,
            }
        })
    }

    fn notify_committed(&self) {
        if let Some(prepared) = &self.prepared {
            // Keep existing committed observers live even if a custom waker panics.
            let _ordinary =
                std::mem::DropGuard::new(&self.changed, crate::internal::notification::notify_all);
            prepared.changed.notify_all();
        } else {
            self.changed.notify(usize::MAX);
        }
    }

    pub fn create_cell(
        &self,
        capacity: usize,
        value: ErasedValue,
    ) -> Result<NonZeroU64, super::super::TransactionDomainError> {
        use super::super::TransactionDomainError;
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.cells.len() >= capacity {
            return Err(TransactionDomainError::Capacity);
        }
        let id = state
            .next_id
            .checked_add(1)
            .and_then(NonZeroU64::new)
            .ok_or(TransactionDomainError::IdentityExhausted)?;
        state
            .cells
            .reserve_entries(1)
            .map_err(|_| TransactionDomainError::Capacity)?;
        state.cells.insert_reserved(id, Cell { version: 0, value });
        state.next_id = id.get();
        Ok(id)
    }

    pub fn remove_cell(&self, id: NonZeroU64) {
        let removed = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .cells
            .remove(&id);
        // A value's destructor can reenter the domain.
        drop(removed);
    }

    pub fn inspect<T: Any + Send + Sync>(
        &self,
        id: NonZeroU64,
    ) -> Result<(u64, Arc<T>), JournalError> {
        let state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let cell = state.cells.get(&id).ok_or(JournalError::MissingCell)?;
        let value = cell
            .value
            .try_clone()
            .map_err(JournalError::Memory)?
            .into_ordinary::<T>()?;
        Ok((cell.version, value))
    }
}

/// Typed journal branch used by transactional internal journal.
pub(crate) struct JournalBranch {
    reads: Records<ReadRecord>,
    writes: Records<ErasedValue>,
}

pub(crate) struct Observed(Records<ReadRecord>);

#[derive(Clone)]
/// Typed read record used by transactional internal journal.
pub(crate) struct ReadRecord {
    pub version: u64,
    _handle: Arc<ReferenceHandle>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// Typed failure cases for transactional internal journal.
pub(crate) enum JournalError {
    Conflict,
    MixedDomain,
    MissingCell,
    WrongValueType,
    VersionExhausted,
    Memory(MemoryError),
}

/// Typed tx journal used by transactional internal journal.
pub(crate) struct TxJournal {
    pub domain: Arc<DomainInner>,
    pub snapshot_epoch: u64,
    /// Atomic execution cancellation observed by native transaction body checkpoints.
    pub cancellation: Option<crate::Cancellation>,
    reads: Records<ReadRecord>,
    writes: Records<ErasedValue>,
}

impl TxJournal {
    /// Construct a `TxJournal` from its required inputs and explicit policies.
    ///
    /// # Parameters
    /// - `domain` (`Arc<DomainInner>`): Stable identity or provider of the operation’s transaction domain.
    ///
    /// # Returns
    /// Type: `Result<Self, MemoryError>`.
    pub fn new(
        domain: Arc<DomainInner>,
        reserved: Option<JournalBranch>,
    ) -> Result<Self, MemoryError> {
        let JournalBranch { reads, writes } = match reserved {
            Some(records) => records,
            None => domain.reserve_journals()?,
        };
        let snapshot_epoch = domain
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .epoch;
        Ok(Self {
            domain,
            snapshot_epoch,
            cancellation: None,
            reads,
            writes,
        })
    }

    pub fn recycle(self) -> JournalBranch {
        let Self {
            mut reads,
            mut writes,
            ..
        } = self;
        writes.clear();
        reads.clear();
        JournalBranch { reads, writes }
    }

    pub fn has_reads(&self) -> bool {
        !self.reads.is_empty()
    }

    /// Read the selected state without assuming it has been modified.
    ///
    /// # Type parameters
    /// - `T`: Value or item type. Bounds: `Any + Send + Sync`.
    ///
    /// # Parameters
    /// - `handle` (`&Arc<ReferenceHandle>`): Owned or borrowed capability handle kept live through this operation.
    ///
    /// # Returns
    /// Type: `Result<Arc<T>, JournalError>`.
    pub fn read<T: Any + Send + Sync>(
        &mut self,
        handle: &Arc<ReferenceHandle>,
    ) -> Result<Arc<T>, JournalError> {
        self.read_erased(handle, false)?.into_ordinary()
    }

    pub fn read_for_write<T: Any + Send + Sync>(
        &mut self,
        handle: &Arc<ReferenceHandle>,
    ) -> Result<Arc<T>, JournalError> {
        self.read_erased(handle, true)?.into_ordinary()
    }

    // Check both selected tables before recording an observation for a write.
    // All branch records are private to this running attempt.
    pub fn read_erased(
        &mut self,
        handle: &Arc<ReferenceHandle>,
        reserve_write: bool,
    ) -> Result<ErasedValue, JournalError> {
        if !Arc::ptr_eq(&self.domain, &handle.domain) {
            return Err(JournalError::MixedDomain);
        }
        let state = self
            .domain
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.epoch != self.snapshot_epoch {
            return Err(JournalError::Conflict);
        }
        let cell = state
            .cells
            .get(&handle.id)
            .ok_or(JournalError::MissingCell)?;
        let staged = self.writes.get(&handle.id);
        if reserve_write && staged.is_none() {
            self.writes
                .reserve_entries(1)
                .map_err(JournalError::Memory)?;
        }
        let new_read = self.reads.get(&handle.id).is_none();
        if new_read {
            self.reads
                .reserve_entries(1)
                .map_err(JournalError::Memory)?;
        }
        let value = staged
            .unwrap_or(&cell.value)
            .try_clone()
            .map_err(JournalError::Memory)?;
        if new_read {
            self.reads.insert_reserved(
                handle.id,
                ReadRecord {
                    version: cell.version,
                    _handle: Arc::clone(handle),
                },
            );
        }
        Ok(value)
    }

    /// Record the supplied value through the selected state or transport capability.
    ///
    /// # Type parameters
    /// - `T`: Value or item type. Bounds: `Any + Send + Sync`.
    ///
    /// # Parameters
    /// - `handle` (`&Arc<ReferenceHandle>`): Owned or borrowed capability handle kept live through this operation.
    /// - `value` (`Arc<T>`): Value read, transformed, encoded, or stored by this operation.
    ///
    /// # Returns
    /// Type: `Result<(), JournalError>`.
    pub fn write<T: Any + Send + Sync>(
        &mut self,
        handle: &Arc<ReferenceHandle>,
        value: Arc<T>,
    ) -> Result<(), JournalError> {
        self.write_erased(handle, ErasedValue::ordinary(value))
    }

    pub fn write_erased(
        &mut self,
        handle: &Arc<ReferenceHandle>,
        value: ErasedValue,
    ) -> Result<(), JournalError> {
        if !Arc::ptr_eq(&self.domain, &handle.domain) {
            return Err(JournalError::MixedDomain);
        }
        let state = self
            .domain
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.epoch != self.snapshot_epoch {
            return Err(JournalError::Conflict);
        }
        let cell = state
            .cells
            .get(&handle.id)
            .ok_or(JournalError::MissingCell)?;
        let staged = self.writes.get(&handle.id);
        staged.unwrap_or(&cell.value).check_type(&value)?;
        if staged.is_none() {
            self.writes
                .reserve_entries(1)
                .map_err(JournalError::Memory)?;
        }
        if self.reads.get(&handle.id).is_none() {
            self.reads
                .reserve_entries(1)
                .map_err(JournalError::Memory)?;
            self.reads.insert_reserved(
                handle.id,
                ReadRecord {
                    version: cell.version,
                    _handle: Arc::clone(handle),
                },
            );
        }
        drop(state);
        if let Some(staged) = self.writes.get_mut(&handle.id) {
            *staged = value;
        } else {
            self.writes.insert_reserved(handle.id, value);
        }
        Ok(())
    }

    /// Fork the current transaction journal for an alternative computation.
    ///
    /// # Returns
    /// Type: `Result<JournalBranch, MemoryError>`.
    pub fn branch(&self) -> Result<JournalBranch, MemoryError> {
        let mut branch = self.domain.reserve_journals()?;
        for (&id, record) in self.reads.iter() {
            branch.reads.insert_reserved(id, record.clone());
        }
        for (&id, value) in self.writes.iter() {
            branch.writes.insert_reserved(id, value.try_clone()?);
        }
        Ok(branch)
    }

    /// Discard the writes recorded in this journal branch.
    ///
    /// # Parameters
    /// - `branch` (`JournalBranch`): The branch supplied to this operation.
    ///
    /// # Returns
    /// The abandoned branch's read owners, retained for a potential union.
    pub fn rollback(&mut self, branch: JournalBranch) -> Observed {
        let abandoned = std::mem::replace(&mut self.reads, branch.reads);
        self.writes = branch.writes;
        Observed(abandoned)
    }

    /// Retain just the observed reads during an explicit retry.
    pub fn reserve_observed(&self) -> Result<Observed, MemoryError> {
        let mut observed = match &self.domain.prepared {
            Some(prepared) => prepared.retry_reads.reserve()?,
            None => Records::default(),
        };
        for (&id, record) in self.reads.iter() {
            observed.insert_reserved(id, record.clone());
        }
        Ok(Observed(observed))
    }

    /// Merge another branch’s read observations into this journal.
    ///
    /// # Parameters
    /// - `other` (`Observed`): The first branch's retained read set.
    ///
    /// # Returns
    /// Type: `Result<(), MemoryError>`.
    pub fn merge_observed(&mut self, other: Observed) -> Result<(), MemoryError> {
        let missing = other
            .0
            .iter()
            .filter(|(id, _)| self.reads.get(id).is_none())
            .count();
        self.reads.reserve_entries(missing)?;
        for (&id, record) in other.0.iter() {
            if self.reads.get(&id).is_none() {
                self.reads.insert_reserved(id, record.clone());
            }
        }
        Ok(())
    }

    /// Validate recorded reads and publish this journal’s writes atomically.
    ///
    /// # Returns
    /// Type: `Result<bool, JournalError>`.
    pub fn commit(&mut self) -> Result<bool, JournalError> {
        let mut state = self
            .domain
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.epoch != self.snapshot_epoch {
            return Err(JournalError::Conflict);
        }
        for (id, record) in self.reads.iter() {
            if state.cells.get(id).map(|cell| cell.version) != Some(record.version) {
                return Err(JournalError::Conflict);
            }
        }
        if self.writes.is_empty() {
            return Ok(false);
        }
        let next_epoch = state
            .epoch
            .checked_add(1)
            .ok_or(JournalError::VersionExhausted)?;
        for (id, _) in self.writes.iter() {
            state
                .cells
                .get(id)
                .ok_or(JournalError::MissingCell)?
                .version
                .checked_add(1)
                .ok_or(JournalError::VersionExhausted)?;
        }
        // Reuse the write set to carry replaced values out of the commit gate.
        // A user value's Drop may reenter this domain.
        for (id, value) in self.writes.iter_mut() {
            let cell = state.cells.get_mut(id).ok_or(JournalError::MissingCell)?;
            cell.version += 1;
            std::mem::swap(&mut cell.value, value);
        }
        state.epoch = next_epoch;
        drop(state);
        self.domain.notify_committed();
        self.writes.clear();
        Ok(true)
    }
}

impl Observed {
    pub fn is_stale(&self, domain: &DomainInner) -> bool {
        let state = domain
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        self.0.iter().any(|(id, record)| {
            state.cells.get(id).map(|cell| cell.version) != Some(record.version)
        })
    }
}
