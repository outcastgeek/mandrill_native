use super::{JournalCursor, JournalStream, ReplicaId};
use crate::Effect;
use std::collections::BTreeMap;
use std::num::NonZeroUsize;
use std::sync::{Arc, Mutex};

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ReplicaCursorKey {
    pub source: ReplicaId,
    pub stream: JournalStream,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReplicaCursorAdvance {
    Advanced(JournalCursor),
    AlreadyAdvanced(JournalCursor),
    Conflict(JournalCursor),
}

/// Cursor advancement is separate from the journal append authority. A
/// failed cursor write leaves the imported events deduplicable on retry.
pub trait ReplicaCursorStore: Send + Sync + 'static {
    type Error: Send + 'static;
    fn load(&self, key: ReplicaCursorKey) -> Effect<JournalCursor, Self::Error>;
    fn advance(
        &self,
        key: ReplicaCursorKey,
        expected: JournalCursor,
        next: JournalCursor,
    ) -> Effect<ReplicaCursorAdvance, Self::Error>;
}

pub trait HasReplicaCursorStore {
    type Store: ReplicaCursorStore;
    fn replica_cursor_store(&self) -> Arc<Self::Store>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MemoryReplicaCursorError {
    Capacity,
    Regression,
}

pub struct MemoryReplicaCursorStore {
    state: Arc<Mutex<BTreeMap<ReplicaCursorKey, JournalCursor>>>,
    capacity: NonZeroUsize,
}

impl MemoryReplicaCursorStore {
    #[must_use]
    pub fn new(capacity: NonZeroUsize) -> Self {
        Self {
            state: Arc::new(Mutex::new(BTreeMap::new())),
            capacity,
        }
    }
}

impl ReplicaCursorStore for MemoryReplicaCursorStore {
    type Error = MemoryReplicaCursorError;

    fn load(&self, key: ReplicaCursorKey) -> Effect<JournalCursor, Self::Error> {
        let state = Arc::clone(&self.state);
        Effect::sync(move || {
            state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .get(&key)
                .copied()
                .unwrap_or(JournalCursor::new(0))
        })
    }

    fn advance(
        &self,
        key: ReplicaCursorKey,
        expected: JournalCursor,
        next: JournalCursor,
    ) -> Effect<ReplicaCursorAdvance, Self::Error> {
        let state = Arc::clone(&self.state);
        let capacity = self.capacity;
        Effect::try_sync(
            move || {
                if next.value() < expected.value() {
                    return Err(MemoryReplicaCursorError::Regression);
                }
                let mut state = state
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                let current = state.get(&key).copied().unwrap_or(JournalCursor::new(0));
                if current == next {
                    return Ok(ReplicaCursorAdvance::AlreadyAdvanced(current));
                }
                if current != expected {
                    return Ok(ReplicaCursorAdvance::Conflict(current));
                }
                if !state.contains_key(&key) && state.len() >= capacity.get() {
                    return Err(MemoryReplicaCursorError::Capacity);
                }
                state.insert(key.clone(), next);
                Ok(ReplicaCursorAdvance::Advanced(next))
            },
            std::convert::identity,
        )
    }
}
