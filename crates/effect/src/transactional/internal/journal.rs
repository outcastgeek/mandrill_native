//! Read and write journals for atomic transactions.

use crate::transactional::tx_ref::ReferenceHandle;
use event_listener::Event;
use std::any::Any;
use std::collections::BTreeMap;
use std::num::NonZeroU64;
use std::sync::{Arc, Mutex};

/// Shared cell value behind an `Any` vtable. The journal checks its concrete
/// type before reading it; the erased handle never leaves the transaction domain.
pub(crate) type ErasedValue = Arc<dyn Any + Send + Sync>;

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
    pub cells: BTreeMap<NonZeroU64, Cell>,
}

/// Typed domain inner used by transactional internal journal.
pub(crate) struct DomainInner {
    pub state: Mutex<DomainState>,
    pub changed: Event,
}

impl Default for DomainInner {
    ///
    /// # Returns
    /// Type: `Self`.
    fn default() -> Self {
        Self {
            state: Mutex::new(DomainState::default()),
            changed: Event::new(),
        }
    }
}

#[derive(Clone)]
/// Typed journal branch used by transactional internal journal.
pub(crate) struct JournalBranch {
    reads: BTreeMap<NonZeroU64, ReadRecord>,
    writes: BTreeMap<NonZeroU64, ErasedValue>,
}

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
}

/// Typed tx journal used by transactional internal journal.
pub(crate) struct TxJournal {
    pub domain: Arc<DomainInner>,
    pub snapshot_epoch: u64,
    /// Atomic execution cancellation observed by native transaction body checkpoints.
    pub cancellation: Option<crate::Cancellation>,
    reads: BTreeMap<NonZeroU64, ReadRecord>,
    writes: BTreeMap<NonZeroU64, ErasedValue>,
}

impl TxJournal {
    /// Construct a `TxJournal` from its required inputs and explicit policies.
    ///
    /// # Parameters
    /// - `domain` (`Arc<DomainInner>`): Stable identity or provider of the operation’s transaction domain.
    ///
    /// # Returns
    /// Type: `Self`.
    pub fn new(domain: Arc<DomainInner>) -> Self {
        let snapshot_epoch = domain
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .epoch;
        Self {
            domain,
            snapshot_epoch,
            cancellation: None,
            reads: BTreeMap::new(),
            writes: BTreeMap::new(),
        }
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
        self.reads.entry(handle.id).or_insert_with(|| ReadRecord {
            version: cell.version,
            _handle: Arc::clone(handle),
        });
        let value = self
            .writes
            .get(&handle.id)
            .map_or_else(|| Arc::clone(&cell.value), Arc::clone);
        Arc::downcast::<T>(value).map_err(|_| JournalError::WrongValueType)
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
        let _ = self.read::<T>(handle)?;
        self.writes.insert(handle.id, value);
        Ok(())
    }

    /// Fork the current transaction journal for an alternative computation.
    ///
    /// # Returns
    /// Type: `JournalBranch`.
    pub fn branch(&self) -> JournalBranch {
        JournalBranch {
            reads: self.reads.clone(),
            writes: self.writes.clone(),
        }
    }

    /// Discard the writes recorded in this journal branch.
    ///
    /// # Parameters
    /// - `branch` (`JournalBranch`): The branch supplied to this operation.
    ///
    /// # Returns
    /// Type: `()`.
    pub fn rollback(&mut self, branch: JournalBranch) {
        self.reads = branch.reads;
        self.writes = branch.writes;
    }

    /// Return the cells read by this branch for conflict validation.
    ///
    /// # Returns
    /// Type: `BTreeMap<NonZeroU64, ReadRecord>`.
    pub fn observed(&self) -> BTreeMap<NonZeroU64, ReadRecord> {
        self.reads.clone()
    }

    /// Merge another branch’s read observations into this journal.
    ///
    /// # Parameters
    /// - `other` (`BTreeMap<NonZeroU64, ReadRecord>`): Second value used for comparison or composition.
    ///
    /// # Returns
    /// Type: `()`.
    pub fn merge_observed(&mut self, other: BTreeMap<NonZeroU64, ReadRecord>) {
        for (id, record) in other {
            self.reads.entry(id).or_insert(record);
        }
    }

    /// Validate recorded reads and publish this journal’s writes atomically.
    ///
    /// # Returns
    /// Type: `Result<bool, JournalError>`.
    pub fn commit(self) -> Result<bool, JournalError> {
        let mut state = self
            .domain
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.epoch != self.snapshot_epoch {
            return Err(JournalError::Conflict);
        }
        for (id, record) in &self.reads {
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
        for id in self.writes.keys() {
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
        let mut retired = self.writes;
        for (id, value) in &mut retired {
            let cell = state.cells.get_mut(id).ok_or(JournalError::MissingCell)?;
            cell.version += 1;
            std::mem::swap(&mut cell.value, value);
        }
        state.epoch = next_epoch;
        drop(state);
        self.domain.changed.notify(usize::MAX);
        drop(retired);
        Ok(true)
    }
}
