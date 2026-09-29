use super::{ClusterAddress, ClusterObserverId, ClusterOwner, ClusterRequestId};
use crate::{Effect, UtcMillis};
use std::num::{NonZeroU32, NonZeroU64, NonZeroUsize};
use std::sync::Arc;
use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PayloadConflictPolicy {
    Reject,
    /// Report an existing accepted identity without executing the new payload.
    /// The caller must treat this as a conflict, not a successful deduplication.
    ReportExisting,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AcceptancePolicy {
    pub dedup_retention: Duration,
    pub reply_retention: Duration,
    pub payload_conflict: PayloadConflictPolicy,
    pub maximum_attempts: NonZeroU32,
}

#[derive(Clone)]
pub struct AcceptedRequest {
    pub address: ClusterAddress,
    pub id: ClusterRequestId,
    pub payload: Arc<[u8]>,
    /// Extracted from the original typed RPC request before entity wrapping.
    pub idempotency_key: Option<Arc<[u8]>>,
    pub accepted_at: UtcMillis,
    pub policy: AcceptancePolicy,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MessageDisposition {
    Pending,
    Claimed,
    Completed,
    DeadLetter,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MessageAcceptance {
    Accepted { order: NonZeroU64 },
    Duplicate { disposition: MessageDisposition },
    PayloadConflict { disposition: MessageDisposition },
}

#[derive(Clone)]
pub struct ClaimOptions {
    pub owner: ClusterOwner,
    pub now: UtcMillis,
    pub lease_until: UtcMillis,
    pub limit: NonZeroUsize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClusterStoreError {
    LeaseNotAfterClaim,
    RetentionOverflow,
    SequenceExhausted,
    ClaimGenerationExhausted,
    AttemptExhausted,
    CapacityExceeded,
    InputConsumed,
    MissingRequest,
    WrongOwner,
    StaleClaim,
    AlreadyCompleted,
    ChunkGap,
    ChunkConflict,
    ReplyConflict,
    InvalidAcknowledgement,
    PayloadConflict,
    ActivationLeaseExpired,
    ActivationGenerationExhausted,
    LeaseRegression,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClaimToken {
    pub address: ClusterAddress,
    pub request: ClusterRequestId,
    pub owner: ClusterOwner,
    pub claim_generation: NonZeroU64,
    pub attempt: NonZeroU32,
    pub lease_until: UtcMillis,
}

pub struct MessageClaim {
    pub token: ClaimToken,
    pub payload: Arc<[u8]>,
    pub accepted_at: UtcMillis,
    pub chunks: Vec<SavedChunk>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SavedChunk {
    pub sequence: NonZeroU64,
    pub payload: Arc<[u8]>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReplyKind {
    Success,
    DomainError,
    Defect,
    Interrupted,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SavedReply {
    pub kind: ReplyKind,
    pub payload: Arc<[u8]>,
}

pub struct CompletedReply {
    pub token: ClaimToken,
    pub reply: SavedReply,
    pub completed_at: UtcMillis,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StoreMutation {
    Stored,
    AlreadyStored,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StoreCompletion {
    Completed,
    AlreadyCompleted,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObserverCursor {
    pub observer: ClusterObserverId,
    pub last_acknowledged: Option<NonZeroU64>,
    pub reply_acknowledged: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SavedMessageState {
    Pending,
    Claimed(ClaimToken),
    Completed {
        reply: SavedReply,
        completed_at: UtcMillis,
        retained_until: UtcMillis,
    },
    DeadLetter {
        attempts: NonZeroU32,
    },
}

pub struct SavedMessage {
    pub address: ClusterAddress,
    pub request: ClusterRequestId,
    pub payload: Arc<[u8]>,
    pub accepted_at: UtcMillis,
    pub policy: AcceptancePolicy,
    pub state: SavedMessageState,
    pub chunks: Vec<SavedChunk>,
    pub observers: Vec<ObserverCursor>,
}

pub struct StoreObservation {
    pub chunks: Vec<SavedChunk>,
    pub reply: Option<SavedReply>,
    pub disposition: MessageDisposition,
}

#[derive(Clone)]
pub struct MessageScan {
    pub address: ClusterAddress,
    pub limit: NonZeroUsize,
}

pub struct MessageInspection {
    pub messages: Vec<SavedMessage>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MessageStoreCapacity {
    Unbounded,
    Bounded(NonZeroUsize),
}

/// The application or provider implements this stronger contract directly.
/// Ordinary KeyValueStore or PersistedQueue cannot synthesize its atomic
/// acceptance, ownership fencing and saved streaming replies.
pub trait ClusterMessageStore: Send + Sync + 'static {
    type Error: Send + 'static;

    fn accept(&self, request: AcceptedRequest) -> Effect<MessageAcceptance, Self::Error>;

    fn claim(&self, options: ClaimOptions) -> Effect<Vec<MessageClaim>, Self::Error>;

    fn save_chunk(
        &self,
        token: ClaimToken,
        chunk: SavedChunk,
        now: UtcMillis,
    ) -> Effect<StoreMutation, Self::Error>;

    fn complete(&self, completion: CompletedReply) -> Effect<StoreCompletion, Self::Error>;

    fn inspect(
        &self,
        address: ClusterAddress,
        request: ClusterRequestId,
    ) -> Effect<Option<SavedMessage>, Self::Error>;

    fn scan(&self, scan: MessageScan) -> Effect<MessageInspection, Self::Error>;

    fn observe(
        &self,
        address: ClusterAddress,
        request: ClusterRequestId,
        observer: ClusterObserverId,
        after: Option<NonZeroU64>,
    ) -> Effect<StoreObservation, Self::Error>;

    fn acknowledge_chunk(
        &self,
        address: ClusterAddress,
        request: ClusterRequestId,
        observer: ClusterObserverId,
        sequence: NonZeroU64,
    ) -> Effect<StoreMutation, Self::Error>;

    fn acknowledge_reply(
        &self,
        address: ClusterAddress,
        request: ClusterRequestId,
        observer: ClusterObserverId,
    ) -> Effect<StoreMutation, Self::Error>;

    fn detach_observer(
        &self,
        address: ClusterAddress,
        request: ClusterRequestId,
        observer: ClusterObserverId,
    ) -> Effect<bool, Self::Error>;

    fn cleanup(&self, now: UtcMillis) -> Effect<usize, Self::Error>;
}

/// Explicit recoverability marker; the memory implementation is volatile.
pub trait DurableClusterMessageStore: ClusterMessageStore {}

pub trait HasClusterMessageStore {
    type Store: ClusterMessageStore;
    fn cluster_message_store(&self) -> Arc<Self::Store>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClusterStoreOperation {
    Accept,
    Claim,
    SaveChunk,
    Complete,
    AcknowledgeChunk,
    AcknowledgeReply,
    DetachObserver,
    Cleanup,
}
