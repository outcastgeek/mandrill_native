use super::UtcMillis;
use crate::encoding::{BinaryError, BinaryIdentity, BinaryLimits, SchemaBinary};
use crate::{Effect, JsonValue, KeyDomain, KeyError, PureCodec, StableKey, TimeError};
use std::marker::PhantomData;
use std::num::{NonZeroU32, NonZeroU64, NonZeroUsize};
use std::sync::Arc;
use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PersistedQueueDefinitionError {
    EmptyName,
    EmptyOwner,
    LeaseNotAfterClaim,
    RetryBeforeSettlement,
}

impl std::fmt::Display for PersistedQueueDefinitionError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(self, formatter)
    }
}

impl std::error::Error for PersistedQueueDefinitionError {}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct PersistedQueueName(Arc<str>);

impl PersistedQueueName {
    pub fn new(name: impl Into<Arc<str>>) -> Result<Self, PersistedQueueDefinitionError> {
        let name = name.into();
        if name.trim().is_empty() {
            Err(PersistedQueueDefinitionError::EmptyName)
        } else {
            Ok(Self(name))
        }
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Construction from a canonical StableKey preserves its domain, namespace
/// and segment boundaries. The store treats the resulting bytes opaquely.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct QueueMessageId(Arc<[u8]>);

impl QueueMessageId {
    #[must_use]
    pub fn from_key<D: KeyDomain>(key: StableKey<D>) -> Self {
        Self(key.into_bytes())
    }

    pub fn from_encoded<D: KeyDomain>(encoded: Arc<[u8]>) -> Result<Self, KeyError> {
        StableKey::<D>::from_encoded(encoded).map(Self::from_key)
    }

    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QueueOwner(Arc<str>);

impl QueueOwner {
    pub fn new(value: impl Into<Arc<str>>) -> Result<Self, PersistedQueueDefinitionError> {
        let value = value.into();
        if value.trim().is_empty() {
            Err(PersistedQueueDefinitionError::EmptyOwner)
        } else {
            Ok(Self(value))
        }
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Retention begins at acknowledgement, never while work is unfinished.
/// A zero retention is an explicit choice to allow immediate ID reuse after
/// acknowledgement. Dead letters remain until explicitly requeued or removed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct QueueMessagePolicy {
    pub maximum_attempts: NonZeroU32,
    pub acknowledged_retention: Duration,
}

pub struct QueueOffer {
    queue: PersistedQueueName,
    id: QueueMessageId,
    payload: Arc<[u8]>,
    policy: QueueMessagePolicy,
    offered_at: UtcMillis,
    available_at: UtcMillis,
}

impl QueueOffer {
    #[must_use]
    pub fn new(
        queue: PersistedQueueName,
        id: QueueMessageId,
        payload: Arc<[u8]>,
        policy: QueueMessagePolicy,
        offered_at: UtcMillis,
        available_at: UtcMillis,
    ) -> Self {
        Self {
            queue,
            id,
            payload,
            policy,
            offered_at,
            available_at,
        }
    }

    #[must_use]
    pub fn queue(&self) -> &PersistedQueueName {
        &self.queue
    }
    #[must_use]
    pub fn id(&self) -> &QueueMessageId {
        &self.id
    }
    #[must_use]
    pub fn payload(&self) -> &[u8] {
        &self.payload
    }
    #[must_use]
    pub const fn policy(&self) -> QueueMessagePolicy {
        self.policy
    }
    #[must_use]
    pub const fn offered_at(&self) -> UtcMillis {
        self.offered_at
    }
    #[must_use]
    pub const fn available_at(&self) -> UtcMillis {
        self.available_at
    }

    #[must_use]
    pub fn into_parts(
        self,
    ) -> (
        PersistedQueueName,
        QueueMessageId,
        Arc<[u8]>,
        QueueMessagePolicy,
        UtcMillis,
        UtcMillis,
    ) {
        (
            self.queue,
            self.id,
            self.payload,
            self.policy,
            self.offered_at,
            self.available_at,
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QueueDisposition {
    Ready,
    Claimed,
    Acknowledged,
    DeadLetter,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QueueOfferOutcome {
    Accepted { sequence: NonZeroU64 },
    Duplicate { disposition: QueueDisposition },
    ConflictingId,
}

#[derive(Clone)]
pub struct QueueClaimOptions {
    owner: QueueOwner,
    now: UtcMillis,
    lease_until: UtcMillis,
    limit: NonZeroUsize,
}

impl QueueClaimOptions {
    pub fn new(
        owner: QueueOwner,
        now: UtcMillis,
        lease_until: UtcMillis,
        limit: NonZeroUsize,
    ) -> Result<Self, PersistedQueueDefinitionError> {
        if lease_until <= now {
            return Err(PersistedQueueDefinitionError::LeaseNotAfterClaim);
        }
        Ok(Self {
            owner,
            now,
            lease_until,
            limit,
        })
    }

    #[must_use]
    pub fn owner(&self) -> &QueueOwner {
        &self.owner
    }
    #[must_use]
    pub const fn now(&self) -> UtcMillis {
        self.now
    }
    #[must_use]
    pub const fn lease_until(&self) -> UtcMillis {
        self.lease_until
    }
    #[must_use]
    pub const fn limit(&self) -> NonZeroUsize {
        self.limit
    }
}

/// Generation and owner fence each settlement. A token from an expired lease
/// cannot commit even when no replacement worker has claimed the item yet.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QueueClaimToken {
    queue: PersistedQueueName,
    id: QueueMessageId,
    owner: QueueOwner,
    generation: NonZeroU64,
}

impl QueueClaimToken {
    #[must_use]
    pub fn new(
        queue: PersistedQueueName,
        id: QueueMessageId,
        owner: QueueOwner,
        generation: NonZeroU64,
    ) -> Self {
        Self {
            queue,
            id,
            owner,
            generation,
        }
    }

    #[must_use]
    pub fn queue(&self) -> &PersistedQueueName {
        &self.queue
    }
    #[must_use]
    pub fn id(&self) -> &QueueMessageId {
        &self.id
    }
    #[must_use]
    pub fn owner(&self) -> &QueueOwner {
        &self.owner
    }
    #[must_use]
    pub const fn generation(&self) -> NonZeroU64 {
        self.generation
    }
}

#[derive(Debug)]
pub struct QueueClaim {
    pub token: QueueClaimToken,
    pub payload: Arc<[u8]>,
    pub attempt: NonZeroU32,
    pub offered_at: UtcMillis,
    pub available_at: UtcMillis,
    pub claimed_at: UtcMillis,
    pub lease_until: UtcMillis,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum QueueDeadLetterReason {
    AttemptsExhausted,
    CorruptPayload,
    HandlerRejected(Arc<str>),
    Administrative(Arc<str>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum QueueRecordState {
    Ready,
    Claimed {
        token: QueueClaimToken,
        claimed_at: UtcMillis,
        lease_until: UtcMillis,
    },
    Acknowledged {
        retained_until: UtcMillis,
    },
    DeadLetter {
        reason: QueueDeadLetterReason,
    },
}

impl QueueRecordState {
    #[must_use]
    pub const fn disposition(&self) -> QueueDisposition {
        match self {
            Self::Ready => QueueDisposition::Ready,
            Self::Claimed { .. } => QueueDisposition::Claimed,
            Self::Acknowledged { .. } => QueueDisposition::Acknowledged,
            Self::DeadLetter { .. } => QueueDisposition::DeadLetter,
        }
    }
}

#[derive(Clone)]
pub struct QueueRecord {
    pub id: QueueMessageId,
    pub payload: Arc<[u8]>,
    pub policy: QueueMessagePolicy,
    pub sequence: NonZeroU64,
    pub offered_at: UtcMillis,
    pub available_at: UtcMillis,
    pub attempts: u32,
    pub state: QueueRecordState,
}

pub struct QueueRetry {
    now: UtcMillis,
    available_at: UtcMillis,
}

impl QueueRetry {
    pub fn new(
        now: UtcMillis,
        available_at: UtcMillis,
    ) -> Result<Self, PersistedQueueDefinitionError> {
        if available_at < now {
            return Err(PersistedQueueDefinitionError::RetryBeforeSettlement);
        }
        Ok(Self { now, available_at })
    }

    #[must_use]
    pub const fn now(&self) -> UtcMillis {
        self.now
    }
    #[must_use]
    pub const fn available_at(&self) -> UtcMillis {
        self.available_at
    }
}

pub enum QueueSettlementAction {
    Acknowledge {
        at: UtcMillis,
    },
    Retry(QueueRetry),
    DeadLetter {
        at: UtcMillis,
        reason: QueueDeadLetterReason,
    },
}

impl QueueSettlementAction {
    #[must_use]
    pub const fn at(&self) -> UtcMillis {
        match self {
            Self::Acknowledge { at } | Self::DeadLetter { at, .. } => *at,
            Self::Retry(retry) => retry.now,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QueueSettlement {
    Applied,
    StaleClaim,
    Missing,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QueueRequeueAttempts {
    Preserve,
    Reset,
}

pub struct QueueRequeue {
    now: UtcMillis,
    available_at: UtcMillis,
    attempts: QueueRequeueAttempts,
}

impl QueueRequeue {
    pub fn new(
        now: UtcMillis,
        available_at: UtcMillis,
        attempts: QueueRequeueAttempts,
    ) -> Result<Self, PersistedQueueDefinitionError> {
        if available_at < now {
            return Err(PersistedQueueDefinitionError::RetryBeforeSettlement);
        }
        Ok(Self {
            now,
            available_at,
            attempts,
        })
    }

    #[must_use]
    pub const fn now(&self) -> UtcMillis {
        self.now
    }
    #[must_use]
    pub const fn available_at(&self) -> UtcMillis {
        self.available_at
    }
    #[must_use]
    pub const fn attempts(&self) -> QueueRequeueAttempts {
        self.attempts
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QueueRequeueOutcome {
    Requeued,
    /// Preserving the count cannot make another delivery claimable. Choose
    /// `Reset` explicitly if another attempt is intended.
    AttemptsExhausted,
    NotDeadLetter,
    Missing,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct QueueScan {
    pub after_sequence: Option<NonZeroU64>,
    pub limit: NonZeroUsize,
}

/// Providers must atomically accept/deduplicate, claim/fence, settle and
/// requeue each named message. Claims are ordered by availability and offer
/// sequence. A handler side effect before acknowledgement can run again.
/// A failed mutation acknowledgement is uncertain to a remote caller.
pub trait PersistedQueueStore: Send + Sync + 'static {
    type Error: Send + 'static;

    fn offer(&self, offer: QueueOffer) -> Effect<QueueOfferOutcome, Self::Error>;
    fn claim(
        &self,
        queue: PersistedQueueName,
        options: QueueClaimOptions,
    ) -> Effect<Vec<QueueClaim>, Self::Error>;
    fn settle(
        &self,
        token: QueueClaimToken,
        action: QueueSettlementAction,
    ) -> Effect<QueueSettlement, Self::Error>;
    fn inspect(
        &self,
        queue: PersistedQueueName,
        id: QueueMessageId,
    ) -> Effect<Option<QueueRecord>, Self::Error>;
    fn dead_letters(
        &self,
        queue: PersistedQueueName,
        scan: QueueScan,
    ) -> Effect<Vec<QueueRecord>, Self::Error>;
    fn requeue(
        &self,
        queue: PersistedQueueName,
        id: QueueMessageId,
        request: QueueRequeue,
    ) -> Effect<QueueRequeueOutcome, Self::Error>;
    /// Remove only acknowledged entries whose declared retention elapsed.
    /// Pending, claimed and dead-letter entries remain recoverable.
    fn cleanup(&self, queue: PersistedQueueName, now: UtcMillis) -> Effect<usize, Self::Error>;
}

/// A separate guarantee that acknowledged state and unacknowledged work
/// survive the provider's documented restart boundary. Memory never claims it.
pub trait DurablePersistedQueueStore: PersistedQueueStore {}

pub trait HasPersistedQueueStore {
    type Store: PersistedQueueStore;
    fn persisted_queue_store(&self) -> Arc<Self::Store>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QueueOperation {
    Offer,
    Claim,
    Settle,
    Requeue,
    Cleanup,
}

#[derive(Debug)]
pub enum PersistedQueueError<Backend> {
    Encoding(BinaryError),
    WrongQueue,
    InputConsumed,
    Backend(Backend),
    Uncertain {
        operation: QueueOperation,
        queue: PersistedQueueName,
        id: Option<QueueMessageId>,
        source: Backend,
    },
}

/// Failed payload decoding returns the complete claim, including raw bytes
/// and its fence, so the worker can inspect or dead-letter it explicitly.
#[derive(Debug)]
pub struct QueueDecodeFailure {
    pub claim: Box<QueueClaim>,
    pub error: QueueClaimError,
}

#[derive(Debug)]
pub enum QueueClaimError {
    WrongQueue,
    InvalidIdentity(KeyError),
    Decode(BinaryError),
}

pub struct DecodedQueueClaim<P> {
    pub claim: QueueClaim,
    pub payload: P,
}

struct TypedQueue<P: Send + 'static, D: KeyDomain> {
    name: PersistedQueueName,
    payload: PureCodec<P, JsonValue>,
    binary: SchemaBinary,
    policy: QueueMessagePolicy,
    domain: PhantomData<fn() -> D>,
}

/// A typed, inert queue description. The actual store and worker runtime are
/// supplied through R. Payload/error types are never erased by the store.
pub struct PersistedQueue<P: Send + 'static, D: KeyDomain> {
    inner: Arc<TypedQueue<P, D>>,
}

impl<P: Send + 'static, D: KeyDomain> Clone for PersistedQueue<P, D> {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

impl<P: Send + 'static, D: KeyDomain> PersistedQueue<P, D> {
    #[must_use]
    pub fn new(
        name: PersistedQueueName,
        payload: PureCodec<P, JsonValue>,
        identity: BinaryIdentity,
        limits: BinaryLimits,
        policy: QueueMessagePolicy,
    ) -> Self {
        let binary = SchemaBinary::new(payload.ast().clone(), identity, limits);
        Self {
            inner: Arc::new(TypedQueue {
                name,
                payload,
                binary,
                policy,
                domain: PhantomData,
            }),
        }
    }

    #[must_use]
    pub fn name(&self) -> &PersistedQueueName {
        &self.inner.name
    }

    pub fn decode_claim(
        &self,
        claim: QueueClaim,
    ) -> Result<DecodedQueueClaim<P>, QueueDecodeFailure> {
        if claim.token.queue() != &self.inner.name {
            return Err(QueueDecodeFailure {
                claim: Box::new(claim),
                error: QueueClaimError::WrongQueue,
            });
        }
        if let Err(error) = StableKey::<D>::from_encoded(Arc::clone(&claim.token.id.0)) {
            return Err(QueueDecodeFailure {
                claim: Box::new(claim),
                error: QueueClaimError::InvalidIdentity(error),
            });
        }
        match self
            .inner
            .binary
            .decode_typed(&self.inner.payload, &claim.payload)
        {
            Ok(payload) => Ok(DecodedQueueClaim { claim, payload }),
            Err(error) => Err(QueueDecodeFailure {
                claim: Box::new(claim),
                error: QueueClaimError::Decode(error),
            }),
        }
    }

    /// Each evaluation manufactures and owns one payload. The store's offer
    /// result distinguishes acceptance, duplicate and conflicting reuse.
    pub fn offer<R>(
        &self,
        id: StableKey<D>,
        payload: impl Fn() -> P + Send + Sync + 'static,
        offered_at: UtcMillis,
        available_at: UtcMillis,
    ) -> Effect<
        QueueOfferOutcome,
        PersistedQueueError<<<R as HasPersistedQueueStore>::Store as PersistedQueueStore>::Error>,
        R,
    >
    where
        R: HasPersistedQueueStore + Send + Sync + 'static,
    {
        let queue = self.clone();
        Effect::try_sync(
            move || {
                let value = payload();
                let bytes = queue
                    .inner
                    .binary
                    .encode_typed(&queue.inner.payload, &value)?;
                Ok(QueueOffer::new(
                    queue.inner.name.clone(),
                    QueueMessageId::from_key(id.clone()),
                    Arc::from(bytes),
                    queue.inner.policy,
                    offered_at,
                    available_at,
                ))
            },
            PersistedQueueError::Encoding,
        )
        .flat_map(move |offer| {
            let queue = offer.queue().clone();
            let id = offer.id().clone();
            let offer = std::sync::Mutex::new(Some(offer));
            Effect::service(|env: &R| env.persisted_queue_store()).flat_map(move |store| {
                let offer = offer
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .take();
                let Some(offer) = offer else {
                    return Effect::from_result_once(Err(PersistedQueueError::InputConsumed));
                };
                let queue = queue.clone();
                let id = id.clone();
                store
                    .offer(offer)
                    .map_error(move |source| PersistedQueueError::Uncertain {
                        operation: QueueOperation::Offer,
                        queue: queue.clone(),
                        id: Some(id.clone()),
                        source,
                    })
                    .provide_some(|_: &R| ())
            })
        })
    }

    pub fn claim<R>(
        &self,
        options: QueueClaimOptions,
    ) -> Effect<
        Vec<QueueClaim>,
        PersistedQueueError<<<R as HasPersistedQueueStore>::Store as PersistedQueueStore>::Error>,
        R,
    >
    where
        R: HasPersistedQueueStore + Send + Sync + 'static,
    {
        let name = self.inner.name.clone();
        Effect::service(|env: &R| env.persisted_queue_store()).flat_map(move |store| {
            let name_for_error = name.clone();
            store
                .claim(name.clone(), options.clone())
                .map_error(move |source| PersistedQueueError::Uncertain {
                    operation: QueueOperation::Claim,
                    queue: name_for_error.clone(),
                    id: None,
                    source,
                })
                .provide_some(|_: &R| ())
        })
    }

    pub fn inspect<R>(
        &self,
        id: StableKey<D>,
    ) -> Effect<
        Option<QueueRecord>,
        PersistedQueueError<<<R as HasPersistedQueueStore>::Store as PersistedQueueStore>::Error>,
        R,
    >
    where
        R: HasPersistedQueueStore + Send + Sync + 'static,
    {
        let name = self.inner.name.clone();
        Effect::service(|env: &R| env.persisted_queue_store()).flat_map(move |store| {
            store
                .inspect(name.clone(), QueueMessageId::from_key(id.clone()))
                .map_error(PersistedQueueError::Backend)
                .provide_some(|_: &R| ())
        })
    }

    pub fn settle<R>(
        &self,
        token: QueueClaimToken,
        action: QueueSettlementAction,
    ) -> Effect<
        QueueSettlement,
        PersistedQueueError<<<R as HasPersistedQueueStore>::Store as PersistedQueueStore>::Error>,
        R,
    >
    where
        R: HasPersistedQueueStore + Send + Sync + 'static,
    {
        if token.queue() != &self.inner.name {
            return Effect::fail_with(|| PersistedQueueError::WrongQueue);
        }
        let queue = self.inner.name.clone();
        let id = token.id().clone();
        let action = std::sync::Mutex::new(Some(action));
        Effect::service(|env: &R| env.persisted_queue_store()).flat_map(move |store| {
            let action = action
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take();
            let Some(action) = action else {
                return Effect::fail_with(|| PersistedQueueError::InputConsumed);
            };
            store
                .settle(token.clone(), action)
                .map_error({
                    let queue = queue.clone();
                    let id = id.clone();
                    move |source| PersistedQueueError::Uncertain {
                        operation: QueueOperation::Settle,
                        queue: queue.clone(),
                        id: Some(id.clone()),
                        source,
                    }
                })
                .provide_some(|_: &R| ())
        })
    }

    pub fn dead_letters<R>(
        &self,
        scan: QueueScan,
    ) -> Effect<
        Vec<QueueRecord>,
        PersistedQueueError<<<R as HasPersistedQueueStore>::Store as PersistedQueueStore>::Error>,
        R,
    >
    where
        R: HasPersistedQueueStore + Send + Sync + 'static,
    {
        let queue = self.inner.name.clone();
        Effect::service(|env: &R| env.persisted_queue_store()).flat_map(move |store| {
            store
                .dead_letters(queue.clone(), scan)
                .map_error(PersistedQueueError::Backend)
                .provide_some(|_: &R| ())
        })
    }

    pub fn requeue<R>(
        &self,
        id: StableKey<D>,
        request: QueueRequeue,
    ) -> Effect<
        QueueRequeueOutcome,
        PersistedQueueError<<<R as HasPersistedQueueStore>::Store as PersistedQueueStore>::Error>,
        R,
    >
    where
        R: HasPersistedQueueStore + Send + Sync + 'static,
    {
        let queue = self.inner.name.clone();
        let request = std::sync::Mutex::new(Some(request));
        Effect::service(|env: &R| env.persisted_queue_store()).flat_map(move |store| {
            let request = request
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take();
            let Some(request) = request else {
                return Effect::fail_with(|| PersistedQueueError::InputConsumed);
            };
            let id = QueueMessageId::from_key(id.clone());
            store
                .requeue(queue.clone(), id.clone(), request)
                .map_error({
                    let queue = queue.clone();
                    move |source| PersistedQueueError::Uncertain {
                        operation: QueueOperation::Requeue,
                        queue: queue.clone(),
                        id: Some(id.clone()),
                        source,
                    }
                })
                .provide_some(|_: &R| ())
        })
    }

    pub fn cleanup<R>(
        &self,
        now: UtcMillis,
    ) -> Effect<
        usize,
        PersistedQueueError<<<R as HasPersistedQueueStore>::Store as PersistedQueueStore>::Error>,
        R,
    >
    where
        R: HasPersistedQueueStore + Send + Sync + 'static,
    {
        let queue = self.inner.name.clone();
        Effect::service(|env: &R| env.persisted_queue_store()).flat_map(move |store| {
            store
                .cleanup(queue.clone(), now)
                .map_error({
                    let queue = queue.clone();
                    move |source| PersistedQueueError::Uncertain {
                        operation: QueueOperation::Cleanup,
                        queue: queue.clone(),
                        id: None,
                        source,
                    }
                })
                .provide_some(|_: &R| ())
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QueueStoreCapacity {
    Unbounded,
    Bounded(NonZeroUsize),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MemoryQueueStoreError {
    CapacityExceeded,
    SequenceOverflow,
    GenerationOverflow,
    AttemptOverflow,
    InconsistentState,
    Time(TimeError),
}

impl std::fmt::Display for MemoryQueueStoreError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(self, formatter)
    }
}

impl std::error::Error for MemoryQueueStoreError {}
