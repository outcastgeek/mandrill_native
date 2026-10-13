use effect::cluster::{
    AcceptancePolicy, AcceptedRequest, ActivationId, ActivationOutcome, ClaimOptions,
    ClusterActivationStore, ClusterAddress, ClusterEntityId, ClusterEntityType, ClusterExecutor,
    ClusterMessageStore, ClusterRequestId, ClusterRunner, ClusterRunnerError,
    ClusterWorkflowEngine, EntityDispatcher, HasClusterExecutor, HasClusterMessageStore,
    MemoryClusterMessageStore, MessageAcceptance, MessageStoreCapacity, PayloadConflictPolicy,
    ReplyKind, SavedMessageState, SavedReply, StoreCompletion,
};
use effect::encoding::{FrameDecoder, FramePolicy};
use effect::rpc::{
    OperationId, RpcCallId, RpcEnvelope, RpcEnvelopeKind, RpcEnvelopePolicy, RpcOperation,
    RpcVersion,
};
use effect::workflow::{
    CancellationOutcome, HasWorkflowEngine, MemoryWorkflowEngine, WorkflowCommand,
    WorkflowDefinitionId, WorkflowEngine, WorkflowEngineError, WorkflowHandle,
    WorkflowRegistration, WorkflowRunId, WorkflowStatus, WorkflowVersion,
};
use effect::{
    Defect, Effect, Exit, HasTaskSpawner, HasWallClock, JsonValue, KeyDomain, KeyNamespace,
    KeySegment, ParseLimits, ScheduledTask, SchemaIssue, SchemaIssueKind, StableKey, TaskSpawner,
    TestWallClock, UtcMillis, json_bytes, json_string,
};
use effect::{codec, effect_map, effect_try_sync, rpc_operation};
use futures_executor::block_on;
use std::collections::BTreeMap;
use std::num::{NonZeroU32, NonZeroUsize};
use std::sync::Arc;
use std::time::Duration;

struct Domain;
impl KeyDomain for Domain {
    const TAG: &'static str = "cluster-runner-test";
}

struct Threads;
impl TaskSpawner for Threads {
    fn spawn(&self, task: ScheduledTask) -> Result<(), Defect> {
        std::thread::Builder::new()
            .spawn(move || block_on(task))
            .map(|_| ())
            .map_err(Defect::from_io)
    }
}

struct WorkflowExecutor {
    handle: WorkflowHandle,
}
impl ClusterExecutor<Environment> for WorkflowExecutor {
    type Error = effect::cluster::ClusterWorkflowError<WorkflowEngineError>;
    fn execute(
        &self,
        address: ClusterAddress,
        encoded_request: Arc<[u8]>,
    ) -> Effect<SavedReply, Self::Error, Environment> {
        if address.entity.as_bytes() != self.handle.run.as_bytes() {
            return effect_try_sync! { body: { let __dsl_error = || effect::cluster::ClusterWorkflowError::InputConsumed; move || Err(__dsl_error()) }, on_error: ::core::convert::identity, };
        }
        effect_map! { program: ClusterWorkflowEngine::dispatch(WorkflowCommand::Start {
            handle: self.handle.clone(),
            input: encoded_request,
        }), map: |_| SavedReply {
            kind: ReplyKind::Success,
            payload: Arc::from(&b"accepted"[..]),
        }, }
    }
}

struct Environment {
    store: Arc<MemoryClusterMessageStore>,
    engine: Arc<MemoryWorkflowEngine>,
    executor: Arc<WorkflowExecutor>,
    tasks: Threads,
    clock: TestWallClock,
}

impl HasClusterMessageStore for Environment {
    type Store = MemoryClusterMessageStore;
    fn cluster_message_store(&self) -> Arc<Self::Store> {
        Arc::clone(&self.store)
    }
}
impl HasClusterExecutor for Environment {
    type Executor = WorkflowExecutor;
    fn cluster_executor(&self) -> Arc<Self::Executor> {
        Arc::clone(&self.executor)
    }
}
impl HasWorkflowEngine for Environment {
    type Engine = MemoryWorkflowEngine;
    fn workflow_engine(&self) -> Arc<Self::Engine> {
        Arc::clone(&self.engine)
    }
}
impl HasTaskSpawner for Environment {
    type Tasks = Threads;
    fn task_spawner(&self) -> &Self::Tasks {
        &self.tasks
    }
}
impl HasWallClock for Environment {
    type Clock = TestWallClock;
    fn wall_clock(&self) -> &Self::Clock {
        &self.clock
    }
}

struct EncodedEnvironment {
    store: Arc<MemoryClusterMessageStore>,
    engine: Arc<MemoryWorkflowEngine>,
    executor: Arc<EntityDispatcher<EncodedEnvironment>>,
    tasks: Threads,
    clock: TestWallClock,
}

impl HasClusterMessageStore for EncodedEnvironment {
    type Store = MemoryClusterMessageStore;
    fn cluster_message_store(&self) -> Arc<Self::Store> {
        Arc::clone(&self.store)
    }
}
impl HasClusterExecutor for EncodedEnvironment {
    type Executor = EntityDispatcher<Self>;
    fn cluster_executor(&self) -> Arc<Self::Executor> {
        Arc::clone(&self.executor)
    }
}
impl HasWorkflowEngine for EncodedEnvironment {
    type Engine = MemoryWorkflowEngine;
    fn workflow_engine(&self) -> Arc<Self::Engine> {
        Arc::clone(&self.engine)
    }
}
impl HasTaskSpawner for EncodedEnvironment {
    type Tasks = Threads;
    fn task_spawner(&self) -> &Self::Tasks {
        &self.tasks
    }
}
impl HasWallClock for EncodedEnvironment {
    type Clock = TestWallClock;
    fn wall_clock(&self) -> &Self::Clock {
        &self.clock
    }
}

fn cancel_operation(
    id: OperationId,
    version: RpcVersion,
    limits: ParseLimits,
) -> RpcOperation<
    WorkflowCommand,
    effect::workflow::WorkflowReply,
    effect::cluster::ClusterWorkflowError<WorkflowEngineError>,
> {
    let request = codec! { transform_pure; source: json_bytes(None), decode: |bytes| {
        StableKey::<Domain>::from_encoded(Arc::from(bytes))
            .map(WorkflowRunId::from_stable)
            .map(|run| WorkflowCommand::Cancel { run })
            .map_err(|_| SchemaIssue::new(SchemaIssueKind::InvalidTag))
    }, encode: |command| {
        if let WorkflowCommand::Cancel { run } = command {
            Ok(run.as_bytes().to_vec())
        } else {
            Err(SchemaIssue::new(SchemaIssueKind::InvalidTag))
        }
    }, };
    let success = codec! { transform_pure; source: json_string(), decode: |name| {
        let outcome = match name.as_str() {
            "cancelled" => CancellationOutcome::Cancelled,
            "already-cancelled" => CancellationOutcome::AlreadyCancelled,
            "active" => CancellationOutcome::Active,
            "already-completed" => CancellationOutcome::AlreadyCompleted,
            _ => return Err(SchemaIssue::new(SchemaIssueKind::InvalidTag)),
        };
        Ok(effect::workflow::WorkflowReply::Cancellation(outcome))
    }, encode: |reply| {
        if let effect::workflow::WorkflowReply::Cancellation(outcome) = reply {
            Ok(match outcome {
                CancellationOutcome::Cancelled => "cancelled",
                CancellationOutcome::AlreadyCancelled => "already-cancelled",
                CancellationOutcome::Active => "active",
                CancellationOutcome::AlreadyCompleted => "already-completed",
            }
            .to_owned())
        } else {
            Err(SchemaIssue::new(SchemaIssueKind::InvalidTag))
        }
    }, };
    let domain_error = codec! { transform_pure; source: json_string(), decode: |name| {
        use WorkflowEngineError as Error;
        let error = match name.as_str() {
            "InputConsumed" => return Ok(effect::cluster::ClusterWorkflowError::InputConsumed),
            "Engine(DefinitionConflict)" => Error::DefinitionConflict,
            "Engine(RunConflict)" => Error::RunConflict,
            "Engine(MissingRun)" => Error::MissingRun,
            "Engine(DefinitionMismatch)" => Error::DefinitionMismatch,
            "Engine(TerminalRun)" => Error::TerminalRun,
            "Engine(ActivityConflict)" => Error::ActivityConflict,
            "Engine(ActivityAttemptConflict)" => Error::ActivityAttemptConflict,
            "Engine(SignalConflict)" => Error::SignalConflict,
            "Engine(CompensationConflict)" => Error::CompensationConflict,
            "Engine(RaceConflict)" => Error::RaceConflict,
            "Engine(RaceWinnerMissing)" => Error::RaceWinnerMissing,
            "Engine(ChildConflict)" => Error::ChildConflict,
            "Engine(StateConflict)" => Error::StateConflict,
            "Engine(InputConsumed)" => Error::InputConsumed,
            "Engine(InvalidTransition)" => Error::InvalidTransition,
            "Engine(EngineShutdown)" => Error::EngineShutdown,
            "Engine(MigrationConflict)" => Error::MigrationConflict,
            _ => return Err(SchemaIssue::new(SchemaIssueKind::InvalidTag)),
        };
        Ok(effect::cluster::ClusterWorkflowError::Engine(error))
    }, encode: |error| Ok(format!("{error:?}")), };
    rpc_operation! { id: id, version: version, request: request, success: success, error: domain_error, limits: limits,  }
}

#[test]
fn accepted_cluster_request_drives_workflow_and_records_reply() -> Result<(), String> {
    let limit = NonZeroUsize::MIN;
    let namespace = KeyNamespace::<Domain>::new("workflow")
        .map_err(|error| format!("workflow namespace: {error:?}"))?;
    let key = |name: &'static str| {
        StableKey::new(&namespace, [KeySegment::Text(Arc::from(name))])
            .map_err(|error| format!("{name} identity: {error:?}"))
    };
    let run = WorkflowRunId::from_stable(key("run")?);
    let definition = WorkflowDefinitionId::new("cluster-workflow")
        .map_err(|error| format!("cluster workflow: {error:?}"))?;
    let version = WorkflowVersion::new(NonZeroU32::MIN);
    let handle = WorkflowHandle {
        run: run.clone(),
        definition: definition.clone(),
        version,
    };
    let address = ClusterAddress {
        entity_type: ClusterEntityType::new("workflow")
            .map_err(|error| format!("workflow entity type: {error:?}"))?,
        entity: ClusterEntityId::from_stable(key("run")?),
        shard: effect::cluster::ClusterShardId::from_stable(key("shard")?),
    };
    let store = Arc::new(MemoryClusterMessageStore::new(
        MessageStoreCapacity::Bounded(limit),
    ));
    let engine = Arc::new(MemoryWorkflowEngine::new());
    let environment = Arc::new(Environment {
        store: Arc::clone(&store),
        engine: Arc::clone(&engine),
        executor: Arc::new(WorkflowExecutor { handle }),
        tasks: Threads,
        clock: TestWallClock::new(UtcMillis::new(1)),
    });
    let registration = WorkflowRegistration {
        id: definition,
        version,
        compatibility: Arc::from(&b"v1"[..]),
    };
    let request_id = ClusterRequestId::from_stable(key("request")?);
    let accepted = AcceptedRequest {
        address: address.clone(),
        id: request_id.clone(),
        payload: Arc::from(&b"workflow input"[..]),
        idempotency_key: None,
        accepted_at: UtcMillis::new(1),
        policy: AcceptancePolicy {
            dedup_retention: Duration::ZERO,
            reply_retention: Duration::ZERO,
            payload_conflict: PayloadConflictPolicy::Reject,
            maximum_attempts: NonZeroU32::MIN.saturating_add(1),
        },
    };
    assert!(matches!(
        block_on(store.accept(accepted).run(Arc::new(()))),
        Exit::Success(MessageAcceptance::Accepted { .. })
    ));
    let Exit::Success(ActivationOutcome::Activated { lease, .. }) = block_on(
        store
            .activate(
                address.clone(),
                ActivationId::from_stable(key("activation")?),
                UtcMillis::new(1),
                UtcMillis::new(10),
            )
            .run(Arc::new(())),
    ) else {
        return Err(String::from("workflow activation must succeed"));
    };
    let claim = ClaimOptions {
        owner: lease.owner.clone(),
        now: UtcMillis::new(1),
        lease_until: UtcMillis::new(5),
        limit,
    };
    let result = ClusterRunner::run_once(claim, limit);
    assert!(
        matches!(block_on(result.run(Arc::clone(&environment))), Exit::Success(outcomes) if matches!(outcomes.as_slice(), [Err(ClusterRunnerError::Execution { source: effect::cluster::ClusterWorkflowError::Engine(WorkflowEngineError::DefinitionMismatch), .. })]))
    );
    assert!(
        matches!(block_on(store.inspect(address.clone(), request_id.clone()).run(Arc::new(()))), Exit::Success(Some(message)) if matches!(message.state, SavedMessageState::Claimed(_)))
    );
    assert!(matches!(
        block_on(engine.register(registration).run(Arc::new(()))),
        Exit::Success(())
    ));
    environment.clock.set(UtcMillis::new(6));
    let claim = ClaimOptions {
        owner: lease.owner,
        now: UtcMillis::new(6),
        lease_until: UtcMillis::new(9),
        limit,
    };
    let result = ClusterRunner::run_once(claim, limit);
    assert!(
        matches!(block_on(result.run(Arc::clone(&environment))), Exit::Success(outcomes) if matches!(outcomes.as_slice(), [Ok(outcome)] if outcome.completion == StoreCompletion::Completed && outcome.request == request_id))
    );
    assert!(
        matches!(block_on(engine.inspect(run.clone()).run(Arc::new(()))), Exit::Success(Some(snapshot)) if snapshot.status == WorkflowStatus::Ready)
    );
    assert!(
        matches!(block_on(store.inspect(address.clone(), request_id).run(Arc::new(()))), Exit::Success(Some(message)) if matches!(message.state, SavedMessageState::Completed { .. }))
    );

    let wire_limit = NonZeroUsize::MIN.saturating_add(511);
    let wire_policy = RpcEnvelopePolicy {
        frame: FramePolicy {
            maximum_payload: wire_limit,
        },
        json: ParseLimits {
            bytes: wire_limit,
            depth: wire_limit,
            nodes: wire_limit,
            collection: wire_limit,
            issues: wire_limit,
        },
    };
    let operation_id = OperationId::new("cancel-workflow")
        .map_err(|error| format!("cancel operation: {error:?}"))?;
    let rpc_version = RpcVersion::new(NonZeroU32::MIN);
    let operation = cancel_operation(operation_id.clone(), rpc_version, wire_policy.json);
    let request_payload = operation
        .request_codec()
        .encode_ref(&WorkflowCommand::Cancel { run: run.clone() })
        .map_err(|error| format!("cancel command encoding: {error:?}"))?;
    let encoded_request = RpcEnvelope {
        operation: operation_id,
        version: rpc_version,
        call: RpcCallId::new("cancel-call").map_err(|error| format!("cancel call: {error:?}"))?,
        kind: RpcEnvelopeKind::Request,
        sequence: None,
        payload: request_payload,
        metadata: BTreeMap::new(),
    }
    .encode(wire_policy)
    .map_err(|error| format!("cancel envelope: {error:?}"))?
    .into_bytes()
    .map_err(|error| format!("cancel frame: {error:?}"))?;
    let dispatcher = ClusterWorkflowEngine::dispatcher::<EncodedEnvironment>(
        wire_policy,
        address.entity_type.clone(),
        operation,
    )
    .map_err(|error| format!("workflow dispatcher: {error:?}"))?;
    let encoded_store = Arc::new(MemoryClusterMessageStore::new(
        MessageStoreCapacity::Bounded(limit),
    ));
    let encoded_environment = Arc::new(EncodedEnvironment {
        store: Arc::clone(&encoded_store),
        engine: Arc::clone(&engine),
        executor: Arc::new(dispatcher),
        tasks: Threads,
        clock: TestWallClock::new(UtcMillis::new(1)),
    });
    let cancel_request = ClusterRequestId::from_stable(key("cancel-request")?);
    assert!(matches!(
        block_on(
            encoded_store
                .accept(AcceptedRequest {
                    address: address.clone(),
                    id: cancel_request.clone(),
                    payload: Arc::from(encoded_request),
                    idempotency_key: None,
                    accepted_at: UtcMillis::new(1),
                    policy: AcceptancePolicy {
                        dedup_retention: Duration::ZERO,
                        reply_retention: Duration::ZERO,
                        payload_conflict: PayloadConflictPolicy::Reject,
                        maximum_attempts: NonZeroU32::MIN,
                    },
                })
                .run(Arc::new(()))
        ),
        Exit::Success(MessageAcceptance::Accepted { .. })
    ));
    let Exit::Success(ActivationOutcome::Activated {
        lease: cancel_lease,
        ..
    }) = block_on(
        encoded_store
            .activate(
                address.clone(),
                ActivationId::from_stable(key("cancel-activation")?),
                UtcMillis::new(1),
                UtcMillis::new(10),
            )
            .run(Arc::new(())),
    )
    else {
        return Err(String::from("encoded activation must succeed"));
    };
    let cancel_claim = ClaimOptions {
        owner: cancel_lease.owner,
        now: UtcMillis::new(1),
        lease_until: UtcMillis::new(5),
        limit,
    };
    assert!(matches!(
        block_on(ClusterRunner::run_once(cancel_claim, limit).run(encoded_environment)),
        Exit::Success(outcomes) if matches!(outcomes.as_slice(), [Ok(outcome)] if outcome.completion == StoreCompletion::Completed && outcome.request == cancel_request)
    ));
    let Exit::Success(Some(saved)) = block_on(
        encoded_store
            .inspect(address, cancel_request)
            .run(Arc::new(())),
    ) else {
        return Err(String::from("encoded reply must be saved"));
    };
    let SavedMessageState::Completed { reply, .. } = saved.state else {
        return Err(String::from("encoded reply must be completed"));
    };
    assert_eq!(reply.kind, ReplyKind::Success);
    let mut frames = FrameDecoder::new(wire_policy.frame);
    let decoded_frames = frames
        .push(&reply.payload)
        .map_err(|error| format!("reply frames: {error:?}"))?;
    frames
        .finish()
        .map_err(|error| format!("reply framing: {error:?}"))?;
    let first = decoded_frames.first().ok_or("missing reply frame")?;
    let reply_envelope = RpcEnvelope::decode(first, wire_policy)
        .map_err(|error| format!("reply envelope: {error:?}"))?;
    assert_eq!(reply_envelope.kind, RpcEnvelopeKind::Success);
    assert_eq!(reply_envelope.payload, JsonValue::from("cancelled"));
    assert!(matches!(
        block_on(engine.inspect(run).run(Arc::new(()))),
        Exit::Success(Some(snapshot)) if snapshot.status == WorkflowStatus::Cancelled
    ));
    Ok(())
}
