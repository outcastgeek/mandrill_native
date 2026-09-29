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
pub enum DurableQueueError<QueueError, EngineError> {
    Queue(PersistedQueueError<QueueError>),
    Signal(DurableSignalError<EngineError>),
    RunIdentityMismatch,
    WrongQueue,
    StaleClaim,
    MissingClaim,
    ResultNotRecorded,
}

#[derive(Debug)]
pub enum DurableQueueWorkerError<QueueError, EngineError, HandlerError> {
    Queue(DurableQueueError<QueueError, EngineError>),
    Decode(QueueDecodeFailure),
    Handler(HandlerError),
}

/// The queue and workflow engine remain independently selected through `R`.
pub type DurableQueueEffect<A, R> = Effect<
    A,
    DurableQueueError<
        <<R as HasPersistedQueueStore>::Store as PersistedQueueStore>::Error,
        <<R as HasWorkflowEngine>::Engine as WorkflowEngine>::Error,
    >,
    R,
>;

pub type DurableQueueWorkerEffect<A, HandlerError, R> = Effect<
    A,
    DurableQueueWorkerError<
        <<R as HasPersistedQueueStore>::Store as PersistedQueueStore>::Error,
        <<R as HasWorkflowEngine>::Engine as WorkflowEngine>::Error,
        HandlerError,
    >,
    R,
>;

pub struct DurableQueue<P: Send + 'static, D: KeyDomain> {
    queue: PersistedQueue<P, D>,
}

impl<P: Send + 'static, D: KeyDomain> DurableQueue<P, D> {
    #[must_use]
    pub fn new(queue: PersistedQueue<P, D>) -> Self {
        Self { queue }
    }

    #[must_use]
    pub fn queue(&self) -> &PersistedQueue<P, D> {
        &self.queue
    }

    /// Require physical recovery guarantees for both records involved in a
    /// queue-to-workflow handoff. The environment is fixed for every operation
    /// on the returned value; no core memory provider meets these bounds.
    #[must_use]
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

    pub fn claim<R>(&self, options: QueueClaimOptions) -> DurableQueueEffect<Vec<QueueClaim>, R>
    where
        R: HasPersistedQueueStore + HasWorkflowEngine + Send + Sync + 'static,
    {
        self.queue
            .claim(options)
            .map_error(DurableQueueError::Queue)
    }

    pub fn decode_claim(
        &self,
        claim: QueueClaim,
    ) -> Result<DecodedQueueClaim<P>, QueueDecodeFailure> {
        self.queue.decode_claim(claim)
    }

    pub fn inspect<R>(&self, id: StableKey<D>) -> DurableQueueEffect<Option<QueueRecord>, R>
    where
        R: HasPersistedQueueStore + HasWorkflowEngine + Send + Sync + 'static,
    {
        self.queue.inspect(id).map_error(DurableQueueError::Queue)
    }

    pub fn dead_letters<R>(&self, scan: QueueScan) -> DurableQueueEffect<Vec<QueueRecord>, R>
    where
        R: HasPersistedQueueStore + HasWorkflowEngine + Send + Sync + 'static,
    {
        self.queue
            .dead_letters(scan)
            .map_error(DurableQueueError::Queue)
    }

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
    pub fn claim(&self, options: QueueClaimOptions) -> DurableQueueEffect<Vec<QueueClaim>, R> {
        self.inner.claim(options)
    }

    pub fn decode_claim(
        &self,
        claim: QueueClaim,
    ) -> Result<DecodedQueueClaim<P>, QueueDecodeFailure> {
        self.inner.decode_claim(claim)
    }

    pub fn inspect(&self, id: StableKey<D>) -> DurableQueueEffect<Option<QueueRecord>, R> {
        self.inner.inspect(id)
    }

    pub fn dead_letters(&self, scan: QueueScan) -> DurableQueueEffect<Vec<QueueRecord>, R> {
        self.inner.dead_letters(scan)
    }

    pub fn requeue(
        &self,
        id: StableKey<D>,
        request: QueueRequeue,
    ) -> DurableQueueEffect<QueueRequeueOutcome, R> {
        self.inner.requeue(id, request)
    }

    pub fn retry(
        &self,
        claim: QueueClaimToken,
        retry: QueueRetry,
    ) -> DurableQueueEffect<QueueSettlement, R> {
        self.inner.retry(claim, retry)
    }

    pub fn dead_letter(
        &self,
        claim: QueueClaimToken,
        at: UtcMillis,
        reason: QueueDeadLetterReason,
    ) -> DurableQueueEffect<QueueSettlement, R> {
        self.inner.dead_letter(claim, at, reason)
    }

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
