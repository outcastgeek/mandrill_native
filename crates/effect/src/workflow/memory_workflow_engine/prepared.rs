//! Fixed resident workflow tables and independently retained replay pages.

use crate::memory::{Bytes, FixedVector, MemoryError, PreparedMemory, RegionId};
use crate::stream::{PreparedChunkPool, PreparedChunkValues};
use crate::workflow::{
    RecordedActivity, RecordedChild, RecordedCompensation, RecordedRace, RecordedSignal,
    WorkflowDefinitionId, WorkflowEngineError, WorkflowRunId, WorkflowVersion,
};
use std::alloc::Layout;
use std::collections::BTreeMap;
use std::num::NonZeroUsize;
use std::ops::{Deref, DerefMut};
use std::sync::{Arc, Mutex, Weak};

use super::{PreparedMemoryWorkflowEngine, PreparedWorkflowError, Run, State};

/// Startup bounds for the volatile Workflow engine's selected working sets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PreparedWorkflowCapacity {
    /// Registered definition versions retained by the engine.
    pub definitions: NonZeroUsize,
    /// Run identities retained by the engine.
    pub runs: NonZeroUsize,
    /// Recorded activity attempts per run.
    pub activities_per_run: NonZeroUsize,
    /// Recorded child links per run, including links to unstarted children.
    pub children_per_run: NonZeroUsize,
    /// Recorded signals per run.
    pub signals_per_run: NonZeroUsize,
    /// Registered compensations per run.
    pub compensations_per_run: NonZeroUsize,
    /// Recorded race winners per run.
    pub races_per_run: NonZeroUsize,
    /// Concurrently retained replay pages.
    pub pages: NonZeroUsize,
    /// Records in one replay page.
    pub records_per_page: NonZeroUsize,
    /// Bytes in any one stable identity retained by the selected engine.
    pub bytes_per_identity: Bytes,
    /// Bytes in any one encoded payload or compatibility signature.
    pub bytes_per_payload: Bytes,
    /// Aggregate encoded input, history payloads and terminal payload bytes per run.
    pub bytes_per_run: Bytes,
}

pub(super) enum Table<K, V> {
    Ordinary(BTreeMap<K, V>),
    Prepared(FixedVector<'static, (K, V)>),
}

impl<K: Ord, V> Table<K, V> {
    pub(super) const fn ordinary() -> Self {
        Self::Ordinary(BTreeMap::new())
    }

    pub(super) fn storage_bytes(capacity: NonZeroUsize) -> Result<Bytes, MemoryError> {
        Layout::array::<(K, V)>(capacity.get())
            .map(|layout| Bytes::new(layout.size()))
            .map_err(|_| MemoryError::InvalidLayout)
    }

    pub(super) fn prepare(
        memory: &PreparedMemory<'static>,
        region: RegionId,
        capacity: NonZeroUsize,
    ) -> Result<Self, MemoryError> {
        Self::storage_bytes(capacity)?;
        Ok(Self::Prepared(FixedVector::prepare(
            memory.allocator(region)?,
            capacity.get(),
        )?))
    }

    pub(super) fn get(&self, key: &K) -> Option<&V> {
        match self {
            Self::Ordinary(entries) => entries.get(key),
            Self::Prepared(entries) => {
                let index = entries
                    .as_slice()
                    .binary_search_by(|(stored, _)| stored.cmp(key))
                    .ok()?;
                entries.as_slice().get(index).map(|(_, value)| value)
            }
        }
    }

    pub(super) fn get_mut(&mut self, key: &K) -> Option<&mut V> {
        match self {
            Self::Ordinary(entries) => entries.get_mut(key),
            Self::Prepared(entries) => {
                let index = entries
                    .as_slice()
                    .binary_search_by(|(stored, _)| stored.cmp(key))
                    .ok()?;
                entries
                    .as_mut_slice()
                    .get_mut(index)
                    .map(|(_, value)| value)
            }
        }
    }

    pub(super) fn len(&self) -> usize {
        match self {
            Self::Ordinary(entries) => entries.len(),
            Self::Prepared(entries) => entries.len(),
        }
    }

    pub(super) fn iter(&self) -> impl Iterator<Item = (&K, &V)> {
        let (ordinary, prepared) = match self {
            Self::Ordinary(entries) => (Some(entries.iter()), None),
            Self::Prepared(entries) => (None, Some(entries.as_slice().iter())),
        };
        ordinary.into_iter().flatten().chain(
            prepared
                .into_iter()
                .flatten()
                .map(|(key, value)| (key, value)),
        )
    }

    pub(super) fn values(&self) -> impl Iterator<Item = &V> {
        self.iter().map(|(_, value)| value)
    }

    pub(super) fn iter_mut(&mut self) -> impl Iterator<Item = (&K, &mut V)> {
        let (ordinary, prepared) = match self {
            Self::Ordinary(entries) => (Some(entries.iter_mut()), None),
            Self::Prepared(entries) => (None, Some(entries.as_mut_slice().iter_mut())),
        };
        ordinary.into_iter().flatten().chain(
            prepared
                .into_iter()
                .flatten()
                .map(|(key, value)| (&*key, value)),
        )
    }

    pub(super) fn values_mut(&mut self) -> impl Iterator<Item = &mut V> {
        self.iter_mut().map(|(_, value)| value)
    }

    pub(super) fn try_insert(&mut self, key: K, value: V) -> Result<Option<V>, MemoryError> {
        match self {
            Self::Ordinary(entries) => Ok(entries.insert(key, value)),
            Self::Prepared(entries) => {
                let position = entries
                    .as_slice()
                    .binary_search_by(|(stored, _)| stored.cmp(&key));
                match position {
                    Ok(index) => {
                        let (_, previous) = entries
                            .as_mut_slice()
                            .get_mut(index)
                            .ok_or(MemoryError::InvalidRange)?;
                        Ok(Some(std::mem::replace(previous, value)))
                    }
                    Err(index) => {
                        entries
                            .push((key, value))
                            .map_err(|_| MemoryError::NoSlot)?;
                        if let Some(suffix) = entries.as_mut_slice().get_mut(index..) {
                            suffix.rotate_right(1);
                        }
                        Ok(None)
                    }
                }
            }
        }
    }
}

impl<K: Ord, V> Default for Table<K, V> {
    fn default() -> Self {
        Self::ordinary()
    }
}

impl<K, V> From<BTreeMap<K, V>> for Table<K, V> {
    fn from(entries: BTreeMap<K, V>) -> Self {
        Self::Ordinary(entries)
    }
}

pub(super) enum History<T: Send + 'static> {
    Ordinary(Vec<T>),
    Prepared(PreparedChunkValues<T>),
}

impl<T: Send + 'static> History<T> {
    pub(super) const fn ordinary() -> Self {
        Self::Ordinary(Vec::new())
    }

    pub(super) fn try_push(&mut self, record: T) -> Result<(), MemoryError> {
        match self {
            Self::Ordinary(records) => {
                records.push(record);
                Ok(())
            }
            Self::Prepared(records) => records.try_push(record).map_err(|_| MemoryError::NoSlot),
        }
    }
}

impl<T: Send + 'static> From<Vec<T>> for History<T> {
    fn from(records: Vec<T>) -> Self {
        Self::Ordinary(records)
    }
}

impl<T: Send + 'static> Deref for History<T> {
    type Target = [T];
    fn deref(&self) -> &[T] {
        match self {
            Self::Ordinary(records) => records,
            Self::Prepared(records) => records.as_slice(),
        }
    }
}

impl<T: Send + 'static> DerefMut for History<T> {
    fn deref_mut(&mut self) -> &mut [T] {
        match self {
            Self::Ordinary(records) => records,
            Self::Prepared(records) => records.as_mut_slice(),
        }
    }
}

impl<'a, T: Send + 'static> IntoIterator for &'a History<T> {
    type Item = &'a T;
    type IntoIter = std::slice::Iter<'a, T>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

pub(super) struct PreparedWorkflowHistories {
    pub(super) activities: History<RecordedActivity>,
    pub(super) children: History<RecordedChild>,
    pub(super) signals: History<RecordedSignal>,
    pub(super) compensations: History<RecordedCompensation>,
    pub(super) races: History<RecordedRace>,
}

pub(super) struct PreparedWorkflowStorage {
    pub(super) capacity: PreparedWorkflowCapacity,
    activities: PreparedChunkPool<RecordedActivity>,
    children: PreparedChunkPool<RecordedChild>,
    signals: PreparedChunkPool<RecordedSignal>,
    compensations: PreparedChunkPool<RecordedCompensation>,
    races: PreparedChunkPool<RecordedRace>,
    pages: PreparedChunkPool<WorkflowHistoryRecord>,
    descendants: PreparedChunkPool<WorkflowRunId>,
}

impl PreparedWorkflowCapacity {
    fn descendant_capacity(self) -> Result<NonZeroUsize, MemoryError> {
        self.runs
            .checked_mul(self.children_per_run)
            .ok_or(MemoryError::ArithmeticOverflow)
    }

    /// Inline tables and slot backing in the selected region. Region allocator
    /// headers and alignment overhead are charged by the enclosing memory plan.
    /// Encoded values retain caller backing within the stated byte limits.
    pub fn storage_bytes(self) -> Result<Bytes, MemoryError> {
        Table::<(WorkflowDefinitionId, WorkflowVersion), Arc<[u8]>>::storage_bytes(
            self.definitions,
        )?
        .checked_add(Table::<WorkflowRunId, Run>::storage_bytes(self.runs)?)?
        .checked_add(PreparedChunkPool::<RecordedActivity>::storage_bytes(
            self.runs,
            self.activities_per_run,
        )?)?
        .checked_add(PreparedChunkPool::<RecordedChild>::storage_bytes(
            self.runs,
            self.children_per_run,
        )?)?
        .checked_add(PreparedChunkPool::<RecordedSignal>::storage_bytes(
            self.runs,
            self.signals_per_run,
        )?)?
        .checked_add(PreparedChunkPool::<RecordedCompensation>::storage_bytes(
            self.runs,
            self.compensations_per_run,
        )?)?
        .checked_add(PreparedChunkPool::<RecordedRace>::storage_bytes(
            self.runs,
            self.races_per_run,
        )?)?
        .checked_add(PreparedChunkPool::<WorkflowHistoryRecord>::storage_bytes(
            self.pages,
            self.records_per_page,
        )?)?
        .checked_add(PreparedChunkPool::<WorkflowRunId>::storage_bytes(
            NonZeroUsize::MIN,
            self.descendant_capacity()?,
        )?)
    }
}

impl PreparedWorkflowStorage {
    pub(super) fn prepare(
        memory: &PreparedMemory<'static>,
        region: RegionId,
        capacity: PreparedWorkflowCapacity,
    ) -> Result<Self, MemoryError> {
        capacity.storage_bytes()?;
        Ok(Self {
            capacity,
            activities: PreparedChunkPool::prepare(
                memory,
                region,
                capacity.runs,
                capacity.activities_per_run,
            )?,
            children: PreparedChunkPool::prepare(
                memory,
                region,
                capacity.runs,
                capacity.children_per_run,
            )?,
            signals: PreparedChunkPool::prepare(
                memory,
                region,
                capacity.runs,
                capacity.signals_per_run,
            )?,
            compensations: PreparedChunkPool::prepare(
                memory,
                region,
                capacity.runs,
                capacity.compensations_per_run,
            )?,
            races: PreparedChunkPool::prepare(
                memory,
                region,
                capacity.runs,
                capacity.races_per_run,
            )?,
            pages: PreparedChunkPool::prepare(
                memory,
                region,
                capacity.pages,
                capacity.records_per_page,
            )?,
            descendants: PreparedChunkPool::prepare(
                memory,
                region,
                NonZeroUsize::MIN,
                capacity.descendant_capacity()?,
            )?,
        })
    }

    pub(super) fn histories(&self) -> Result<PreparedWorkflowHistories, MemoryError> {
        Ok(PreparedWorkflowHistories {
            activities: History::Prepared(self.activities.try_checkout()?),
            children: History::Prepared(self.children.try_checkout()?),
            signals: History::Prepared(self.signals.try_checkout()?),
            compensations: History::Prepared(self.compensations.try_checkout()?),
            races: History::Prepared(self.races.try_checkout()?),
        })
    }

    pub(super) fn collect_descendants(
        &self,
        runs: &Table<WorkflowRunId, Run>,
        root: &WorkflowRunId,
    ) -> Result<PreparedChunkValues<WorkflowRunId>, PreparedWorkflowError> {
        let mut branch = Some(runs.get(root).ok_or(WorkflowEngineError::MissingRun)?);
        let mut entries = self.descendants.try_checkout()?;
        let mut visited = 0;
        loop {
            // A node is expanded only once. Each retained occurrence is one
            // edge, so runs times children_per_run bounds the entire workspace.
            if let Some(node) = branch {
                for child in &node.children {
                    entries
                        .try_push(child.handle.run.clone())
                        .map_err(|_| MemoryError::NoSlot)?;
                }
            }
            if visited == entries.len() {
                return Ok(entries);
            }
            // The visited prefix is the result in the same depth-first order
            // as the ordinary traversal. The pending suffix is a LIFO stack.
            let run = entries
                .remove_at(entries.len() - 1)
                .ok_or(MemoryError::InvalidRange)?;
            if &run == root || entries.as_slice().split_at(visited).0.contains(&run) {
                return Err(WorkflowEngineError::ChildConflict.into());
            }
            branch = runs.get(&run);
            entries.push_reserved(run);
            entries
                .as_mut_slice()
                .split_at_mut(visited)
                .1
                .rotate_right(1);
            visited += 1;
        }
    }
}

/// A bounded page entry in the engine's replay export order.
pub enum WorkflowHistoryRecord {
    /// Recorded activity attempt.
    Activity(RecordedActivity),
    /// Recorded child link.
    Child(RecordedChild),
    /// Delivered signal.
    Signal(RecordedSignal),
    /// Registered compensation and current settlement.
    Compensation(RecordedCompensation),
    /// Recorded race result.
    Race(RecordedRace),
}

/// A continuation bound to one engine, run, and history revision.
pub struct PreparedWorkflowHistoryCursor {
    engine: Weak<Mutex<State>>,
    run: WorkflowRunId,
    revision: u64,
    next: usize,
}

/// A finite replay page whose inline records outlive the engine or a later write.
pub struct PreparedWorkflowHistoryPage {
    records: PreparedChunkValues<WorkflowHistoryRecord>,
    next: Option<PreparedWorkflowHistoryCursor>,
}

impl PreparedWorkflowHistoryPage {
    /// Borrow the retained records in scan order.
    #[must_use]
    pub fn records(&self) -> &[WorkflowHistoryRecord] {
        self.records.as_slice()
    }

    /// Read the next position while retaining this page.
    #[must_use]
    pub fn next_cursor(&self) -> Option<&PreparedWorkflowHistoryCursor> {
        self.next.as_ref()
    }

    /// Move the continuation out and return this page's physical backing.
    #[must_use]
    pub fn into_next_cursor(self) -> Option<PreparedWorkflowHistoryCursor> {
        self.next
    }
}

/// A precise refusal from the selected paging contract.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PreparedWorkflowHistoryError {
    /// The run could not be found.
    Engine(WorkflowEngineError),
    /// The selected physical page could not be admitted.
    Memory(MemoryError),
    /// The requested record count exceeds one prepared page.
    PageTooWide {
        /// Requested number of records.
        requested: NonZeroUsize,
        /// Provisioned records per page.
        capacity: NonZeroUsize,
    },
    /// The supplied continuation belongs to another engine or run.
    CursorMismatch,
    /// The run's history changed between pages. Start a new scan.
    HistoryChanged,
}

fn history_suffix<'a, T>(records: &'a [T], offset: &mut usize) -> &'a [T] {
    let local = (*offset).min(records.len());
    *offset -= local;
    records.split_at(local).1
}

impl PreparedMemoryWorkflowEngine {
    /// Read one bounded page of the recorded workflow history. Records are
    /// grouped by kind and preserve their original order within each group.
    /// Obtain the run header separately through the engine's header API.
    /// An append or in-place compensation settlement invalidates prior
    /// continuations; already returned pages remain valid.
    pub fn read_history_page(
        &self,
        run: &WorkflowRunId,
        cursor: Option<&PreparedWorkflowHistoryCursor>,
        limit: NonZeroUsize,
    ) -> Result<PreparedWorkflowHistoryPage, PreparedWorkflowHistoryError> {
        let engine = Arc::downgrade(&self.state);
        if cursor.is_some_and(|cursor| !cursor.engine.ptr_eq(&engine) || &cursor.run != run) {
            return Err(PreparedWorkflowHistoryError::CursorMismatch);
        }
        let state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let current = state
            .runs
            .get(run)
            .ok_or(PreparedWorkflowHistoryError::Engine(
                WorkflowEngineError::MissingRun,
            ))?;
        let revision = current.history_revision;
        if cursor.is_some_and(|cursor| cursor.revision != revision) {
            return Err(PreparedWorkflowHistoryError::HistoryChanged);
        }
        let storage = state
            .prepared
            .as_ref()
            .ok_or(PreparedWorkflowHistoryError::Memory(
                MemoryError::UnsupportedRunContract,
            ))?;
        if limit > storage.capacity.records_per_page {
            return Err(PreparedWorkflowHistoryError::PageTooWide {
                requested: limit,
                capacity: storage.capacity.records_per_page,
            });
        }
        let offset = cursor.map_or(0, |cursor| cursor.next);
        let total = [
            current.activities.len(),
            current.children.len(),
            current.signals.len(),
            current.compensations.len(),
            current.races.len(),
        ]
        .into_iter()
        .try_fold(0usize, usize::checked_add)
        .ok_or(PreparedWorkflowHistoryError::Memory(
            MemoryError::ArithmeticOverflow,
        ))?;
        let remaining = total
            .checked_sub(offset)
            .ok_or(PreparedWorkflowHistoryError::CursorMismatch)?;
        let width = remaining.min(limit.get());
        let end = offset + width;
        let mut records = storage
            .pages
            .try_checkout()
            .map_err(PreparedWorkflowHistoryError::Memory)?;
        let mut skip = offset;
        let history = history_suffix(&current.activities, &mut skip)
            .iter()
            .cloned()
            .map(WorkflowHistoryRecord::Activity)
            .chain(
                history_suffix(&current.children, &mut skip)
                    .iter()
                    .cloned()
                    .map(WorkflowHistoryRecord::Child),
            )
            .chain(
                history_suffix(&current.signals, &mut skip)
                    .iter()
                    .cloned()
                    .map(WorkflowHistoryRecord::Signal),
            )
            .chain(
                history_suffix(&current.compensations, &mut skip)
                    .iter()
                    .cloned()
                    .map(WorkflowHistoryRecord::Compensation),
            )
            .chain(
                history_suffix(&current.races, &mut skip)
                    .iter()
                    .cloned()
                    .map(WorkflowHistoryRecord::Race),
            );
        for record in history.take(width) {
            records.push_reserved(record);
        }
        let next = (end < total).then(|| PreparedWorkflowHistoryCursor {
            engine,
            run: run.clone(),
            revision,
            next: end,
        });
        Ok(PreparedWorkflowHistoryPage { records, next })
    }
}
