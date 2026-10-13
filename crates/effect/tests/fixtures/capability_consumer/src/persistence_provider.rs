
use effect::effect_sync;
use effect::effect_try_sync;
use effect::Effect;
use effect::persistence::{
    BatchReadItem, KeyValueStore, StoreEntry, StoreRead, StoreRevision, StoredValue, UtcMillis,
};
use std::collections::BTreeMap;
use std::num::NonZeroUsize;
use std::sync::{Arc, Mutex};

#[derive(Debug)]
pub enum StoreError {
    Capacity,
    RevisionExhausted,
}

struct State {
    entries: BTreeMap<Arc<[u8]>, StoreEntry>,
    next_revision: u64,
}

/// A consumer-authored KeyValueStore. It intentionally implements no claim,
/// compare-and-set or durability contract through the ordinary store trait.
pub struct ExternalKeyValueStore {
    state: Arc<Mutex<State>>,
    capacity: NonZeroUsize,
}

impl ExternalKeyValueStore {
    pub fn new(capacity: NonZeroUsize) -> Self {
        Self {
            state: Arc::new(Mutex::new(State {
                entries: BTreeMap::new(),
                next_revision: 0,
            })),
            capacity,
        }
    }

    fn classify(entry: Option<&StoreEntry>, now: UtcMillis) -> StoreRead {
        match entry {
            None => StoreRead::Missing,
            Some(entry) if entry.value.expires_at.is_some_and(|at| now >= at) => {
                StoreRead::Expired(entry.clone())
            }
            Some(entry) => StoreRead::Found(entry.clone()),
        }
    }
}

impl KeyValueStore for ExternalKeyValueStore {
    type Error = StoreError;

    fn get(&self, key: Arc<[u8]>, now: UtcMillis) -> Effect<StoreRead, Self::Error> {
        let state = Arc::clone(&self.state);
        effect_sync! { once; body: move || {
            let state = state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            Self::classify(state.entries.get(&key), now)
        }, }
    }

    fn get_many(
        &self,
        keys: Vec<Arc<[u8]>>,
        now: UtcMillis,
    ) -> Effect<Vec<BatchReadItem<Self::Error>>, Self::Error> {
        let state = Arc::clone(&self.state);
        effect_sync! { once; body: move || {
            let state = state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            keys.into_iter()
                .map(|key| BatchReadItem {
                    result: Ok(Self::classify(state.entries.get(&key), now)),
                    key,
                })
                .collect()
        }, }
    }

    fn put(&self, key: Arc<[u8]>, value: StoredValue) -> Effect<StoreRevision, Self::Error> {
        let state = Arc::clone(&self.state);
        let capacity = self.capacity;
        effect_try_sync! { once; body: move || {
                let (revision, removed) = {
                    let mut state = state
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    if !state.entries.contains_key(&key) && state.entries.len() >= capacity.get() {
                        return Err(StoreError::Capacity);
                    }
                    let next = state
                        .next_revision
                        .checked_add(1)
                        .ok_or(StoreError::RevisionExhausted)?;
                    let revision = StoreRevision::from_recorded(next)
                        .map_err(|_| StoreError::RevisionExhausted)?;
                    state.next_revision = next;
                    let removed = state.entries.insert(key, StoreEntry { value, revision });
                    (revision, removed)
                };
                drop(removed);
                Ok(revision)
            }, on_error: std::convert::identity, }
    }

    fn remove(&self, key: Arc<[u8]>) -> Effect<Option<StoreEntry>, Self::Error> {
        let state = Arc::clone(&self.state);
        effect_sync! { once; body: move || {
            state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .entries
                .remove(&key)
        }, }
    }

    fn cleanup_expired(&self, now: UtcMillis) -> Effect<usize, Self::Error> {
        let state = Arc::clone(&self.state);
        effect_sync! { once; body: move || {
            let expired = {
                let mut state = state
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                let keys = state
                    .entries
                    .iter()
                    .filter(|(_, entry)| entry.value.expires_at.is_some_and(|at| now >= at))
                    .map(|(key, _)| Arc::clone(key))
                    .collect::<Vec<_>>();
                keys.into_iter()
                    .filter_map(|key| state.entries.remove(&key))
                    .collect::<Vec<_>>()
            };
            let count = expired.len();
            drop(expired);
            count
        }, }
    }
}
