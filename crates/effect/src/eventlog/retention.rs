//! Compaction permissions and history-retention policies.

use super::{JournalCursor, JournalStream};
use crate::{KeyDomain, StableKey};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
/// Stable identity used by eventlog retention operations.
pub struct RetentionConsumerId(Arc<[u8]>);

impl RetentionConsumerId {
    #[must_use]
    /// Wrap a typed stable key while preserving its identity bytes.
    ///
    /// # Type parameters
    /// - `D`: Identity-domain type. Bounds: `KeyDomain`.
    ///
    /// # Parameters
    /// - `key` (`StableKey<D>`): Stable key used to locate or identify the selected value.
    ///
    /// # Returns
    /// Type: `Self`.
    pub fn from_stable<D: KeyDomain>(key: StableKey<D>) -> Self {
        Self(key.into_bytes())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// The replay policy that determines which journal history must remain available.
pub enum ReplayRetention {
    /// The state was reconstructed from a recorded snapshot. Contains `JournalCursor`.
    Snapshot(JournalCursor),
    /// Discard eligible retained history after authorization.
    DiscardHistory,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// A missing authority, unsafe boundary, or invalid compaction request.
pub enum RetentionError {
    /// The proposed consumer moves backward from recorded state.
    ConsumerRegression,
    /// The proposed snapshot moves backward from recorded state.
    SnapshotRegression,
    /// The required replay policy is missing.
    MissingReplayPolicy,
    /// A required consumer has not advanced beyond the proposed compaction boundary.
    ConsumerBehind,
    /// The replay cursor precedes retained journal history.
    ReplayBehind,
}

#[derive(Default)]
/// Typed retention state used by eventlog retention.
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
    ///
    /// # Returns
    /// Type: `Self`.
    fn default() -> Self {
        Self::new()
    }
}

impl MemoryRetentionAuthority {
    #[must_use]
    /// Construct a `MemoryRetentionAuthority` from its required inputs and explicit policies.
    ///
    /// # Returns
    /// Type: `Self`.
    pub fn new() -> Self {
        Self {
            state: Arc::new(Mutex::new(RetentionState::default())),
        }
    }

    /// Register the supplied definition, rejecting an identity or compatibility conflict.
    ///
    /// # Parameters
    /// - `stream` (`JournalStream`): Scoped source of ordered chunks or events.
    /// - `consumer` (`RetentionConsumerId`): The consumer supplied to this operation.
    /// - `checkpoint` (`JournalCursor`): The checkpoint supplied to this operation.
    ///
    /// # Returns
    /// Type: `Result<(), RetentionError>`.
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

    /// Return the retention rule required by registered replay consumers.
    ///
    /// # Parameters
    /// - `stream` (`JournalStream`): Scoped source of ordered chunks or events.
    /// - `policy` (`ReplayRetention`): The caller-supplied behavior policy.
    ///
    /// # Returns
    /// Type: `Result<(), RetentionError>`.
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
    /// Issue a compaction permit only when required consumers can still replay.
    ///
    /// # Parameters
    /// - `stream` (`JournalStream`): Scoped source of ordered chunks or events.
    /// - `through` (`JournalCursor`): Last cursor or sequence included in this bounded operation.
    ///
    /// # Returns
    /// Type: `CompactionPermit`.
    pub fn permit(&self, stream: JournalStream, through: JournalCursor) -> CompactionPermit {
        CompactionPermit {
            state: Arc::clone(&self.state),
            stream,
            through,
        }
    }
}

/// An authority-issued permission to compact journal history through one boundary.
pub struct CompactionPermit {
    pub(crate) state: Arc<Mutex<RetentionState>>,
    pub(crate) stream: JournalStream,
    pub(crate) through: JournalCursor,
}

impl RetentionState {
    /// # Parameters
    /// - `stream` (`&JournalStream`): Scoped source of ordered chunks or events.
    /// - `through` (`JournalCursor`): Last cursor or sequence included in this bounded operation.
    ///
    /// # Returns
    /// Type: `Result<(), RetentionError>`.
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
