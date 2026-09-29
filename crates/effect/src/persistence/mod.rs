//! Typed persistence capabilities and memory providers.
//!
//! Stable keys and schemas define what a record means; store traits define
//! what operations a provider can guarantee. A [`KeyValueStore`] does not
//! imply conditional writes, atomic queue claims, or crash durability.
//! [`PersistedQueue`] has separate ready, claimed, acknowledged, and dead-letter
//! states; settlement is fenced by an owner and claim generation. Memory
//! providers implement these contracts within one process, while durable
//! provider markers require the external implementation to uphold recovery.

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
/// An invalid or exhausted revision used for conditional store writes.
pub enum StoreRevisionError {
    /// A positive value is required here.
    Zero,
}

impl StoreRevision {
    #[must_use]
    /// # Returns
    /// Type: `u64`.
    pub const fn value(self) -> u64 {
        self.0
    }

    /// # Parameters
    /// - `value` (`u64`): Value read, transformed, encoded, or stored by this operation.
    ///
    /// # Returns
    /// Type: `Result<Self, StoreRevisionError>`.
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
    /// The owned encoded bytes. Type: `Arc<[u8]>`.
    pub bytes: Arc<[u8]>,
    /// The time at which the expires occurred. Type: `Option<UtcMillis>`.
    pub expires_at: Option<UtcMillis>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// A key, stored value, and revision read together from a key/value store.
pub struct StoreEntry {
    /// The value carried by this record. Type: `StoredValue`.
    pub value: StoredValue,
    /// The recorded storage revision. Type: `StoreRevision`.
    pub revision: StoreRevision,
}

/// Expired and missing are observable, different store states. Expiry does not
/// erase authoritative bytes until an explicit removal or cleanup operation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StoreRead {
    /// No matching value exists.
    Missing,
    /// The value exists but has passed its expiry instant. Carries `StoreEntry`.
    Expired(StoreEntry),
    /// A matching value was found. Carries `StoreEntry`.
    Found(StoreEntry),
}

/// Batch results retain both request order and the key used for each lookup.
///
/// # Type parameters
/// - `E`: Domain-error type.
pub struct BatchReadItem<E> {
    /// Key used to address the stored value. Type: `Arc<[u8]>`.
    pub key: Arc<[u8]>,
    /// The result produced by this operation. Type: `Result<StoreRead, E>`.
    pub result: Result<StoreRead, E>,
}

/// Ordinary key/value operations carry no atomic-claim or durable guarantee.
/// Implementations may be used from an unrelated crate.
pub trait KeyValueStore: Send + Sync + 'static {
    /// Provider-specific failure type returned by this capability.
    type Error: Send + 'static;

    /// # Parameters
    /// - `key` (`Arc<[u8]>`): Stable key used to locate or identify the selected value.
    /// - `now` (`UtcMillis`): The caller-supplied current time.
    ///
    /// # Returns
    /// Type: `Effect<StoreRead, Self::Error>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    fn get(&self, key: Arc<[u8]>, now: UtcMillis) -> Effect<StoreRead, Self::Error>;

    /// A batch may contain per-key failures. A top-level failure means the
    /// batch itself was unavailable or could not be admitted.
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
    ) -> Effect<Vec<BatchReadItem<Self::Error>>, Self::Error>;

    /// # Parameters
    /// - `key` (`Arc<[u8]>`): Stable key used to locate or identify the selected value.
    /// - `value` (`StoredValue`): Value read, transformed, encoded, or stored by this operation.
    ///
    /// # Returns
    /// Type: `Effect<StoreRevision, Self::Error>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    fn put(&self, key: Arc<[u8]>, value: StoredValue) -> Effect<StoreRevision, Self::Error>;

    /// # Parameters
    /// - `key` (`Arc<[u8]>`): Stable key used to locate or identify the selected value.
    ///
    /// # Returns
    /// Type: `Effect<Option<StoreEntry>, Self::Error>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    fn remove(&self, key: Arc<[u8]>) -> Effect<Option<StoreEntry>, Self::Error>;

    /// Explicit cleanup makes retention independent from the read/TTL check.
    ///
    /// # Parameters
    /// - `now` (`UtcMillis`): The caller-supplied current time.
    ///
    /// # Returns
    /// Type: `Effect<usize, Self::Error>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    fn cleanup_expired(&self, now: UtcMillis) -> Effect<usize, Self::Error>;
}

/// A separate capability with a linearizable per-key condition.
/// It does not imply multi-key atomicity, queue claims or physical durability.
pub trait ConditionalKeyValueStore: KeyValueStore {
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
    ) -> Effect<CompareExchange, Self::Error>;
}

/// An external provider explicitly warrants that an acknowledged write or
/// removal survives its documented restart boundary and later reads recover
/// the acknowledged state. No core memory store implements this trait.
pub trait DurableKeyValueStore: KeyValueStore {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// The atomic decision after comparing an expected revision with the stored one.
pub enum CompareExchange {
    /// The conditional write committed. Carries `StoreRevision`.
    Written(StoreRevision),
    /// The conditional write did not match the stored revision.
    Conflict {
        /// Revision currently stored, or absence. Type: `Option<StoreRevision>`.
        actual: Option<StoreRevision>,
    },
}

/// The application environment chooses the store. No store registry or
/// provider type parameter is added to Effect itself.
pub trait HasKeyValueStore {
    /// Store selected by this environment; its trait bound defines the required guarantees.
    type Store: KeyValueStore;
    /// # Returns
    /// Type: `Arc<Self::Store>`.
    fn key_value_store(&self) -> Arc<Self::Store>;
}

/// Selects a key-value provider that explicitly warrants acknowledged-state recovery.
pub trait HasDurableKeyValueStore {
    /// Store selected by this environment; its trait bound defines the required guarantees.
    type Store: DurableKeyValueStore;
    /// # Returns
    /// Type: `Arc<Self::Store>`.
    fn durable_key_value_store(&self) -> Arc<Self::Store>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// Explicit resource bounds for persistence mod.
pub enum StoreCapacity {
    /// No explicit capacity bound is imposed.
    Unbounded,
    /// The supplied positive capacity bound is enforced. Contains `NonZeroUsize`.
    Bounded(NonZeroUsize),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// A capacity or state failure in the process-local key/value store.
pub enum MemoryStoreError {
    /// The configured capacity bound was exceeded.
    CapacityExceeded,
    /// Computing revision exceeded its supported range.
    RevisionOverflow,
    /// An owned input was already consumed.
    InputConsumed,
}

impl std::fmt::Display for MemoryStoreError {
    ///
    /// # Parameters
    /// - `formatter` (`&mut std::fmt::Formatter<'_>`): Destination formatter used by the Rust formatting trait.
    ///
    /// # Returns
    /// Type: `std::fmt::Result`.
    /// Returns `std::fmt::Result`.
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(self, formatter)
    }
}

impl std::error::Error for MemoryStoreError {}

/// Identity of a mutation whose acknowledgement may be lost.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PersistenceMutation {
    /// Record the selected value through its store.
    Write,
    /// Delete the selected record.
    Remove,
    /// Replace a value only if its recorded revision matches.
    CompareExchange,
}

/// Distinct persistence failures preserve the provider's concrete error type.
#[derive(Debug)]
///
/// # Type parameters
/// - `Backend`: Backend-specific error or provider type.
pub enum PersistenceError<Backend> {
    /// The stable identity is invalid or incompatible. Retains the `KeyError` detail.
    Identity(KeyError),
    /// Encoding or decoding the typed value failed. Retains the `SchemaIssue` detail.
    Codec(SchemaIssue),
    /// The selected backing provider failed. Retains the `Backend` detail.
    Backend(Backend),
    /// The required value or capability is unavailable. Retains the `Backend` detail.
    Unavailable(Backend),
    /// The operation may have committed before acknowledgement was lost.
    Uncertain {
        /// Registered operation associated with this value. Type: `PersistenceMutation`.
        operation: PersistenceMutation,
        /// Key used to address the stored value. Type: `Arc<[u8]>`.
        key: Arc<[u8]>,
        /// The underlying failure or provider cause. Type: `Backend`.
        source: Backend,
    },
    /// The recorded version differs from the declared version.
    VersionMismatch,
    /// The stored representation failed its integrity checks. Retains the `SchemaIssue` detail.
    Corrupt(SchemaIssue),
    /// The batch differs from the required or recorded value.
    BatchMismatch,
    /// The acknowledgement failed validation.
    InvalidAcknowledgement {
        /// Registered operation associated with this value. Type: `PersistenceMutation`.
        operation: PersistenceMutation,
        /// Key used to address the stored value. Type: `Arc<[u8]>`.
        key: Arc<[u8]>,
    },
}

impl MemoryKeyValueStore {
    /// The memory service is explicitly volatile and scoped by the resulting
    /// handle. A caller chooses its capacity rather than receiving a hidden cap.
    #[must_use]
    /// # Parameters
    /// - `capacity` (`StoreCapacity`): The caller-supplied capacity bound.
    ///
    /// # Returns
    /// Type: `Layer<(), MemoryStoreError, Self>`.
    pub fn layer(capacity: StoreCapacity) -> Layer<(), MemoryStoreError, Self> {
        Layer::make_with(move || Self::new(capacity))
    }
}
