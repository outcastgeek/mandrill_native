//! Atomic acceptance, claims, replies, and acknowledgement contracts.

use super::{ClusterAddress, ClusterObserverId, ClusterOwner, ClusterRequestId};
use crate::{Effect, UtcMillis};
use std::num::{NonZeroU32, NonZeroU64, NonZeroUsize};
use std::sync::Arc;
use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// The response when one request identity is reused with different payload bytes.
pub enum PayloadConflictPolicy {
    /// Reject the conflicting input without applying its proposed state change.
    Reject,
    /// Report an existing accepted identity without executing the new payload.
    /// The caller must treat this as a conflict, not a successful deduplication.
    ReportExisting,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// Deduplication, reply retention, conflict, and delivery-attempt rules for accepted messages.
pub struct AcceptancePolicy {
    /// Time to retain accepted request identities for deduplication. Type: `Duration`.
    pub dedup_retention: Duration,
    /// Time to retain completed replies. Type: `Duration`.
    pub reply_retention: Duration,
    /// Response to an identity reused with different payload bytes. Type: `PayloadConflictPolicy`.
    pub payload_conflict: PayloadConflictPolicy,
    /// Maximum delivery attempts before exhaustion. Type: `NonZeroU32`.
    pub maximum_attempts: NonZeroU32,
}

#[derive(Clone)]
/// An addressed request whose identity, payload, and acceptance policy have been recorded.
pub struct AcceptedRequest {
    /// Destination entity address. Type: `ClusterAddress`.
    pub address: ClusterAddress,
    /// Stable identity of the accepted request. Type: `ClusterRequestId`.
    pub id: ClusterRequestId,
    /// The payload carried by this request or record. Type: `Arc<[u8]>`.
    pub payload: Arc<[u8]>,
    /// Extracted from the original typed RPC request before entity wrapping.
    pub idempotency_key: Option<Arc<[u8]>>,
    /// The time at which the accepted occurred. Type: `UtcMillis`.
    pub accepted_at: UtcMillis,
    /// The policy governing this operation. Type: `AcceptancePolicy`.
    pub policy: AcceptancePolicy,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// The pending, claimed, completed, or dead-letter state of an accepted message.
pub enum MessageDisposition {
    /// The operation has not completed.
    Pending,
    /// A worker currently holds a fenced claim.
    Claimed,
    /// The operation completed and its outcome is available.
    Completed,
    /// The message is retained for inspection after delivery exhaustion.
    DeadLetter,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// The atomic decision to accept, deduplicate, or reject a conflicting request.
pub enum MessageAcceptance {
    /// The request was atomically accepted for delivery.
    Accepted {
        /// Stable acceptance order. Type: `NonZeroU64`.
        order: NonZeroU64,
    },
    /// A request with the same identity and payload was already accepted.
    Duplicate {
        /// Recorded state of the original request. Type: `MessageDisposition`.
        disposition: MessageDisposition,
    },
    /// The request identity was reused with different payload bytes.
    PayloadConflict {
        /// Recorded state of the original request. Type: `MessageDisposition`.
        disposition: MessageDisposition,
    },
}

#[derive(Clone)]
/// The owner, current instant, lease expiry, and limit for a claim attempt.
pub struct ClaimOptions {
    /// The owner of the current claim or activation. Type: `ClusterOwner`.
    pub owner: ClusterOwner,
    /// Caller-supplied current instant used to evaluate claims. Type: `UtcMillis`.
    pub now: UtcMillis,
    /// Instant at which the claim lease expires. Type: `UtcMillis`.
    pub lease_until: UtcMillis,
    /// The caller-supplied bound. Type: `NonZeroUsize`.
    pub limit: NonZeroUsize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// A typed rejection of an invalid message-store state transition.
pub enum ClusterStoreError {
    /// The lease expiry must be later than the claim instant.
    LeaseNotAfterClaim,
    /// Computing retention exceeded its supported range.
    RetentionOverflow,
    /// No further sequence can be allocated without exceeding its range.
    SequenceExhausted,
    /// No further claim generation can be allocated without exceeding its range.
    ClaimGenerationExhausted,
    /// No further attempt can be allocated without exceeding its range.
    AttemptExhausted,
    /// The configured capacity bound was exceeded.
    CapacityExceeded,
    /// An owned input was already consumed.
    InputConsumed,
    /// The required request is missing.
    MissingRequest,
    /// The supplied owner does not match the required or recorded value.
    WrongOwner,
    /// The supplied claim token no longer owns this message.
    StaleClaim,
    /// The record already has a terminal outcome.
    AlreadyCompleted,
    /// The next reply chunk sequence is not contiguous.
    ChunkGap,
    /// An incompatible chunk is already recorded.
    ChunkConflict,
    /// An incompatible reply is already recorded.
    ReplyConflict,
    /// The acknowledgement failed validation.
    InvalidAcknowledgement,
    /// An incompatible payload is already recorded.
    PayloadConflict,
    /// The activation lease expired before the operation settled.
    ActivationLeaseExpired,
    /// No further activation generation can be allocated without exceeding its range.
    ActivationGenerationExhausted,
    /// The proposed lease ends before the recorded lease.
    LeaseRegression,
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// Stable identity used by cluster message store operations.
pub struct ClaimToken {
    /// Entity address targeted by this message. Type: `ClusterAddress`.
    pub address: ClusterAddress,
    /// The request being handled. Type: `ClusterRequestId`.
    pub request: ClusterRequestId,
    /// The owner of the current claim or activation. Type: `ClusterOwner`.
    pub owner: ClusterOwner,
    /// The ownership generation for claim. Type: `NonZeroU64`.
    pub claim_generation: NonZeroU64,
    /// Number of the current delivery or execution attempt. Type: `NonZeroU32`.
    pub attempt: NonZeroU32,
    /// Instant at which this claim or ownership lease expires. Type: `UtcMillis`.
    pub lease_until: UtcMillis,
}

/// An accepted message leased to one owner under a fenced claim token.
pub struct MessageClaim {
    /// Fencing token used to reject stale owners or claims. Type: `ClaimToken`.
    pub token: ClaimToken,
    /// The payload carried by this request or record. Type: `Arc<[u8]>`.
    pub payload: Arc<[u8]>,
    /// The time at which the accepted occurred. Type: `UtcMillis`.
    pub accepted_at: UtcMillis,
    /// Ordered chunks retained for this reply. Type: `Vec<SavedChunk>`.
    pub chunks: Vec<SavedChunk>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// One ordered piece of a reply retained for later observation.
pub struct SavedChunk {
    /// The ordered sequence position. Type: `NonZeroU64`.
    pub sequence: NonZeroU64,
    /// The payload carried by this request or record. Type: `Arc<[u8]>`.
    pub payload: Arc<[u8]>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// The terminal category of a recorded cluster reply.
pub enum ReplyKind {
    /// The operation succeeded.
    Success,
    /// The application handler returned its declared error type.
    DomainError,
    /// An unexpected defect occurred.
    Defect,
    /// Execution was interrupted.
    Interrupted,
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// A reply whose chunks and terminal kind have been recorded by the store.
pub struct SavedReply {
    /// Typed discriminator for this state. Type: `ReplyKind`.
    pub kind: ReplyKind,
    /// The payload carried by this request or record. Type: `Arc<[u8]>`.
    pub payload: Arc<[u8]>,
}

/// A claim token paired with the reply used to settle that claim.
pub struct CompletedReply {
    /// Fencing token used to reject stale owners or claims. Type: `ClaimToken`.
    pub token: ClaimToken,
    /// Reply recorded for the accepted request. Type: `SavedReply`.
    pub reply: SavedReply,
    /// The time at which the completed occurred. Type: `UtcMillis`.
    pub completed_at: UtcMillis,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// A requested atomic state change in the cluster message store.
pub enum StoreMutation {
    /// The value was durably or logically recorded by its store.
    Stored,
    /// An equivalent record was already stored.
    AlreadyStored,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// The store decision returned after applying a message transition.
pub enum StoreCompletion {
    /// The operation completed and its outcome is available.
    Completed,
    /// The record already contains a terminal outcome.
    AlreadyCompleted,
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// One observer’s position and reply acknowledgement state.
pub struct ObserverCursor {
    /// Identity of the client observing this reply. Type: `ClusterObserverId`.
    pub observer: ClusterObserverId,
    /// Last chunk sequence confirmed by this observer. Type: `Option<NonZeroU64>`.
    pub last_acknowledged: Option<NonZeroU64>,
    /// Whether this observer confirmed the terminal reply. Type: `bool`.
    pub reply_acknowledged: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// The claim, completion, or dead-letter data retained for a message.
pub enum SavedMessageState {
    /// The operation has not completed.
    Pending,
    /// A worker currently holds a fenced claim. Contains `ClaimToken`.
    Claimed(ClaimToken),
    /// The operation completed and its outcome is available.
    Completed {
        /// Reply recorded for the accepted request. Type: `SavedReply`.
        reply: SavedReply,
        /// The time at which the completed occurred. Type: `UtcMillis`.
        completed_at: UtcMillis,
        /// Instant through which the record must remain available. Type: `UtcMillis`.
        retained_until: UtcMillis,
    },
    /// The message is retained for inspection after delivery exhaustion.
    DeadLetter {
        /// Number of attempts already recorded. Type: `NonZeroU32`.
        attempts: NonZeroU32,
    },
}

/// An accepted request with its current state, chunks, and observer cursors.
pub struct SavedMessage {
    /// Entity address targeted by this message. Type: `ClusterAddress`.
    pub address: ClusterAddress,
    /// The request being handled. Type: `ClusterRequestId`.
    pub request: ClusterRequestId,
    /// The payload carried by this request or record. Type: `Arc<[u8]>`.
    pub payload: Arc<[u8]>,
    /// The time at which the accepted occurred. Type: `UtcMillis`.
    pub accepted_at: UtcMillis,
    /// The policy governing this operation. Type: `AcceptancePolicy`.
    pub policy: AcceptancePolicy,
    /// Recorded lifecycle state. Type: `SavedMessageState`.
    pub state: SavedMessageState,
    /// Ordered chunks retained for this reply. Type: `Vec<SavedChunk>`.
    pub chunks: Vec<SavedChunk>,
    /// Per-observer reply cursors retained with this message. Type: `Vec<ObserverCursor>`.
    pub observers: Vec<ObserverCursor>,
}

/// The chunks, reply, and disposition visible to an observer at one instant.
pub struct StoreObservation {
    /// Ordered chunks retained for this reply. Type: `Vec<SavedChunk>`.
    pub chunks: Vec<SavedChunk>,
    /// Reply recorded for the accepted request. Type: `Option<SavedReply>`.
    pub reply: Option<SavedReply>,
    /// Recorded delivery status of this message. Type: `MessageDisposition`.
    pub disposition: MessageDisposition,
}

#[derive(Clone)]
/// A bounded view of messages selected from the store.
pub struct MessageScan {
    /// Entity address targeted by this message. Type: `ClusterAddress`.
    pub address: ClusterAddress,
    /// The caller-supplied bound. Type: `NonZeroUsize`.
    pub limit: NonZeroUsize,
}

/// Detailed recorded state returned when inspecting one accepted message.
pub struct MessageInspection {
    /// Message records selected by this scan. Type: `Vec<SavedMessage>`.
    pub messages: Vec<SavedMessage>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// Explicit resource bounds for cluster message store.
pub enum MessageStoreCapacity {
    /// No explicit capacity bound is imposed.
    Unbounded,
    /// The supplied positive capacity bound is enforced. Contains `NonZeroUsize`.
    Bounded(NonZeroUsize),
}

/// The application or provider implements this stronger contract directly.
/// Ordinary KeyValueStore or PersistedQueue cannot synthesize its atomic
/// acceptance, ownership fencing and saved streaming replies.
pub trait ClusterMessageStore: Send + Sync + 'static {
    /// Provider-specific failure type returned by this capability.
    type Error: Send + 'static;

    /// # Parameters
    /// - `request` (`AcceptedRequest`): The typed request to process.
    ///
    /// # Returns
    /// Type: `Effect<MessageAcceptance, Self::Error>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    fn accept(&self, request: AcceptedRequest) -> Effect<MessageAcceptance, Self::Error>;

    /// # Parameters
    /// - `options` (`ClaimOptions`): Caller-selected bounds and behavior for this operation.
    ///
    /// # Returns
    /// Type: `Effect<Vec<MessageClaim>, Self::Error>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    fn claim(&self, options: ClaimOptions) -> Effect<Vec<MessageClaim>, Self::Error>;

    /// # Parameters
    /// - `token` (`ClaimToken`): Fencing token that identifies the current claim or owner.
    /// - `chunk` (`SavedChunk`): The chunk supplied to this operation.
    /// - `now` (`UtcMillis`): The caller-supplied current time.
    ///
    /// # Returns
    /// Type: `Effect<StoreMutation, Self::Error>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    fn save_chunk(
        &self,
        token: ClaimToken,
        chunk: SavedChunk,
        now: UtcMillis,
    ) -> Effect<StoreMutation, Self::Error>;

    /// # Parameters
    /// - `completion` (`CompletedReply`): The completion supplied to this operation.
    ///
    /// # Returns
    /// Type: `Effect<StoreCompletion, Self::Error>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    fn complete(&self, completion: CompletedReply) -> Effect<StoreCompletion, Self::Error>;

    /// # Parameters
    /// - `address` (`ClusterAddress`): Stable destination address of the entity receiving this request.
    /// - `request` (`ClusterRequestId`): The typed request to process.
    ///
    /// # Returns
    /// Type: `Effect<Option<SavedMessage>, Self::Error>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    fn inspect(
        &self,
        address: ClusterAddress,
        request: ClusterRequestId,
    ) -> Effect<Option<SavedMessage>, Self::Error>;

    /// # Parameters
    /// - `scan` (`MessageScan`): Bounded request for records in a selected state.
    ///
    /// # Returns
    /// Type: `Effect<MessageInspection, Self::Error>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    fn scan(&self, scan: MessageScan) -> Effect<MessageInspection, Self::Error>;

    /// # Parameters
    /// - `address` (`ClusterAddress`): Stable destination address of the entity receiving this request.
    /// - `request` (`ClusterRequestId`): The typed request to process.
    /// - `observer` (`ClusterObserverId`): Stable identity of the client observing recorded results.
    /// - `after` (`Option<NonZeroU64>`): Cursor or instant after which results are selected.
    ///
    /// # Returns
    /// Type: `Effect<StoreObservation, Self::Error>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    fn observe(
        &self,
        address: ClusterAddress,
        request: ClusterRequestId,
        observer: ClusterObserverId,
        after: Option<NonZeroU64>,
    ) -> Effect<StoreObservation, Self::Error>;

    /// # Parameters
    /// - `address` (`ClusterAddress`): Stable destination address of the entity receiving this request.
    /// - `request` (`ClusterRequestId`): The typed request to process.
    /// - `observer` (`ClusterObserverId`): Stable identity of the client observing recorded results.
    /// - `sequence` (`NonZeroU64`): Ordered stream position used to reject gaps or duplicates.
    ///
    /// # Returns
    /// Type: `Effect<StoreMutation, Self::Error>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    fn acknowledge_chunk(
        &self,
        address: ClusterAddress,
        request: ClusterRequestId,
        observer: ClusterObserverId,
        sequence: NonZeroU64,
    ) -> Effect<StoreMutation, Self::Error>;

    /// # Parameters
    /// - `address` (`ClusterAddress`): Stable destination address of the entity receiving this request.
    /// - `request` (`ClusterRequestId`): The typed request to process.
    /// - `observer` (`ClusterObserverId`): Stable identity of the client observing recorded results.
    ///
    /// # Returns
    /// Type: `Effect<StoreMutation, Self::Error>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    fn acknowledge_reply(
        &self,
        address: ClusterAddress,
        request: ClusterRequestId,
        observer: ClusterObserverId,
    ) -> Effect<StoreMutation, Self::Error>;

    /// # Parameters
    /// - `address` (`ClusterAddress`): Stable destination address of the entity receiving this request.
    /// - `request` (`ClusterRequestId`): The typed request to process.
    /// - `observer` (`ClusterObserverId`): Stable identity of the client observing recorded results.
    ///
    /// # Returns
    /// Type: `Effect<bool, Self::Error>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    fn detach_observer(
        &self,
        address: ClusterAddress,
        request: ClusterRequestId,
        observer: ClusterObserverId,
    ) -> Effect<bool, Self::Error>;

    /// # Parameters
    /// - `now` (`UtcMillis`): The caller-supplied current time.
    ///
    /// # Returns
    /// Type: `Effect<usize, Self::Error>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    fn cleanup(&self, now: UtcMillis) -> Effect<usize, Self::Error>;
}

/// Explicit recoverability marker; the memory implementation is volatile.
pub trait DurableClusterMessageStore: ClusterMessageStore {}

/// Selects the atomic Cluster message store used by an application.
pub trait HasClusterMessageStore {
    /// Store selected by this environment; its trait bound defines the required guarantees.
    type Store: ClusterMessageStore;
    /// # Returns
    /// Type: `Arc<Self::Store>`.
    fn cluster_message_store(&self) -> Arc<Self::Store>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// The selected message-store action represented as a typed operation.
pub enum ClusterStoreOperation {
    /// Accept the proposed value under the current policy.
    Accept,
    /// A worker has acquired a fenced claim.
    Claim,
    /// Persist one ordered reply chunk.
    SaveChunk,
    /// The operation has a recorded terminal result.
    Complete,
    /// Acknowledge consumption of one reply chunk.
    AcknowledgeChunk,
    /// Acknowledge the terminal reply.
    AcknowledgeReply,
    /// Remove an observer without deleting the message.
    DetachObserver,
    /// The failure arose during protected cleanup.
    Cleanup,
}
