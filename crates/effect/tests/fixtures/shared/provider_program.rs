use effect::StoredOutcome;
use effect::cluster::{
    AcceptancePolicy, AcceptedRequest, ActivationId, ActivationOutcome, ClaimOptions,
    ClusterActivationStore, ClusterAddress, ClusterDefinition, ClusterEntityId, ClusterEntityType,
    ClusterExecutor, ClusterMessageStore, ClusterRequestId, ClusterRunOutcome, ClusterRunner,
    ClusterRunnerError, ClusterShardId, ClusterStoreError, HasClusterActivationStore,
    HasClusterExecutor, HasClusterMessageStore, MemoryClusterMessageStore, MessageStoreCapacity,
    PayloadConflictPolicy, ReplyKind, SavedReply, StoreCompletion,
};
use effect::encoding::{BinaryIdentity, BinaryLimits, FrameDecoder, FrameParts, FramePolicy};
use effect::eventlog::{
    AggregateId, AtomicProjectionStore, EventCommitPolicy, EventEnvelope, EventGroupId, EventId,
    EventInvocation, EventJournal, EventJournalId, EventLog, EventObservation,
    EventObservationError, EventReplicator, EventRunError, EventRunOutcome, EventTag, EventVersion,
    HasAtomicProjectionStore, HasEventJournal, HasReplicaCursorStore, JournalCursor, JournalStream,
    MemoryEventJournal, MemoryJournalCapacity, MemoryProjectionStore, MemoryReplicaCursorStore,
    ProjectionAdvance, ProjectionId, ProjectionKey, ProjectionVersion, ReplicaCursorKey,
    ReplicaCursorStore, ReplicaExport, ReplicaId, ReplicaSync,
};
use effect::persistence::{
    HasKeyValueStore, HasPersistedQueueStore, KeyValueStore, MemoryPersistedQueueStore,
    PersistedQueueName, PersistenceError, QueueClaimOptions, QueueMessagePolicy, QueueOwner,
    QueueRecordState, QueueSettlement, RecordPolicy, RecordVersion, StoredLookup,
};
use effect::reactivity::{
    AtomError, AtomId, AtomRegistry, InvalidationError, InvalidationKey, InvalidationRegistry,
    RpcMutationAtom, RpcMutationError,
};
use effect::rpc::{
    InProcessRpcChannel, OperationId, RpcCallId, RpcClient, RpcEnvelope, RpcEnvelopeKind,
    RpcEnvelopePolicy, RpcServerError, RpcUnaryServer, RpcVersion,
};
use effect::transactional::{
    HasTransactionDomain, Transaction, TransactionError, TransactionPolicy, TxRef,
    TxSubscriptionError, TxSubscriptionRef,
};
use effect::workflow::{
    ActivityRunError, DurableDeferred, DurableQueueError, DurableQueueWorkerError,
    DurableSignalWait, HasWorkflowEngine, MemoryWorkflowEngine, WorkflowActivityId,
    WorkflowDefinitionId, WorkflowEngine, WorkflowHandle, WorkflowRegistration, WorkflowRunError,
    WorkflowRunId, WorkflowRunResult, WorkflowSignalId, WorkflowVersion,
};
use effect::{
    Cancellation, Codec, Defect, DuplicateFields, Effect, HasBlockingExecutor, HasCpuExecutor,
    HasTaskSpawner, JsonValue, OutcomeStatus, ParseLimits, ParsePolicy, PubSubOptions,
    PubSubOverflow, PubSubShutdown, ReplayPolicy, SchemaErrorMode, SchemaIssue, Scope,
    SubscriptionEvent, UnknownFields, json_i64,
};
use effect::{HasClock, HasWallClock, TestWallClock, WallClock};
use effect::{KeyDomain, KeyNamespace, KeySegment, StableKey, UtcMillis};
use effect::{
    activity, cluster, codec, durable_queue, effect, effect_all_concurrent, effect_blocking,
    effect_cpu, effect_die, effect_future, effect_map, effect_map_error, effect_scope, effect_sync,
    effect_try_sync, entity, entity_resource, event, persistable, persisted_queue, projection,
    reactive_derived, rpc_operation, rpc_protocol, schema, transaction, workflow,
};
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
    let task = {
        let __dsl_program: Effect<usize, (), R> = effect_sync! { once; body: move || {
            task_overlap.wait();
            20
        }, };
        __dsl_program
    };
    let cpu = {
        let __dsl_program: Effect<usize, (), R> = effect_cpu! { once; body: move |_| {
            overlap.wait();
            21
        }, };
        __dsl_program
    };
    effect! {
    bind;
    program: effect_all_concurrent! { concurrency: NonZeroUsize::MIN.saturating_add(1), items: vec![task, cpu], },
    body: |values| {
               { let __dsl_program: Effect < usize , () , R > = effect_blocking! { once; body: move |_| values.into_iter().sum::<usize>() + 1, }; __dsl_program }
           },
    }
}

pub fn cancel_and_join<R>() -> Effect<bool, (), R>
where
    R: HasTaskSpawner + Send + Sync + 'static,
{
    effect_scope! {
    body: |scope| {
            let entered = Cancellation::new();
            let witness = entered.clone();
            let child = { let __dsl_program: Effect < () , () , R > = effect_future! { body: move |token| {
                let witness = witness.clone();
                async move {
                    let _ = witness.request();
                    token.cancelled().await;
                }
            }, }; __dsl_program };
            effect! {
     bind;
     program: scope.fork(child),
     body: move |fiber| {
                let entered = entered.clone();
                effect! {
     bind;
     program: { let __dsl_program: Effect < () , () , R > = effect_future! { body: move |_| {
                    let entered = entered.clone();
                    async move { entered.cancelled().await }
                }, }; __dsl_program },
     body: move |_| {
                    effect_map! { program: fiber
                        .interrupt_shared(), map: |exit| exit.status() == OutcomeStatus::Interrupted, }
                },
     }
            },
     }
        },
    }
}

pub fn rpc_cpu<R>(
    limits: ParseLimits,
    frame: FramePolicy,
) -> Effect<FrameParts, RpcServerError<i64>, R>
where
    R: HasCpuExecutor + Send + Sync + 'static,
{
    let program = (|| -> Result<_, String> {
        let operation_id = OperationId::new("provider-rpc-increment")
            .map_err(|error| format!("operation: {error:?}"))?;
        let operation = rpc_operation! {
            id: operation_id,
            version: RpcVersion::new(NonZeroU32::MIN),
            request: json_i64(), success: json_i64(), error: json_i64(), limits: limits,
        };
        let policy = RpcEnvelopePolicy {
            frame,
            json: limits,
        };
        let handler = rpc_operation! { operation: operation.clone(), handler: |request| {
            { let __dsl_program: Effect < i64 , i64 , R > = effect_cpu! { once; body: move |_| request + 1, }; __dsl_program }
        }, };
        let envelope = RpcEnvelope {
            operation: operation.spec().id.clone(),
            version: operation.spec().version,
            call: RpcCallId::new("provider-rpc-call")
                .map_err(|error| format!("call: {error:?}"))?,
            kind: RpcEnvelopeKind::Request,
            sequence: None,
            payload: JsonValue::from(41),
            metadata: Default::default(),
        };
        let request = envelope
            .encode(policy)
            .map_err(|error| format!("envelope: {error:?}"))?
            .into_bytes()
            .map_err(|error| format!("frame: {error:?}"))?;
        Ok(RpcUnaryServer::new(handler, policy).handle(request))
    })();
    match program {
        Ok(program) => program,
        Err(error) => effect_die! { defect: Defect::new(error), },
    }
}

pub fn schema_cpu<R>() -> Effect<i64, SchemaIssue, R>
where
    R: HasCpuExecutor + Send + Sync + 'static,
{
    let program = (|| -> Result<_, String> {
        struct Record {
            value: i64,
        }
        let policy = ParsePolicy {
            limits: rpc_limits(),
            errors: SchemaErrorMode::First,
            unknown_fields: UnknownFields::Reject,
            duplicate_fields: DuplicateFields::Reject,
        };
        let record = schema! {
            struct Record { value: json_i64() } policy: policy
        }
        .map_err(|error| format!("schema definition: {error:?}"))?;
        let encoded = record
            .encode_ref(&Record { value: 42 })
            .map_err(|error| format!("schema encoding: {error:?}"))?;
        assert_eq!(
            record
                .decode_ref(&encoded)
                .map_err(|error| format!("schema decoding: {error:?}"))?
                .value,
            42
        );
        let codec = {
            let __dsl_program: Codec<i64, JsonValue, R, R> = codec! { decode: |encoded| {
                           effect! {
            bind;
            program: { let __dsl_program: Effect < Result < i64 , SchemaIssue > , SchemaIssue , R > = effect_cpu! { once; body: move |_| {
                               json_i64().decode_ref(&encoded)
                           }, }; __dsl_program },
            body: |value| effect_try_sync! { once; body: move || value, on_error: ::core::convert::identity, },
            }
                       }, encode: |value| {
                           effect! {
            bind;
            program: { let __dsl_program: Effect < Result < JsonValue , SchemaIssue > , SchemaIssue , R > = effect_cpu! { once; body: move |_| {
                               json_i64().encode_ref(&value)
                           }, }; __dsl_program },
            body: |value| effect_try_sync! { once; body: move || value, on_error: ::core::convert::identity, },
            }
                       }, };
            __dsl_program
        };
        Ok(codec.as_effect_schema().decode_once(JsonValue::from(42)))
    })();
    match program {
        Ok(program) => program,
        Err(error) => effect_die! { defect: Defect::new(error), },
    }
}

#[must_use]
pub fn rpc_limits() -> ParseLimits {
    ParseLimits {
        bytes: NonZeroUsize::MAX,
        depth: NonZeroUsize::MAX,
        nodes: NonZeroUsize::MAX,
        collection: NonZeroUsize::MAX,
        issues: NonZeroUsize::MAX,
    }
}

#[must_use]
pub fn rpc_frame() -> FramePolicy {
    FramePolicy {
        maximum_payload: NonZeroUsize::MAX,
    }
}

pub fn assert_rpc_response(
    response: FrameParts,
    limits: ParseLimits,
    frame: FramePolicy,
) -> Result<(), String> {
    let policy = RpcEnvelopePolicy {
        frame,
        json: limits,
    };
    let bytes = response
        .into_bytes()
        .map_err(|error| format!("response bytes: {error:?}"))?;
    let mut decoder = FrameDecoder::new(frame);
    let frames = decoder
        .push(&bytes)
        .map_err(|error| format!("response frame: {error:?}"))?;
    decoder
        .finish()
        .map_err(|error| format!("response completion: {error:?}"))?;
    assert_eq!(frames.len(), 1);
    let frame = frames.first().ok_or("missing response frame")?;
    let envelope = RpcEnvelope::decode(frame, policy)
        .map_err(|error| format!("response envelope: {error:?}"))?;
    assert_eq!(envelope.kind, RpcEnvelopeKind::Success);
    assert_eq!(envelope.payload, JsonValue::from(42));
    Ok(())
}

struct ProviderDomain;

impl KeyDomain for ProviderDomain {
    const TAG: &'static str = "provider-exercise";
}

fn event_key(
    namespace: &KeyNamespace<ProviderDomain>,
    value: &'static str,
) -> Result<StableKey<ProviderDomain>, String> {
    StableKey::new(namespace, [KeySegment::Text(value.into())])
        .map_err(|error| format!("{value} key: {error:?}"))
}

type EventCpuError<R> = EventRunError<<<R as HasEventJournal>::Journal as EventJournal>::Error, ()>;

pub fn event_cpu<R>(limits: BinaryLimits) -> Effect<EventRunOutcome<i64, i64>, EventCpuError<R>, R>
where
    R: HasEventJournal + HasCpuExecutor + Send + Sync + 'static,
{
    let program = (|| -> Result<_, String> {
        let namespace = KeyNamespace::<ProviderDomain>::new("events")
            .map_err(|error| format!("event namespace: {error:?}"))?;
        let identity = |name| {
            BinaryIdentity::new(name, NonZeroU64::MIN)
                .map_err(|error| format!("binary identity: {error:?}"))
        };
        let group =
            EventGroupId::new("provider-events").map_err(|error| format!("group: {error:?}"))?;
        let tag = EventTag::new("incremented").map_err(|error| format!("tag: {error:?}"))?;
        let payload_identity = identity("provider-payload")?;
        let success_identity = identity("provider-success")?;
        let error_identity = identity("provider-error")?;
        let definition = event! {
            group: group,
            tag: tag,
            version: EventVersion::new(NonZeroU32::MIN),
            payload: json_i64(), success: json_i64(), error: json_i64(),
            payload_identity: payload_identity,
            success_identity: success_identity,
            error_identity: error_identity, limits: limits,
        };
        Ok(
            EventLog::new(definition, EventCommitPolicy::SuccessOnly).execute(
                EventInvocation {
                    journal: EventJournalId::from_stable(event_key(&namespace, "journal")?),
                    aggregate: AggregateId::from_stable(event_key(&namespace, "aggregate")?),
                    event: EventId::from_stable(event_key(&namespace, "event")?),
                    expected: JournalCursor::new(0),
                    occurred_at: UtcMillis::new(0),
                    payload: Arc::new(41),
                },
                |payload| {
                    let __dsl_program: Effect<Result<i64, i64>, (), R> =
                        effect_cpu! { once; body: move |_| Ok(*payload + 1), };
                    __dsl_program
                },
            ),
        )
    })();
    match program {
        Ok(program) => program,
        Err(error) => effect_die! { defect: Defect::new(error), },
    }
}

#[must_use]
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
    let fixture = (|| -> Result<_, String> {
        let namespace = KeyNamespace::<ProviderDomain>::new("rpc-event-notification")
            .map_err(|error| format!("RPC event namespace: {error:?}"))?;
        let journal_id = EventJournalId::from_stable(event_key(&namespace, "journal")?);
        let aggregate = AggregateId::from_stable(event_key(&namespace, "aggregate")?);
        let event_id = EventId::from_stable(event_key(&namespace, "event")?);
        let stream = JournalStream {
            journal: journal_id.clone(),
            aggregate: aggregate.clone(),
        };
        let identity = |name| {
            BinaryIdentity::new(name, NonZeroU64::MIN)
                .map_err(|error| format!("identity: {error:?}"))
        };
        let definition = event! { group: EventGroupId::new("provider-rpc-events")
        .map_err(|error| format!("group: {error:?}"))?, tag: EventTag::new("mutated").map_err(|error| format!("tag: {error:?}"))?, version: EventVersion::new(NonZeroU32::MIN), payload: json_i64(), success: json_i64(), error: json_i64(), payload_identity: identity("rpc-event-payload")?, success_identity: identity("rpc-event-success")?, error_identity: identity("rpc-event-error")?, limits: event_limits(), };
        let log = EventLog::new(definition, EventCommitPolicy::SuccessOnly);
        let executions = Arc::new(AtomicUsize::new(0));
        let handler_executions = Arc::clone(&executions);
        let operation = rpc_operation! { id: OperationId::new("provider-event-mutation")
        .map_err(|error| format!("operation: {error:?}"))?, version: RpcVersion::new(NonZeroU32::MIN), request: json_i64(), success: json_i64(), error: json_i64(), limits: rpc_limits(),  };
        let handler = rpc_operation! { operation: operation.clone(), handler: move |request| {
                   let executions = Arc::clone(&handler_executions);
                   effect! {
        bind;
        program: effect_map_error! { program: log.execute(
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
                           { let __dsl_program: Effect < Result < i64 , i64 > , () , R > = effect_cpu! { once; body: move |_| {
                               executions.fetch_add(1, Ordering::AcqRel);
                               Ok(request + 1)
                           }, }; __dsl_program }
                       },
                   ), map: |_| -2, },
        body: |outcome| match outcome {
                       EventRunOutcome::Committed {
                           result: Ok(value), ..
                       }
                       | EventRunOutcome::Duplicate {
                           result: Ok(value), ..
                       } => effect_sync! { once; body: move || value, },
                       EventRunOutcome::Committed { result: Err(_), .. }
                       | EventRunOutcome::Duplicate { result: Err(_), .. }
                       | EventRunOutcome::Accepted
                       | EventRunOutcome::Rejected(_)
                       | EventRunOutcome::ConflictingEventId
                       | EventRunOutcome::CompactedEventId
                       | EventRunOutcome::StaleCursor { .. } => effect_try_sync! { body: { let __dsl_error = || -3; move || Err(__dsl_error()) }, on_error: ::core::convert::identity, },
                   },
        }
               }, };
        let policy = RpcEnvelopePolicy {
            frame: rpc_frame(),
            json: rpc_limits(),
        };
        let server = RpcUnaryServer::new(handler, policy);
        let channel = Arc::new(InProcessRpcChannel::new(server, environment.clone()));
        let client = RpcClient::<i64, i64, i64, RpcServerError<i64>>::new(
            operation,
            policy,
            NonZeroUsize::MIN,
        );
        let registry = InvalidationRegistry::new(PubSubOptions {
            capacity: NonZeroUsize::MIN,
            overflow: PubSubOverflow::Backpressure,
            replay: ReplayPolicy::None,
        })
        .map_err(|error| format!("invalidation registry: {error:?}"))?;
        let key = InvalidationKey::from_stable(event_key(&namespace, "query")?);
        let mutation = RpcMutationAtom::new(client, registry.clone(), [key]);
        let client_environment = Arc::new(channel);
        let call =
            RpcCallId::new("provider-event-call").map_err(|error| format!("call: {error:?}"))?;
        Ok((
            stream,
            executions,
            registry,
            mutation,
            client_environment,
            call,
        ))
    })();
    let Ok((stream, executions, registry, mutation, client_environment, call)) = fixture else {
        return false;
    };
    let shutdown = registry
        .shutdown(PubSubShutdown::Discard)
        .run(Arc::clone(&client_environment))
        .await;
    if !matches!(shutdown, effect::Exit::Success(true)) {
        return false;
    }
    let result = mutation
        .execute(41, call, Default::default())
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
    let fixture = (|| -> Result<_, String> {
        let namespace = KeyNamespace::<ProviderDomain>::new("replica-observation")
            .map_err(|error| format!("replica namespace: {error:?}"))?;
        let stream = JournalStream {
            journal: EventJournalId::from_stable(event_key(&namespace, "journal")?),
            aggregate: AggregateId::from_stable(event_key(&namespace, "aggregate")?),
        };
        let group = EventGroupId::new("provider-replica-events")
            .map_err(|error| format!("group: {error:?}"))?;
        let tag = EventTag::new("changed").map_err(|error| format!("tag: {error:?}"))?;
        let event_id = EventId::from_stable(event_key(&namespace, "event")?);
        let source = ReplicaId::from_stable(event_key(&namespace, "source")?);
        let projection_id = ProjectionId::from_stable(event_key(&namespace, "projection")?);
        let key = InvalidationKey::from_stable(event_key(&namespace, "query")?);
        Ok((stream, group, tag, event_id, source, projection_id, key))
    })();
    let Ok((stream, group, tag, event_id, source, projection_id, key)) = fixture else {
        return false;
    };
    let version = EventVersion::new(NonZeroU32::MIN);
    let payload = {
        let __dsl_program: Effect<Arc<[u8]>, (), R> =
            effect_cpu! { once; body: |_| Arc::from(&b"change"[..]), };
        __dsl_program
    }
    .run(Arc::clone(&environment))
    .await;
    let effect::Exit::Success(payload) = payload else {
        return false;
    };
    let event = EventEnvelope {
        journal: stream.journal.clone(),
        aggregate: stream.aggregate.clone(),
        id: event_id,
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
    let Some(first) = batch.entries().first() else {
        return false;
    };
    let Some(ahead) = first.sequence.get().checked_add(1) else {
        return false;
    };
    let ahead = JournalCursor::new(ahead);
    let cursor_key = ReplicaCursorKey {
        source: source.clone(),
        stream: stream.clone(),
    };
    let seeded = out_of_order
        .cursors
        .advance(cursor_key, JournalCursor::new(0), ahead)
        .run(Arc::new(()))
        .await;
    let rejected =
        EventReplicator::import_and_ack::<ReplicaTarget>(batch.clone(), replica_permit(&batch))
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
    let imported =
        EventReplicator::import_and_ack::<ReplicaTarget>(batch.clone(), replica_permit(&batch))
            .run(Arc::clone(&target))
            .await;
    if !matches!(imported, effect::Exit::Success(ReplicaSync::Applied {
        cursor, count: 1,
    }) if cursor.value() == 1)
    {
        return false;
    }
    let projection = projection! {
        key: ProjectionKey {
            id: projection_id,
            stream: stream.clone(),
        },
        version: ProjectionVersion::new(NonZeroU32::MIN),
        accepted: BTreeMap::from([((group, tag), version)]),
        initial: || 0usize,
        fold: |count: &usize, _: &effect::eventlog::JournalEntry| Ok::<usize, ()>(count + 1),
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
    let Ok(registry) = InvalidationRegistry::new(PubSubOptions {
        capacity: NonZeroUsize::MIN,
        overflow: PubSubOverflow::Backpressure,
        replay: ReplayPolicy::None,
    }) else {
        return false;
    };
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
    let Ok(restarted) = InvalidationRegistry::new(PubSubOptions {
        capacity: NonZeroUsize::MIN,
        overflow: PubSubOverflow::Backpressure,
        replay: ReplayPolicy::None,
    }) else {
        return false;
    };
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
    let retry =
        EventReplicator::import_and_ack::<ReplicaTarget>(batch.clone(), replica_permit(&batch))
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
    let program = (|| -> Result<_, String> {
        let namespace = KeyNamespace::<ProviderDomain>::new("workflow")
            .map_err(|error| format!("workflow namespace: {error:?}"))?;
        let run = WorkflowRunId::from_stable(event_key(&namespace, "run")?);
        let activity_id = WorkflowActivityId::from_stable(event_key(&namespace, "activity")?);
        let definition_id = WorkflowDefinitionId::new("provider-workflow")
            .map_err(|error| format!("workflow definition: {error:?}"))?;
        let activity = activity! {
            name: activity_id,
            input: json_i64(),
            success: json_i64(), error: json_i64(), limits: rpc_limits(),
            body |value: Arc<i64>| {
                let observed = *value;
                { let __dsl_program: Effect < Result < i64 , i64 > , () , R > = effect_cpu! { once; body: move |_| Ok(observed + 1), }; __dsl_program }?
            },
        };
        let execution_run = run.clone();
        let definition = workflow! {
            name: definition_id,
            version: WorkflowVersion::new(NonZeroU32::MIN),
            payload: json_i64(), success: json_i64(), error: json_i64(),
            compatibility: Arc::from(&b"provider-v1"[..]),
            limits: rpc_limits(),
            run |value: i64| {
                let activity = activity.clone(); let execution_run = execution_run.clone();
                activity.call(execution_run, NonZeroU32::MIN, value)?
            },
        };
        Ok(definition.run(run, 41))
    })();
    match program {
        Ok(program) => program,
        Err(error) => effect_die! { defect: Defect::new(error), },
    }
}

pub fn reactivity_cpu<R>() -> Effect<bool, AtomError<()>, R>
where
    R: HasCpuExecutor + Send + Sync + 'static,
{
    let program = (|| -> Result<_, String> {
        let namespace = KeyNamespace::<ProviderDomain>::new("reactivity")
            .map_err(|error| format!("reactivity namespace: {error:?}"))?;
        let registry = AtomRegistry::new(PubSubOptions {
            capacity: NonZeroUsize::MIN,
            overflow: PubSubOverflow::Backpressure,
            replay: ReplayPolicy::None,
        })
        .map_err(|error| format!("atom registry: {error:?}"))?;
        let id = AtomId::from_stable(event_key(&namespace, "derived")?);
        let atom = reactive_derived! {
            registry: registry,
            id: id,
            capacity: NonZeroUsize::MIN,
            overflow: PubSubOverflow::Backpressure,
            evaluate |_| { { let __dsl_program: Effect < i64 , () , R > = effect_cpu! { once; body: |_| 42, }; __dsl_program }? },
        }
        .map_err(|error| format!("derived atom: {error:?}"))?;
        let read = atom.clone();
        Ok(effect! {
        bind;
        program: atom.refresh(),
        body: move |_| {
                   effect_map! { program: read.read(), map: |state| {
                       state
                           .current_success()
                           .is_some_and(|value| value.with(|number| *number == 42).unwrap_or(false))
                   }, }
               },
        })
    })();
    match program {
        Ok(program) => program,
        Err(error) => effect_die! { defect: Defect::new(error), },
    }
}

type StoreError<R> = <<R as HasKeyValueStore>::Store as KeyValueStore>::Error;
type PersistenceCpuError<R> = PersistenceError<StoreError<R>>;

pub fn persistence_cpu<R>() -> Effect<StoredLookup<i64, i64>, PersistenceCpuError<R>, R>
where
    R: HasKeyValueStore + HasCpuExecutor + Send + Sync + 'static,
{
    let program = (|| -> Result<_, String> {
        let namespace = KeyNamespace::<ProviderDomain>::new("persistence")
            .map_err(|error| format!("persistence namespace: {error:?}"))?;
        let version = RecordVersion::new("provider-v1")
            .map_err(|error| format!("record version: {error:?}"))?;
        let definition = persistable! {
            namespace: namespace,
            request: i64,
            key: |_: &i64| -> &'static str { "entry" },
            success: json_i64(), error: json_i64(),
            version: RecordPolicy::new(
                version,
                rpc_limits(),
            ),
        };
        Ok(effect! {
        bind;
        program: { let __dsl_program: Effect < i64 , PersistenceCpuError < R > , R > = effect_cpu! { once; body: |_| 42, }; __dsl_program },
        body: move |value| {
                       let read_definition = definition.clone();
                       effect! {
        bind;
        program: effect::persistence::write(
                           definition.clone(),
                           || 41_i64,
                           move || StoredOutcome::Success(value),
                           None,
                       ),
        body: move |_| {
                           effect::persistence::read(read_definition.clone(), || 41_i64, UtcMillis::new(0))
                       },
        }
                   },
        })
    })();
    match program {
        Ok(program) => program,
        Err(error) => effect_die! { defect: Defect::new(error), },
    }
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
    let fixture = (|| -> Result<_, String> {
        let namespace = KeyNamespace::<ProviderDomain>::new("queue-workflow")
            .map_err(|error| format!("queue workflow namespace: {error:?}"))?;
        let identity = event_key(&namespace, "run")?;
        let signal = WorkflowSignalId::from_stable(event_key(&namespace, "result")?);
        let definition = WorkflowDefinitionId::new("provider-queue-workflow")
            .map_err(|error| format!("workflow definition: {error:?}"))?;
        let queue = durable_queue! { queue: persisted_queue! { name: PersistedQueueName::new("provider-jobs")
            .map_err(|error| format!("queue name: {error:?}"))?, payload: json_i64(), identity: BinaryIdentity::new("provider-job-payload", NonZeroU64::MIN)
            .map_err(|error| format!("queue format: {error:?}"))?, limits: event_limits(), policy: QueueMessagePolicy {
            maximum_attempts: NonZeroU32::MIN.saturating_add(1),
            acknowledged_retention: Duration::ZERO,
        },  }, };
        let options = QueueClaimOptions::new(
            QueueOwner::new("provider-worker").map_err(|error| format!("worker: {error:?}"))?,
            UtcMillis::new(0),
            UtcMillis::new(1),
            NonZeroUsize::MIN,
        )
        .map_err(|error| format!("claim window: {error:?}"))?;
        let replacement_options = QueueClaimOptions::new(
            QueueOwner::new("replacement-worker")
                .map_err(|error| format!("replacement worker: {error:?}"))?,
            UtcMillis::new(1),
            UtcMillis::new(2),
            NonZeroUsize::MIN,
        )
        .map_err(|error| format!("replacement window: {error:?}"))?;
        Ok((
            identity,
            signal,
            definition,
            queue,
            options,
            replacement_options,
        ))
    })();
    let Ok((identity, signal, definition, queue, options, replacement_options)) = fixture else {
        return false;
    };
    let run = WorkflowRunId::from_stable(identity.clone());
    let version = WorkflowVersion::new(NonZeroU32::MIN);
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
        signal,
        version,
        codec: json_i64(),
        limits: rpc_limits(),
    };
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
    let claimed = queue.claim(options).run(Arc::clone(&environment)).await;
    let effect::Exit::Success(mut claims) = claimed else {
        return false;
    };
    if claims.len() != 1 {
        return false;
    }
    let Some(stale_claim) = claims.pop() else {
        return false;
    };
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
    let Some(replacement_claim) = replacement_claims.pop() else {
        return false;
    };
    let valid_token = replacement_claim.token;
    let executions = Arc::new(AtomicUsize::new(0));
    let handler_executions = Arc::clone(&executions);
    let stale = queue
        .handle_claim(
            stale_claim,
            deferred.clone(),
            move |value| {
                let executions = Arc::clone(&handler_executions);
                {
                    let __dsl_program: Effect<i64, (), R> = effect_cpu! { once; body: move |_| {
                        executions.fetch_add(1, Ordering::AcqRel);
                        value + 1
                    }, };
                    __dsl_program
                }
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
    effect! {
    bind;
    program: { let __dsl_program: Effect < i64 , TransactionError < () > , R > = effect_cpu! { once; body: |_| 42, }; __dsl_program },
    body: move |value| {
           let read = reference.clone();
           let write = reference.clone();
           let transaction: Transaction<i64, ()> = transaction! {
               write.write(Arc::new(value))?;
               let stored = read.read()?;
               *stored
           };
           transaction.atomically(policy)
       },
    }
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
    effect! {
     bind;
     program: effect_map_error! { program: source
            .subscribe(), map: TransactionReactiveError::Subscription, },
     body: move |(initial, subscription)| {
                let source = source.clone();
                effect! {
     bind;
     program: effect_map_error! { program: { let __dsl_transaction = source
                    .set::<()>(Arc::new(0)); transaction! {
    __dsl_transaction.clone()?; Transaction::<(), ()>::fail_with(|| ())?
    } }
                    .atomically(policy)
                    .either(), map: TransactionReactiveError::Transaction, },
     body: move |aborted| {
                        if !matches!(aborted, Err(TransactionError::Domain(()))) {
                            return effect_try_sync! { body: { let __dsl_error = || TransactionReactiveError::AbortInvariant; move || Err(__dsl_error()) }, on_error: ::core::convert::identity, };
                        }
                        let source = source.clone();
                        let subscription = subscription.clone();
                        let initial = Arc::clone(&initial);
                        effect! {
     bind;
     program: effect_map_error! { program: source
                            .read_committed::<R>(), map: TransactionReactiveError::Subscription, },
     body: move |before| {
                                if *before != 41 {
                                    return effect_try_sync! { body: { let __dsl_error = || {
                                        TransactionReactiveError::AbortInvariant
                                    }; move || Err(__dsl_error()) }, on_error: ::core::convert::identity, };
                                }
                                let source = source.clone();
                                let subscription = subscription.clone();
                                let initial = Arc::clone(&initial);
                                effect! {
     bind;
     program: { let __dsl_program: Effect < i64 , TransactionReactiveError , R > = effect_cpu! { once; body: |_| 42, }; __dsl_program },
     body: move |value| {
                                        let subscription = subscription.clone();
                                        let initial = Arc::clone(&initial);
                                        effect! {
     bind;
     program: effect_map_error! { program: source
                                            .set::<()>(Arc::new(value))
                                            .atomically(policy), map: TransactionReactiveError::Transaction, },
     body: move |_| {
                                                let disposal = subscription.clone();
                                                let initial = Arc::clone(&initial);
                                                effect_map! { program: effect_map_error! { program: subscription
                                                    .next(), map: TransactionReactiveError::Subscription, }, map: move |committed| {
                                                        let _ = disposal.dispose();
                                                        *initial == 41 && *committed == 42
                                                    }, }
                                            },
     }
                                    },
     }
                            },
     }
                    },
     }
            },
     }
}

pub fn notification_loss_after_commit<R>(
    source: TxSubscriptionRef<i64>,
) -> Effect<bool, TransactionReactiveError, R>
where
    R: HasTransactionDomain + HasCpuExecutor + Send + Sync + 'static,
{
    let program = (|| -> Result<_, String> {
        let namespace = KeyNamespace::<ProviderDomain>::new("transaction-notification")
            .map_err(|error| format!("notification namespace: {error:?}"))?;
        let key = InvalidationKey::from_stable(event_key(&namespace, "source")?);
        let registry = InvalidationRegistry::new(PubSubOptions {
            capacity: NonZeroUsize::MIN,
            overflow: PubSubOverflow::Backpressure,
            replay: ReplayPolicy::None,
        })
        .map_err(|error| format!("invalidation registry: {error:?}"))?;
        let policy = TransactionPolicy {
            max_conflicts: NonZeroUsize::MIN,
        };
        Ok(effect! {
        bind;
        program: { let __dsl_program: Effect < i64 , TransactionReactiveError , R > = effect_cpu! { once; body: |_| 42, }; __dsl_program },
        body: move |value| {
                       let registry = registry.clone();
                       let source = source.clone();
                       let key = key.clone();
                       effect! {
        bind;
        program: effect_map_error! { program: source
                           .set::<()>(Arc::new(value))
                           .atomically(policy), map: TransactionReactiveError::Transaction, },
        body: move |_| {
                               let registry = registry.clone();
                               let source = source.clone();
                               let key = key.clone();
                               effect! {
        bind;
        program: effect_map_error! { program: registry
                                   .shutdown(PubSubShutdown::Discard), map: TransactionReactiveError::Invalidation, },
        body: move |_| {
                                       let source = source.clone();
                                       effect! {
        bind;
        program: effect_map_error! { program: registry
                                           .invalidate(vec![key.clone()])
                                           .either(), map: TransactionReactiveError::Invalidation, },
        body: move |notification| {
                                               if !matches!(
                                                   notification,
                                                   Err(InvalidationError::PubSub(
                                                       effect::PubSubError::Shutdown
                                                   ))
                                               ) {
                                                   return effect_try_sync! { body: { let __dsl_error = || {
                                                       TransactionReactiveError::NotificationInvariant
                                                   }; move || Err(__dsl_error()) }, on_error: ::core::convert::identity, };
                                               }
                                               effect_map! { program: effect_map_error! { program: source
                                                   .read_committed::<R>(), map: TransactionReactiveError::Subscription, }, map: |committed| *committed == 42, }
                                           },
        }
                                   },
        }
                           },
        }
                   },
        })
    })();
    match program {
        Ok(program) => program,
        Err(error) => effect_die! { defect: Defect::new(error), },
    }
}

pub struct CpuClusterExecutor<R>(PhantomData<fn() -> R>);

impl<R> CpuClusterExecutor<R> {
    #[must_use]
    pub fn new() -> Self {
        Self(PhantomData)
    }
}

impl<R> Default for CpuClusterExecutor<R> {
    fn default() -> Self {
        Self::new()
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
        effect_cpu! { once; body: move |_| SavedReply {
            kind: ReplyKind::Success,
            payload: encoded_request,
        }, }
    }
}

#[must_use]
pub fn cluster_store() -> Arc<MemoryClusterMessageStore> {
    Arc::new(MemoryClusterMessageStore::new(
        MessageStoreCapacity::Bounded(NonZeroUsize::MIN),
    ))
}

/// Dispatch through a typed catalog after acquiring its checked activation owner.
pub async fn entity_macro<R>(environment: Arc<R>) -> bool
where
    R: HasClusterActivationStore + HasWallClock + Send + Sync + 'static,
{
    let fixture = (|| -> Result<_, String> {
        let policy = RpcEnvelopePolicy {
            frame: rpc_frame(),
            json: rpc_limits(),
        };
        let operation_id = OperationId::new("provider-entity-increment")
            .map_err(|error| format!("entity operation: {error:?}"))?;
        let operation = rpc_operation! {
            id: operation_id,
            version: RpcVersion::new(NonZeroU32::MIN),
            request: json_i64(), success: json_i64(), error: json_i64(),
            limits: rpc_limits(),
        };
        let handler = rpc_operation! { operation: operation.clone(), handler: |request| {
            { let __dsl_program: Effect < i64 , i64 , R > = effect_sync! { once; body: move || request + 1, }; __dsl_program }
        }, };
        let entity_type = ClusterEntityType::new("provider-macro-entity")
            .map_err(|error| format!("entity type: {error:?}"))?;
        let protocol = rpc_protocol! {
            policy: policy,
            handlers: [handler.clone()],
        }
        .map_err(|error| format!("protocol registration: {error:?}"))?;
        let definition = entity! {
            entity_type: entity_type.clone(),
            resources: entity_resource! { () },
            handlers: [operation.clone() => |_state: Arc<()>, request: i64| { request + 1 }],
        };
        let catalog: ClusterDefinition<(), R> = cluster! {
            policy: policy, entities: [definition],
        }
        .map_err(|error| format!("entity catalog: {error:?}"))?;
        let namespace = KeyNamespace::<ProviderDomain>::new("macro-entity")
            .map_err(|error| format!("entity namespace: {error:?}"))?;
        let address = ClusterAddress {
            entity_type,
            entity: ClusterEntityId::from_stable(event_key(&namespace, "entity")?),
            shard: ClusterShardId::from_stable(event_key(&namespace, "shard")?),
        };
        let frame = RpcEnvelope {
            operation: operation.spec().id.clone(),
            version: operation.spec().version,
            call: RpcCallId::new("provider-entity-call")
                .map_err(|error| format!("entity call: {error:?}"))?,
            kind: RpcEnvelopeKind::Request,
            sequence: None,
            payload: JsonValue::from(41),
            metadata: Default::default(),
        }
        .encode(policy)
        .map_err(|error| format!("entity frame: {error:?}"))?
        .into_bytes()
        .map_err(|error| format!("entity bytes: {error:?}"))?;
        let activation = ActivationId::from_stable(event_key(&namespace, "activation")?);
        Ok((protocol, catalog, address, frame, activation))
    })();
    let Ok((protocol, catalog, address, frame, activation)) = fixture else {
        return false;
    };
    if !matches!(
        protocol
            .handle(frame.clone())
            .run(Arc::clone(&environment))
            .await,
        effect::Exit::Success(_)
    ) {
        return false;
    }
    let effect::Exit::Success(now) = environment.wall_clock().now().run(Arc::new(())).await else {
        return false;
    };
    let ownership = environment
        .cluster_activation_store()
        .activate(
            address.clone(),
            activation,
            now,
            UtcMillis::new(now.value() + 100),
        )
        .run(Arc::new(()))
        .await;
    let effect::Exit::Success(ActivationOutcome::Activated { lease, .. }) = ownership else {
        return false;
    };
    let catalog = Arc::new(catalog);
    let frame: Arc<[u8]> = frame.into();
    let checked = effect_scope! {
        body |scope: Scope<()>| {
            let catalog = Arc::clone(&catalog); let owner = lease.owner.clone();
            let address = address.clone(); let frame = Arc::clone(&frame);
            let live = effect_map_error! { program: catalog.activate(scope, owner), map: |_| (), }?;
            effect_map_error! { program: live.execute(address, frame), map: |_| (), }?
        },
    };
    matches!(
        checked.run(environment).await,
        effect::Exit::Success(SavedReply {
            kind: ReplyKind::Success,
            ..
        })
    )
}

pub fn cluster_prepare(
    store: Arc<MemoryClusterMessageStore>,
) -> Effect<ClaimOptions, ClusterStoreError> {
    let program = (|| -> Result<_, String> {
        let namespace = KeyNamespace::<ProviderDomain>::new("cluster")
            .map_err(|error| format!("cluster namespace: {error:?}"))?;
        let address = ClusterAddress {
            entity_type: ClusterEntityType::new("provider-entity")
                .map_err(|error| format!("entity type: {error:?}"))?,
            entity: ClusterEntityId::from_stable(event_key(&namespace, "entity")?),
            shard: ClusterShardId::from_stable(event_key(&namespace, "shard")?),
        };
        let request = AcceptedRequest {
            address: address.clone(),
            id: ClusterRequestId::from_stable(event_key(&namespace, "request")?),
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
        let activation = ActivationId::from_stable(event_key(&namespace, "activation")?);
        Ok(effect! {
        bind;
        program: store.accept(request),
        body: move |_| {
                   effect! {
        bind;
        program: store
                       .activate(
                           address.clone(),
                           activation.clone(),
                           UtcMillis::new(1),
                           UtcMillis::new(3),
                       ),
        body: |outcome| {
                           let lease = match outcome {
                               ActivationOutcome::Activated { lease, .. }
                               | ActivationOutcome::AlreadyActive(lease) => lease,
                               ActivationOutcome::Busy(_) => {
                                   return effect_die! { defect: Defect::new("fixture activation was busy"), };
                               }
                           };
                           effect_sync! { once; body: move || ClaimOptions {
                               owner: lease.owner,
                               now: UtcMillis::new(1),
                               lease_until: UtcMillis::new(2),
                               limit: NonZeroUsize::MIN,
                           }, }
                       },
        }
               },
        })
    })();
    match program {
        Ok(program) => program,
        Err(error) => effect_die! { defect: Defect::new(error), },
    }
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

#[must_use]
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
    effect! {
    bind;
    program: Effect::<(), (), R>::sleep(duration),
    body: |_| Effect::<Duration, (), R>::clock_now(),
    }
}

fn replica_permit(batch: &effect::eventlog::ReplicaBatch) -> effect::eventlog::ReplicaImportPermit {
    match effect::eventlog::ReplicaImportPermit::authorize(
        batch.source().clone(),
        batch.stream().clone(),
        |_, _| Ok::<(), std::convert::Infallible>(()),
    ) {
        Ok(permit) => permit,
        Err(infallible) => match infallible {},
    }
}
