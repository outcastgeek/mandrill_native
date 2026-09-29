use super::{
    AtomicProjectionStore, JournalCursor, ProjectionKey, ProjectionSnapshot, ProjectionWrite,
};
use crate::Effect;
use std::collections::BTreeMap;
use std::num::NonZeroUsize;
use std::sync::{Arc, Mutex};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MemoryProjectionError {
    Capacity,
    InputConsumed,
    DuplicateSnapshot,
    KeyMismatch,
    CheckpointRegression,
}

/// Process-local atomic state/checkpoint store. Its single lock is the
/// compare-and-set linearization point; fold and journal I/O run outside it.
pub struct MemoryProjectionStore<S: Send + Sync + 'static> {
    state: Arc<Mutex<BTreeMap<ProjectionKey, ProjectionSnapshot<S>>>>,
    capacity: NonZeroUsize,
}

impl<S: Send + Sync + 'static> MemoryProjectionStore<S> {
    #[must_use]
    pub fn new(capacity: NonZeroUsize) -> Self {
        Self {
            state: Arc::new(Mutex::new(BTreeMap::new())),
            capacity,
        }
    }

    pub fn restore(
        capacity: NonZeroUsize,
        snapshots: impl IntoIterator<Item = ProjectionSnapshot<S>>,
    ) -> Result<Self, MemoryProjectionError> {
        let mut state = BTreeMap::new();
        for snapshot in snapshots {
            if state.insert(snapshot.key.clone(), snapshot).is_some() {
                return Err(MemoryProjectionError::DuplicateSnapshot);
            }
            if state.len() > capacity.get() {
                return Err(MemoryProjectionError::Capacity);
            }
        }
        Ok(Self {
            state: Arc::new(Mutex::new(state)),
            capacity,
        })
    }

    #[must_use]
    pub fn snapshots(&self) -> Vec<ProjectionSnapshot<S>> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .values()
            .cloned()
            .collect()
    }
}

impl<S: Send + Sync + 'static> AtomicProjectionStore<S> for MemoryProjectionStore<S> {
    type Error = MemoryProjectionError;

    fn load(&self, key: ProjectionKey) -> Effect<Option<ProjectionSnapshot<S>>, Self::Error> {
        let state = Arc::clone(&self.state);
        Effect::sync(move || {
            state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .get(&key)
                .cloned()
        })
    }

    fn compare_exchange(
        &self,
        key: ProjectionKey,
        expected: Option<JournalCursor>,
        next: ProjectionSnapshot<S>,
    ) -> Effect<ProjectionWrite<S>, Self::Error> {
        let state = Arc::clone(&self.state);
        let capacity = self.capacity;
        let next = Mutex::new(Some(next));
        Effect::try_sync(
            move || {
                let mut next = next
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                let proposed = next.as_ref().ok_or(MemoryProjectionError::InputConsumed)?;
                if proposed.key != key {
                    return Err(MemoryProjectionError::KeyMismatch);
                }
                let mut state = state
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                let existing = state.get(&key);
                if let Some(existing) = existing
                    && existing.version != proposed.version
                {
                    return Ok(ProjectionWrite::VersionConflict(existing.version));
                }
                if existing.map(|value| value.checkpoint) != expected {
                    return Ok(ProjectionWrite::Conflict(existing.cloned()));
                }
                if existing.is_none() && state.len() >= capacity.get() {
                    return Err(MemoryProjectionError::Capacity);
                }
                let next = next.take().ok_or(MemoryProjectionError::InputConsumed)?;
                if expected.is_some_and(|checkpoint| next.checkpoint < checkpoint) {
                    return Err(MemoryProjectionError::CheckpointRegression);
                }
                let retired = state.insert(key.clone(), next.clone());
                drop(state);
                drop(retired);
                Ok(ProjectionWrite::Committed(next))
            },
            std::convert::identity,
        )
    }
}
