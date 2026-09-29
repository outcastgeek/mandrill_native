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
            return Effect::fail_with(|| effect::cluster::ClusterWorkflowError::InputConsumed);
        }
        ClusterWorkflowEngine::dispatch(WorkflowCommand::Start {
            handle: self.handle.clone(),
            input: encoded_request,
        })
        .map(|_| SavedReply {
            kind: ReplyKind::Success,
            payload: Arc::from(&b"accepted"[..]),
        })
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
    let request = json_bytes(None).transform(
        |bytes| {
            StableKey::<Domain>::from_encoded(Arc::from(bytes))
                .map(WorkflowRunId::from_stable)
                .map(|run| WorkflowCommand::Cancel { run })
                .map_err(|_| SchemaIssue::new(SchemaIssueKind::InvalidTag))
        },
        |command| match command {
            WorkflowCommand::Cancel { run } => Ok(run.as_bytes().to_vec()),
            _ => Err(SchemaIssue::new(SchemaIssueKind::InvalidTag)),
        },
    );
    let success = json_string().transform(
        |name| {
            let outcome = match name.as_str() {
                "cancelled" => CancellationOutcome::Cancelled,
                "already-cancelled" => CancellationOutcome::AlreadyCancelled,
                "active" => CancellationOutcome::Active,
                "already-completed" => CancellationOutcome::AlreadyCompleted,
                _ => return Err(SchemaIssue::new(SchemaIssueKind::InvalidTag)),
            };
            Ok(effect::workflow::WorkflowReply::Cancellation(outcome))
        },
        |reply| match reply {
            effect::workflow::WorkflowReply::Cancellation(outcome) => Ok(match outcome {
                CancellationOutcome::Cancelled => "cancelled",
                CancellationOutcome::AlreadyCancelled => "already-cancelled",
                CancellationOutcome::Active => "active",
                CancellationOutcome::AlreadyCompleted => "already-completed",
            }
            .to_owned()),
            _ => Err(SchemaIssue::new(SchemaIssueKind::InvalidTag)),
        },
    );
    let domain_error = json_string().transform(
        |name| {
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
        },
        |error| Ok(format!("{error:?}")),
    );
    RpcOperation::unary(id, version, request, success, domain_error, limits)
}

#[test]
fn accepted_cluster_request_drives_workflow_and_records_reply() {
    let limit = NonZeroUsize::new(1).unwrap();
    let namespace = KeyNamespace::<Domain>::new("workflow").unwrap();
    let key = |name: &'static str| {
        StableKey::new(&namespace, [KeySegment::Text(Arc::from(name))]).unwrap()
    };
    let run = WorkflowRunId::from_stable(key("run"));
    let definition = WorkflowDefinitionId::new("cluster-workflow").unwrap();
    let version = WorkflowVersion::new(NonZeroU32::new(1).unwrap());
    let handle = WorkflowHandle {
        run: run.clone(),
        definition: definition.clone(),
        version,
    };
    let address = ClusterAddress {
        entity_type: ClusterEntityType::new("workflow").unwrap(),
        entity: ClusterEntityId::from_stable(key("run")),
        shard: effect::cluster::ClusterShardId::from_stable(key("shard")),
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
    let request_id = ClusterRequestId::from_stable(key("request"));
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
            maximum_attempts: NonZeroU32::new(2).unwrap(),
        },
    };
    assert!(matches!(
        block_on(store.accept(accepted).run(Arc::new(()))),
        Exit::Success(MessageAcceptance::Accepted { .. })
    ));
    let lease = match block_on(
        store
            .activate(
                address.clone(),
                ActivationId::from_stable(key("activation")),
                UtcMillis::new(1),
                UtcMillis::new(10),
            )
            .run(Arc::new(())),
    ) {
        Exit::Success(ActivationOutcome::Activated { lease, .. }) => lease,
        _ => panic!("activation failed"),
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

    let wire_limit = NonZeroUsize::new(512).unwrap();
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
    let operation_id = OperationId::new("cancel-workflow").unwrap();
    let rpc_version = RpcVersion::new(NonZeroU32::MIN);
    let operation = cancel_operation(operation_id.clone(), rpc_version, wire_policy.json);
    let request_payload = operation
        .request_codec()
        .encode_ref(&WorkflowCommand::Cancel { run: run.clone() })
        .unwrap();
    let encoded_request = RpcEnvelope {
        operation: operation_id,
        version: rpc_version,
        call: RpcCallId::new("cancel-call").unwrap(),
        kind: RpcEnvelopeKind::Request,
        sequence: None,
        payload: request_payload,
        metadata: BTreeMap::new(),
    }
    .encode(wire_policy)
    .unwrap()
    .into_bytes()
    .unwrap();
    let dispatcher = ClusterWorkflowEngine::dispatcher::<EncodedEnvironment>(
        wire_policy,
        address.entity_type.clone(),
        operation,
    )
    .unwrap();
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
    let cancel_request = ClusterRequestId::from_stable(key("cancel-request"));
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
    let cancel_lease = match block_on(
        encoded_store
            .activate(
                address.clone(),
                ActivationId::from_stable(key("cancel-activation")),
                UtcMillis::new(1),
                UtcMillis::new(10),
            )
            .run(Arc::new(())),
    ) {
        Exit::Success(ActivationOutcome::Activated { lease, .. }) => lease,
        _ => panic!("encoded activation failed"),
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
    let saved = match block_on(
        encoded_store
            .inspect(address, cancel_request)
            .run(Arc::new(())),
    ) {
        Exit::Success(Some(message)) => message,
        _ => panic!("encoded reply missing"),
    };
    let SavedMessageState::Completed { reply, .. } = saved.state else {
        panic!("encoded reply not completed");
    };
    assert_eq!(reply.kind, ReplyKind::Success);
    let mut frames = FrameDecoder::new(wire_policy.frame);
    let decoded_frames = frames.push(&reply.payload).unwrap();
    frames.finish().unwrap();
    let reply_envelope = RpcEnvelope::decode(&decoded_frames[0], wire_policy).unwrap();
    assert_eq!(reply_envelope.kind, RpcEnvelopeKind::Success);
    assert_eq!(reply_envelope.payload, JsonValue::from("cancelled"));
    assert!(matches!(
        block_on(engine.inspect(run).run(Arc::new(()))),
        Exit::Success(Some(snapshot)) if snapshot.status == WorkflowStatus::Cancelled
    ));
}
