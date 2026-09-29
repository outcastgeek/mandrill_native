//! Replica sequence cursors and resume positions.

use super::{JournalCursor, JournalStream, ReplicaId};
use crate::Effect;
use std::collections::BTreeMap;
use std::num::NonZeroUsize;
use std::sync::{Arc, Mutex};

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
/// Stable identity used by eventlog replica cursor operations.
pub struct ReplicaCursorKey {
    /// The underlying failure or provider cause. Type: `ReplicaId`.
    pub source: ReplicaId,
    /// Stable identity of the ordered stream. Type: `JournalStream`.
    pub stream: JournalStream,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// The atomic result of moving a replica’s recorded cursor forward.
pub enum ReplicaCursorAdvance {
    /// The recorded cursor moved forward. Contains `JournalCursor`.
    Advanced(JournalCursor),
    /// The recorded cursor is already at or beyond this position. Contains `JournalCursor`.
    AlreadyAdvanced(JournalCursor),
    /// The requested state conflicts with the recorded state. Carries `JournalCursor`.
    Conflict(JournalCursor),
}

/// Cursor advancement is separate from the journal append authority. A
/// failed cursor write leaves the imported events deduplicable on retry.
pub trait ReplicaCursorStore: Send + Sync + 'static {
    /// Provider-specific failure type returned by this capability.
    type Error: Send + 'static;
    /// # Parameters
    /// - `key` (`ReplicaCursorKey`): Stable key used to locate or identify the selected value.
    ///
    /// # Returns
    /// Type: `Effect<JournalCursor, Self::Error>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    fn load(&self, key: ReplicaCursorKey) -> Effect<JournalCursor, Self::Error>;
    /// # Parameters
    /// - `key` (`ReplicaCursorKey`): Stable key used to locate or identify the selected value.
    /// - `expected` (`JournalCursor`): Value or revision required for the proposed state transition.
    /// - `next` (`JournalCursor`): Next value or state produced by the transition.
    ///
    /// # Returns
    /// Type: `Effect<ReplicaCursorAdvance, Self::Error>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    fn advance(
        &self,
        key: ReplicaCursorKey,
        expected: JournalCursor,
        next: JournalCursor,
    ) -> Effect<ReplicaCursorAdvance, Self::Error>;
}

/// Selects the persistent cursor store for replica progress.
pub trait HasReplicaCursorStore {
    /// Store selected by this environment; its trait bound defines the required guarantees.
    type Store: ReplicaCursorStore;
    /// # Returns
    /// Type: `Arc<Self::Store>`.
    fn replica_cursor_store(&self) -> Arc<Self::Store>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// A capacity or regression failure in the process-local replica cursor store.
pub enum MemoryReplicaCursorError {
    /// The configured capacity cannot admit this operation.
    Capacity,
    /// The proposed cursor moves backward from recorded state.
    Regression,
}

/// Provider-backed state for eventlog replica cursor.
pub struct MemoryReplicaCursorStore {
    state: Arc<Mutex<BTreeMap<ReplicaCursorKey, JournalCursor>>>,
    capacity: NonZeroUsize,
}

impl MemoryReplicaCursorStore {
    #[must_use]
    /// Construct a `MemoryReplicaCursorStore` from its required inputs and explicit policies.
    ///
    /// # Parameters
    /// - `capacity` (`NonZeroUsize`): The caller-supplied capacity bound.
    ///
    /// # Returns
    /// Type: `Self`.
    pub fn new(capacity: NonZeroUsize) -> Self {
        Self {
            state: Arc::new(Mutex::new(BTreeMap::new())),
            capacity,
        }
    }
}

impl ReplicaCursorStore for MemoryReplicaCursorStore {
    /// Type alias for error in eventlog replica cursor.
    type Error = MemoryReplicaCursorError;

    ///
    /// # Parameters
    /// - `key` (`ReplicaCursorKey`): Stable key used to locate or identify the selected value.
    ///
    /// # Returns
    /// Type: `Effect<JournalCursor, Self::Error>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
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

    ///
    /// # Parameters
    /// - `key` (`ReplicaCursorKey`): Stable key used to locate or identify the selected value.
    /// - `expected` (`JournalCursor`): Value or revision required for the proposed state transition.
    /// - `next` (`JournalCursor`): Next value or state produced by the transition.
    ///
    /// # Returns
    /// Type: `Effect<ReplicaCursorAdvance, Self::Error>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
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
