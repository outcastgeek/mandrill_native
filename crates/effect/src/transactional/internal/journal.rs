use crate::transactional::tx_ref::ReferenceHandle;
use event_listener::Event;
use std::any::Any;
use std::collections::BTreeMap;
use std::num::NonZeroU64;
use std::sync::{Arc, Mutex};

pub(crate) type ErasedValue = Arc<dyn Any + Send + Sync>;

pub(crate) struct Cell {
    pub version: u64,
    pub value: ErasedValue,
}

#[derive(Default)]
pub(crate) struct DomainState {
    pub epoch: u64,
    pub next_id: u64,
    pub cells: BTreeMap<NonZeroU64, Cell>,
}

pub(crate) struct DomainInner {
    pub state: Mutex<DomainState>,
    pub changed: Event,
}

impl Default for DomainInner {
    fn default() -> Self {
        Self {
            state: Mutex::new(DomainState::default()),
            changed: Event::new(),
        }
    }
}

#[derive(Clone)]
pub(crate) struct JournalBranch {
    reads: BTreeMap<NonZeroU64, ReadRecord>,
    writes: BTreeMap<NonZeroU64, ErasedValue>,
}

#[derive(Clone)]
pub(crate) struct ReadRecord {
    pub version: u64,
    _handle: Arc<ReferenceHandle>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum JournalError {
    Conflict,
    MixedDomain,
    MissingCell,
    WrongValueType,
    VersionExhausted,
}

pub(crate) struct TxJournal {
    pub domain: Arc<DomainInner>,
    pub snapshot_epoch: u64,
    reads: BTreeMap<NonZeroU64, ReadRecord>,
    writes: BTreeMap<NonZeroU64, ErasedValue>,
}

impl TxJournal {
    pub fn new(domain: Arc<DomainInner>) -> Self {
        let snapshot_epoch = domain
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .epoch;
        Self {
            domain,
            snapshot_epoch,
            reads: BTreeMap::new(),
            writes: BTreeMap::new(),
        }
    }

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

    pub fn write<T: Any + Send + Sync>(
        &mut self,
        handle: &Arc<ReferenceHandle>,
        value: Arc<T>,
    ) -> Result<(), JournalError> {
        let _ = self.read::<T>(handle)?;
        self.writes.insert(handle.id, value);
        Ok(())
    }

    pub fn branch(&self) -> JournalBranch {
        JournalBranch {
            reads: self.reads.clone(),
            writes: self.writes.clone(),
        }
    }

    pub fn rollback(&mut self, branch: JournalBranch) {
        self.reads = branch.reads;
        self.writes = branch.writes;
    }

    pub fn observed(&self) -> BTreeMap<NonZeroU64, ReadRecord> {
        self.reads.clone()
    }

    pub fn merge_observed(&mut self, other: BTreeMap<NonZeroU64, ReadRecord>) {
        for (id, record) in other {
            self.reads.entry(id).or_insert(record);
        }
    }

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
