//! Atomic in-memory queue claims and settlement.

use super::super::persisted_queue::{
    HasPersistedQueueStore, MemoryQueueStoreError, PersistedQueueName, PersistedQueueStore,
    QueueClaim, QueueClaimOptions, QueueClaimToken, QueueDeadLetterReason, QueueMessageId,
    QueueOffer, QueueOfferOutcome, QueueRecord, QueueRecordState, QueueRequeue,
    QueueRequeueAttempts, QueueRequeueOutcome, QueueScan, QueueSettlement, QueueSettlementAction,
    QueueStoreCapacity,
};
use crate::{Effect, Layer, UtcMillis};
use std::collections::BTreeMap;
use std::num::{NonZeroU32, NonZeroU64};
use std::sync::{Arc, Mutex};

/// Type alias for address in persistence internal queue delivery.
type Address = (PersistedQueueName, QueueMessageId);

/// Typed state used by persistence internal queue delivery.
struct State {
    records: BTreeMap<Address, QueueRecord>,
    next_sequence: u64,
    next_generation: u64,
    capacity: QueueStoreCapacity,
}

/// Volatile but contract-complete atomic message store. Every state change
/// linearizes under one short gate; no user callback or await runs under it.
pub struct MemoryPersistedQueueStore {
    state: Arc<Mutex<State>>,
}

impl Clone for MemoryPersistedQueueStore {
    ///
    /// # Returns
    /// Type: `Self`.
    fn clone(&self) -> Self {
        Self {
            state: Arc::clone(&self.state),
        }
    }
}

impl HasPersistedQueueStore for Arc<MemoryPersistedQueueStore> {
    /// Type alias for store in persistence internal queue delivery.
    type Store = MemoryPersistedQueueStore;

    ///
    /// # Returns
    /// Type: `Arc<Self::Store>`.
    /// Returns `Arc<Self::Store>`.
    fn persisted_queue_store(&self) -> Arc<Self::Store> {
        Arc::clone(self)
    }
}

impl MemoryPersistedQueueStore {
    #[must_use]
    /// Construct a `MemoryPersistedQueueStore` from its required inputs and explicit policies.
    ///
    /// # Parameters
    /// - `capacity` (`QueueStoreCapacity`): The caller-supplied capacity bound.
    ///
    /// # Returns
    /// Type: `Self`.
    pub fn new(capacity: QueueStoreCapacity) -> Self {
        Self {
            state: Arc::new(Mutex::new(State {
                records: BTreeMap::new(),
                next_sequence: 0,
                next_generation: 0,
                capacity,
            })),
        }
    }

    #[must_use]
    /// Build a scoped dependency layer for this service.
    ///
    /// # Parameters
    /// - `capacity` (`QueueStoreCapacity`): The caller-supplied capacity bound.
    ///
    /// # Returns
    /// Type: `Layer<(), MemoryQueueStoreError, Arc<Self>>`.
    pub fn layer(capacity: QueueStoreCapacity) -> Layer<(), MemoryQueueStoreError, Arc<Self>> {
        Layer::make_with(move || Arc::new(Self::new(capacity)))
    }

    ///
    /// # Parameters
    /// - `state` (`&State`): Current or replacement state owned by this operation.
    ///
    /// # Returns
    /// Type: `bool`.
    /// Returns whether the requested condition holds.
    fn fits(state: &State) -> bool {
        match state.capacity {
            QueueStoreCapacity::Unbounded => true,
            QueueStoreCapacity::Bounded(maximum) => state.records.len() < maximum.get(),
        }
    }
}

/// Typed candidate used by persistence internal queue delivery.
struct Candidate {
    address: Address,
    available_at: UtcMillis,
    sequence: NonZeroU64,
    exhausted: bool,
}

impl PersistedQueueStore for MemoryPersistedQueueStore {
    /// Type alias for error in persistence internal queue delivery.
    type Error = MemoryQueueStoreError;

    ///
    /// # Parameters
    /// - `offer` (`QueueOffer`): The offer supplied to this operation.
    ///
    /// # Returns
    /// Type: `Effect<QueueOfferOutcome, Self::Error>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    fn offer(&self, offer: QueueOffer) -> Effect<QueueOfferOutcome, Self::Error> {
        let state = Arc::clone(&self.state);
        Effect::try_sync_once(
            move || {
                let (queue, id, payload, policy, offered_at, available_at) = offer.into_parts();
                let address = (queue, id.clone());
                let (outcome, removed) = {
                    let mut state = state
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    let old = state.records.get(&address);
                    if let Some(record) = old {
                        let retention_elapsed = matches!(
                            &record.state,
                            QueueRecordState::Acknowledged { retained_until }
                                if *retained_until <= offered_at
                        );
                        if !retention_elapsed {
                            if record.payload.as_ref() != payload.as_ref()
                                || record.policy != policy
                            {
                                return Ok(QueueOfferOutcome::ConflictingId);
                            }
                            return Ok(QueueOfferOutcome::Duplicate {
                                disposition: record.state.disposition(),
                            });
                        }
                    } else if !Self::fits(&state) {
                        return Err(MemoryQueueStoreError::CapacityExceeded);
                    }
                    let next = state
                        .next_sequence
                        .checked_add(1)
                        .ok_or(MemoryQueueStoreError::SequenceOverflow)?;
                    let sequence =
                        NonZeroU64::new(next).ok_or(MemoryQueueStoreError::SequenceOverflow)?;
                    state.next_sequence = next;
                    let record = QueueRecord {
                        id,
                        payload,
                        policy,
                        sequence,
                        offered_at,
                        available_at,
                        attempts: 0,
                        state: QueueRecordState::Ready,
                    };
                    let removed = state.records.insert(address, record);
                    (QueueOfferOutcome::Accepted { sequence }, removed)
                };
                drop(removed);
                Ok(outcome)
            },
            std::convert::identity,
        )
    }

    ///
    /// # Parameters
    /// - `queue` (`PersistedQueueName`): Queue definition or store that owns the selected messages.
    /// - `options` (`QueueClaimOptions`): Caller-selected bounds and behavior for this operation.
    ///
    /// # Returns
    /// Type: `Effect<Vec<QueueClaim>, Self::Error>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    fn claim(
        &self,
        queue: PersistedQueueName,
        options: QueueClaimOptions,
    ) -> Effect<Vec<QueueClaim>, Self::Error> {
        let state = Arc::clone(&self.state);
        Effect::try_sync_once(
            move || {
                let now = options.now();
                let (claims, discarded) = {
                    let mut state = state
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    let mut candidates = Vec::new();
                    for (address, record) in &state.records {
                        if address.0 != queue || record.available_at > now {
                            continue;
                        }
                        let due = match &record.state {
                            QueueRecordState::Ready => true,
                            QueueRecordState::Claimed { lease_until, .. } => *lease_until <= now,
                            QueueRecordState::Acknowledged { .. }
                            | QueueRecordState::DeadLetter { .. } => false,
                        };
                        if due {
                            candidates.push(Candidate {
                                address: address.clone(),
                                available_at: record.available_at,
                                sequence: record.sequence,
                                exhausted: record.attempts >= record.policy.maximum_attempts.get(),
                            });
                        }
                    }
                    candidates.sort_unstable_by_key(|candidate| {
                        (candidate.available_at, candidate.sequence)
                    });
                    let claimable = candidates
                        .iter()
                        .filter(|item| !item.exhausted)
                        .take(options.limit().get())
                        .count();
                    let count = u64::try_from(claimable)
                        .map_err(|_| MemoryQueueStoreError::GenerationOverflow)?;
                    state
                        .next_generation
                        .checked_add(count)
                        .ok_or(MemoryQueueStoreError::GenerationOverflow)?;
                    let mut next_generation = state.next_generation;
                    let mut claims = Vec::new();
                    let mut discarded = Vec::new();
                    for candidate in candidates {
                        let record = state
                            .records
                            .get_mut(&candidate.address)
                            .ok_or(MemoryQueueStoreError::InconsistentState)?;
                        if candidate.exhausted {
                            let previous = std::mem::replace(
                                &mut record.state,
                                QueueRecordState::DeadLetter {
                                    reason: QueueDeadLetterReason::AttemptsExhausted,
                                },
                            );
                            discarded.push(previous);
                            continue;
                        }
                        if claims.len() == options.limit().get() {
                            continue;
                        }
                        next_generation = next_generation
                            .checked_add(1)
                            .ok_or(MemoryQueueStoreError::GenerationOverflow)?;
                        let generation = NonZeroU64::new(next_generation)
                            .ok_or(MemoryQueueStoreError::GenerationOverflow)?;
                        let attempt = record
                            .attempts
                            .checked_add(1)
                            .and_then(NonZeroU32::new)
                            .ok_or(MemoryQueueStoreError::AttemptOverflow)?;
                        let token = QueueClaimToken::new(
                            queue.clone(),
                            record.id.clone(),
                            options.owner().clone(),
                            generation,
                        );
                        let previous = std::mem::replace(
                            &mut record.state,
                            QueueRecordState::Claimed {
                                token: token.clone(),
                                claimed_at: now,
                                lease_until: options.lease_until(),
                            },
                        );
                        discarded.push(previous);
                        record.attempts = attempt.get();
                        claims.push(QueueClaim {
                            token,
                            payload: Arc::clone(&record.payload),
                            attempt,
                            offered_at: record.offered_at,
                            available_at: record.available_at,
                            claimed_at: now,
                            lease_until: options.lease_until(),
                        });
                    }
                    state.next_generation = next_generation;
                    (claims, discarded)
                };
                drop(discarded);
                Ok(claims)
            },
            std::convert::identity,
        )
    }

    ///
    /// # Parameters
    /// - `token` (`QueueClaimToken`): Fencing token that identifies the current claim or owner.
    /// - `action` (`QueueSettlementAction`): State transition selected for this operation.
    ///
    /// # Returns
    /// Type: `Effect<QueueSettlement, Self::Error>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    fn settle(
        &self,
        token: QueueClaimToken,
        action: QueueSettlementAction,
    ) -> Effect<QueueSettlement, Self::Error> {
        let state = Arc::clone(&self.state);
        Effect::try_sync_once(
            move || {
                let at = action.at();
                let address = (token.queue().clone(), token.id().clone());
                let (result, removed) = {
                    let mut state = state
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    let Some(record) = state.records.get_mut(&address) else {
                        return Ok(QueueSettlement::Missing);
                    };
                    let QueueRecordState::Claimed {
                        token: current,
                        claimed_at,
                        lease_until,
                    } = &record.state
                    else {
                        return Ok(QueueSettlement::StaleClaim);
                    };
                    if current != &token || at < *claimed_at || at >= *lease_until {
                        return Ok(QueueSettlement::StaleClaim);
                    }
                    let (next, next_available) = match action {
                        QueueSettlementAction::Acknowledge { at } => {
                            let retained_until = at
                                .checked_add(record.policy.acknowledged_retention)
                                .map_err(MemoryQueueStoreError::Time)?;
                            (QueueRecordState::Acknowledged { retained_until }, None)
                        }
                        QueueSettlementAction::Retry(retry) => {
                            (QueueRecordState::Ready, Some(retry.available_at()))
                        }
                        QueueSettlementAction::DeadLetter { reason, .. } => {
                            (QueueRecordState::DeadLetter { reason }, None)
                        }
                    };
                    if let Some(available_at) = next_available {
                        record.available_at = available_at;
                    }
                    let previous = std::mem::replace(&mut record.state, next);
                    (QueueSettlement::Applied, previous)
                };
                drop(removed);
                Ok(result)
            },
            std::convert::identity,
        )
    }

    ///
    /// # Parameters
    /// - `queue` (`PersistedQueueName`): Queue definition or store that owns the selected messages.
    /// - `id` (`QueueMessageId`): Stable identity of the selected record or operation.
    ///
    /// # Returns
    /// Type: `Effect<Option<QueueRecord>, Self::Error>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    fn inspect(
        &self,
        queue: PersistedQueueName,
        id: QueueMessageId,
    ) -> Effect<Option<QueueRecord>, Self::Error> {
        let state = Arc::clone(&self.state);
        Effect::sync_once(move || {
            state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .records
                .get(&(queue, id))
                .cloned()
        })
    }

    ///
    /// # Parameters
    /// - `queue` (`PersistedQueueName`): Queue definition or store that owns the selected messages.
    /// - `scan` (`QueueScan`): Bounded request for records in a selected state.
    ///
    /// # Returns
    /// Type: `Effect<Vec<QueueRecord>, Self::Error>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    fn dead_letters(
        &self,
        queue: PersistedQueueName,
        scan: QueueScan,
    ) -> Effect<Vec<QueueRecord>, Self::Error> {
        let state = Arc::clone(&self.state);
        Effect::sync_once(move || {
            let state = state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let mut records: Vec<_> = state
                .records
                .iter()
                .filter(|((name, _), record)| {
                    *name == queue
                        && matches!(&record.state, QueueRecordState::DeadLetter { .. })
                        && scan
                            .after_sequence
                            .is_none_or(|cursor| record.sequence > cursor)
                })
                .map(|(_, record)| record)
                .collect();
            records.sort_unstable_by_key(|record| record.sequence);
            records.truncate(scan.limit.get());
            records.into_iter().cloned().collect()
        })
    }

    ///
    /// # Parameters
    /// - `queue` (`PersistedQueueName`): Queue definition or store that owns the selected messages.
    /// - `id` (`QueueMessageId`): Stable identity of the selected record or operation.
    /// - `request` (`QueueRequeue`): Typed request submitted to a handler or transport.
    ///
    /// # Returns
    /// Type: `Effect<QueueRequeueOutcome, Self::Error>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    fn requeue(
        &self,
        queue: PersistedQueueName,
        id: QueueMessageId,
        request: QueueRequeue,
    ) -> Effect<QueueRequeueOutcome, Self::Error> {
        let state = Arc::clone(&self.state);
        Effect::sync_once(move || {
            let previous = {
                let mut state = state
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                let Some(record) = state.records.get_mut(&(queue, id)) else {
                    return QueueRequeueOutcome::Missing;
                };
                if !matches!(&record.state, QueueRecordState::DeadLetter { .. }) {
                    return QueueRequeueOutcome::NotDeadLetter;
                }
                if request.attempts() == QueueRequeueAttempts::Preserve
                    && record.attempts >= record.policy.maximum_attempts.get()
                {
                    return QueueRequeueOutcome::AttemptsExhausted;
                }
                if request.attempts() == QueueRequeueAttempts::Reset {
                    record.attempts = 0;
                }
                record.available_at = request.available_at();
                std::mem::replace(&mut record.state, QueueRecordState::Ready)
            };
            drop(previous);
            QueueRequeueOutcome::Requeued
        })
    }

    ///
    /// # Parameters
    /// - `queue` (`PersistedQueueName`): Queue definition or store that owns the selected messages.
    /// - `now` (`UtcMillis`): The caller-supplied current time.
    ///
    /// # Returns
    /// Type: `Effect<usize, Self::Error>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    fn cleanup(&self, queue: PersistedQueueName, now: UtcMillis) -> Effect<usize, Self::Error> {
        let state = Arc::clone(&self.state);
        Effect::sync_once(move || {
            let removed = {
                let mut state = state
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                let expired: Vec<_> = state
                    .records
                    .iter()
                    .filter(|((name, _), record)| {
                        *name == queue
                            && matches!(&record.state,
                            QueueRecordState::Acknowledged { retained_until }
                                if *retained_until <= now)
                    })
                    .map(|(address, _)| address.clone())
                    .collect();
                expired
                    .into_iter()
                    .filter_map(|address| state.records.remove(&address))
                    .collect::<Vec<_>>()
            };
            let count = removed.len();
            drop(removed);
            count
        })
    }
}
