use effect::StoredOutcome;
use effect::cluster::{
    AcceptancePolicy, AcceptedRequest, ActivationId, ActivationOutcome, ClaimOptions,
    ClusterActivationStore, ClusterAddress, ClusterEntityId, ClusterEntityType, ClusterExecutor,
    ClusterMessageStore, ClusterRequestId, ClusterRunOutcome, ClusterRunner, ClusterRunnerError,
    ClusterShardId, ClusterStoreError, EntityDispatcher, HasClusterExecutor,
    HasClusterMessageStore, MemoryClusterMessageStore, MessageStoreCapacity, PayloadConflictPolicy,
    ReplyKind, SavedReply, StoreCompletion,
};
use effect::encoding::{BinaryIdentity, BinaryLimits, FrameDecoder, FrameParts, FramePolicy};
use effect::eventlog::{
    AggregateId, AtomicProjectionStore, EventBinaryFormats, EventCodecs, EventCommitPolicy,
    EventDefinition, EventEnvelope, EventGroupId, EventId, EventInvocation, EventJournal,
    EventJournalId, EventLog, EventObservation, EventObservationError, EventReplicator,
    EventRunError, EventRunOutcome, EventShape, EventTag, EventVersion, HasAtomicProjectionStore,
    HasEventJournal, HasReplicaCursorStore, JournalCursor, JournalStream, MemoryEventJournal,
    MemoryJournalCapacity, MemoryProjectionStore, MemoryReplicaCursorStore, ProjectionAdvance,
    ProjectionId, ProjectionKey, ProjectionVersion, ReplicaCursorKey, ReplicaCursorStore,
    ReplicaExport, ReplicaId, ReplicaSync,
};
use effect::persistence::{
    HasKeyValueStore, HasPersistedQueueStore, KeyValueStore, MemoryPersistedQueueStore,
    PersistedQueue, PersistedQueueName, PersistenceError, QueueClaimOptions, QueueMessagePolicy,
    QueueOwner, QueueRecordState, QueueSettlement, RecordPolicy, RecordVersion, StoredLookup,
};
use effect::reactivity::{
    AtomError, AtomId, AtomRegistry, InvalidationError, InvalidationKey, InvalidationRegistry,
    RpcMutationAtom, RpcMutationError,
};
use effect::rpc::{
    InProcessRpcChannel, OperationId, RpcCallId, RpcClient, RpcEnvelope, RpcEnvelopeKind,
    RpcEnvelopePolicy, RpcHandler, RpcOperation, RpcServerError, RpcUnaryServer, RpcVersion,
};
use effect::transactional::{
    HasTransactionDomain, Transaction, TransactionError, TransactionPolicy, TxRef,
    TxSubscriptionError, TxSubscriptionRef,
};
use effect::workflow::{
    ActivityRunError, DurableDeferred, DurableQueue, DurableQueueError, DurableQueueWorkerError,
    DurableSignalWait, HasWorkflowEngine, MemoryWorkflowEngine, WorkflowActivityId,
    WorkflowDefinitionId, WorkflowEngine, WorkflowHandle, WorkflowRegistration, WorkflowRunError,
    WorkflowRunId, WorkflowRunResult, WorkflowSignalId, WorkflowVersion,
};
use effect::{
    Cancellation, Codec, DuplicateFields, Effect, HasBlockingExecutor, HasCpuExecutor,
    HasTaskSpawner, JsonValue, OutcomeStatus, ParseLimits, ParsePolicy, PubSubOptions,
    PubSubOverflow, PubSubShutdown, ReplayPolicy, SchemaErrorMode, SchemaIssue, Scope,
    SubscriptionEvent, UnknownFields, json_i64,
};
use effect::{HasClock, HasWallClock, TestWallClock};
use effect::{KeyDomain, KeyNamespace, KeySegment, StableKey, UtcMillis};
use std::collections::BTreeMap;
use std::marker::PhantomData;
use std::num::NonZeroU32;
use std::num::NonZeroU64;
use std::num::NonZeroUsize;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Barrier};
use std::time::Duration;

/// The same provider-neutral program is executed by both consumer-authored
/// workers and a Tokio-task/Rayon-CPU/Tokio-blocking environment. The barrier
/// requires the task and CPU callbacks to make independent progress.
pub fn run<R>() -> Effect<usize, (), R>
where
    R: HasTaskSpawner + HasCpuExecutor + HasBlockingExecutor + Send + Sync + 'static,
{
    let overlap = Arc::new(Barrier::new(2));
    let task_overlap = Arc::clone(&overlap);
    let task = Effect::<usize, (), R>::sync_once(move || {
        task_overlap.wait();
        20
    });
    let cpu = Effect::<usize, (), R>::cpu_once(move |_| {
        overlap.wait();
        21
    });
    Effect::all_concurrent(
        NonZeroUsize::new(2).expect("two independent branches"),
        vec![task, cpu],
    )
    .flat_map(|values| {
        Effect::<usize, (), R>::blocking_once(move |_| values.into_iter().sum::<usize>() + 1)
    })
}

pub fn cancel_and_join<R>() -> Effect<bool, (), R>
where
    R: HasTaskSpawner + Send + Sync + 'static,
{
    Scope::scoped(|scope| {
        let entered = Cancellation::new();
        let witness = entered.clone();
        let child = Effect::<(), (), R>::from_future(move |token| {
            let witness = witness.clone();
            async move {
                let _ = witness.request();
                token.cancelled().await;
            }
        });
        scope.fork(child).flat_map(move |fiber| {
            let entered = entered.clone();
            Effect::<(), (), R>::from_future(move |_| {
                let entered = entered.clone();
                async move { entered.cancelled().await }
            })
            .flat_map(move |_| {
                fiber
                    .interrupt_shared()
                    .map(|exit| exit.status() == OutcomeStatus::Interrupted)
            })
        })
    })
}

pub fn rpc_cpu<R>(
    limits: ParseLimits,
    frame: FramePolicy,
) -> Effect<FrameParts, RpcServerError<i64>, R>
where
    R: HasCpuExecutor + Send + Sync + 'static,
{
    let operation = effect::rpc_operation! {
        id: OperationId::new("provider-rpc-increment").expect("stable operation identity"),
        version: RpcVersion::new(NonZeroU32::MIN),
        request: json_i64(), success: json_i64(), error: json_i64(), limits: limits,
    };
    let policy = RpcEnvelopePolicy {
        frame,
        json: limits,
    };
    let handler = RpcHandler::new(operation.clone(), |request| {
        Effect::<i64, i64, R>::cpu_once(move |_| request + 1)
    });
    let envelope = RpcEnvelope {
        operation: operation.spec().id.clone(),
        version: operation.spec().version,
        call: RpcCallId::new("provider-rpc-call").expect("stable call identity"),
        kind: RpcEnvelopeKind::Request,
        sequence: None,
        payload: JsonValue::from(41),
        metadata: Default::default(),
    };
    let request = envelope
        .encode(policy)
        .expect("valid fixture envelope")
        .into_bytes()
        .expect("valid fixture frame");
    RpcUnaryServer::new(handler, policy).handle(request)
}

pub fn schema_cpu<R>() -> Effect<i64, SchemaIssue, R>
where
    R: HasCpuExecutor + Send + Sync + 'static,
{
    struct Record {
        value: i64,
    }
    let policy = ParsePolicy {
        limits: rpc_limits(),
        errors: SchemaErrorMode::First,
        unknown_fields: UnknownFields::Reject,
        duplicate_fields: DuplicateFields::Reject,
    };
    let record = effect::schema! {
        struct Record { value: json_i64() } policy: policy
    }
    .expect("valid external schema macro definition");
    let encoded = record
        .encode_ref(&Record { value: 42 })
        .expect("valid external schema encoding");
    assert_eq!(
        record
            .decode_ref(&encoded)
            .expect("valid external schema decoding")
            .value,
        42
    );
    let codec = Codec::<i64, JsonValue, R, R>::new(
        |encoded| {
            Effect::<Result<i64, SchemaIssue>, SchemaIssue, R>::cpu_once(move |_| {
                json_i64().decode_ref(&encoded)
            })
            .flat_map(Effect::from_result_once)
        },
        |value| {
            Effect::<Result<JsonValue, SchemaIssue>, SchemaIssue, R>::cpu_once(move |_| {
                json_i64().encode_ref(&value)
            })
            .flat_map(Effect::from_result_once)
        },
    );
    codec.as_effect_schema().decode_once(JsonValue::from(42))
}

pub fn rpc_limits() -> ParseLimits {
    ParseLimits {
        bytes: NonZeroUsize::MAX,
        depth: NonZeroUsize::MAX,
        nodes: NonZeroUsize::MAX,
        collection: NonZeroUsize::MAX,
        issues: NonZeroUsize::MAX,
    }
}

pub fn rpc_frame() -> FramePolicy {
    FramePolicy {
        maximum_payload: NonZeroUsize::MAX,
    }
}

pub fn assert_rpc_response(response: FrameParts, limits: ParseLimits, frame: FramePolicy) {
    let policy = RpcEnvelopePolicy {
        frame,
        json: limits,
    };
    let bytes = response.into_bytes().expect("valid response frame");
    let mut decoder = FrameDecoder::new(frame);
    let frames = decoder.push(&bytes).expect("response frame decodes");
    decoder.finish().expect("response frame complete");
    assert_eq!(frames.len(), 1);
    let envelope = RpcEnvelope::decode(&frames[0], policy).expect("response envelope decodes");
    assert_eq!(envelope.kind, RpcEnvelopeKind::Success);
    assert_eq!(envelope.payload, JsonValue::from(42));
}

struct ProviderDomain;

impl KeyDomain for ProviderDomain {
    const TAG: &'static str = "provider-exercise";
}

fn event_key(
    namespace: &KeyNamespace<ProviderDomain>,
    value: &'static str,
) -> StableKey<ProviderDomain> {
    StableKey::new(namespace, [KeySegment::Text(value.into())]).expect("valid event fixture key")
}

type EventCpuError<R> = EventRunError<<<R as HasEventJournal>::Journal as EventJournal>::Error, ()>;

pub fn event_cpu<R>(limits: BinaryLimits) -> Effect<EventRunOutcome<i64, i64>, EventCpuError<R>, R>
where
    R: HasEventJournal + HasCpuExecutor + Send + Sync + 'static,
{
    let namespace =
        KeyNamespace::<ProviderDomain>::new("events").expect("valid event fixture namespace");
    let identity =
        |name| BinaryIdentity::new(name, NonZeroU64::MIN).expect("valid binary identity");
    let definition = effect::event! {
        group: EventGroupId::new("provider-events").expect("valid group"),
        tag: EventTag::new("incremented").expect("valid tag"),
        version: EventVersion::new(NonZeroU32::MIN),
        payload: json_i64(), success: json_i64(), error: json_i64(),
        payload_identity: identity("provider-payload"),
        success_identity: identity("provider-success"),
        error_identity: identity("provider-error"), limits: limits,
    };
    EventLog::new(definition, EventCommitPolicy::SuccessOnly).execute(
        EventInvocation {
            journal: EventJournalId::from_stable(event_key(&namespace, "journal")),
            aggregate: AggregateId::from_stable(event_key(&namespace, "aggregate")),
            event: EventId::from_stable(event_key(&namespace, "event")),
            expected: JournalCursor::new(0),
            occurred_at: UtcMillis::new(0),
            payload: Arc::new(41),
        },
        |payload| Effect::<Result<i64, i64>, (), R>::cpu_once(move |_| Ok(*payload + 1)),
    )
}

pub fn event_limits() -> BinaryLimits {
    BinaryLimits {
        bytes: NonZeroUsize::MAX,
        depth: NonZeroUsize::MAX,
        nodes: NonZeroUsize::MAX,
        collection: NonZeroUsize::MAX,
        steps: NonZeroUsize::MAX,
    }
}

/// An RPC response acknowledges an event append before local invalidation.
/// Closing the registry injects notification failure without changing the
/// journal's authoritative cursor or asking the client to repeat the call.
pub async fn rpc_event_notification_loss<R>(environment: Arc<R>) -> bool
where
    R: HasEventJournal<Journal = MemoryEventJournal> + HasCpuExecutor + Send + Sync + 'static,
{
    let namespace = KeyNamespace::<ProviderDomain>::new("rpc-event-notification")
        .expect("valid RPC event namespace");
    let journal_id = EventJournalId::from_stable(event_key(&namespace, "journal"));
    let aggregate = AggregateId::from_stable(event_key(&namespace, "aggregate"));
    let event_id = EventId::from_stable(event_key(&namespace, "event"));
    let stream = JournalStream {
        journal: journal_id.clone(),
        aggregate: aggregate.clone(),
    };
    let identity =
        |name| BinaryIdentity::new(name, NonZeroU64::MIN).expect("valid binary identity");
    let definition = EventDefinition::new(
        EventShape {
            group: EventGroupId::new("provider-rpc-events").expect("valid group"),
            tag: EventTag::new("mutated").expect("valid tag"),
            version: EventVersion::new(NonZeroU32::MIN),
        },
        EventCodecs {
            payload: json_i64(),
            success: json_i64(),
            error: json_i64(),
        },
        EventBinaryFormats {
            payload: identity("rpc-event-payload"),
            success: identity("rpc-event-success"),
            error: identity("rpc-event-error"),
            limits: event_limits(),
        },
    );
    let log = EventLog::new(definition, EventCommitPolicy::SuccessOnly);
    let executions = Arc::new(AtomicUsize::new(0));
    let handler_executions = Arc::clone(&executions);
    let operation = RpcOperation::unary(
        OperationId::new("provider-event-mutation").expect("valid operation"),
        RpcVersion::new(NonZeroU32::MIN),
        json_i64(),
        json_i64(),
        json_i64(),
        rpc_limits(),
    );
    let handler = RpcHandler::new(operation.clone(), move |request| {
        let executions = Arc::clone(&handler_executions);
        log.execute(
            EventInvocation {
                journal: journal_id.clone(),
                aggregate: aggregate.clone(),
                event: event_id.clone(),
                expected: JournalCursor::new(0),
                occurred_at: UtcMillis::new(0),
                payload: Arc::new(request),
            },
            move |_| {
                let executions = Arc::clone(&executions);
                Effect::<Result<i64, i64>, (), R>::cpu_once(move |_| {
                    executions.fetch_add(1, Ordering::AcqRel);
                    Ok(request + 1)
                })
            },
        )
        .map_error(|_| -2)
        .flat_map(|outcome| match outcome {
            EventRunOutcome::Committed {
                result: Ok(value), ..
            }
            | EventRunOutcome::Duplicate {
                result: Ok(value), ..
            } => Effect::sync_once(move || value),
            _ => Effect::fail_with(|| -3),
        })
    });
    let policy = RpcEnvelopePolicy {
        frame: rpc_frame(),
        json: rpc_limits(),
    };
    let server = RpcUnaryServer::new(handler, policy);
    let channel = Arc::new(InProcessRpcChannel::new(server, environment.clone()));
    let client =
        RpcClient::<i64, i64, i64, RpcServerError<i64>>::new(operation, policy, NonZeroUsize::MIN);
    let registry = InvalidationRegistry::new(PubSubOptions {
        capacity: NonZeroUsize::MIN,
        overflow: PubSubOverflow::Backpressure,
        replay: ReplayPolicy::None,
    })
    .expect("valid invalidation registry");
    let key = InvalidationKey::from_stable(event_key(&namespace, "query"));
    let mutation = RpcMutationAtom::new(client, registry.clone(), [key]);
    let client_environment = Arc::new(channel);
    let shutdown = registry
        .shutdown(PubSubShutdown::Discard)
        .run(Arc::clone(&client_environment))
        .await;
    if !matches!(shutdown, effect::Exit::Success(true)) {
        return false;
    }
    let result = mutation
        .execute(
            41,
            RpcCallId::new("provider-event-call").expect("valid call"),
            Default::default(),
        )
        .run(client_environment)
        .await;
    let cursor = environment
        .event_journal()
        .cursor(stream)
        .run(Arc::new(()))
        .await;
    matches!(
        result,
        effect::Exit::Fail(RpcMutationError::CommittedWithoutInvalidation(
            InvalidationError::PubSub(effect::PubSubError::Shutdown)
        ))
    ) && matches!(cursor, effect::Exit::Success(value) if value.value() == 1)
        && executions.load(Ordering::Acquire) == 1
}

struct ReplicaTarget {
    journal: Arc<MemoryEventJournal>,
    cursors: Arc<MemoryReplicaCursorStore>,
    projections: Arc<MemoryProjectionStore<usize>>,
}

impl HasEventJournal for ReplicaTarget {
    type Journal = MemoryEventJournal;
    fn event_journal(&self) -> Arc<Self::Journal> {
        Arc::clone(&self.journal)
    }
}

impl HasReplicaCursorStore for ReplicaTarget {
    type Store = MemoryReplicaCursorStore;
    fn replica_cursor_store(&self) -> Arc<Self::Store> {
        Arc::clone(&self.cursors)
    }
}

impl HasAtomicProjectionStore<usize> for ReplicaTarget {
    type Store = MemoryProjectionStore<usize>;
    fn atomic_projection_store(&self) -> Arc<Self::Store> {
        Arc::clone(&self.projections)
    }
}

/// A replica checkpoint and projection survive local notification loss. A
/// restarted observer replays from the journal; notification is never used
/// as the source cursor or as proof of an append.
pub async fn replica_observer_restart<R>(environment: Arc<R>) -> bool
where
    R: HasEventJournal<Journal = MemoryEventJournal> + HasCpuExecutor + Send + Sync + 'static,
{
    let namespace = KeyNamespace::<ProviderDomain>::new("replica-observation")
        .expect("valid replica namespace");
    let stream = JournalStream {
        journal: EventJournalId::from_stable(event_key(&namespace, "journal")),
        aggregate: AggregateId::from_stable(event_key(&namespace, "aggregate")),
    };
    let group = EventGroupId::new("provider-replica-events").expect("valid event group");
    let tag = EventTag::new("changed").expect("valid event tag");
    let version = EventVersion::new(NonZeroU32::MIN);
    let payload = Effect::<Arc<[u8]>, (), R>::cpu_once(|_| Arc::from(&b"change"[..]))
        .run(Arc::clone(&environment))
        .await;
    let effect::Exit::Success(payload) = payload else {
        return false;
    };
    let event = EventEnvelope {
        journal: stream.journal.clone(),
        aggregate: stream.aggregate.clone(),
        id: EventId::from_stable(event_key(&namespace, "event")),
        group: group.clone(),
        tag: tag.clone(),
        version,
        payload,
        outcome: None,
        occurred_at: UtcMillis::new(0),
    };
    let appended = environment
        .event_journal()
        .append(event, JournalCursor::new(0))
        .run(Arc::new(()))
        .await;
    if !matches!(
        appended,
        effect::Exit::Success(effect::eventlog::AppendReceipt::Decided(
            effect::eventlog::AppendDecision::Committed(_)
        ))
    ) {
        return false;
    }
    let source = ReplicaId::from_stable(event_key(&namespace, "source"));
    let exported = EventReplicator::export::<R>(
        source.clone(),
        stream.clone(),
        JournalCursor::new(0),
        NonZeroUsize::MIN,
    )
    .run(Arc::clone(&environment))
    .await;
    let effect::Exit::Success(ReplicaExport::Batch {
        batch,
        has_more: false,
    }) = exported
    else {
        return false;
    };
    let out_of_order = Arc::new(ReplicaTarget {
        journal: Arc::new(MemoryEventJournal::new(MemoryJournalCapacity {
            streams: NonZeroUsize::MIN,
            entries_per_stream: NonZeroUsize::MIN,
            tombstones_per_stream: NonZeroUsize::MIN,
        })),
        cursors: Arc::new(MemoryReplicaCursorStore::new(NonZeroUsize::MIN)),
        projections: Arc::new(MemoryProjectionStore::new(NonZeroUsize::MIN)),
    });
    let ahead = JournalCursor::new(
        batch.entries()[0]
            .sequence
            .get()
            .checked_add(1)
            .expect("fixture cursor has a successor"),
    );
    let cursor_key = ReplicaCursorKey {
        source: source.clone(),
        stream: stream.clone(),
    };
    let seeded = out_of_order
        .cursors
        .advance(cursor_key, JournalCursor::new(0), ahead)
        .run(Arc::new(()))
        .await;
    let rejected = EventReplicator::import_and_ack::<ReplicaTarget>(batch.clone())
        .run(Arc::clone(&out_of_order))
        .await;
    let untouched = out_of_order
        .journal
        .cursor(stream.clone())
        .run(Arc::new(()))
        .await;
    if !matches!(seeded, effect::Exit::Success(_))
        || !matches!(rejected, effect::Exit::Success(ReplicaSync::CursorConflict { actual }) if actual == ahead)
        || !matches!(untouched, effect::Exit::Success(cursor) if cursor.value() == 0)
    {
        return false;
    }
    let target = Arc::new(ReplicaTarget {
        journal: Arc::new(MemoryEventJournal::new(MemoryJournalCapacity {
            streams: NonZeroUsize::MIN,
            entries_per_stream: NonZeroUsize::MIN,
            tombstones_per_stream: NonZeroUsize::MIN,
        })),
        cursors: Arc::new(MemoryReplicaCursorStore::new(NonZeroUsize::MIN)),
        projections: Arc::new(MemoryProjectionStore::new(NonZeroUsize::MIN)),
    });
    let imported = EventReplicator::import_and_ack::<ReplicaTarget>(batch.clone())
        .run(Arc::clone(&target))
        .await;
    if !matches!(imported, effect::Exit::Success(ReplicaSync::Applied {
        cursor, count: 1,
    }) if cursor.value() == 1)
    {
        return false;
    }
    let projection = effect::projection! {
        key: ProjectionKey {
            id: ProjectionId::from_stable(event_key(&namespace, "projection")),
            stream: stream.clone(),
        },
        version: ProjectionVersion::new(NonZeroU32::MIN),
        accepted: BTreeMap::from([((group, tag), version)]),
        initial: || 0usize,
        fold: |count, _| Ok::<usize, ()>(count + 1),
    };
    let advanced = projection
        .advance(NonZeroUsize::MIN)
        .run(Arc::clone(&target))
        .await;
    if !matches!(advanced, effect::Exit::Success(ProjectionAdvance::Advanced {
        snapshot, ..
    }) if *snapshot.state == 1 && snapshot.checkpoint.value() == 1)
    {
        return false;
    }
    let key = InvalidationKey::from_stable(event_key(&namespace, "query"));
    let registry = InvalidationRegistry::new(PubSubOptions {
        capacity: NonZeroUsize::MIN,
        overflow: PubSubOverflow::Backpressure,
        replay: ReplayPolicy::None,
    })
    .expect("valid invalidation registry");
    let observation = EventObservation::new(registry.clone(), {
        let key = key.clone();
        move |_| vec![key.clone()]
    });
    if !matches!(
        registry
            .shutdown(PubSubShutdown::Discard)
            .run(Arc::clone(&target))
            .await,
        effect::Exit::Success(true)
    ) {
        return false;
    }
    let failed = observation
        .reconcile(stream.clone(), JournalCursor::new(0), NonZeroUsize::MIN)
        .run(Arc::clone(&target))
        .await;
    if !matches!(failed, effect::Exit::Fail(EventObservationError::Notification {
        replay_after, ..
    }) if replay_after.value() == 0)
    {
        return false;
    }
    let cursor_key = ReplicaCursorKey {
        source,
        stream: stream.clone(),
    };
    let checkpoint = target
        .cursors
        .load(cursor_key.clone())
        .run(Arc::new(()))
        .await;
    if !matches!(checkpoint, effect::Exit::Success(cursor) if cursor.value() == 1) {
        return false;
    }
    let restarted = InvalidationRegistry::new(PubSubOptions {
        capacity: NonZeroUsize::MIN,
        overflow: PubSubOverflow::Backpressure,
        replay: ReplayPolicy::None,
    })
    .expect("valid restarted registry");
    let subscriber = restarted
        .subscribe::<ReplicaTarget>()
        .run(Arc::clone(&target))
        .await;
    let effect::Exit::Success(subscriber) = subscriber else {
        return false;
    };
    let observation = EventObservation::new(restarted, {
        let key = key.clone();
        move |_| vec![key.clone()]
    });
    let recovered = observation
        .reconcile(stream.clone(), JournalCursor::new(0), NonZeroUsize::MIN)
        .run(Arc::clone(&target))
        .await;
    let notification = subscriber.take().run(Arc::clone(&target)).await;
    let retry = EventReplicator::import_and_ack::<ReplicaTarget>(batch)
        .run(Arc::clone(&target))
        .await;
    let journal_cursor = target.journal.cursor(stream).run(Arc::new(())).await;
    let projection_snapshot = target
        .projections
        .load(projection.key)
        .run(Arc::new(()))
        .await;
    matches!(recovered, effect::Exit::Success(report) if report.next.value() == 1)
        && matches!(notification, effect::Exit::Success(SubscriptionEvent::Item(batch))
            if batch.keys.as_ref() == [key])
        && matches!(retry, effect::Exit::Success(ReplicaSync::Applied { cursor, .. })
            if cursor.value() == 1)
        && matches!(journal_cursor, effect::Exit::Success(cursor) if cursor.value() == 1)
        && matches!(projection_snapshot, effect::Exit::Success(Some(snapshot))
            if *snapshot.state == 1 && snapshot.checkpoint.value() == 1)
}

type EngineError<R> = <<R as HasWorkflowEngine>::Engine as WorkflowEngine>::Error;
type WorkflowCpuError<R> = WorkflowRunError<EngineError<R>, ActivityRunError<EngineError<R>, ()>>;

pub fn workflow_cpu<R>() -> Effect<WorkflowRunResult<i64, i64>, WorkflowCpuError<R>, R>
where
    R: HasWorkflowEngine + HasCpuExecutor + Send + Sync + 'static,
{
    let namespace =
        KeyNamespace::<ProviderDomain>::new("workflow").expect("valid workflow fixture namespace");
    let run = WorkflowRunId::from_stable(event_key(&namespace, "run"));
    let activity = effect::activity! {
        name: WorkflowActivityId::from_stable(event_key(&namespace, "activity")),
        input: json_i64(),
        success: json_i64(), error: json_i64(), limits: rpc_limits(),
        body |value| {
            let observed = *value;
            Effect::<Result<i64, i64>, (), R>::cpu_once(move |_| Ok(observed + 1))?
        },
    };
    let execution_run = run.clone();
    let definition = effect::workflow! {
        name: WorkflowDefinitionId::new("provider-workflow").expect("valid definition"),
        version: WorkflowVersion::new(NonZeroU32::MIN),
        payload: json_i64(), success: json_i64(), error: json_i64(),
        compatibility: Arc::from(&b"provider-v1"[..]),
        limits: rpc_limits(),
        run |value| {
            activity.call(execution_run.clone(), NonZeroU32::MIN, value)?
        },
    };
    definition.run(run, 41)
}

pub fn reactivity_cpu<R>() -> Effect<bool, AtomError<()>, R>
where
    R: HasCpuExecutor + Send + Sync + 'static,
{
    let namespace = KeyNamespace::<ProviderDomain>::new("reactivity")
        .expect("valid reactive fixture namespace");
    let registry = AtomRegistry::new(PubSubOptions {
        capacity: NonZeroUsize::MIN,
        overflow: PubSubOverflow::Backpressure,
        replay: ReplayPolicy::None,
    })
    .expect("valid atom registry");
    let atom = effect::reactive_derived! {
        registry: registry,
        id: AtomId::from_stable(event_key(&namespace, "derived")),
        capacity: NonZeroUsize::MIN,
        overflow: PubSubOverflow::Backpressure,
        evaluate: |_| { Effect::<i64, (), R>::cpu_once(|_| 42)? },
    }
    .expect("valid derived atom");
    let read = atom.clone();
    atom.refresh().flat_map(move |_| {
        read.read().map(|state| {
            state
                .current_success()
                .is_some_and(|value| value.with(|number| *number == 42).unwrap_or(false))
        })
    })
}

type StoreError<R> = <<R as HasKeyValueStore>::Store as KeyValueStore>::Error;
type PersistenceCpuError<R> = PersistenceError<StoreError<R>>;

pub fn persistence_cpu<R>() -> Effect<StoredLookup<i64, i64>, PersistenceCpuError<R>, R>
where
    R: HasKeyValueStore + HasCpuExecutor + Send + Sync + 'static,
{
    let namespace = KeyNamespace::<ProviderDomain>::new("persistence")
        .expect("valid persistence fixture namespace");
    let definition = effect::persistable! {
        namespace: namespace,
        request: i64,
        key: |_: &i64| -> &'static str { "entry" },
        success: json_i64(), error: json_i64(),
        version: RecordPolicy::new(
            RecordVersion::new("provider-v1").expect("valid record version"),
            rpc_limits(),
        ),
    };
    Effect::<i64, PersistenceCpuError<R>, R>::cpu_once(|_| 42).flat_map(move |value| {
        let read_definition = definition.clone();
        effect::persistence::write(
            definition.clone(),
            || 41_i64,
            move || StoredOutcome::Success(value),
            None,
        )
        .flat_map(move |_| {
            effect::persistence::read(read_definition.clone(), || 41_i64, UtcMillis::new(0))
        })
    })
}

/// A stale queue settlement can occur after the workflow result is recorded.
/// Recovery acknowledges that result with the valid fence and never reruns
/// the handler; queue and workflow storage remain independent capabilities.
pub async fn queue_workflow_race<R>(environment: Arc<R>) -> bool
where
    R: HasPersistedQueueStore<Store = MemoryPersistedQueueStore>
        + HasWorkflowEngine<Engine = MemoryWorkflowEngine>
        + HasCpuExecutor
        + Send
        + Sync
        + 'static,
{
    let namespace = KeyNamespace::<ProviderDomain>::new("queue-workflow")
        .expect("valid queue workflow namespace");
    let identity = event_key(&namespace, "run");
    let run = WorkflowRunId::from_stable(identity.clone());
    let version = WorkflowVersion::new(NonZeroU32::MIN);
    let definition =
        WorkflowDefinitionId::new("provider-queue-workflow").expect("valid workflow definition");
    let registration = WorkflowRegistration {
        id: definition.clone(),
        version,
        compatibility: Arc::from(&b"provider-v1"[..]),
    };
    if !matches!(
        environment
            .workflow_engine()
            .register(registration)
            .run(Arc::new(()))
            .await,
        effect::Exit::Success(())
    ) {
        return false;
    }
    let handle = WorkflowHandle {
        run: run.clone(),
        definition,
        version,
    };
    if !matches!(
        environment
            .workflow_engine()
            .start(handle, Arc::from(&b"input"[..]))
            .run(Arc::new(()))
            .await,
        effect::Exit::Success(_)
    ) {
        return false;
    }
    let deferred = DurableDeferred {
        run,
        signal: WorkflowSignalId::from_stable(event_key(&namespace, "result")),
        version,
        codec: json_i64(),
        limits: rpc_limits(),
    };
    let queue = DurableQueue::new(PersistedQueue::new(
        PersistedQueueName::new("provider-jobs").expect("valid queue name"),
        json_i64(),
        BinaryIdentity::new("provider-job-payload", NonZeroU64::MIN)
            .expect("valid queue payload format"),
        event_limits(),
        QueueMessagePolicy {
            maximum_attempts: NonZeroU32::new(2).expect("stale and valid attempts"),
            acknowledged_retention: Duration::ZERO,
        },
    ));
    if !matches!(
        queue
            .enqueue(
                identity.clone(),
                || 41,
                &deferred,
                UtcMillis::new(0),
                UtcMillis::new(0)
            )
            .run(Arc::clone(&environment))
            .await,
        effect::Exit::Success(_)
    ) {
        return false;
    }
    let options = QueueClaimOptions::new(
        QueueOwner::new("provider-worker").expect("valid worker"),
        UtcMillis::new(0),
        UtcMillis::new(1),
        NonZeroUsize::MIN,
    )
    .expect("valid claim window");
    let claimed = queue.claim(options).run(Arc::clone(&environment)).await;
    let effect::Exit::Success(mut claims) = claimed else {
        return false;
    };
    if claims.len() != 1 {
        return false;
    }
    let stale_claim = claims.pop().expect("one claim");
    let replacement_options = QueueClaimOptions::new(
        QueueOwner::new("replacement-worker").expect("valid replacement worker"),
        UtcMillis::new(1),
        UtcMillis::new(2),
        NonZeroUsize::MIN,
    )
    .expect("valid replacement claim window");
    let replacement = queue
        .claim(replacement_options)
        .run(Arc::clone(&environment))
        .await;
    let effect::Exit::Success(mut replacement_claims) = replacement else {
        return false;
    };
    if replacement_claims.len() != 1 {
        return false;
    }
    let valid_token = replacement_claims
        .pop()
        .expect("one replacement claim")
        .token;
    let executions = Arc::new(AtomicUsize::new(0));
    let handler_executions = Arc::clone(&executions);
    let stale = queue
        .handle_claim(
            stale_claim,
            deferred.clone(),
            move |value| {
                let executions = Arc::clone(&handler_executions);
                Effect::<i64, (), R>::cpu_once(move |_| {
                    executions.fetch_add(1, Ordering::AcqRel);
                    value + 1
                })
            },
            UtcMillis::new(0),
        )
        .run(Arc::clone(&environment))
        .await;
    if !matches!(
        stale,
        effect::Exit::Fail(DurableQueueWorkerError::Queue(
            DurableQueueError::StaleClaim
        ))
    ) {
        return false;
    }
    let recorded = deferred.wait().run(Arc::clone(&environment)).await;
    let claim_state = queue
        .inspect(identity.clone())
        .run(Arc::clone(&environment))
        .await;
    if !matches!(
        recorded,
        effect::Exit::Success(DurableSignalWait::Ready(42))
    ) || !matches!(claim_state, effect::Exit::Success(Some(record))
            if matches!(record.state, QueueRecordState::Claimed { .. }))
    {
        return false;
    }
    let recovered = queue
        .acknowledge_recorded(valid_token, deferred, UtcMillis::new(1))
        .run(Arc::clone(&environment))
        .await;
    let settled = queue.inspect(identity).run(environment).await;
    matches!(recovered, effect::Exit::Success(QueueSettlement::Applied))
        && matches!(settled, effect::Exit::Success(Some(record))
            if matches!(record.state, QueueRecordState::Acknowledged { .. }))
        && executions.load(Ordering::Acquire) == 1
}

pub fn transaction_cpu<R>(reference: TxRef<i64>) -> Effect<i64, TransactionError<()>, R>
where
    R: HasTransactionDomain + HasCpuExecutor + Send + Sync + 'static,
{
    let policy = TransactionPolicy {
        max_conflicts: NonZeroUsize::MIN,
    };
    Effect::<i64, TransactionError<()>, R>::cpu_once(|_| 42).flat_map(move |value| {
        let read = reference.clone();
        let write = reference.clone();
        let transaction: Transaction<i64, ()> = effect::transaction! {
            write.write(Arc::new(value))?;
            let stored = read.read()?;
            *stored
        };
        transaction.atomically(policy)
    })
}

#[derive(Debug)]
pub enum TransactionReactiveError {
    Subscription(TxSubscriptionError),
    Transaction(TransactionError<()>),
    Invalidation(InvalidationError),
    AbortInvariant,
    NotificationInvariant,
}

impl std::fmt::Display for TransactionReactiveError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Subscription(error) => std::fmt::Debug::fmt(error, formatter),
            Self::Transaction(error) => std::fmt::Debug::fmt(error, formatter),
            Self::Invalidation(error) => std::fmt::Debug::fmt(error, formatter),
            Self::AbortInvariant => formatter.write_str("aborted transaction became visible"),
            Self::NotificationInvariant => {
                formatter.write_str("failed notification was reported as published")
            }
        }
    }
}

pub fn transactional_reactive<R>(
    source: TxSubscriptionRef<i64>,
) -> Effect<bool, TransactionReactiveError, R>
where
    R: HasTransactionDomain + HasCpuExecutor + Send + Sync + 'static,
{
    let policy = TransactionPolicy {
        max_conflicts: NonZeroUsize::MIN,
    };
    source
        .subscribe()
        .map_error(TransactionReactiveError::Subscription)
        .flat_map(move |(initial, subscription)| {
            let source = source.clone();
            source
                .set::<()>(Arc::new(0))
                .flat_map(|_| Transaction::<(), ()>::fail_with(|| ()))
                .atomically(policy)
                .either()
                .map_error(TransactionReactiveError::Transaction)
                .flat_map(move |aborted| {
                    if !matches!(aborted, Err(TransactionError::Domain(()))) {
                        return Effect::fail_with(|| TransactionReactiveError::AbortInvariant);
                    }
                    let source = source.clone();
                    let subscription = subscription.clone();
                    let initial = Arc::clone(&initial);
                    source
                        .read_committed::<R>()
                        .map_error(TransactionReactiveError::Subscription)
                        .flat_map(move |before| {
                            if *before != 41 {
                                return Effect::fail_with(|| {
                                    TransactionReactiveError::AbortInvariant
                                });
                            }
                            let source = source.clone();
                            let subscription = subscription.clone();
                            let initial = Arc::clone(&initial);
                            Effect::<i64, TransactionReactiveError, R>::cpu_once(|_| 42).flat_map(
                                move |value| {
                                    let subscription = subscription.clone();
                                    let initial = Arc::clone(&initial);
                                    source
                                        .set::<()>(Arc::new(value))
                                        .atomically(policy)
                                        .map_error(TransactionReactiveError::Transaction)
                                        .flat_map(move |_| {
                                            let disposal = subscription.clone();
                                            let initial = Arc::clone(&initial);
                                            subscription
                                                .next()
                                                .map_error(TransactionReactiveError::Subscription)
                                                .map(move |committed| {
                                                    let _ = disposal.dispose();
                                                    *initial == 41 && *committed == 42
                                                })
                                        })
                                },
                            )
                        })
                })
        })
}

pub fn notification_loss_after_commit<R>(
    source: TxSubscriptionRef<i64>,
) -> Effect<bool, TransactionReactiveError, R>
where
    R: HasTransactionDomain + HasCpuExecutor + Send + Sync + 'static,
{
    let namespace = KeyNamespace::<ProviderDomain>::new("transaction-notification")
        .expect("valid notification fixture namespace");
    let key = InvalidationKey::from_stable(event_key(&namespace, "source"));
    let registry = InvalidationRegistry::new(PubSubOptions {
        capacity: NonZeroUsize::MIN,
        overflow: PubSubOverflow::Backpressure,
        replay: ReplayPolicy::None,
    })
    .expect("valid invalidation registry");
    let policy = TransactionPolicy {
        max_conflicts: NonZeroUsize::MIN,
    };
    Effect::<i64, TransactionReactiveError, R>::cpu_once(|_| 42).flat_map(move |value| {
        let registry = registry.clone();
        let source = source.clone();
        let key = key.clone();
        source
            .set::<()>(Arc::new(value))
            .atomically(policy)
            .map_error(TransactionReactiveError::Transaction)
            .flat_map(move |_| {
                let registry = registry.clone();
                let source = source.clone();
                let key = key.clone();
                registry
                    .shutdown(PubSubShutdown::Discard)
                    .map_error(TransactionReactiveError::Invalidation)
                    .flat_map(move |_| {
                        let source = source.clone();
                        registry
                            .invalidate(vec![key.clone()])
                            .either()
                            .map_error(TransactionReactiveError::Invalidation)
                            .flat_map(move |notification| {
                                if !matches!(
                                    notification,
                                    Err(InvalidationError::PubSub(effect::PubSubError::Shutdown))
                                ) {
                                    return Effect::fail_with(|| {
                                        TransactionReactiveError::NotificationInvariant
                                    });
                                }
                                source
                                    .read_committed::<R>()
                                    .map_error(TransactionReactiveError::Subscription)
                                    .map(|committed| *committed == 42)
                            })
                    })
            })
    })
}

pub struct CpuClusterExecutor<R>(PhantomData<fn() -> R>);

impl<R> CpuClusterExecutor<R> {
    pub fn new() -> Self {
        Self(PhantomData)
    }
}

impl<R> ClusterExecutor<R> for CpuClusterExecutor<R>
where
    R: HasCpuExecutor + Send + Sync + 'static,
{
    type Error = ();

    fn execute(
        &self,
        _address: ClusterAddress,
        encoded_request: Arc<[u8]>,
    ) -> Effect<SavedReply, Self::Error, R> {
        Effect::cpu_once(move |_| SavedReply {
            kind: ReplyKind::Success,
            payload: encoded_request,
        })
    }
}

pub fn cluster_store() -> Arc<MemoryClusterMessageStore> {
    Arc::new(MemoryClusterMessageStore::new(
        MessageStoreCapacity::Bounded(NonZeroUsize::MIN),
    ))
}

pub async fn entity_macro<R>(environment: Arc<R>) -> bool
where
    R: Send + Sync + 'static,
{
    let policy = RpcEnvelopePolicy {
        frame: rpc_frame(),
        json: rpc_limits(),
    };
    let operation = effect::rpc_operation! {
        id: OperationId::new("provider-entity-increment").expect("entity operation"),
        version: RpcVersion::new(NonZeroU32::MIN),
        request: json_i64(), success: json_i64(), error: json_i64(),
        limits: rpc_limits(),
    };
    let handler = RpcHandler::new(operation.clone(), |request| {
        Effect::<i64, i64, R>::sync_once(move || request + 1)
    });
    let entity_type = ClusterEntityType::new("provider-macro-entity").expect("entity type");
    let protocol = effect::rpc_protocol! {
        policy: policy,
        handlers: [handler.clone()],
    }
    .expect("typed protocol registration");
    let dispatcher: EntityDispatcher<R> = effect::entity! {
        policy: policy,
        entity_type: entity_type.clone(),
        handler: handler,
    }
    .expect("typed entity registration");
    let namespace = KeyNamespace::<ProviderDomain>::new("macro-entity").expect("entity namespace");
    let address = ClusterAddress {
        entity_type,
        entity: ClusterEntityId::from_stable(event_key(&namespace, "entity")),
        shard: ClusterShardId::from_stable(event_key(&namespace, "shard")),
    };
    let frame = RpcEnvelope {
        operation: operation.spec().id.clone(),
        version: operation.spec().version,
        call: RpcCallId::new("provider-entity-call").expect("entity call"),
        kind: RpcEnvelopeKind::Request,
        sequence: None,
        payload: JsonValue::from(41),
        metadata: Default::default(),
    }
    .encode(policy)
    .expect("entity request frame")
    .into_bytes()
    .expect("entity request bytes");
    if !matches!(
        protocol
            .handle(frame.clone())
            .run(Arc::clone(&environment))
            .await,
        effect::Exit::Success(_)
    ) {
        return false;
    }
    matches!(
        dispatcher
            .execute(address, Arc::from(frame))
            .run(environment)
            .await,
        effect::Exit::Success(SavedReply {
            kind: ReplyKind::Success,
            ..
        })
    )
}

pub fn cluster_prepare(
    store: Arc<MemoryClusterMessageStore>,
) -> Effect<ClaimOptions, ClusterStoreError> {
    let namespace =
        KeyNamespace::<ProviderDomain>::new("cluster").expect("valid cluster fixture namespace");
    let address = ClusterAddress {
        entity_type: ClusterEntityType::new("provider-entity").expect("valid entity type"),
        entity: ClusterEntityId::from_stable(event_key(&namespace, "entity")),
        shard: ClusterShardId::from_stable(event_key(&namespace, "shard")),
    };
    let request = AcceptedRequest {
        address: address.clone(),
        id: ClusterRequestId::from_stable(event_key(&namespace, "request")),
        payload: Arc::from(&b"provider-message"[..]),
        idempotency_key: None,
        accepted_at: UtcMillis::new(1),
        policy: AcceptancePolicy {
            dedup_retention: Duration::ZERO,
            reply_retention: Duration::ZERO,
            payload_conflict: PayloadConflictPolicy::Reject,
            maximum_attempts: NonZeroU32::MIN,
        },
    };
    let activation = ActivationId::from_stable(event_key(&namespace, "activation"));
    store.accept(request).flat_map(move |_| {
        store
            .activate(
                address.clone(),
                activation.clone(),
                UtcMillis::new(1),
                UtcMillis::new(3),
            )
            .map(|outcome| {
                let lease = match outcome {
                    ActivationOutcome::Activated { lease, .. }
                    | ActivationOutcome::AlreadyActive(lease) => lease,
                    ActivationOutcome::Busy(_) => panic!("fixture activation was busy"),
                };
                ClaimOptions {
                    owner: lease.owner,
                    now: UtcMillis::new(1),
                    lease_until: UtcMillis::new(2),
                    limit: NonZeroUsize::MIN,
                }
            })
    })
}

type ClusterCpuError = ClusterRunnerError<ClusterStoreError, (), std::convert::Infallible>;

pub fn cluster_cpu<R>(
    claim: ClaimOptions,
) -> Effect<Vec<Result<ClusterRunOutcome, ClusterCpuError>>, ClusterCpuError, R>
where
    R: HasClusterMessageStore<Store = MemoryClusterMessageStore>
        + HasClusterExecutor<Executor = CpuClusterExecutor<R>>
        + HasTaskSpawner
        + HasWallClock<Clock = TestWallClock>
        + HasCpuExecutor
        + Send
        + Sync
        + 'static,
{
    ClusterRunner::run_once(claim, NonZeroUsize::MIN)
}

pub fn completed_cluster_run(outcomes: &[Result<ClusterRunOutcome, ClusterCpuError>]) -> bool {
    matches!(
        outcomes,
        [Ok(ClusterRunOutcome {
            completion: StoreCompletion::Completed,
            ..
        })]
    )
}

pub fn monotonic_timer<R>(duration: Duration) -> Effect<Duration, (), R>
where
    R: HasClock + Send + Sync + 'static,
{
    Effect::<(), (), R>::sleep(duration).flat_map(|_| Effect::<Duration, (), R>::clock_now())
}
