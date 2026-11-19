//! Regional invalidation publication and revision tracking.

use super::InvalidationKey;
use crate::memory::{
    Bytes, FixedVector, MemoryError, PreparedMemory, RegionId, SharedSlot, SlotPool,
};
use crate::{
    CallbackOverflow, PreparedChunkPool, PreparedChunkValues, PreparedPanicPool, PreparedPubSub,
    PreparedPubSubCapacity, PreparedPubSubError, PreparedSubscription, PubSubOptions,
    PubSubOverflow, PubSubShutdown, PublishReport, ReplayPolicy,
};
use std::alloc::Layout;
use std::num::{NonZeroU64, NonZeroUsize};
use std::sync::{Mutex, PoisonError};

/// Capacity and delivery policy for selected invalidation.
/// Lagging subscribers must resnapshot their dependencies.
#[derive(Clone, Copy, Debug)]
pub struct PreparedInvalidationCapacity {
    /// Distinct keys whose last committed revisions can be remembered.
    pub keys: NonZeroUsize,
    /// Distinct keys in a single publication after deduplication.
    pub keys_per_batch: NonZeroUsize,
    /// Maximum length of each stable identity retained from the caller.
    pub bytes_per_key: Bytes,
    /// Physical publication and subscriber capacity.
    pub delivery: PreparedPubSubCapacity,
    /// Per-subscriber logical queue length.
    pub buffer: NonZeroUsize,
    /// Queue policy on lag. Publication never waits while holding graph state.
    pub overflow: CallbackOverflow,
    /// Committed batches offered to new subscribers.
    pub replay: ReplayPolicy,
}

impl PreparedInvalidationCapacity {
    fn options(self) -> PubSubOptions {
        PubSubOptions {
            capacity: self.buffer,
            overflow: match self.overflow {
                CallbackOverflow::DropNewest => PubSubOverflow::DropNewest,
                CallbackOverflow::DropOldest => PubSubOverflow::DropOldest,
            },
            replay: self.replay,
        }
    }

    fn batches(self) -> Result<NonZeroUsize, MemoryError> {
        NonZeroUsize::new(self.delivery.payloads).ok_or(MemoryError::InvalidLayout)
    }

    /// Region bytes for revision records, fanout and retained batches. Caller
    /// key pointees, panic storage and allocator overhead are separate.
    pub fn storage_bytes(self) -> Result<Bytes, PreparedInvalidationError> {
        let revisions = Layout::array::<KeyRevision>(self.keys.get())
            .map_err(|_| MemoryError::InvalidLayout)?;
        let delivery = PreparedPubSub::<PreparedInvalidationBatch>::demand(
            self.options(),
            self.delivery,
            false,
        )?;
        Ok(SlotPool::<Mutex<RevisionState>>::storage_bytes(1)?
            .checked_add(Bytes::new(revisions.size()))?
            .checked_add(PreparedChunkPool::<InvalidationKey>::storage_bytes(
                self.batches()?,
                self.keys_per_batch,
            )?)?
            .checked_add(delivery.region_bytes)?)
    }
}

/// Refusal before graph mutation and invalidation publication.
#[derive(Debug)]
pub enum PreparedInvalidationError {
    /// Physical or configured identity capacity.
    Memory(MemoryError),
    /// Prepared delivery policy or lifecycle failure.
    Delivery(crate::PubSubError),
    /// All representable revision numbers have been used.
    RevisionExhausted,
}

impl From<MemoryError> for PreparedInvalidationError {
    fn from(error: MemoryError) -> Self {
        Self::Memory(error)
    }
}

impl From<PreparedPubSubError> for PreparedInvalidationError {
    fn from(error: PreparedPubSubError) -> Self {
        match error {
            PreparedPubSubError::Memory(error) => Self::Memory(error),
            PreparedPubSubError::Policy(error) => Self::Delivery(error),
        }
    }
}

/// One committed revision and its region-backed, deduplicated key list.
pub struct PreparedInvalidationBatch {
    /// Revision visible to dependency snapshots before subscribers wake.
    pub revision: NonZeroU64,
    keys: PreparedChunkValues<InvalidationKey>,
}

impl PreparedInvalidationBatch {
    /// Stable keys in sorted order. The delivery owner retains the batch slot.
    #[must_use]
    pub fn keys(&self) -> &[InvalidationKey] {
        self.keys.as_slice()
    }
}

struct KeyRevision {
    key: InvalidationKey,
    revision: NonZeroU64,
}

struct RevisionState {
    next: Option<NonZeroU64>,
    keys: FixedVector<'static, KeyRevision>,
}

/// Prepared invalidation hub and its bounded per-key committed revision index.
pub struct PreparedInvalidationRegistry {
    hub: PreparedPubSub<PreparedInvalidationBatch>,
    state: SharedSlot<'static, Mutex<RevisionState>>,
    batches: PreparedChunkPool<InvalidationKey>,
    bytes_per_key: Bytes,
}

impl PreparedInvalidationRegistry {
    /// Prepare physical revision, batch and delivery storage before sealing.
    pub fn prepare(
        memory: &PreparedMemory<'static>,
        region: RegionId,
        capacity: PreparedInvalidationCapacity,
        panics: &PreparedPanicPool,
    ) -> Result<Self, PreparedInvalidationError> {
        capacity.storage_bytes()?;
        let states = memory.slots(region, 1, |_| {
            Ok(Mutex::new(RevisionState {
                next: Some(NonZeroU64::MIN),
                keys: FixedVector::prepare(memory.allocator(region)?, capacity.keys.get())?,
            }))
        })?;
        let state = states.checkout()?.share();
        let batches = PreparedChunkPool::prepare(
            memory,
            region,
            capacity.batches()?,
            capacity.keys_per_batch,
        )?;
        let hub = PreparedPubSub::prepare(
            memory,
            region,
            capacity.options(),
            capacity.delivery,
            false,
            panics,
        )?;
        Ok(Self {
            hub,
            state,
            batches,
            bytes_per_key: capacity.bytes_per_key,
        })
    }

    /// Fallibly retain the same revision and delivery authority.
    pub fn try_clone(&self) -> Result<Self, MemoryError> {
        Ok(Self {
            hub: self.hub.try_clone()?,
            state: self.state.try_clone()?,
            batches: self.batches.try_clone()?,
            bytes_per_key: self.bytes_per_key,
        })
    }

    /// Register for future changes and the configured replay.
    pub fn subscribe(
        &self,
    ) -> Result<PreparedSubscription<PreparedInvalidationBatch>, PreparedInvalidationError> {
        Ok(self.hub.subscribe()?)
    }

    /// Snapshot the highest revision for these dependencies in one gate.
    #[must_use]
    pub fn revision_of(&self, keys: &[InvalidationKey]) -> Option<NonZeroU64> {
        let state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state
            .keys
            .as_slice()
            .iter()
            .filter_map(|entry| keys.contains(&entry.key).then_some(entry.revision))
            .max()
    }

    /// Publish one batch and update all of its remembered revisions together.
    /// An empty slice is a no-op and consumes no revision.
    pub fn invalidate(
        &self,
        keys: &[InvalidationKey],
    ) -> Result<Option<PublishReport>, PreparedInvalidationError> {
        self.publish_guarded(keys.iter().cloned(), || {})
    }

    /// Stop accepting publications and settle existing deliveries.
    pub fn shutdown_now(&self, mode: PubSubShutdown) -> Result<bool, MemoryError> {
        self.hub.shutdown_now(mode)
    }

    // Trusted callers preflight graph mutation and move its guard into commit.
    // The commit releases that guard before subscriber notification.
    pub(crate) fn publish_guarded(
        &self,
        keys: impl IntoIterator<Item = InvalidationKey>,
        commit: impl FnOnce(),
    ) -> Result<Option<PublishReport>, PreparedInvalidationError> {
        let mut keys = keys.into_iter();
        let Some(first) = keys.next() else {
            commit();
            return Ok(None);
        };
        let mut batch = self.batches.try_checkout()?;
        for key in std::iter::once(first).chain(keys) {
            let requested = Bytes::new(key.as_bytes().len());
            if requested > self.bytes_per_key {
                return Err(MemoryError::ValueTooLarge {
                    requested,
                    capacity: self.bytes_per_key,
                }
                .into());
            }
            if !batch.as_slice().contains(&key) {
                batch
                    .try_push(key)
                    .map_err(|_| MemoryError::CollectionReservationFailed)?;
            }
        }
        batch.as_mut_slice().sort_unstable();

        let publication = self.hub.admit_publication_mode(false)?;
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        let revision = state
            .next
            .ok_or(PreparedInvalidationError::RevisionExhausted)?;
        let missing = batch
            .as_slice()
            .iter()
            .filter(|key| !state.keys.as_slice().iter().any(|entry| entry.key == **key))
            .count();
        if missing > state.keys.capacity() - state.keys.len() {
            return Err(MemoryError::CollectionReservationFailed.into());
        }
        let report = publication
            .publish_committed(
                PreparedInvalidationBatch {
                    revision,
                    keys: batch,
                },
                move |batch| {
                    state.next = revision.get().checked_add(1).and_then(NonZeroU64::new);
                    for key in batch.keys() {
                        if let Some(entry) = state
                            .keys
                            .as_mut_slice()
                            .iter_mut()
                            .find(|entry| &entry.key == key)
                        {
                            entry.revision = revision;
                        } else {
                            state.keys.push_reserved(KeyRevision {
                                key: key.clone(),
                                revision,
                            });
                        }
                    }
                    commit();
                    drop(state);
                },
            )
            .map_err(PreparedInvalidationError::Delivery)?;
        Ok(Some(report))
    }
}
