use super::{JournalCursor, JournalStream};
use crate::{KeyDomain, StableKey};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RetentionConsumerId(Arc<[u8]>);

impl RetentionConsumerId {
    #[must_use]
    pub fn from_stable<D: KeyDomain>(key: StableKey<D>) -> Self {
        Self(key.into_bytes())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReplayRetention {
    Snapshot(JournalCursor),
    DiscardHistory,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RetentionError {
    ConsumerRegression,
    SnapshotRegression,
    MissingReplayPolicy,
    ConsumerBehind,
    ReplayBehind,
}

#[derive(Default)]
pub(crate) struct RetentionState {
    pub consumers: BTreeMap<JournalStream, BTreeMap<RetentionConsumerId, JournalCursor>>,
    pub replay: BTreeMap<JournalStream, ReplayRetention>,
}

/// Registration is authoritative for this memory journal's compaction gate.
/// Every projection and replica that needs old entries must register before
/// compaction; a future consumer starts from an acknowledged snapshot or gets
/// an explicit truncated-history result.
#[derive(Clone)]
pub struct MemoryRetentionAuthority {
    pub(crate) state: Arc<Mutex<RetentionState>>,
}

impl Default for MemoryRetentionAuthority {
    fn default() -> Self {
        Self::new()
    }
}

impl MemoryRetentionAuthority {
    #[must_use]
    pub fn new() -> Self {
        Self {
            state: Arc::new(Mutex::new(RetentionState::default())),
        }
    }

    pub fn register(
        &self,
        stream: JournalStream,
        consumer: RetentionConsumerId,
        checkpoint: JournalCursor,
    ) -> Result<(), RetentionError> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let registered = state.consumers.entry(stream).or_default();
        if registered
            .get(&consumer)
            .is_some_and(|prior| checkpoint < *prior)
        {
            return Err(RetentionError::ConsumerRegression);
        }
        registered.insert(consumer, checkpoint);
        Ok(())
    }

    pub fn replay_policy(
        &self,
        stream: JournalStream,
        policy: ReplayRetention,
    ) -> Result<(), RetentionError> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let (Some(ReplayRetention::Snapshot(previous)), ReplayRetention::Snapshot(next)) =
            (state.replay.get(&stream), policy)
            && next < *previous
        {
            return Err(RetentionError::SnapshotRegression);
        }
        state.replay.insert(stream, policy);
        Ok(())
    }

    #[must_use]
    pub fn permit(&self, stream: JournalStream, through: JournalCursor) -> CompactionPermit {
        CompactionPermit {
            state: Arc::clone(&self.state),
            stream,
            through,
        }
    }
}

pub struct CompactionPermit {
    pub(crate) state: Arc<Mutex<RetentionState>>,
    pub(crate) stream: JournalStream,
    pub(crate) through: JournalCursor,
}

impl RetentionState {
    pub(crate) fn validate(
        &self,
        stream: &JournalStream,
        through: JournalCursor,
    ) -> Result<(), RetentionError> {
        match self.replay.get(stream) {
            None => return Err(RetentionError::MissingReplayPolicy),
            Some(ReplayRetention::Snapshot(checkpoint)) if *checkpoint < through => {
                return Err(RetentionError::ReplayBehind);
            }
            _ => {}
        }
        if self
            .consumers
            .get(stream)
            .is_some_and(|consumers| consumers.values().any(|checkpoint| *checkpoint < through))
        {
            return Err(RetentionError::ConsumerBehind);
        }
        Ok(())
    }
}
