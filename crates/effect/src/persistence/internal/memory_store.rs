//! Volatile key-value store operations.

use super::super::{
    BatchReadItem, CompareExchange, ConditionalKeyValueStore, KeyValueStore, MemoryStoreError,
    StoreCapacity, StoreEntry, StoreRead, StoreRevision, StoredValue, UtcMillis,
};
use crate::{Effect, Exit};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

/// Typed state used by persistence internal memory store.
struct State {
    entries: BTreeMap<Arc<[u8]>, StoreEntry>,
    next_revision: u64,
    capacity: StoreCapacity,
}

/// Executor-independent in-memory encoded store. All per-key conditional
/// operations linearize under one short gate; callbacks never run under it.
/// State does not survive process loss or construction of a new handle.
pub struct MemoryKeyValueStore {
    state: Arc<Mutex<State>>,
}

impl Clone for MemoryKeyValueStore {
    ///
    /// # Returns
    /// Type: `Self`.
    fn clone(&self) -> Self {
        Self {
            state: Arc::clone(&self.state),
        }
    }
}

impl MemoryKeyValueStore {
    #[must_use]
    /// Construct a `MemoryKeyValueStore` from its required inputs and explicit policies.
    ///
    /// # Parameters
    /// - `capacity` (`StoreCapacity`): The caller-supplied capacity bound.
    ///
    /// # Returns
    /// Type: `Self`.
    pub fn new(capacity: StoreCapacity) -> Self {
        Self {
            state: Arc::new(Mutex::new(State {
                entries: BTreeMap::new(),
                next_revision: 0,
                capacity,
            })),
        }
    }

    ///
    /// # Parameters
    /// - `entry` (`&StoreEntry`): Stored record or event selected for this transition.
    /// - `now` (`UtcMillis`): The caller-supplied current time.
    ///
    /// # Returns
    /// Type: `bool`.
    /// Returns whether the requested condition holds.
    fn expired(entry: &StoreEntry, now: UtcMillis) -> bool {
        entry
            .value
            .expires_at
            .is_some_and(|deadline| now >= deadline)
    }

    ///
    /// # Parameters
    /// - `state` (`&mut State`): Current or replacement state owned by this operation.
    /// - `now` (`UtcMillis`): The caller-supplied current time.
    ///
    /// # Returns
    /// Type: `Vec<StoreEntry>`.
    /// Returns `Vec<StoreEntry>`.
    fn remove_expired(state: &mut State, now: UtcMillis) -> Vec<StoreEntry> {
        let expired: Vec<_> = state
            .entries
            .iter()
            .filter(|(_, entry)| Self::expired(entry, now))
            .map(|(key, _)| Arc::clone(key))
            .collect();
        expired
            .into_iter()
            .filter_map(|key| state.entries.remove(&key))
            .collect()
    }

    ///
    /// # Parameters
    /// - `state` (`&mut State`): Current or replacement state owned by this operation.
    ///
    /// # Returns
    /// Type: `Result<StoreRevision, MemoryStoreError>`.
    fn next_revision(state: &mut State) -> Result<StoreRevision, MemoryStoreError> {
        let next = state
            .next_revision
            .checked_add(1)
            .ok_or(MemoryStoreError::RevisionOverflow)?;
        state.next_revision = next;
        Ok(StoreRevision(next))
    }

    ///
    /// # Parameters
    /// - `state` (`&State`): Current or replacement state owned by this operation.
    /// - `key` (`&[u8]`): Stable key used to locate or identify the selected value.
    ///
    /// # Returns
    /// Type: `bool`.
    /// Returns whether the requested condition holds.
    fn fits(state: &State, key: &[u8]) -> bool {
        state.entries.contains_key(key)
            || match state.capacity {
                StoreCapacity::Unbounded => true,
                StoreCapacity::Bounded(limit) => state.entries.len() < limit.get(),
            }
    }
}

impl KeyValueStore for MemoryKeyValueStore {
    /// Type alias for error in persistence internal memory store.
    type Error = MemoryStoreError;

    ///
    /// # Parameters
    /// - `key` (`Arc<[u8]>`): Stable key used to locate or identify the selected value.
    /// - `now` (`UtcMillis`): The caller-supplied current time.
    ///
    /// # Returns
    /// Type: `Effect<StoreRead, Self::Error>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    fn get(&self, key: Arc<[u8]>, now: UtcMillis) -> Effect<StoreRead, Self::Error> {
        let store = self.clone();
        Effect::sync(move || {
            let state = store
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            match state.entries.get(&key) {
                None => StoreRead::Missing,
                Some(entry) if Self::expired(entry, now) => StoreRead::Expired(entry.clone()),
                Some(entry) => StoreRead::Found(entry.clone()),
            }
        })
    }

    ///
    /// # Parameters
    /// - `keys` (`Vec<Arc<[u8]>>`): The keys supplied to this operation.
    /// - `now` (`UtcMillis`): The caller-supplied current time.
    ///
    /// # Returns
    /// Type: `Effect<Vec<BatchReadItem<Self::Error>>, Self::Error>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    fn get_many(
        &self,
        keys: Vec<Arc<[u8]>>,
        now: UtcMillis,
    ) -> Effect<Vec<BatchReadItem<Self::Error>>, Self::Error> {
        let store = self.clone();
        Effect::sync_once(move || {
            let state = store
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            keys.into_iter()
                .map(|key| {
                    let result = match state.entries.get(&key) {
                        None => StoreRead::Missing,
                        Some(entry) if Self::expired(entry, now) => {
                            StoreRead::Expired(entry.clone())
                        }
                        Some(entry) => StoreRead::Found(entry.clone()),
                    };
                    BatchReadItem {
                        key,
                        result: Ok(result),
                    }
                })
                .collect()
        })
    }

    ///
    /// # Parameters
    /// - `key` (`Arc<[u8]>`): Stable key used to locate or identify the selected value.
    /// - `value` (`StoredValue`): Value read, transformed, encoded, or stored by this operation.
    ///
    /// # Returns
    /// Type: `Effect<StoreRevision, Self::Error>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    fn put(&self, key: Arc<[u8]>, value: StoredValue) -> Effect<StoreRevision, Self::Error> {
        let store = self.clone();
        let value = Mutex::new(Some(value));
        Effect::ready(move |_, _| {
            let value = value
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take();
            let Some(value) = value else {
                return Exit::Fail(MemoryStoreError::InputConsumed);
            };
            let (result, removed) = {
                let mut state = store
                    .state
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                let mut removed = Vec::new();
                let result = if !Self::fits(&state, &key) {
                    Exit::Fail(MemoryStoreError::CapacityExceeded)
                } else {
                    match Self::next_revision(&mut state) {
                        Ok(revision) => {
                            if let Some(previous) = state
                                .entries
                                .insert(Arc::clone(&key), StoreEntry { value, revision })
                            {
                                removed.push(previous);
                            }
                            Exit::Success(revision)
                        }
                        Err(error) => Exit::Fail(error),
                    }
                };
                (result, removed)
            };
            drop(removed);
            result
        })
    }

    ///
    /// # Parameters
    /// - `key` (`Arc<[u8]>`): Stable key used to locate or identify the selected value.
    ///
    /// # Returns
    /// Type: `Effect<Option<StoreEntry>, Self::Error>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    fn remove(&self, key: Arc<[u8]>) -> Effect<Option<StoreEntry>, Self::Error> {
        let store = self.clone();
        Effect::sync(move || {
            store
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .entries
                .remove(&key)
        })
    }

    ///
    /// # Parameters
    /// - `now` (`UtcMillis`): The caller-supplied current time.
    ///
    /// # Returns
    /// Type: `Effect<usize, Self::Error>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    fn cleanup_expired(&self, now: UtcMillis) -> Effect<usize, Self::Error> {
        let store = self.clone();
        Effect::sync(move || {
            let removed = {
                let mut state = store
                    .state
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                Self::remove_expired(&mut state, now)
            };
            let count = removed.len();
            drop(removed);
            count
        })
    }
}

impl ConditionalKeyValueStore for MemoryKeyValueStore {
    ///
    /// # Parameters
    /// - `key` (`Arc<[u8]>`): Stable key used to locate or identify the selected value.
    /// - `expected` (`Option<StoreRevision>`): Value or revision required for the proposed state transition.
    /// - `replacement` (`Option<StoredValue>`): The replacement supplied to this operation.
    ///
    /// # Returns
    /// Type: `Effect<CompareExchange, Self::Error>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    fn compare_exchange(
        &self,
        key: Arc<[u8]>,
        expected: Option<StoreRevision>,
        replacement: Option<StoredValue>,
    ) -> Effect<CompareExchange, Self::Error> {
        let store = self.clone();
        let replacement = Mutex::new(Some(replacement));
        Effect::ready(move |_, _| {
            let replacement = replacement
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take();
            let Some(replacement) = replacement else {
                return Exit::Fail(MemoryStoreError::InputConsumed);
            };
            let (result, removed) = {
                let mut state = store
                    .state
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                let mut removed = Vec::new();
                let actual = state.entries.get(&key).map(|entry| entry.revision);
                let result = if actual != expected {
                    Exit::Success(CompareExchange::Conflict { actual })
                } else if replacement.is_some() && !Self::fits(&state, &key) {
                    Exit::Fail(MemoryStoreError::CapacityExceeded)
                } else {
                    match Self::next_revision(&mut state) {
                        Err(error) => Exit::Fail(error),
                        Ok(revision) => {
                            let previous = match replacement {
                                Some(value) => state
                                    .entries
                                    .insert(Arc::clone(&key), StoreEntry { value, revision }),
                                None => state.entries.remove(&key),
                            };
                            removed.extend(previous);
                            Exit::Success(CompareExchange::Written(revision))
                        }
                    }
                };
                (result, removed)
            };
            drop(removed);
            result
        })
    }
}
