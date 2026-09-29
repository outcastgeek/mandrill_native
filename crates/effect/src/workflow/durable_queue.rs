//! Persisted-queue handoff to workflow completion.

use super::{
    DurableDeferred, DurableSignalError, DurableWorkflowEngine, HasWorkflowEngine, WorkflowEngine,
    WorkflowRunId,
};
use crate::persistence::{
    DecodedQueueClaim, DurablePersistedQueueStore, HasPersistedQueueStore, PersistedQueue,
    PersistedQueueError, PersistedQueueStore, QueueClaim, QueueClaimOptions, QueueClaimToken,
    QueueDeadLetterReason, QueueDecodeFailure, QueueOfferOutcome, QueueRecord, QueueRequeue,
    QueueRequeueOutcome, QueueRetry, QueueScan, QueueSettlement, QueueSettlementAction,
};
use crate::{Effect, KeyDomain, StableKey, UtcMillis};
use std::marker::PhantomData;

#[derive(Debug)]
/// A queue or workflow-engine failure in result-before-ack composition.
///
/// # Type parameters
/// - `QueueError`: Error returned by the persisted queue store.
/// - `EngineError`: Error returned by the workflow engine.
pub enum DurableQueueError<QueueError, EngineError> {
    /// The selected queue rejected or failed the operation. Retains the `PersistedQueueError<QueueError>` detail.
    Queue(PersistedQueueError<QueueError>),
    /// Recording or waiting for the named signal failed. Retains the `DurableSignalError<EngineError>` detail.
    Signal(DurableSignalError<EngineError>),
    /// The run identity differs from the required or recorded value.
    RunIdentityMismatch,
    /// The supplied queue does not match the required or recorded value.
    WrongQueue,
    /// The supplied claim token no longer owns this message.
    StaleClaim,
    /// The required claim is missing.
    MissingClaim,
    /// The workflow result has not been recorded, so acknowledgement is unsafe.
    ResultNotRecorded,
}

#[derive(Debug)]
/// A worker failure while claiming, recording, or settling a durable queued job.
///
/// # Type parameters
/// - `QueueError`: Error returned by the persisted queue store.
/// - `EngineError`: Error returned by the workflow engine.
/// - `HandlerError`: Error produced by the registered handler.
pub enum DurableQueueWorkerError<QueueError, EngineError, HandlerError> {
    /// The selected queue rejected or failed the operation. Retains the `DurableQueueError<QueueError, EngineError>` detail.
    Queue(DurableQueueError<QueueError, EngineError>),
    /// Decoding the stored or transported value failed. Retains the `QueueDecodeFailure` detail.
    Decode(QueueDecodeFailure),
    /// The registered typed handler failed. Retains the `HandlerError` detail.
    Handler(HandlerError),
}

/// The queue and workflow engine remain independently selected through `R`.
///
/// # Type parameters
/// - `A`: Application value type.
/// - `R`: Environment type.
pub type DurableQueueEffect<A, R> = Effect<
    A,
    DurableQueueError<
        <<R as HasPersistedQueueStore>::Store as PersistedQueueStore>::Error,
        <<R as HasWorkflowEngine>::Engine as WorkflowEngine>::Error,
    >,
    R,
>;

/// Type alias for durable queue worker effect in workflow durable queue.
///
/// # Type parameters
/// - `A`: Application value type.
/// - `HandlerError`: Error produced by the registered handler.
/// - `R`: Environment type.
pub type DurableQueueWorkerEffect<A, HandlerError, R> = Effect<
    A,
    DurableQueueWorkerError<
        <<R as HasPersistedQueueStore>::Store as PersistedQueueStore>::Error,
        <<R as HasWorkflowEngine>::Engine as WorkflowEngine>::Error,
        HandlerError,
    >,
    R,
>;

/// A persisted queue composed with a workflow result so recorded completion precedes acknowledgement.
///
/// # Type parameters
/// - `P`: Payload type. Bound or type: `Send + 'static`.
/// - `D`: Identity-domain type. Bound or type: `KeyDomain`.
pub struct DurableQueue<P: Send + 'static, D: KeyDomain> {
    queue: PersistedQueue<P, D>,
}

impl<P: Send + 'static, D: KeyDomain> DurableQueue<P, D> {
    #[must_use]
    /// Construct a `DurableQueue` from its required inputs and explicit policies.
    ///
    /// # Type parameters
    /// - `D`: Stable-key domain type.
    /// - `P`: Payload type.
    ///
    /// # Parameters
    /// - `queue` (`PersistedQueue<P, D>`): Queue definition or store that owns the selected messages.
    ///
    /// # Returns
    /// Type: `Self`.
    pub fn new(queue: PersistedQueue<P, D>) -> Self {
        Self { queue }
    }

    #[must_use]
    /// Select the queue associated with this definition.
    ///
    /// # Type parameters
    /// - `D`: Stable-key domain type.
    /// - `P`: Payload type.
    ///
    /// # Returns
    /// Type: `&PersistedQueue<P, D>`.
    pub fn queue(&self) -> &PersistedQueue<P, D> {
        &self.queue
    }

    /// Require physical recovery guarantees for both records involved in a
    /// queue-to-workflow handoff. The environment is fixed for every operation
    /// on the returned value; no core memory provider meets these bounds.
    #[must_use]
    /// # Type parameters
    /// - `R`: Environment type. Additional bound: `HasPersistedQueueStore + HasWorkflowEngine + Send + Sync + 'static`.
    /// - `D`: Stable-key domain type.
    /// - `P`: Payload type.
    ///
    /// # Returns
    /// Type: `DurableRequiredQueue<P, D, R>`.
    pub fn require_physical_durability<R>(self) -> DurableRequiredQueue<P, D, R>
    where
        R: HasPersistedQueueStore + HasWorkflowEngine + Send + Sync + 'static,
        <R as HasPersistedQueueStore>::Store: DurablePersistedQueueStore,
        <R as HasWorkflowEngine>::Engine: DurableWorkflowEngine,
    {
        DurableRequiredQueue {
            inner: self,
            environment: PhantomData,
        }
    }

    /// Lease eligible work to the supplied owner under a fenced claim token.
    ///
    /// # Type parameters
    /// - `P`: Payload type. Bound: `Send + 'static`.
    /// - `D`: Identity-domain type. Bound: `KeyDomain`.
    /// - `R`: Environment type. Additional bound: `HasPersistedQueueStore + HasWorkflowEngine + Send + Sync + 'static`.
    ///
    /// # Parameters
    /// - `options` (`QueueClaimOptions`): Caller-selected bounds and behavior for this operation.
    ///
    /// # Returns
    /// Type: `DurableQueueEffect<Vec<QueueClaim>, R>`.
    pub fn claim<R>(&self, options: QueueClaimOptions) -> DurableQueueEffect<Vec<QueueClaim>, R>
    where
        R: HasPersistedQueueStore + HasWorkflowEngine + Send + Sync + 'static,
    {
        self.queue
            .claim(options)
            .map_error(DurableQueueError::Queue)
    }

    /// Decode a claimed payload while retaining its settlement token.
    ///
    /// # Type parameters
    /// - `D`: Identity-domain type. Bound: `KeyDomain`.
    /// - `P`: Payload type.
    ///
    /// # Parameters
    /// - `claim` (`QueueClaim`): Fenced ownership token for the selected message.
    ///
    /// # Returns
    /// Type: `Result<DecodedQueueClaim<P>, QueueDecodeFailure>`.
    pub fn decode_claim(
        &self,
        claim: QueueClaim,
    ) -> Result<DecodedQueueClaim<P>, QueueDecodeFailure> {
        self.queue.decode_claim(claim)
    }

    /// Read recorded details without changing the authoritative state.
    ///
    /// # Type parameters
    /// - `P`: Payload type. Bound: `Send + 'static`.
    /// - `R`: Environment type. Additional bound: `HasPersistedQueueStore + HasWorkflowEngine + Send + Sync + 'static`.
    /// - `D`: Stable-key domain type.
    ///
    /// # Parameters
    /// - `id` (`StableKey<D>`): Stable identity of the selected record or operation.
    ///
    /// # Returns
    /// Type: `DurableQueueEffect<Option<QueueRecord>, R>`.
    pub fn inspect<R>(&self, id: StableKey<D>) -> DurableQueueEffect<Option<QueueRecord>, R>
    where
        R: HasPersistedQueueStore + HasWorkflowEngine + Send + Sync + 'static,
    {
        self.queue.inspect(id).map_error(DurableQueueError::Queue)
    }

    /// Inspect messages retained after ordinary delivery was exhausted.
    ///
    /// # Type parameters
    /// - `P`: Payload type. Bound: `Send + 'static`.
    /// - `D`: Identity-domain type. Bound: `KeyDomain`.
    /// - `R`: Environment type. Additional bound: `HasPersistedQueueStore + HasWorkflowEngine + Send + Sync + 'static`.
    ///
    /// # Parameters
    /// - `scan` (`QueueScan`): Bounded request for records in a selected state.
    ///
    /// # Returns
    /// Type: `DurableQueueEffect<Vec<QueueRecord>, R>`.
    pub fn dead_letters<R>(&self, scan: QueueScan) -> DurableQueueEffect<Vec<QueueRecord>, R>
    where
        R: HasPersistedQueueStore + HasWorkflowEngine + Send + Sync + 'static,
    {
        self.queue
            .dead_letters(scan)
            .map_error(DurableQueueError::Queue)
    }

    /// Make a retained message eligible for delivery again under the supplied policy.
    ///
    /// # Type parameters
    /// - `P`: Payload type. Bound: `Send + 'static`.
    /// - `R`: Environment type. Additional bound: `HasPersistedQueueStore + HasWorkflowEngine + Send + Sync + 'static`.
    /// - `D`: Stable-key domain type.
    ///
    /// # Parameters
    /// - `id` (`StableKey<D>`): Stable identity of the selected record or operation.
    /// - `request` (`QueueRequeue`): The typed request to process.
    ///
    /// # Returns
    /// Type: `DurableQueueEffect<QueueRequeueOutcome, R>`.
    pub fn requeue<R>(
        &self,
        id: StableKey<D>,
        request: QueueRequeue,
    ) -> DurableQueueEffect<QueueRequeueOutcome, R>
    where
        R: HasPersistedQueueStore + HasWorkflowEngine + Send + Sync + 'static,
    {
        self.queue
            .requeue(id, request)
            .map_error(DurableQueueError::Queue)
    }

    /// Describe another attempt under the supplied retry policy.
    ///
    /// # Type parameters
    /// - `P`: Payload type. Bound: `Send + 'static`.
    /// - `D`: Identity-domain type. Bound: `KeyDomain`.
    /// - `R`: Environment type. Additional bound: `HasPersistedQueueStore + HasWorkflowEngine + Send + Sync + 'static`.
    ///
    /// # Parameters
    /// - `claim` (`QueueClaimToken`): Fenced ownership token for the selected message.
    /// - `retry` (`QueueRetry`): The retry supplied to this operation.
    ///
    /// # Returns
    /// Type: `DurableQueueEffect<QueueSettlement, R>`.
    pub fn retry<R>(
        &self,
        claim: QueueClaimToken,
        retry: QueueRetry,
    ) -> DurableQueueEffect<QueueSettlement, R>
    where
        R: HasPersistedQueueStore + HasWorkflowEngine + Send + Sync + 'static,
    {
        self.queue
            .settle(claim, QueueSettlementAction::Retry(retry))
            .map_error(DurableQueueError::Queue)
    }

    /// Retain the selected message outside ordinary delivery with a recorded reason.
    ///
    /// # Type parameters
    /// - `P`: Payload type. Bound: `Send + 'static`.
    /// - `D`: Identity-domain type. Bound: `KeyDomain`.
    /// - `R`: Environment type. Additional bound: `HasPersistedQueueStore + HasWorkflowEngine + Send + Sync + 'static`.
    ///
    /// # Parameters
    /// - `claim` (`QueueClaimToken`): Fenced ownership token for the selected message.
    /// - `at` (`UtcMillis`): The caller-supplied time of this operation.
    /// - `reason` (`QueueDeadLetterReason`): The reason supplied to this operation.
    ///
    /// # Returns
    /// Type: `DurableQueueEffect<QueueSettlement, R>`.
    pub fn dead_letter<R>(
        &self,
        claim: QueueClaimToken,
        at: UtcMillis,
        reason: QueueDeadLetterReason,
    ) -> DurableQueueEffect<QueueSettlement, R>
    where
        R: HasPersistedQueueStore + HasWorkflowEngine + Send + Sync + 'static,
    {
        self.queue
            .settle(claim, QueueSettlementAction::DeadLetter { at, reason })
            .map_error(DurableQueueError::Queue)
    }

    /// Execute one decoded delivery. A handler failure leaves the fenced
    /// claim unsettled so the caller can explicitly retry or dead-letter it.
    /// The signal is recorded before the claim is acknowledged.
    ///
    /// # Type parameters
    /// - `D`: Identity-domain type. Bound: `KeyDomain`.
    /// - `R`: Environment type. Additional bound: `HasPersistedQueueStore + HasWorkflowEngine + Send + Sync + 'static`.
    /// - `T`: Value or item type. Additional bound: `Send + 'static`.
    /// - `HandlerError`: Error produced by the registered handler. Additional bound: `Send + 'static`.
    /// - `P`: Payload type.
    ///
    /// # Parameters
    /// - `delivery` (`DecodedQueueClaim<P>`): The delivery supplied to this operation.
    /// - `deferred` (`DurableDeferred<T>`): One-shot result awaited or completed by this operation.
    /// - `handler` (`impl Fn(P) -> Effect<T, HandlerError, R> + Send + Sync + 'static`): The handler invoked for the matching request or event.
    /// - `acknowledged_at` (`UtcMillis`): Instant at which delivery was confirmed.
    ///
    /// # Returns
    /// Type: `DurableQueueWorkerEffect<QueueSettlement, HandlerError, R>`.
    pub fn handle<R, T, HandlerError>(
        &self,
        delivery: DecodedQueueClaim<P>,
        deferred: DurableDeferred<T>,
        handler: impl Fn(P) -> Effect<T, HandlerError, R> + Send + Sync + 'static,
        acknowledged_at: UtcMillis,
    ) -> DurableQueueWorkerEffect<QueueSettlement, HandlerError, R>
    where
        T: Send + 'static,
        HandlerError: Send + 'static,
        R: HasPersistedQueueStore + HasWorkflowEngine + Send + Sync + 'static,
    {
        if delivery.claim.token.queue() != self.queue.name() {
            return Effect::fail_with(|| {
                DurableQueueWorkerError::Queue(DurableQueueError::WrongQueue)
            });
        }
        if delivery.claim.token.id().as_bytes() != deferred.run.as_bytes() {
            return Effect::fail_with(|| {
                DurableQueueWorkerError::Queue(DurableQueueError::RunIdentityMismatch)
            });
        }
        let queue = Self::new(self.queue.clone());
        let token = delivery.claim.token;
        Effect::sync_once(move || delivery.payload).flat_map(move |payload| {
            let queue = Self::new(queue.queue.clone());
            let token = token.clone();
            let deferred = deferred.clone();
            handler(payload)
                .map_error(DurableQueueWorkerError::Handler)
                .flat_map(move |result| {
                    queue
                        .complete(token.clone(), deferred.clone(), result, acknowledged_at)
                        .map_error(DurableQueueWorkerError::Queue)
                })
        })
    }

    /// Decode a raw claim and run its handler. A decoding failure retains
    /// the original claim and fence for explicit dead-letter handling.
    /// After an uncertain acknowledgement, call `acknowledge_recorded` on
    /// redelivery before choosing to execute the handler again.
    ///
    /// # Type parameters
    /// - `D`: Identity-domain type. Bound: `KeyDomain`.
    /// - `R`: Environment type. Additional bound: `HasPersistedQueueStore + HasWorkflowEngine + Send + Sync + 'static`.
    /// - `T`: Value or item type. Additional bound: `Send + 'static`.
    /// - `HandlerError`: Error produced by the registered handler. Additional bound: `Send + 'static`.
    /// - `P`: Payload type.
    ///
    /// # Parameters
    /// - `claim` (`QueueClaim`): Fenced ownership token for the selected message.
    /// - `deferred` (`DurableDeferred<T>`): One-shot result awaited or completed by this operation.
    /// - `handler` (`impl Fn(P) -> Effect<T, HandlerError, R> + Send + Sync + 'static`): The handler invoked for the matching request or event.
    /// - `acknowledged_at` (`UtcMillis`): Instant at which delivery was confirmed.
    ///
    /// # Returns
    /// Type: `DurableQueueWorkerEffect<QueueSettlement, HandlerError, R>`.
    pub fn handle_claim<R, T, HandlerError>(
        &self,
        claim: QueueClaim,
        deferred: DurableDeferred<T>,
        handler: impl Fn(P) -> Effect<T, HandlerError, R> + Send + Sync + 'static,
        acknowledged_at: UtcMillis,
    ) -> DurableQueueWorkerEffect<QueueSettlement, HandlerError, R>
    where
        T: Send + 'static,
        HandlerError: Send + 'static,
        R: HasPersistedQueueStore + HasWorkflowEngine + Send + Sync + 'static,
    {
        match self.decode_claim(claim) {
            Ok(delivery) => self.handle(delivery, deferred, handler, acknowledged_at),
            Err(failure) => Effect::from_result_once(Err(DurableQueueWorkerError::Decode(failure))),
        }
    }

    /// Queue identity and workflow identity are derived from the same stable
    /// key. The deferred signal may arrive before the workflow first waits.
    ///
    /// # Type parameters
    /// - `R`: Environment type. Additional bound: `HasPersistedQueueStore + HasWorkflowEngine + Send + Sync + 'static`.
    /// - `T`: Value or item type. Bounds: `Send + 'static`.
    /// - `D`: Stable-key domain type.
    /// - `P`: Payload type.
    ///
    /// # Parameters
    /// - `id` (`StableKey<D>`): Stable identity of the selected record or operation.
    /// - `payload` (`impl Fn() -> P + Send + Sync + 'static`): Application data moved into the message or stored record.
    /// - `deferred` (`&DurableDeferred<T>`): One-shot result awaited or completed by this operation.
    /// - `offered_at` (`UtcMillis`): The offered at supplied to this operation.
    /// - `available_at` (`UtcMillis`): Instant when the rate-limited action may next be admitted.
    ///
    /// # Returns
    /// Type: `DurableQueueEffect<QueueOfferOutcome, R>`.
    pub fn enqueue<R, T: Send + 'static>(
        &self,
        id: StableKey<D>,
        payload: impl Fn() -> P + Send + Sync + 'static,
        deferred: &DurableDeferred<T>,
        offered_at: UtcMillis,
        available_at: UtcMillis,
    ) -> DurableQueueEffect<QueueOfferOutcome, R>
    where
        R: HasPersistedQueueStore + HasWorkflowEngine + Send + Sync + 'static,
    {
        if deferred.run != WorkflowRunId::from_stable(id.clone()) {
            return Effect::fail_with(|| DurableQueueError::RunIdentityMismatch);
        }
        self.queue
            .offer(id, payload, offered_at, available_at)
            .map_error(DurableQueueError::Queue)
    }

    /// Record the workflow result before acknowledging the queue delivery.
    /// If acknowledgement is uncertain, redelivery uses
    /// `acknowledge_recorded` without repeating the handler's completed work.
    ///
    /// # Type parameters
    /// - `P`: Payload type. Bound: `Send + 'static`.
    /// - `D`: Identity-domain type. Bound: `KeyDomain`.
    /// - `R`: Environment type. Additional bound: `HasPersistedQueueStore + HasWorkflowEngine + Send + Sync + 'static`.
    /// - `T`: Value or item type. Bounds: `Send + 'static`.
    ///
    /// # Parameters
    /// - `claim` (`QueueClaimToken`): Fenced ownership token for the selected message.
    /// - `deferred` (`DurableDeferred<T>`): One-shot result awaited or completed by this operation.
    /// - `result` (`T`): The result to record or return.
    /// - `acknowledged_at` (`UtcMillis`): Instant at which delivery was confirmed.
    ///
    /// # Returns
    /// Type: `DurableQueueEffect<QueueSettlement, R>`.
    pub fn complete<R, T: Send + 'static>(
        &self,
        claim: QueueClaimToken,
        deferred: DurableDeferred<T>,
        result: T,
        acknowledged_at: UtcMillis,
    ) -> DurableQueueEffect<QueueSettlement, R>
    where
        R: HasPersistedQueueStore + HasWorkflowEngine + Send + Sync + 'static,
    {
        if claim.queue() != self.queue.name() {
            return Effect::fail_with(|| DurableQueueError::WrongQueue);
        }
        if claim.id().as_bytes() != deferred.run.as_bytes() {
            return Effect::fail_with(|| DurableQueueError::RunIdentityMismatch);
        }
        let queue = self.queue.clone();
        deferred
            .complete(result)
            .map_error(DurableQueueError::Signal)
            .flat_map(move |_| {
                queue
                    .settle(
                        claim.clone(),
                        QueueSettlementAction::Acknowledge {
                            at: acknowledged_at,
                        },
                    )
                    .map_error(DurableQueueError::Queue)
                    .flat_map(|settlement| match settlement {
                        QueueSettlement::Applied => Effect::sync(|| QueueSettlement::Applied),
                        QueueSettlement::StaleClaim => {
                            Effect::fail_with(|| DurableQueueError::StaleClaim)
                        }
                        QueueSettlement::Missing => {
                            Effect::fail_with(|| DurableQueueError::MissingClaim)
                        }
                    })
            })
    }

    /// On redelivery after an uncertain acknowledgement, inspect the
    /// recorded result and acknowledge without executing the handler again.
    ///
    /// # Type parameters
    /// - `P`: Payload type. Bound: `Send + 'static`.
    /// - `D`: Identity-domain type. Bound: `KeyDomain`.
    /// - `R`: Environment type. Additional bound: `HasPersistedQueueStore + HasWorkflowEngine + Send + Sync + 'static`.
    /// - `T`: Value or item type. Bounds: `Send + 'static`.
    ///
    /// # Parameters
    /// - `claim` (`QueueClaimToken`): Fenced ownership token for the selected message.
    /// - `deferred` (`DurableDeferred<T>`): One-shot result awaited or completed by this operation.
    /// - `acknowledged_at` (`UtcMillis`): Instant at which delivery was confirmed.
    ///
    /// # Returns
    /// Type: `DurableQueueEffect<QueueSettlement, R>`.
    pub fn acknowledge_recorded<R, T: Send + 'static>(
        &self,
        claim: QueueClaimToken,
        deferred: DurableDeferred<T>,
        acknowledged_at: UtcMillis,
    ) -> DurableQueueEffect<QueueSettlement, R>
    where
        R: HasPersistedQueueStore + HasWorkflowEngine + Send + Sync + 'static,
    {
        if claim.queue() != self.queue.name() {
            return Effect::fail_with(|| DurableQueueError::WrongQueue);
        }
        if claim.id().as_bytes() != deferred.run.as_bytes() {
            return Effect::fail_with(|| DurableQueueError::RunIdentityMismatch);
        }
        let queue = self.queue.clone();
        deferred
            .inspect::<R>()
            .map_error(DurableQueueError::Signal)
            .flat_map(move |recorded| match recorded {
                Some(_) => queue
                    .settle(
                        claim.clone(),
                        QueueSettlementAction::Acknowledge {
                            at: acknowledged_at,
                        },
                    )
                    .map_error(DurableQueueError::Queue)
                    .flat_map(|settlement| match settlement {
                        QueueSettlement::Applied => Effect::sync(|| QueueSettlement::Applied),
                        QueueSettlement::StaleClaim => {
                            Effect::fail_with(|| DurableQueueError::StaleClaim)
                        }
                        QueueSettlement::Missing => {
                            Effect::fail_with(|| DurableQueueError::MissingClaim)
                        }
                    }),
                None => Effect::fail_with(|| DurableQueueError::ResultNotRecorded),
            })
    }
}

/// A queue-to-workflow handoff bound to one environment whose providers
/// warrant recovery after their documented restart boundaries. Construction
/// fails to typecheck with the core's volatile memory providers.
///
/// # Type parameters
/// - `P`: Payload type. Bound or type: `Send + 'static`.
/// - `D`: Identity-domain type. Bound or type: `KeyDomain`.
/// - `R`: Environment type.
pub struct DurableRequiredQueue<P: Send + 'static, D: KeyDomain, R> {
    inner: DurableQueue<P, D>,
    environment: PhantomData<fn() -> R>,
}

impl<P: Send + 'static, D: KeyDomain, R> DurableRequiredQueue<P, D, R>
where
    R: HasPersistedQueueStore + HasWorkflowEngine + Send + Sync + 'static,
    <R as HasPersistedQueueStore>::Store: DurablePersistedQueueStore,
    <R as HasWorkflowEngine>::Engine: DurableWorkflowEngine,
{
    /// Lease eligible work to the supplied owner under a fenced claim token.
    ///
    /// # Type parameters
    /// - `P`: Payload type. Bound: `Send + 'static`.
    /// - `D`: Identity-domain type. Bound: `KeyDomain`.
    /// - `R`: Environment type.
    ///
    /// # Parameters
    /// - `options` (`QueueClaimOptions`): Caller-selected bounds and behavior for this operation.
    ///
    /// # Returns
    /// Type: `DurableQueueEffect<Vec<QueueClaim>, R>`.
    pub fn claim(&self, options: QueueClaimOptions) -> DurableQueueEffect<Vec<QueueClaim>, R> {
        self.inner.claim(options)
    }

    /// Decode a claimed payload while retaining its settlement token.
    ///
    /// # Type parameters
    /// - `D`: Identity-domain type. Bound: `KeyDomain`.
    /// - `R`: Environment type.
    /// - `P`: Payload type.
    ///
    /// # Parameters
    /// - `claim` (`QueueClaim`): Fenced ownership token for the selected message.
    ///
    /// # Returns
    /// Type: `Result<DecodedQueueClaim<P>, QueueDecodeFailure>`.
    pub fn decode_claim(
        &self,
        claim: QueueClaim,
    ) -> Result<DecodedQueueClaim<P>, QueueDecodeFailure> {
        self.inner.decode_claim(claim)
    }

    /// Read recorded details without changing the authoritative state.
    ///
    /// # Type parameters
    /// - `P`: Payload type. Bound: `Send + 'static`.
    /// - `D`: Stable-key domain type.
    /// - `R`: Environment type.
    ///
    /// # Parameters
    /// - `id` (`StableKey<D>`): Stable identity of the selected record or operation.
    ///
    /// # Returns
    /// Type: `DurableQueueEffect<Option<QueueRecord>, R>`.
    pub fn inspect(&self, id: StableKey<D>) -> DurableQueueEffect<Option<QueueRecord>, R> {
        self.inner.inspect(id)
    }

    /// Inspect messages retained after ordinary delivery was exhausted.
    ///
    /// # Type parameters
    /// - `P`: Payload type. Bound: `Send + 'static`.
    /// - `D`: Identity-domain type. Bound: `KeyDomain`.
    /// - `R`: Environment type.
    ///
    /// # Parameters
    /// - `scan` (`QueueScan`): Bounded request for records in a selected state.
    ///
    /// # Returns
    /// Type: `DurableQueueEffect<Vec<QueueRecord>, R>`.
    pub fn dead_letters(&self, scan: QueueScan) -> DurableQueueEffect<Vec<QueueRecord>, R> {
        self.inner.dead_letters(scan)
    }

    /// Make a retained message eligible for delivery again under the supplied policy.
    ///
    /// # Type parameters
    /// - `P`: Payload type. Bound: `Send + 'static`.
    /// - `D`: Stable-key domain type.
    /// - `R`: Environment type.
    ///
    /// # Parameters
    /// - `id` (`StableKey<D>`): Stable identity of the selected record or operation.
    /// - `request` (`QueueRequeue`): The typed request to process.
    ///
    /// # Returns
    /// Type: `DurableQueueEffect<QueueRequeueOutcome, R>`.
    pub fn requeue(
        &self,
        id: StableKey<D>,
        request: QueueRequeue,
    ) -> DurableQueueEffect<QueueRequeueOutcome, R> {
        self.inner.requeue(id, request)
    }

    /// Describe another attempt under the supplied retry policy.
    ///
    /// # Type parameters
    /// - `P`: Payload type. Bound: `Send + 'static`.
    /// - `D`: Identity-domain type. Bound: `KeyDomain`.
    /// - `R`: Environment type.
    ///
    /// # Parameters
    /// - `claim` (`QueueClaimToken`): Fenced ownership token for the selected message.
    /// - `retry` (`QueueRetry`): The retry supplied to this operation.
    ///
    /// # Returns
    /// Type: `DurableQueueEffect<QueueSettlement, R>`.
    pub fn retry(
        &self,
        claim: QueueClaimToken,
        retry: QueueRetry,
    ) -> DurableQueueEffect<QueueSettlement, R> {
        self.inner.retry(claim, retry)
    }

    /// Retain the selected message outside ordinary delivery with a recorded reason.
    ///
    /// # Type parameters
    /// - `P`: Payload type. Bound: `Send + 'static`.
    /// - `D`: Identity-domain type. Bound: `KeyDomain`.
    /// - `R`: Environment type.
    ///
    /// # Parameters
    /// - `claim` (`QueueClaimToken`): Fenced ownership token for the selected message.
    /// - `at` (`UtcMillis`): The caller-supplied time of this operation.
    /// - `reason` (`QueueDeadLetterReason`): The reason supplied to this operation.
    ///
    /// # Returns
    /// Type: `DurableQueueEffect<QueueSettlement, R>`.
    pub fn dead_letter(
        &self,
        claim: QueueClaimToken,
        at: UtcMillis,
        reason: QueueDeadLetterReason,
    ) -> DurableQueueEffect<QueueSettlement, R> {
        self.inner.dead_letter(claim, at, reason)
    }

    /// Dispatch the supplied input to its registered typed handler.
    ///
    /// # Type parameters
    /// - `D`: Identity-domain type. Bound: `KeyDomain`.
    /// - `T`: Value or item type. Additional bound: `Send + 'static`.
    /// - `HandlerError`: Error produced by the registered handler. Additional bound: `Send + 'static`.
    /// - `P`: Payload type.
    /// - `R`: Environment type.
    ///
    /// # Parameters
    /// - `delivery` (`DecodedQueueClaim<P>`): The delivery supplied to this operation.
    /// - `deferred` (`DurableDeferred<T>`): One-shot result awaited or completed by this operation.
    /// - `handler` (`impl Fn(P) -> Effect<T, HandlerError, R> + Send + Sync + 'static`): The handler invoked for the matching request or event.
    /// - `acknowledged_at` (`UtcMillis`): Instant at which delivery was confirmed.
    ///
    /// # Returns
    /// Type: `DurableQueueWorkerEffect<QueueSettlement, HandlerError, R>`.
    pub fn handle<T, HandlerError>(
        &self,
        delivery: DecodedQueueClaim<P>,
        deferred: DurableDeferred<T>,
        handler: impl Fn(P) -> Effect<T, HandlerError, R> + Send + Sync + 'static,
        acknowledged_at: UtcMillis,
    ) -> DurableQueueWorkerEffect<QueueSettlement, HandlerError, R>
    where
        T: Send + 'static,
        HandlerError: Send + 'static,
    {
        self.inner
            .handle(delivery, deferred, handler, acknowledged_at)
    }

    /// Run a claimed queue payload and record its workflow result before acknowledgement.
    ///
    /// # Type parameters
    /// - `D`: Identity-domain type. Bound: `KeyDomain`.
    /// - `T`: Value or item type. Additional bound: `Send + 'static`.
    /// - `HandlerError`: Error produced by the registered handler. Additional bound: `Send + 'static`.
    /// - `P`: Payload type.
    /// - `R`: Environment type.
    ///
    /// # Parameters
    /// - `claim` (`QueueClaim`): Fenced ownership token for the selected message.
    /// - `deferred` (`DurableDeferred<T>`): One-shot result awaited or completed by this operation.
    /// - `handler` (`impl Fn(P) -> Effect<T, HandlerError, R> + Send + Sync + 'static`): The handler invoked for the matching request or event.
    /// - `acknowledged_at` (`UtcMillis`): Instant at which delivery was confirmed.
    ///
    /// # Returns
    /// Type: `DurableQueueWorkerEffect<QueueSettlement, HandlerError, R>`.
    pub fn handle_claim<T, HandlerError>(
        &self,
        claim: QueueClaim,
        deferred: DurableDeferred<T>,
        handler: impl Fn(P) -> Effect<T, HandlerError, R> + Send + Sync + 'static,
        acknowledged_at: UtcMillis,
    ) -> DurableQueueWorkerEffect<QueueSettlement, HandlerError, R>
    where
        T: Send + 'static,
        HandlerError: Send + 'static,
    {
        self.inner
            .handle_claim(claim, deferred, handler, acknowledged_at)
    }

    /// Offer a typed payload to the persisted queue with its stable identity.
    ///
    /// # Type parameters
    /// - `T`: Value or item type. Bounds: `Send + 'static`.
    /// - `D`: Stable-key domain type.
    /// - `P`: Payload type.
    /// - `R`: Environment type.
    ///
    /// # Parameters
    /// - `id` (`StableKey<D>`): Stable identity of the selected record or operation.
    /// - `payload` (`impl Fn() -> P + Send + Sync + 'static`): Application data moved into the message or stored record.
    /// - `deferred` (`&DurableDeferred<T>`): One-shot result awaited or completed by this operation.
    /// - `offered_at` (`UtcMillis`): The offered at supplied to this operation.
    /// - `available_at` (`UtcMillis`): Instant when the rate-limited action may next be admitted.
    ///
    /// # Returns
    /// Type: `DurableQueueEffect<QueueOfferOutcome, R>`.
    pub fn enqueue<T: Send + 'static>(
        &self,
        id: StableKey<D>,
        payload: impl Fn() -> P + Send + Sync + 'static,
        deferred: &DurableDeferred<T>,
        offered_at: UtcMillis,
        available_at: UtcMillis,
    ) -> DurableQueueEffect<QueueOfferOutcome, R> {
        self.inner
            .enqueue(id, payload, deferred, offered_at, available_at)
    }

    /// Record a terminal result once and publish it to current observers.
    ///
    /// # Type parameters
    /// - `P`: Payload type. Bound: `Send + 'static`.
    /// - `D`: Identity-domain type. Bound: `KeyDomain`.
    /// - `T`: Value or item type. Bounds: `Send + 'static`.
    /// - `R`: Environment type.
    ///
    /// # Parameters
    /// - `claim` (`QueueClaimToken`): Fenced ownership token for the selected message.
    /// - `deferred` (`DurableDeferred<T>`): One-shot result awaited or completed by this operation.
    /// - `result` (`T`): The result to record or return.
    /// - `acknowledged_at` (`UtcMillis`): Instant at which delivery was confirmed.
    ///
    /// # Returns
    /// Type: `DurableQueueEffect<QueueSettlement, R>`.
    pub fn complete<T: Send + 'static>(
        &self,
        claim: QueueClaimToken,
        deferred: DurableDeferred<T>,
        result: T,
        acknowledged_at: UtcMillis,
    ) -> DurableQueueEffect<QueueSettlement, R> {
        self.inner
            .complete(claim, deferred, result, acknowledged_at)
    }

    /// Settle a claim after confirming its workflow result was already recorded.
    ///
    /// # Type parameters
    /// - `P`: Payload type. Bound: `Send + 'static`.
    /// - `D`: Identity-domain type. Bound: `KeyDomain`.
    /// - `T`: Value or item type. Bounds: `Send + 'static`.
    /// - `R`: Environment type.
    ///
    /// # Parameters
    /// - `claim` (`QueueClaimToken`): Fenced ownership token for the selected message.
    /// - `deferred` (`DurableDeferred<T>`): One-shot result awaited or completed by this operation.
    /// - `acknowledged_at` (`UtcMillis`): Instant at which delivery was confirmed.
    ///
    /// # Returns
    /// Type: `DurableQueueEffect<QueueSettlement, R>`.
    pub fn acknowledge_recorded<T: Send + 'static>(
        &self,
        claim: QueueClaimToken,
        deferred: DurableDeferred<T>,
        acknowledged_at: UtcMillis,
    ) -> DurableQueueEffect<QueueSettlement, R> {
        self.inner
            .acknowledge_recorded(claim, deferred, acknowledged_at)
    }
}
