//! Volatile in-memory Cluster message storage.

use super::{
    AcceptedRequest, ActivationId, ActivationLease, ActivationOutcome, ActivationPhase,
    ActivationStatus, ClaimOptions, ClaimToken, ClusterActivationStore, ClusterAddress,
    ClusterMessageStore, ClusterObserverId, ClusterOwner, ClusterRequestId, ClusterStoreError,
    CompletedReply, HasClusterActivationStore, HasClusterMessageStore, MessageAcceptance,
    MessageClaim, MessageDisposition, MessageInspection, MessageScan, MessageStoreCapacity,
    ObserverCursor, OwnershipGeneration, PassivationOutcome, PayloadConflictPolicy, SavedChunk,
    SavedMessage, SavedMessageState, SavedReply, StoreCompletion, StoreMutation, StoreObservation,
};
use crate::{Effect, Layer, UtcMillis};
use std::collections::BTreeMap;
use std::num::{NonZeroU32, NonZeroU64, NonZeroUsize};
use std::sync::{Arc, Mutex};

/// Type alias for message key in cluster memory store.
type MessageKey = (ClusterAddress, ClusterRequestId);

/// Variants of internal state used by cluster memory store.
enum InternalState {
    Pending,
    Claimed(ClaimToken),
    Completed {
        token: ClaimToken,
        reply: SavedReply,
        completed_at: UtcMillis,
        retained_until: UtcMillis,
    },
    DeadLetter {
        attempts: NonZeroU32,
    },
}

/// Typed observer state used by cluster memory store.
struct ObserverState {
    last_acknowledged: Option<NonZeroU64>,
    reply_acknowledged: bool,
}

/// Typed record used by cluster memory store.
struct Record {
    request: AcceptedRequest,
    order: NonZeroU64,
    attempts: Option<NonZeroU32>,
    state: InternalState,
    chunks: Vec<SavedChunk>,
    observers: BTreeMap<ClusterObserverId, ObserverState>,
}

/// Typed memory state used by cluster memory store.
struct MemoryState {
    records: BTreeMap<MessageKey, Record>,
    current_owner: BTreeMap<ClusterAddress, ActivationStatus>,
    next_order: Option<NonZeroU64>,
    next_claim_generation: Option<NonZeroU64>,
}

/// Volatile in-process implementation of the stronger ClusterMessageStore
/// contract. It promises atomicity only inside this handle, never durability
/// or multi-process ownership coordination.
pub struct MemoryClusterMessageStore {
    state: Arc<Mutex<MemoryState>>,
    capacity: MessageStoreCapacity,
}

impl Clone for MemoryClusterMessageStore {
    ///
    /// # Returns
    /// Type: `Self`.
    fn clone(&self) -> Self {
        Self {
            state: Arc::clone(&self.state),
            capacity: self.capacity,
        }
    }
}

impl MemoryClusterMessageStore {
    #[must_use]
    /// Construct a `MemoryClusterMessageStore` from its required inputs and explicit policies.
    ///
    /// # Parameters
    /// - `capacity` (`MessageStoreCapacity`): The caller-supplied capacity bound.
    ///
    /// # Returns
    /// Type: `Self`.
    pub fn new(capacity: MessageStoreCapacity) -> Self {
        Self {
            state: Arc::new(Mutex::new(MemoryState {
                records: BTreeMap::new(),
                current_owner: BTreeMap::new(),
                next_order: Some(NonZeroU64::MIN),
                next_claim_generation: Some(NonZeroU64::MIN),
            })),
            capacity,
        }
    }

    #[must_use]
    /// Build a scoped dependency layer for this service.
    ///
    /// # Parameters
    /// - `capacity` (`MessageStoreCapacity`): The caller-supplied capacity bound.
    ///
    /// # Returns
    /// Type: `Layer<(), ClusterStoreError, Self>`.
    pub fn layer(capacity: MessageStoreCapacity) -> Layer<(), ClusterStoreError, Self> {
        Layer::make_with(move || Self::new(capacity))
    }

    ///
    /// # Type parameters
    /// - `I`: Bounds: `Send + 'static`.
    /// - `O`: Bounds: `Send + 'static`.
    ///
    /// # Parameters
    /// - `input` (`I`): Owned input decoded or consumed by this operation.
    /// - `mutate` (`impl FnOnce(&mut MemoryState, I) -> Result<O, ClusterStoreError> + Send + 'static`): The mutate supplied to this operation.
    ///
    /// # Returns
    /// Type: `Effect<O, ClusterStoreError>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    fn transact<I: Send + 'static, O: Send + 'static>(
        &self,
        input: I,
        mutate: impl FnOnce(&mut MemoryState, I) -> Result<O, ClusterStoreError> + Send + 'static,
    ) -> Effect<O, ClusterStoreError> {
        let state = Arc::clone(&self.state);
        let pending = Mutex::new(Some((input, mutate)));
        Effect::try_sync(
            move || {
                let (input, mutate) = pending
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .take()
                    .ok_or(ClusterStoreError::InputConsumed)?;
                let mut state = state
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                mutate(&mut state, input)
            },
            std::convert::identity,
        )
    }
}

///
/// # Parameters
/// - `state` (`&InternalState`): Current or replacement state owned by this operation.
///
/// # Returns
/// Type: `MessageDisposition`.
/// Returns `MessageDisposition`.
fn disposition(state: &InternalState) -> MessageDisposition {
    match state {
        InternalState::Pending => MessageDisposition::Pending,
        InternalState::Claimed(_) => MessageDisposition::Claimed,
        InternalState::Completed { .. } => MessageDisposition::Completed,
        InternalState::DeadLetter { .. } => MessageDisposition::DeadLetter,
    }
}

///
/// # Parameters
/// - `record` (`&Record`): The record supplied to this operation.
///
/// # Returns
/// Type: `bool`.
/// Returns whether the requested condition holds.
fn acknowledged(record: &Record) -> bool {
    record
        .observers
        .values()
        .all(|observer| observer.reply_acknowledged)
}

///
/// # Parameters
/// - `record` (`&Record`): The record supplied to this operation.
/// - `now` (`UtcMillis`): The caller-supplied current time.
///
/// # Returns
/// Type: `bool`.
/// Returns whether the requested condition holds.
fn expiry(record: &Record, now: UtcMillis) -> bool {
    matches!(&record.state, InternalState::Completed { retained_until, .. }
        if now >= *retained_until)
        && acknowledged(record)
}

///
/// # Parameters
/// - `record` (`&Record`): The record supplied to this operation.
///
/// # Returns
/// Type: `SavedMessage`.
/// Returns `SavedMessage`.
fn recorded(record: &Record) -> SavedMessage {
    let state = match &record.state {
        InternalState::Pending => SavedMessageState::Pending,
        InternalState::Claimed(token) => SavedMessageState::Claimed(token.clone()),
        InternalState::Completed {
            reply,
            completed_at,
            retained_until,
            ..
        } => SavedMessageState::Completed {
            reply: reply.clone(),
            completed_at: *completed_at,
            retained_until: *retained_until,
        },
        InternalState::DeadLetter { attempts } => SavedMessageState::DeadLetter {
            attempts: *attempts,
        },
    };
    SavedMessage {
        address: record.request.address.clone(),
        request: record.request.id.clone(),
        payload: Arc::clone(&record.request.payload),
        accepted_at: record.request.accepted_at,
        policy: record.request.policy,
        state,
        chunks: record.chunks.clone(),
        observers: record
            .observers
            .iter()
            .map(|(observer, cursor)| ObserverCursor {
                observer: observer.clone(),
                last_acknowledged: cursor.last_acknowledged,
                reply_acknowledged: cursor.reply_acknowledged,
            })
            .collect(),
    }
}

///
/// # Type parameters
/// - `'a`: Lifetime parameter.
///
/// # Parameters
/// - `state` (`&'a mut MemoryState`): Current or replacement state owned by this operation.
/// - `token` (`&ClaimToken`): Fencing token that identifies the current claim or owner.
/// - `now` (`UtcMillis`): The caller-supplied current time.
///
/// # Returns
/// Type: `Result<&'a mut Record, ClusterStoreError>`.
fn verify_claim<'a>(
    state: &'a mut MemoryState,
    token: &ClaimToken,
    now: UtcMillis,
) -> Result<&'a mut Record, ClusterStoreError> {
    let key = (token.address.clone(), token.request.clone());
    if !state
        .current_owner
        .get(&token.address)
        .is_some_and(|status| {
            status.phase == ActivationPhase::Active
                && status.lease.owner == token.owner
                && now < status.lease.lease_until
        })
    {
        return Err(ClusterStoreError::WrongOwner);
    }
    let record = state
        .records
        .get_mut(&key)
        .ok_or(ClusterStoreError::MissingRequest)?;
    match &record.state {
        InternalState::Claimed(current) if current == token && now < token.lease_until => {
            Ok(record)
        }
        InternalState::Completed { .. } => Err(ClusterStoreError::AlreadyCompleted),
        InternalState::Pending | InternalState::Claimed(_) | InternalState::DeadLetter { .. } => {
            Err(ClusterStoreError::StaleClaim)
        }
    }
}

impl ClusterMessageStore for MemoryClusterMessageStore {
    /// Type alias for error in cluster memory store.
    type Error = ClusterStoreError;

    ///
    /// # Parameters
    /// - `request` (`AcceptedRequest`): Typed request submitted to a handler or transport.
    ///
    /// # Returns
    /// Type: `Effect<MessageAcceptance, Self::Error>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    fn accept(&self, request: AcceptedRequest) -> Effect<MessageAcceptance, Self::Error> {
        let capacity = self.capacity;
        self.transact(request, move |state, request| {
            let key = (request.address.clone(), request.id.clone());
            if let Some(existing) = state.records.get(&key)
                && !expiry(existing, request.accepted_at)
            {
                if existing.request.payload.as_ref() == request.payload.as_ref()
                    && existing.request.idempotency_key == request.idempotency_key
                    && existing.request.policy == request.policy
                {
                    return Ok(MessageAcceptance::Duplicate {
                        disposition: disposition(&existing.state),
                    });
                }
                return match request.policy.payload_conflict {
                    PayloadConflictPolicy::Reject => Err(ClusterStoreError::PayloadConflict),
                    PayloadConflictPolicy::ReportExisting => {
                        Ok(MessageAcceptance::PayloadConflict {
                            disposition: disposition(&existing.state),
                        })
                    }
                };
            }
            let replacing = state.records.contains_key(&key);
            if !replacing
                && matches!(capacity, MessageStoreCapacity::Bounded(limit)
                if state.records.len() >= limit.get())
            {
                return Err(ClusterStoreError::CapacityExceeded);
            }
            let order = state
                .next_order
                .ok_or(ClusterStoreError::SequenceExhausted)?;
            state.next_order = order.get().checked_add(1).and_then(NonZeroU64::new);
            state.records.insert(
                key,
                Record {
                    request,
                    order,
                    attempts: None,
                    state: InternalState::Pending,
                    chunks: Vec::new(),
                    observers: BTreeMap::new(),
                },
            );
            Ok(MessageAcceptance::Accepted { order })
        })
    }

    ///
    /// # Parameters
    /// - `options` (`ClaimOptions`): Caller-selected bounds and behavior for this operation.
    ///
    /// # Returns
    /// Type: `Effect<Vec<MessageClaim>, Self::Error>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    fn claim(&self, options: ClaimOptions) -> Effect<Vec<MessageClaim>, Self::Error> {
        self.transact(options, |state, options| {
            if options.lease_until <= options.now {
                return Err(ClusterStoreError::LeaseNotAfterClaim);
            }
            let owner = &options.owner;
            let status = state
                .current_owner
                .get(&owner.address)
                .ok_or(ClusterStoreError::WrongOwner)?;
            if status.phase != ActivationPhase::Active
                || status.lease.owner != *owner
                || options.now >= status.lease.lease_until
            {
                return Err(ClusterStoreError::WrongOwner);
            }
            if options.lease_until > status.lease.lease_until {
                return Err(ClusterStoreError::ActivationLeaseExpired);
            }
            let mut candidates = state
                .records
                .iter()
                .filter(|((address, _), record)| {
                    address == &owner.address
                        && match &record.state {
                            InternalState::Pending => true,
                            InternalState::Claimed(token) => {
                                token.owner != *owner || options.now >= token.lease_until
                            }
                            InternalState::Completed { .. } | InternalState::DeadLetter { .. } => {
                                false
                            }
                        }
                })
                .map(|(key, record)| (record.order, key.clone()))
                .collect::<Vec<_>>();
            candidates.sort_by_key(|(order, _)| *order);
            let mut selected = Vec::new();
            let mut dead_letters = Vec::new();
            let mut processed = 0usize;
            for (_, key) in candidates {
                if processed >= options.limit.get() {
                    break;
                }
                processed = processed
                    .checked_add(NonZeroUsize::MIN.get())
                    .ok_or(ClusterStoreError::CapacityExceeded)?;
                let record = state
                    .records
                    .get(&key)
                    .ok_or(ClusterStoreError::MissingRequest)?;
                let next_attempt = match record.attempts {
                    None => Some(NonZeroU32::MIN),
                    Some(attempt) => attempt.get().checked_add(1).and_then(NonZeroU32::new),
                };
                if record
                    .attempts
                    .is_some_and(|attempt| attempt >= record.request.policy.maximum_attempts)
                {
                    dead_letters.push((
                        key,
                        record.attempts.ok_or(ClusterStoreError::AttemptExhausted)?,
                    ));
                    continue;
                }
                let attempt = next_attempt.ok_or(ClusterStoreError::AttemptExhausted)?;
                selected.push((key, attempt));
            }
            let first_generation = state.next_claim_generation;
            if !selected.is_empty() {
                let first = first_generation.ok_or(ClusterStoreError::ClaimGenerationExhausted)?;
                let count = u64::try_from(selected.len())
                    .map_err(|_| ClusterStoreError::ClaimGenerationExhausted)?;
                first
                    .get()
                    .checked_add(count - NonZeroU64::MIN.get())
                    .ok_or(ClusterStoreError::ClaimGenerationExhausted)?;
            }
            for (key, attempts) in dead_letters {
                if let Some(record) = state.records.get_mut(&key) {
                    record.state = InternalState::DeadLetter { attempts };
                }
            }
            let mut claims = Vec::with_capacity(selected.len());
            for (key, attempt) in selected {
                let generation = state
                    .next_claim_generation
                    .ok_or(ClusterStoreError::ClaimGenerationExhausted)?;
                state.next_claim_generation =
                    generation.get().checked_add(1).and_then(NonZeroU64::new);
                let record = state
                    .records
                    .get_mut(&key)
                    .ok_or(ClusterStoreError::MissingRequest)?;
                let token = ClaimToken {
                    address: key.0.clone(),
                    request: key.1.clone(),
                    owner: owner.clone(),
                    claim_generation: generation,
                    attempt,
                    lease_until: options.lease_until,
                };
                record.attempts = Some(attempt);
                record.state = InternalState::Claimed(token.clone());
                claims.push(MessageClaim {
                    token,
                    payload: Arc::clone(&record.request.payload),
                    accepted_at: record.request.accepted_at,
                    chunks: record.chunks.clone(),
                });
            }
            Ok(claims)
        })
    }

    ///
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
    ) -> Effect<StoreMutation, Self::Error> {
        self.transact((token, chunk, now), |state, (token, chunk, now)| {
            let record = verify_claim(state, &token, now)?;
            let current =
                usize::try_from(chunk.sequence.get()).map_err(|_| ClusterStoreError::ChunkGap)?;
            if current <= record.chunks.len() {
                let recorded = record
                    .chunks
                    .get(current - 1)
                    .ok_or(ClusterStoreError::ChunkGap)?;
                return if recorded.payload == chunk.payload {
                    Ok(StoreMutation::AlreadyStored)
                } else {
                    Err(ClusterStoreError::ChunkConflict)
                };
            }
            let next = record
                .chunks
                .len()
                .checked_add(1)
                .ok_or(ClusterStoreError::SequenceExhausted)?;
            if current != next {
                return Err(ClusterStoreError::ChunkGap);
            }
            record.chunks.push(chunk);
            Ok(StoreMutation::Stored)
        })
    }

    ///
    /// # Parameters
    /// - `completion` (`CompletedReply`): The completion supplied to this operation.
    ///
    /// # Returns
    /// Type: `Effect<StoreCompletion, Self::Error>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    fn complete(&self, completion: CompletedReply) -> Effect<StoreCompletion, Self::Error> {
        self.transact(completion, |state, completion| {
            let token = completion.token;
            let key = (token.address.clone(), token.request.clone());
            if let Some(record) = state.records.get(&key)
                && let InternalState::Completed {
                    token: saved_token,
                    reply,
                    ..
                } = &record.state
            {
                return if saved_token == &token && reply == &completion.reply {
                    Ok(StoreCompletion::AlreadyCompleted)
                } else {
                    Err(ClusterStoreError::ReplyConflict)
                };
            }
            if !state
                .current_owner
                .get(&token.address)
                .is_some_and(|status| {
                    status.phase == ActivationPhase::Active
                        && status.lease.owner == token.owner
                        && completion.completed_at < status.lease.lease_until
                })
            {
                return Err(ClusterStoreError::WrongOwner);
            }
            let record = state
                .records
                .get_mut(&key)
                .ok_or(ClusterStoreError::MissingRequest)?;
            if !matches!(&record.state, InternalState::Claimed(current)
                if current == &token && completion.completed_at < token.lease_until)
            {
                return Err(ClusterStoreError::StaleClaim);
            }
            let retention = record
                .request
                .policy
                .dedup_retention
                .max(record.request.policy.reply_retention);
            let retained_until = completion
                .completed_at
                .checked_add(retention)
                .map_err(|_| ClusterStoreError::RetentionOverflow)?;
            record.state = InternalState::Completed {
                token,
                reply: completion.reply,
                completed_at: completion.completed_at,
                retained_until,
            };
            Ok(StoreCompletion::Completed)
        })
    }

    ///
    /// # Parameters
    /// - `address` (`ClusterAddress`): Stable destination address of the entity receiving this request.
    /// - `request` (`ClusterRequestId`): Typed request submitted to a handler or transport.
    ///
    /// # Returns
    /// Type: `Effect<Option<SavedMessage>, Self::Error>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    fn inspect(
        &self,
        address: ClusterAddress,
        request: ClusterRequestId,
    ) -> Effect<Option<SavedMessage>, Self::Error> {
        self.transact((address, request), |state, key| {
            Ok(state.records.get(&key).map(recorded))
        })
    }

    ///
    /// # Parameters
    /// - `scan` (`MessageScan`): Bounded request for records in a selected state.
    ///
    /// # Returns
    /// Type: `Effect<MessageInspection, Self::Error>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    fn scan(&self, scan: MessageScan) -> Effect<MessageInspection, Self::Error> {
        self.transact(scan, |state, scan| {
            let mut messages = state
                .records
                .values()
                .filter(|record| record.request.address == scan.address)
                .collect::<Vec<_>>();
            messages.sort_by_key(|record| record.order);
            Ok(MessageInspection {
                messages: messages
                    .into_iter()
                    .take(scan.limit.get())
                    .map(recorded)
                    .collect(),
            })
        })
    }

    ///
    /// # Parameters
    /// - `address` (`ClusterAddress`): Stable destination address of the entity receiving this request.
    /// - `request` (`ClusterRequestId`): Typed request submitted to a handler or transport.
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
    ) -> Effect<StoreObservation, Self::Error> {
        self.transact(
            (address, request, observer, after),
            |state, (address, request, observer, after)| {
                let record = state
                    .records
                    .get_mut(&(address, request))
                    .ok_or(ClusterStoreError::MissingRequest)?;
                record.observers.entry(observer).or_insert(ObserverState {
                    last_acknowledged: None,
                    reply_acknowledged: false,
                });
                let chunks = record
                    .chunks
                    .iter()
                    .filter(|chunk| after.is_none_or(|last| chunk.sequence > last))
                    .cloned()
                    .collect();
                let reply = match &record.state {
                    InternalState::Completed { reply, .. } => Some(reply.clone()),
                    InternalState::Pending
                    | InternalState::Claimed(_)
                    | InternalState::DeadLetter { .. } => None,
                };
                Ok(StoreObservation {
                    chunks,
                    reply,
                    disposition: disposition(&record.state),
                })
            },
        )
    }

    ///
    /// # Parameters
    /// - `address` (`ClusterAddress`): Stable destination address of the entity receiving this request.
    /// - `request` (`ClusterRequestId`): Typed request submitted to a handler or transport.
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
    ) -> Effect<StoreMutation, Self::Error> {
        self.transact(
            (address, request, observer, sequence),
            |state, (address, request, observer, sequence)| {
                let record = state
                    .records
                    .get_mut(&(address, request))
                    .ok_or(ClusterStoreError::MissingRequest)?;
                if usize::try_from(sequence.get())
                    .ok()
                    .is_none_or(|value| value > record.chunks.len())
                {
                    return Err(ClusterStoreError::InvalidAcknowledgement);
                }
                let cursor = record
                    .observers
                    .get_mut(&observer)
                    .ok_or(ClusterStoreError::InvalidAcknowledgement)?;
                if cursor
                    .last_acknowledged
                    .is_some_and(|last| sequence <= last)
                {
                    return Ok(StoreMutation::AlreadyStored);
                }
                cursor.last_acknowledged = Some(sequence);
                Ok(StoreMutation::Stored)
            },
        )
    }

    ///
    /// # Parameters
    /// - `address` (`ClusterAddress`): Stable destination address of the entity receiving this request.
    /// - `request` (`ClusterRequestId`): Typed request submitted to a handler or transport.
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
    ) -> Effect<StoreMutation, Self::Error> {
        self.transact(
            (address, request, observer),
            |state, (address, request, observer)| {
                let record = state
                    .records
                    .get_mut(&(address, request))
                    .ok_or(ClusterStoreError::MissingRequest)?;
                if !matches!(&record.state, InternalState::Completed { .. }) {
                    return Err(ClusterStoreError::InvalidAcknowledgement);
                }
                let cursor = record
                    .observers
                    .get_mut(&observer)
                    .ok_or(ClusterStoreError::InvalidAcknowledgement)?;
                if let Some(last) = record.chunks.last()
                    && cursor
                        .last_acknowledged
                        .is_none_or(|acknowledged| acknowledged < last.sequence)
                {
                    return Err(ClusterStoreError::InvalidAcknowledgement);
                }
                if cursor.reply_acknowledged {
                    return Ok(StoreMutation::AlreadyStored);
                }
                cursor.reply_acknowledged = true;
                Ok(StoreMutation::Stored)
            },
        )
    }

    ///
    /// # Parameters
    /// - `address` (`ClusterAddress`): Stable destination address of the entity receiving this request.
    /// - `request` (`ClusterRequestId`): Typed request submitted to a handler or transport.
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
    ) -> Effect<bool, Self::Error> {
        self.transact(
            (address, request, observer),
            |state, (address, request, observer)| {
                let record = state
                    .records
                    .get_mut(&(address, request))
                    .ok_or(ClusterStoreError::MissingRequest)?;
                Ok(record.observers.remove(&observer).is_some())
            },
        )
    }

    ///
    /// # Parameters
    /// - `now` (`UtcMillis`): The caller-supplied current time.
    ///
    /// # Returns
    /// Type: `Effect<usize, Self::Error>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    fn cleanup(&self, now: UtcMillis) -> Effect<usize, Self::Error> {
        self.transact(now, |state, now| {
            let before = state.records.len();
            state.records.retain(|_, record| !expiry(record, now));
            Ok(before - state.records.len())
        })
    }
}

impl HasClusterMessageStore for Arc<MemoryClusterMessageStore> {
    /// Type alias for store in cluster memory store.
    type Store = MemoryClusterMessageStore;
    ///
    /// # Returns
    /// Type: `Arc<Self::Store>`.
    /// Returns `Arc<Self::Store>`.
    fn cluster_message_store(&self) -> Arc<Self::Store> {
        Arc::clone(self)
    }
}

impl ClusterActivationStore for MemoryClusterMessageStore {
    /// Type alias for error in cluster memory store.
    type Error = ClusterStoreError;

    ///
    /// # Parameters
    /// - `address` (`ClusterAddress`): Stable destination address of the entity receiving this request.
    /// - `activation` (`ActivationId`): The activation supplied to this operation.
    /// - `now` (`UtcMillis`): The caller-supplied current time.
    /// - `lease_until` (`UtcMillis`): Instant at which the claim or activation lease expires.
    ///
    /// # Returns
    /// Type: `Effect<ActivationOutcome, Self::Error>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    fn activate(
        &self,
        address: ClusterAddress,
        activation: ActivationId,
        now: UtcMillis,
        lease_until: UtcMillis,
    ) -> Effect<ActivationOutcome, Self::Error> {
        self.transact(
            (address, activation, now, lease_until),
            |state, (address, activation, now, lease_until)| {
                if lease_until <= now {
                    return Err(ClusterStoreError::LeaseNotAfterClaim);
                }
                if let Some(current) = state.current_owner.get(&address)
                    && current.phase == ActivationPhase::Active
                    && now < current.lease.lease_until
                {
                    return if current.lease.owner.activation == activation {
                        Ok(ActivationOutcome::AlreadyActive(current.lease.clone()))
                    } else {
                        Ok(ActivationOutcome::Busy(current.lease.clone()))
                    };
                }
                let generation = match state.current_owner.get(&address) {
                    None => NonZeroU64::MIN,
                    Some(current) => current
                        .lease
                        .owner
                        .generation
                        .value()
                        .get()
                        .checked_add(NonZeroU64::MIN.get())
                        .and_then(NonZeroU64::new)
                        .ok_or(ClusterStoreError::ActivationGenerationExhausted)?,
                };
                let recoverable_claims = state
                    .records
                    .iter()
                    .filter(|((candidate, _), record)| {
                        candidate == &address && matches!(&record.state, InternalState::Claimed(_))
                    })
                    .count();
                let lease = ActivationLease {
                    owner: ClusterOwner {
                        address: address.clone(),
                        activation,
                        generation: OwnershipGeneration::new(generation),
                    },
                    lease_until,
                };
                state.current_owner.insert(
                    address,
                    ActivationStatus {
                        lease: lease.clone(),
                        phase: ActivationPhase::Active,
                    },
                );
                Ok(ActivationOutcome::Activated {
                    lease,
                    recoverable_claims,
                })
            },
        )
    }

    ///
    /// # Parameters
    /// - `owner` (`ClusterOwner`): Worker or scope identity used to fence stale operations.
    /// - `now` (`UtcMillis`): The caller-supplied current time.
    /// - `lease_until` (`UtcMillis`): Instant at which the claim or activation lease expires.
    ///
    /// # Returns
    /// Type: `Effect<ActivationLease, Self::Error>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    fn renew(
        &self,
        owner: ClusterOwner,
        now: UtcMillis,
        lease_until: UtcMillis,
    ) -> Effect<ActivationLease, Self::Error> {
        self.transact(
            (owner, now, lease_until),
            |state, (owner, now, lease_until)| {
                let status = state
                    .current_owner
                    .get_mut(&owner.address)
                    .ok_or(ClusterStoreError::WrongOwner)?;
                if status.phase != ActivationPhase::Active || status.lease.owner != owner {
                    return Err(ClusterStoreError::WrongOwner);
                }
                if now >= status.lease.lease_until {
                    return Err(ClusterStoreError::ActivationLeaseExpired);
                }
                if lease_until <= status.lease.lease_until {
                    return Err(ClusterStoreError::LeaseRegression);
                }
                status.lease.lease_until = lease_until;
                Ok(status.lease.clone())
            },
        )
    }

    ///
    /// # Parameters
    /// - `owner` (`ClusterOwner`): Worker or scope identity used to fence stale operations.
    /// - `now` (`UtcMillis`): The caller-supplied current time.
    ///
    /// # Returns
    /// Type: `Effect<PassivationOutcome, Self::Error>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    fn passivate(
        &self,
        owner: ClusterOwner,
        now: UtcMillis,
    ) -> Effect<PassivationOutcome, Self::Error> {
        self.transact((owner, now), |state, (owner, now)| {
            let status = state
                .current_owner
                .get(&owner.address)
                .ok_or(ClusterStoreError::WrongOwner)?;
            if status.lease.owner != owner {
                return Err(ClusterStoreError::WrongOwner);
            }
            if status.phase == ActivationPhase::Passivated {
                return Ok(PassivationOutcome::AlreadyPassivated);
            }
            if status.phase != ActivationPhase::Active {
                return Err(ClusterStoreError::WrongOwner);
            }
            if now >= status.lease.lease_until {
                return Err(ClusterStoreError::ActivationLeaseExpired);
            }
            let outstanding = state
                .records
                .iter()
                .filter(|((address, _), record)| {
                    address == &owner.address
                        && matches!(&record.state, InternalState::Claimed(token)
                            if token.owner == owner && now < token.lease_until)
                })
                .count();
            if outstanding != 0 {
                return Ok(PassivationOutcome::OutstandingClaims { count: outstanding });
            }
            if let Some(status) = state.current_owner.get_mut(&owner.address) {
                status.phase = ActivationPhase::Passivated;
            }
            Ok(PassivationOutcome::Passivated)
        })
    }

    ///
    /// # Parameters
    /// - `owner` (`ClusterOwner`): Worker or scope identity used to fence stale operations.
    /// - `now` (`UtcMillis`): The caller-supplied current time.
    ///
    /// # Returns
    /// Type: `Effect<bool, Self::Error>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    fn abandon(&self, owner: ClusterOwner, now: UtcMillis) -> Effect<bool, Self::Error> {
        self.transact((owner, now), |state, (owner, now)| {
            let status = state
                .current_owner
                .get_mut(&owner.address)
                .ok_or(ClusterStoreError::WrongOwner)?;
            if status.lease.owner != owner {
                return Err(ClusterStoreError::WrongOwner);
            }
            if status.phase == ActivationPhase::Abandoned || now >= status.lease.lease_until {
                return Ok(false);
            }
            status.phase = ActivationPhase::Abandoned;
            Ok(true)
        })
    }

    ///
    /// # Parameters
    /// - `address` (`ClusterAddress`): Stable destination address of the entity receiving this request.
    ///
    /// # Returns
    /// Type: `Effect<Option<ActivationStatus>, Self::Error>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    fn inspect_activation(
        &self,
        address: ClusterAddress,
    ) -> Effect<Option<ActivationStatus>, Self::Error> {
        self.transact(address, |state, address| {
            Ok(state.current_owner.get(&address).cloned())
        })
    }
}

impl HasClusterActivationStore for Arc<MemoryClusterMessageStore> {
    /// Type alias for store in cluster memory store.
    type Store = MemoryClusterMessageStore;
    ///
    /// # Returns
    /// Type: `Arc<Self::Store>`.
    /// Returns `Arc<Self::Store>`.
    fn cluster_activation_store(&self) -> Arc<Self::Store> {
        Arc::clone(self)
    }
}
