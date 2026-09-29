mod durable_result_store;
mod internal;
mod persistable;
mod persisted_cache;
mod persisted_queue;
mod rate_limiter;
mod result_store;

use crate::{Effect, KeyError, Layer, SchemaIssue};
use std::num::NonZeroUsize;
use std::sync::Arc;

pub use crate::duration::{TimeError, UtcMillis};

pub use durable_result_store::{
    compare_exchange_durable, read_durable, read_many_durable, remove_durable, write_durable,
};
pub use internal::memory_store::MemoryKeyValueStore;
pub use internal::queue_delivery::MemoryPersistedQueueStore;
pub use internal::rate_limit::MemoryRateLimitStore;
pub use persistable::{
    Persistable, PersistedRecordError, RecordPolicy, RecordVersion, RecordVersionError,
};
pub use persisted_cache::{
    CorruptCacheRecord, PersistedCache, PersistedCacheError, PersistedCacheMemory,
    PersistedCachePolicy,
};
pub use persisted_queue::{
    DecodedQueueClaim, DurablePersistedQueueStore, HasPersistedQueueStore, MemoryQueueStoreError,
    PersistedQueue, PersistedQueueDefinitionError, PersistedQueueError, PersistedQueueName,
    PersistedQueueStore, QueueClaim, QueueClaimError, QueueClaimOptions, QueueClaimToken,
    QueueDeadLetterReason, QueueDecodeFailure, QueueDisposition, QueueMessageId,
    QueueMessagePolicy, QueueOffer, QueueOfferOutcome, QueueOperation, QueueOwner, QueueRecord,
    QueueRecordState, QueueRequeue, QueueRequeueAttempts, QueueRequeueOutcome, QueueRetry,
    QueueScan, QueueSettlement, QueueSettlementAction, QueueStoreCapacity,
};
pub use rate_limiter::{
    AdaptiveFeedback, AdaptiveSignal, AdaptiveWindowPolicy, FeedbackOutcome, HasRateLimitStore,
    MemoryRateLimitError, RateLimitDecision, RateLimitDefinitionError, RateLimitError,
    RateLimitPeriod, RateLimitPolicy, RateLimitStore, RateLimitStoreCapacity, RateLimiter,
};
pub use result_store::{StoredLookup, compare_exchange, read, read_many, remove, write};

/// Revision zero means absent. Writes issue successive positive revisions;
/// exhaustion is a typed error, never wrapping or saturating silently.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct StoreRevision(u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StoreRevisionError {
    Zero,
}

impl StoreRevision {
    #[must_use]
    pub const fn value(self) -> u64 {
        self.0
    }

    pub const fn from_recorded(value: u64) -> Result<Self, StoreRevisionError> {
        if value == 0 {
            Err(StoreRevisionError::Zero)
        } else {
            Ok(Self(value))
        }
    }
}

/// Immutable encoded data. Cloning this handle shares its byte allocation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoredValue {
    pub bytes: Arc<[u8]>,
    pub expires_at: Option<UtcMillis>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoreEntry {
    pub value: StoredValue,
    pub revision: StoreRevision,
}

/// Expired and missing are observable, different store states. Expiry does not
/// erase authoritative bytes until an explicit removal or cleanup operation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StoreRead {
    Missing,
    Expired(StoreEntry),
    Found(StoreEntry),
}

/// Batch results retain both request order and the key used for each lookup.
pub struct BatchReadItem<E> {
    pub key: Arc<[u8]>,
    pub result: Result<StoreRead, E>,
}

/// Ordinary key/value operations carry no atomic-claim or durable guarantee.
/// Implementations may be used from an unrelated crate.
pub trait KeyValueStore: Send + Sync + 'static {
    type Error: Send + 'static;

    fn get(&self, key: Arc<[u8]>, now: UtcMillis) -> Effect<StoreRead, Self::Error>;

    /// A batch may contain per-key failures. A top-level failure means the
    /// batch itself was unavailable or could not be admitted.
    fn get_many(
        &self,
        keys: Vec<Arc<[u8]>>,
        now: UtcMillis,
    ) -> Effect<Vec<BatchReadItem<Self::Error>>, Self::Error>;

    fn put(&self, key: Arc<[u8]>, value: StoredValue) -> Effect<StoreRevision, Self::Error>;

    fn remove(&self, key: Arc<[u8]>) -> Effect<Option<StoreEntry>, Self::Error>;

    /// Explicit cleanup makes retention independent from the read/TTL check.
    fn cleanup_expired(&self, now: UtcMillis) -> Effect<usize, Self::Error>;
}

/// A separate capability with a linearizable per-key condition.
/// It does not imply multi-key atomicity, queue claims or physical durability.
pub trait ConditionalKeyValueStore: KeyValueStore {
    fn compare_exchange(
        &self,
        key: Arc<[u8]>,
        expected: Option<StoreRevision>,
        replacement: Option<StoredValue>,
    ) -> Effect<CompareExchange, Self::Error>;
}

/// An external provider explicitly warrants that an acknowledged write or
/// removal survives its documented restart boundary and later reads recover
/// the acknowledged state. No core memory store implements this trait.
pub trait DurableKeyValueStore: KeyValueStore {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompareExchange {
    Written(StoreRevision),
    Conflict { actual: Option<StoreRevision> },
}

/// The application environment chooses the store. No store registry or
/// provider type parameter is added to Effect itself.
pub trait HasKeyValueStore {
    type Store: KeyValueStore;
    fn key_value_store(&self) -> Arc<Self::Store>;
}

pub trait HasDurableKeyValueStore {
    type Store: DurableKeyValueStore;
    fn durable_key_value_store(&self) -> Arc<Self::Store>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StoreCapacity {
    Unbounded,
    Bounded(NonZeroUsize),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MemoryStoreError {
    CapacityExceeded,
    RevisionOverflow,
    InputConsumed,
}

impl std::fmt::Display for MemoryStoreError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(self, formatter)
    }
}

impl std::error::Error for MemoryStoreError {}

/// Identity of a mutation whose acknowledgement may be lost.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PersistenceMutation {
    Write,
    Remove,
    CompareExchange,
}

/// Distinct persistence failures preserve the provider's concrete error type.
#[derive(Debug)]
pub enum PersistenceError<Backend> {
    Identity(KeyError),
    Codec(SchemaIssue),
    Backend(Backend),
    Unavailable(Backend),
    Uncertain {
        operation: PersistenceMutation,
        key: Arc<[u8]>,
        source: Backend,
    },
    VersionMismatch,
    Corrupt(SchemaIssue),
    BatchMismatch,
    InvalidAcknowledgement {
        operation: PersistenceMutation,
        key: Arc<[u8]>,
    },
}

impl MemoryKeyValueStore {
    /// The memory service is explicitly volatile and scoped by the resulting
    /// handle. A caller chooses its capacity rather than receiving a hidden cap.
    #[must_use]
    pub fn layer(capacity: StoreCapacity) -> Layer<(), MemoryStoreError, Self> {
        Layer::make_with(move || Self::new(capacity))
    }
}
