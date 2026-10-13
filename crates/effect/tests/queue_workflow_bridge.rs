use effect::encoding::{BinaryIdentity, BinaryLimits};
use effect::persistence::{
    HasPersistedQueueStore, MemoryPersistedQueueStore, PersistedQueueName, QueueClaimOptions,
    QueueMessagePolicy, QueueOwner, QueueRecordState, QueueSettlement, QueueStoreCapacity,
};
use effect::workflow::{
    DurableDeferred, DurableSignalWait, HasWorkflowEngine, MemoryWorkflowEngine,
    WorkflowDefinitionId, WorkflowEngine, WorkflowHandle, WorkflowRegistration, WorkflowRunId,
    WorkflowSignalId, WorkflowVersion,
};
use effect::{Effect, Exit, KeyDomain, KeyNamespace, KeySegment, StableKey, UtcMillis, json_i64};
use effect::{durable_queue, effect_sync, effect_try_sync, persisted_queue};
use futures_executor::block_on;
use std::num::{NonZeroU32, NonZeroU64, NonZeroUsize};
use std::sync::Arc;
use std::time::Duration;

struct Domain;
impl KeyDomain for Domain {
    const TAG: &'static str = "queue-workflow-test";
}

struct Environment {
    queue: Arc<MemoryPersistedQueueStore>,
    engine: Arc<MemoryWorkflowEngine>,
}

impl HasPersistedQueueStore for Environment {
    type Store = MemoryPersistedQueueStore;
    fn persisted_queue_store(&self) -> Arc<Self::Store> {
        Arc::clone(&self.queue)
    }
}

impl HasWorkflowEngine for Environment {
    type Engine = MemoryWorkflowEngine;
    fn workflow_engine(&self) -> Arc<Self::Engine> {
        Arc::clone(&self.engine)
    }
}

#[test]
fn queue_handler_records_deferred_result_then_acknowledges_claim() -> Result<(), String> {
    let limit = NonZeroUsize::MIN.saturating_add(127);
    let binary_limits = BinaryLimits {
        bytes: limit,
        depth: limit,
        nodes: limit,
        collection: limit,
        steps: limit,
    };
    let environment = Arc::new(Environment {
        queue: Arc::new(MemoryPersistedQueueStore::new(QueueStoreCapacity::Bounded(
            limit,
        ))),
        engine: Arc::new(MemoryWorkflowEngine::new()),
    });
    let namespace =
        KeyNamespace::<Domain>::new("jobs").map_err(|error| format!("job namespace: {error:?}"))?;
    let key = |name: &'static str| {
        StableKey::new(&namespace, [KeySegment::Text(Arc::from(name))])
            .map_err(|error| format!("{name} identity: {error:?}"))
    };
    let identity = key("run")?;
    let run = WorkflowRunId::from_stable(identity.clone());
    let version = WorkflowVersion::new(NonZeroU32::MIN);
    let definition = WorkflowDefinitionId::new("job-workflow")
        .map_err(|error| format!("job workflow: {error:?}"))?;
    let registration = WorkflowRegistration {
        id: definition.clone(),
        version,
        compatibility: Arc::from(&b"v1"[..]),
    };
    assert!(matches!(
        block_on(environment.engine.register(registration).run(Arc::new(()))),
        Exit::Success(())
    ));
    let handle = WorkflowHandle {
        run: run.clone(),
        definition,
        version,
    };
    assert!(matches!(
        block_on(
            environment
                .engine
                .start(handle, Arc::from(&b"input"[..]))
                .run(Arc::new(()))
        ),
        Exit::Success(_)
    ));

    let deferred = DurableDeferred {
        run,
        signal: WorkflowSignalId::from_stable(key("result")?),
        version,
        codec: json_i64(),
        limits: effect::ParseLimits {
            bytes: limit,
            depth: limit,
            nodes: limit,
            collection: limit,
            issues: limit,
        },
    };
    let queue = durable_queue! { queue: persisted_queue! { name: PersistedQueueName::new("jobs").map_err(|error| format!("job queue: {error:?}"))?, payload: json_i64(), identity: BinaryIdentity::new("job-payload", NonZeroU64::MIN)
        .map_err(|error| format!("job payload: {error:?}"))?, limits: binary_limits, policy: QueueMessagePolicy {
        maximum_attempts: NonZeroU32::MIN.saturating_add(1),
        acknowledged_retention: Duration::ZERO,
    },  }, };
    let offered = queue.enqueue(
        identity.clone(),
        || 41,
        &deferred,
        UtcMillis::new(0),
        UtcMillis::new(0),
    );
    assert!(matches!(
        block_on(offered.run(Arc::clone(&environment))),
        Exit::Success(_)
    ));
    let options = QueueClaimOptions::new(
        QueueOwner::new("worker").map_err(|error| format!("queue owner: {error:?}"))?,
        UtcMillis::new(0),
        UtcMillis::new(10),
        limit,
    )
    .map_err(|error| format!("queue claim options: {error:?}"))?;
    let Exit::Success(claimed) = block_on(queue.claim(options).run(Arc::clone(&environment)))
    else {
        return Err(String::from("queue claim must succeed"));
    };
    assert_eq!(claimed.len(), 1);
    let claim = claimed.into_iter().next().ok_or("missing queue claim")?;
    let failed_claim = effect::persistence::QueueClaim {
        token: claim.token.clone(),
        payload: Arc::clone(&claim.payload),
        attempt: claim.attempt,
        offered_at: claim.offered_at,
        available_at: claim.available_at,
        claimed_at: claim.claimed_at,
        lease_until: claim.lease_until,
    };
    let failed = queue.handle_claim(
        failed_claim,
        deferred.clone(),
        |_| { let __dsl_program: Effect < i64 , & 'static str , Environment > = effect_try_sync! { body: { let __dsl_error = || "handler failed"; move || Err(__dsl_error()) }, on_error: ::core::convert::identity, }; __dsl_program },
        UtcMillis::new(1),
    );
    assert!(matches!(
        block_on(failed.run(Arc::clone(&environment))),
        Exit::Fail(effect::workflow::DurableQueueWorkerError::Handler(
            "handler failed"
        ))
    ));
    assert!(
        matches!(block_on(queue.inspect(identity.clone()).run(Arc::clone(&environment))), Exit::Success(Some(record)) if matches!(record.state, QueueRecordState::Claimed { .. }))
    );
    assert!(matches!(
        block_on(deferred.inspect().run(Arc::clone(&environment))),
        Exit::Success(None)
    ));

    let valid_token = claim.token.clone();
    let stale_token = effect::persistence::QueueClaimToken::new(
        claim.token.queue().clone(),
        claim.token.id().clone(),
        claim.token.owner().clone(),
        claim
            .token
            .generation()
            .checked_add(1)
            .ok_or("claim generation overflow")?,
    );
    let stale_claim = effect::persistence::QueueClaim {
        token: stale_token,
        payload: Arc::clone(&claim.payload),
        attempt: claim.attempt,
        offered_at: claim.offered_at,
        available_at: claim.available_at,
        claimed_at: claim.claimed_at,
        lease_until: claim.lease_until,
    };
    let handled = queue.handle_claim(
        stale_claim,
        deferred.clone(),
        |value| {
            let __dsl_program: Effect<i64, (), Environment> =
                effect_sync! { once; body: move || value + 1, };
            __dsl_program
        },
        UtcMillis::new(1),
    );
    assert!(matches!(
        block_on(handled.run(Arc::clone(&environment))),
        Exit::Fail(effect::workflow::DurableQueueWorkerError::Queue(
            effect::workflow::DurableQueueError::StaleClaim
        ))
    ));
    assert!(matches!(
        block_on(deferred.wait().run(Arc::clone(&environment))),
        Exit::Success(DurableSignalWait::Ready(42))
    ));
    assert!(
        matches!(block_on(queue.inspect(identity.clone()).run(Arc::clone(&environment))), Exit::Success(Some(record)) if matches!(record.state, QueueRecordState::Claimed { .. }))
    );
    assert!(matches!(
        block_on(
            queue
                .acknowledge_recorded(valid_token, deferred, UtcMillis::new(1))
                .run(Arc::clone(&environment))
        ),
        Exit::Success(QueueSettlement::Applied)
    ));
    assert!(
        matches!(block_on(queue.inspect(identity).run(environment)), Exit::Success(Some(record)) if matches!(record.state, QueueRecordState::Acknowledged { .. }))
    );
    Ok(())
}
