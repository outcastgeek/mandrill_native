extern crate effect as fx;
#[path = "../../shared/reference_update.rs"]
mod reference_update;
mod host;
mod host_clock;
mod persistence_provider;
#[path = "../../shared/cluster_workflow_takeover.rs"]
mod cluster_workflow_takeover;
#[path = "../../shared/transactional_collections.rs"]
mod transactional_collections;
#[path = "../../shared/provider_program.rs"]
mod provider_program;

use effect::cluster::{
    AcceptancePolicy, AcceptedRequest, ClusterAddress, ClusterEntityId, ClusterEntityType,
    ClusterMessageStore, ClusterRequestId, ClusterShardId, HasClusterMessageStore,
    MemoryClusterMessageStore,
    MessageAcceptance, MessageStoreCapacity, PayloadConflictPolicy,
};
use effect::encoding::FramePolicy;
use effect::eventlog::{
    AggregateId, AppendDecision, AppendReceipt, EventEnvelope, EventGroupId, EventId,
    EventJournal, EventJournalId, EventTag, EventVersion, HasAtomicProjectionStore,
    HasEventJournal, JournalCursor, JournalReadOutcome, JournalStream, MemoryEventJournal,
    MemoryJournalCapacity, MemoryProjectionStore, Projection, ProjectionAdvance, ProjectionId,
    ProjectionKey, ProjectionVersion,
};
use effect::persistence::{
    HasPersistedQueueStore, KeyValueStore, MemoryPersistedQueueStore,
    QueueStoreCapacity, StoreRead, StoredValue,
};
use effect::reactivity::{AtomId, AtomRef, AtomRegistry};
use effect::rpc::{
    OperationId, RpcCallId, RpcEnvelope, RpcEnvelopeKind, RpcEnvelopePolicy, RpcHandler,
    RpcOperation, RpcUnaryServer, RpcVersion,
};
use effect::transactional::{
    HasTransactionDomain, TransactionDomain, TransactionPolicy, TxSubscriptionRef,
};
use effect::workflow::{
    EncodedWorkflowOutcome, MemoryWorkflowEngine, WorkflowDefinitionId, WorkflowEngine,
    WorkflowHandle, WorkflowOutcomeKind, WorkflowRegistration, WorkflowRunId, WorkflowStatus,
    WorkflowVersion,
};
use effect::{
    Effect, Exit, HasBlockingExecutor, HasCpuExecutor, HasTaskSpawner, JsonValue, KeyDomain,
    KeyNamespace, KeySegment, ParseLimits, PubSubOptions, PubSubOverflow, ReplayPolicy,
    StableKey, UtcMillis, json_i64,
};
use std::future::Future;
use std::collections::BTreeMap;
use std::num::{NonZeroU32, NonZeroUsize};
use std::pin::pin;
use std::sync::Arc;
use std::task::{Context, Poll, Wake, Waker};
use std::thread;
use std::time::Duration;

use host::WorkerPool;
use host_clock::HostClock;
use persistence_provider::{ExternalKeyValueStore, StoreError};

struct Domain;

impl KeyDomain for Domain {
    const TAG: &'static str = "effect-capability-consumer";
}

struct ThreadWake(thread::Thread);

impl Wake for ThreadWake {
    fn wake(self: Arc<Self>) { self.0.unpark(); }
    fn wake_by_ref(self: &Arc<Self>) { self.0.unpark(); }
}

fn drive<F: Future>(future: F) -> F::Output {
    let waker = Waker::from(Arc::new(ThreadWake(thread::current())));
    let mut context = Context::from_waker(&waker);
    let mut future = pin!(future);
    loop {
        match future.as_mut().poll(&mut context) {
            Poll::Ready(value) => return value,
            Poll::Pending => thread::park(),
        }
    }
}

fn success<A, E>(exit: Exit<A, E>) -> A {
    match exit {
        Exit::Success(value) => value,
        _ => panic!("consumer operation failed"),
    }
}

struct HostEnvironment {
    tasks: Arc<WorkerPool>,
    cpu: Arc<WorkerPool>,
    blocking: Arc<WorkerPool>,
    journal: Arc<MemoryEventJournal>,
    engine: Arc<MemoryWorkflowEngine>,
    store: Arc<ExternalKeyValueStore>,
    queue: Arc<MemoryPersistedQueueStore>,
    transaction_domain: Arc<TransactionDomain>,
    cluster_store: Arc<MemoryClusterMessageStore>,
    cluster_executor: Arc<provider_program::CpuClusterExecutor<HostEnvironment>>,
    wall_clock: effect::TestWallClock,
    monotonic_clock: HostClock,
}

impl effect::HasClock for HostEnvironment {
    type Clock = HostClock;
    fn clock(&self) -> &Self::Clock { &self.monotonic_clock }
}

impl HasClusterMessageStore for HostEnvironment {
    type Store = MemoryClusterMessageStore;
    fn cluster_message_store(&self) -> Arc<Self::Store> {
        Arc::clone(&self.cluster_store)
    }
}

impl effect::cluster::HasClusterExecutor for HostEnvironment {
    type Executor = provider_program::CpuClusterExecutor<Self>;
    fn cluster_executor(&self) -> Arc<Self::Executor> {
        Arc::clone(&self.cluster_executor)
    }
}

impl effect::HasWallClock for HostEnvironment {
    type Clock = effect::TestWallClock;
    fn wall_clock(&self) -> &Self::Clock { &self.wall_clock }
}

impl HasTransactionDomain for HostEnvironment {
    fn transaction_domain(&self) -> Arc<TransactionDomain> {
        Arc::clone(&self.transaction_domain)
    }
}

impl effect::persistence::HasKeyValueStore for HostEnvironment {
    type Store = ExternalKeyValueStore;
    fn key_value_store(&self) -> Arc<Self::Store> { Arc::clone(&self.store) }
}

impl HasPersistedQueueStore for HostEnvironment {
    type Store = MemoryPersistedQueueStore;
    fn persisted_queue_store(&self) -> Arc<Self::Store> { Arc::clone(&self.queue) }
}

impl HasEventJournal for HostEnvironment {
    type Journal = MemoryEventJournal;
    fn event_journal(&self) -> Arc<Self::Journal> { Arc::clone(&self.journal) }
}

impl effect::workflow::HasWorkflowEngine for HostEnvironment {
    type Engine = MemoryWorkflowEngine;
    fn workflow_engine(&self) -> Arc<Self::Engine> { Arc::clone(&self.engine) }
}

impl HasTaskSpawner for HostEnvironment {
    type Tasks = WorkerPool;
    fn task_spawner(&self) -> &Self::Tasks { &self.tasks }
}

impl HasCpuExecutor for HostEnvironment {
    type Executor = WorkerPool;
    fn cpu_executor(&self) -> &Self::Executor { &self.cpu }
}

impl HasBlockingExecutor for HostEnvironment {
    type Executor = WorkerPool;
    fn blocking_executor(&self) -> &Self::Executor { &self.blocking }
}

fn supplied_runtime_capabilities() {
    // Two workers witness independent progress; the queue can admit both
    // branches before either worker starts polling them.
    let parallel_workers = NonZeroUsize::new(2).expect("parallel worker count");
    let environment = Arc::new(HostEnvironment {
        tasks: Arc::new(WorkerPool::new(parallel_workers, parallel_workers)
            .expect("task workers")),
        cpu: Arc::new(WorkerPool::new(parallel_workers, parallel_workers)
            .expect("cpu workers")),
        blocking: Arc::new(WorkerPool::new(parallel_workers, parallel_workers)
            .expect("blocking workers")),
        journal: Arc::new(MemoryEventJournal::new(MemoryJournalCapacity {
            streams: NonZeroUsize::new(3).expect("three event fixture streams"),
            entries_per_stream: NonZeroUsize::MIN,
            tombstones_per_stream: NonZeroUsize::MIN,
        })),
        engine: Arc::new(MemoryWorkflowEngine::new()),
        store: Arc::new(ExternalKeyValueStore::new(NonZeroUsize::MIN)),
        queue: Arc::new(MemoryPersistedQueueStore::new(
            QueueStoreCapacity::Bounded(NonZeroUsize::MIN))),
        transaction_domain: Arc::new(TransactionDomain::new(
            NonZeroUsize::new(12).expect("three bridge and nine collection cells"))),
        cluster_store: provider_program::cluster_store(),
        cluster_executor: Arc::new(provider_program::CpuClusterExecutor::new()),
        wall_clock: effect::TestWallClock::new(UtcMillis::new(1)),
        monotonic_clock: HostClock::new(),
    });
    drive(reference_update::exercise(Arc::clone(&environment)));
    let program = provider_program::run::<HostEnvironment>();
    assert_eq!(success(drive(program.run(Arc::clone(&environment)))), 42);
    environment.tasks.wait_idle();
    environment.cpu.wait_idle();
    let claim = success(drive(provider_program::cluster_prepare(
        Arc::clone(&environment.cluster_store)).run(Arc::new(()))));
    let deliveries = success(drive(provider_program::cluster_cpu::<HostEnvironment>(claim)
        .run(Arc::clone(&environment))));
    assert!(provider_program::completed_cluster_run(&deliveries));
    assert!(drive(provider_program::entity_macro(Arc::clone(&environment))));
    assert!(drive(cluster_workflow_takeover::run(Arc::clone(&environment))));
    assert!(matches!(drive(provider_program::monotonic_timer::<HostEnvironment>(
        Duration::from_millis(1)).run(Arc::clone(&environment))), Exit::Success(_)));
    let cancellation = effect::Cancellation::new();
    let timer = effect::Clock::sleep(
        &environment.monotonic_clock,
        Duration::from_secs(1),
        cancellation.clone(),
    );
    let mut timer = pin!(timer);
    let waker = Waker::from(Arc::new(ThreadWake(thread::current())));
    let mut context = Context::from_waker(&waker);
    assert!(timer.as_mut().poll(&mut context).is_pending());
    assert!(cancellation.request());
    assert!(matches!(drive(timer), Err(effect::ClockError::Interrupted)));
    environment.tasks.wait_idle();
    environment.cpu.wait_idle();
    let reference = environment.transaction_domain.reference(41_i64)
        .expect("transaction fixture reference");
    assert_eq!(success(drive(provider_program::transaction_cpu::<HostEnvironment>(reference)
        .run(Arc::clone(&environment)))), 42);
    let source = TxSubscriptionRef::new(&environment.transaction_domain, 41_i64)
        .expect("transactional reactive fixture source");
    assert!(success(drive(provider_program::transactional_reactive::<HostEnvironment>(source)
        .run(Arc::clone(&environment)))));
    let source = TxSubscriptionRef::new(&environment.transaction_domain, 41_i64)
        .expect("notification loss fixture source");
    assert!(success(drive(
        provider_program::notification_loss_after_commit::<HostEnvironment>(source)
            .run(Arc::clone(&environment)))));
    assert!(drive(transactional_collections::run(Arc::clone(&environment))));
    environment.cpu.wait_idle();
    assert!(matches!(success(drive(provider_program::persistence_cpu::<HostEnvironment>()
        .run(Arc::clone(&environment)))),
        effect::persistence::StoredLookup::Found {
            outcome: effect::StoredOutcome::Success(42), ..
        }));
    environment.cpu.wait_idle();
    assert_eq!(success(drive(provider_program::schema_cpu::<HostEnvironment>()
        .run(Arc::clone(&environment)))), 42);
    assert!(success(drive(provider_program::reactivity_cpu::<HostEnvironment>()
        .run(Arc::clone(&environment)))));
    environment.cpu.wait_idle();
    assert!(matches!(success(drive(provider_program::workflow_cpu::<HostEnvironment>()
        .run(Arc::clone(&environment)))),
        effect::workflow::WorkflowRunResult::Completed(Ok(42))));
    assert!(drive(provider_program::queue_workflow_race(
        Arc::clone(&environment))));
    environment.cpu.wait_idle();
    environment.blocking.wait_idle();
    let cancelled = drive(provider_program::cancel_and_join::<HostEnvironment>()
        .run(Arc::clone(&environment)));
    assert!(matches!(cancelled, Exit::Success(true)), "{cancelled:?}");
    environment.tasks.wait_idle();
    let limits = provider_program::rpc_limits();
    let frame = provider_program::rpc_frame();
    let response = success(drive(provider_program::rpc_cpu::<HostEnvironment>(limits, frame)
        .run(Arc::clone(&environment))));
    provider_program::assert_rpc_response(response, limits, frame);
    environment.cpu.wait_idle();
    assert!(matches!(success(drive(provider_program::event_cpu::<HostEnvironment>(
        provider_program::event_limits()).run(Arc::clone(&environment)))),
        effect::eventlog::EventRunOutcome::Committed { result: Ok(42), .. }));
    assert!(drive(provider_program::rpc_event_notification_loss(
        Arc::clone(&environment))));
    assert!(drive(provider_program::replica_observer_restart(
        Arc::clone(&environment))));
    environment.cpu.wait_idle();
    environment.tasks.assert_healthy();
    environment.cpu.assert_healthy();
    environment.blocking.assert_healthy();
}

fn key(namespace: &KeyNamespace<Domain>, value: &'static str) -> StableKey<Domain> {
    StableKey::new(namespace, [KeySegment::Text(Arc::from(value))])
        .expect("valid consumer key")
}

fn schema_and_persistence() {
    let codec = json_i64();
    let encoded = codec.encode_ref(&42).expect("encode value");
    assert_eq!(codec.decode_ref(&encoded).expect("decode value"), 42);

    let store = ExternalKeyValueStore::new(NonZeroUsize::MIN);
    let value = StoredValue {
        bytes: Arc::from(&b"encoded-value"[..]),
        expires_at: None,
    };
    let identity: Arc<[u8]> = Arc::from(&b"record"[..]);
    let revision = success(drive(store.put(Arc::clone(&identity), value).run(Arc::new(()))));
    assert_eq!(revision.value(), 1);
    assert!(matches!(
        success(drive(store.get(Arc::clone(&identity), UtcMillis::new(0))
            .run(Arc::new(())))),
        StoreRead::Found(record) if record.revision == revision
    ));
    let other: Arc<[u8]> = Arc::from(&b"other"[..]);
    let batch = success(drive(store.get_many(
        vec![Arc::clone(&identity), Arc::clone(&other)], UtcMillis::new(0),
    ).run(Arc::new(()))));
    assert!(matches!(batch[0].result, Ok(StoreRead::Found(_))));
    assert!(matches!(batch[1].result, Ok(StoreRead::Missing)));
    assert!(matches!(
        drive(store.put(Arc::clone(&other), StoredValue {
            bytes: Arc::from(&b"full"[..]), expires_at: None,
        }).run(Arc::new(()))),
        Exit::Fail(StoreError::Capacity)
    ));
    assert!(success(drive(store.remove(identity).run(Arc::new(())))).is_some());
    success(drive(store.put(other, StoredValue {
        bytes: Arc::from(&b"expiring"[..]),
        expires_at: Some(UtcMillis::new(1)),
    }).run(Arc::new(()))));
    assert_eq!(success(drive(store.cleanup_expired(UtcMillis::new(1))
        .run(Arc::new(())))), 1);
}

fn rpc() {
    let operation = RpcOperation::unary(
        OperationId::new("increment").expect("operation id"),
        RpcVersion::new(NonZeroU32::MIN),
        json_i64(), json_i64(), json_i64(),
        ParseLimits {
            bytes: NonZeroUsize::MAX,
            depth: NonZeroUsize::MAX,
            nodes: NonZeroUsize::MAX,
            collection: NonZeroUsize::MAX,
            issues: NonZeroUsize::MAX,
        },
    );
    let policy = RpcEnvelopePolicy {
        frame: FramePolicy { maximum_payload: NonZeroUsize::MAX },
        json: operation.limits(),
    };
    let handler = RpcHandler::new(operation.clone(), |request| {
        Effect::<i64, i64, ()>::sync_once(move || request + 1)
    });
    let server = RpcUnaryServer::new(handler, policy);
    let envelope = RpcEnvelope {
        operation: operation.spec().id.clone(),
        version: operation.spec().version,
        call: RpcCallId::new("call").expect("call id"),
        kind: RpcEnvelopeKind::Request,
        sequence: None,
        payload: JsonValue::from(41),
        metadata: Default::default(),
    };
    let request = envelope.encode(policy).expect("frame request")
        .into_bytes().expect("request bytes");
    let response = success(drive(server.handle(request).run(Arc::new(()))));
    let mut decoder = effect::encoding::FrameDecoder::new(policy.frame);
    let frames = decoder.push(&response.into_bytes().expect("response bytes"))
        .expect("decode response frame");
    decoder.finish().expect("complete response frame");
    let response = RpcEnvelope::decode(&frames[0], policy).expect("response envelope");
    assert_eq!(response.kind, RpcEnvelopeKind::Success);
    assert_eq!(response.payload, JsonValue::from(42));
}

fn reactivity(namespace: &KeyNamespace<Domain>) {
    let registry = AtomRegistry::new(PubSubOptions {
        capacity: NonZeroUsize::MIN,
        overflow: PubSubOverflow::Backpressure,
        replay: ReplayPolicy::None,
    }).expect("registry");
    let atom = AtomRef::new(
        registry, AtomId::from_stable(key(namespace, "atom")),
        41_i64, NonZeroUsize::MIN, PubSubOverflow::Backpressure,
    ).expect("atom");
    success(drive(atom.set::<()>(42).run(Arc::new(()))));
    assert_eq!(*success(drive(atom.read::<()>().run(Arc::new(())))), 42);
}

fn cluster(namespace: &KeyNamespace<Domain>) {
    let store = MemoryClusterMessageStore::new(
        MessageStoreCapacity::Bounded(NonZeroUsize::MIN),
    );
    let address = ClusterAddress {
        entity_type: ClusterEntityType::new("counter").expect("entity type"),
        entity: ClusterEntityId::from_stable(key(namespace, "entity")),
        shard: ClusterShardId::from_stable(key(namespace, "shard")),
    };
    let request = AcceptedRequest {
        address: address.clone(),
        id: ClusterRequestId::from_stable(key(namespace, "request")),
        payload: Arc::from(&b"increment"[..]),
        idempotency_key: None,
        accepted_at: UtcMillis::new(0),
        policy: AcceptancePolicy {
            dedup_retention: Duration::ZERO,
            reply_retention: Duration::ZERO,
            payload_conflict: PayloadConflictPolicy::Reject,
            maximum_attempts: NonZeroU32::MIN,
        },
    };
    assert!(matches!(
        success(drive(store.accept(request.clone()).run(Arc::new(())))),
        MessageAcceptance::Accepted { .. }
    ));
    assert!(matches!(
        success(drive(store.accept(request).run(Arc::new(())))),
        MessageAcceptance::Duplicate { .. }
    ));
}

fn workflow(namespace: &KeyNamespace<Domain>) {
    let engine = MemoryWorkflowEngine::new();
    let run = WorkflowRunId::from_stable(key(namespace, "workflow-run"));
    let definition = WorkflowDefinitionId::new("workflow").expect("definition");
    let version = WorkflowVersion::new(NonZeroU32::MIN);
    success(drive(engine.register(WorkflowRegistration {
        id: definition.clone(), version,
        compatibility: Arc::from(&b"v1"[..]),
    }).run(Arc::new(()))));
    let handle = WorkflowHandle { run: run.clone(), definition, version };
    let started = success(drive(engine.start(
        handle, Arc::from(&b"input"[..]),
    ).run(Arc::new(()))));
    assert_eq!(started.status, WorkflowStatus::Ready);
    success(drive(engine.transition(
        run.clone(), success(drive(engine.inspect(run.clone()).run(Arc::new(())))).expect("run").checkpoint(), WorkflowStatus::Running,
    ).run(Arc::new(()))));
    let outcome = EncodedWorkflowOutcome {
        kind: WorkflowOutcomeKind::Succeeded,
        payload: Arc::from(&b"result"[..]),
    };
    let completed = success(drive(engine.complete(run, outcome.clone()).run(Arc::new(()))));
    assert_eq!(completed.status, WorkflowStatus::Completed(outcome));
}

struct EventEnvironment {
    journal: Arc<MemoryEventJournal>,
    projections: Arc<MemoryProjectionStore<usize>>,
}

impl HasEventJournal for EventEnvironment {
    type Journal = MemoryEventJournal;
    fn event_journal(&self) -> Arc<Self::Journal> { Arc::clone(&self.journal) }
}

impl HasAtomicProjectionStore<usize> for EventEnvironment {
    type Store = MemoryProjectionStore<usize>;
    fn atomic_projection_store(&self) -> Arc<Self::Store> {
        Arc::clone(&self.projections)
    }
}

fn eventlog(namespace: &KeyNamespace<Domain>) {
    let journal = Arc::new(MemoryEventJournal::new(MemoryJournalCapacity {
        streams: NonZeroUsize::MIN,
        entries_per_stream: NonZeroUsize::MIN,
        tombstones_per_stream: NonZeroUsize::MIN,
    }));
    let environment = Arc::new(EventEnvironment {
        journal: Arc::clone(&journal),
        projections: Arc::new(MemoryProjectionStore::new(NonZeroUsize::MIN)),
    });
    let stream = JournalStream {
        journal: EventJournalId::from_stable(key(namespace, "journal")),
        aggregate: AggregateId::from_stable(key(namespace, "aggregate")),
    };
    let group = EventGroupId::new("orders").expect("event group");
    let tag = EventTag::new("placed").expect("event tag");
    let version = EventVersion::new(NonZeroU32::MIN);
    let event = EventEnvelope {
        journal: stream.journal.clone(),
        aggregate: stream.aggregate.clone(),
        id: EventId::from_stable(key(namespace, "event")),
        group: group.clone(),
        tag: tag.clone(),
        version,
        payload: Arc::from(&b"placed"[..]),
        outcome: None,
        occurred_at: UtcMillis::new(0),
    };
    assert!(matches!(
        success(drive(journal.append(event, JournalCursor::new(0)).run(Arc::new(())))),
        AppendReceipt::Decided(AppendDecision::Committed(_))
    ));
    let read = success(drive(journal.read(
        stream.clone(), JournalCursor::new(0), NonZeroUsize::MIN,
    ).run(Arc::new(()))));
    assert!(matches!(read, JournalReadOutcome::Complete(page) if page.entries.len() == 1));
    let projection = Projection::new(
        ProjectionKey {
            id: ProjectionId::from_stable(key(namespace, "projection")),
            stream,
        },
        ProjectionVersion::new(NonZeroU32::MIN),
        BTreeMap::from([((group, tag), version)]),
        || 0_usize,
        |count, _| Ok::<usize, ()>(count + 1),
    );
    assert!(matches!(
        success(drive(projection.advance(NonZeroUsize::MIN).run(environment))),
        ProjectionAdvance::Advanced { snapshot, .. } if *snapshot.state == 1
    ));
}

struct TransactionEnvironment(Arc<TransactionDomain>);

impl HasTransactionDomain for TransactionEnvironment {
    fn transaction_domain(&self) -> Arc<TransactionDomain> { Arc::clone(&self.0) }
}

fn transactional() {
    let domain = Arc::new(TransactionDomain::new(NonZeroUsize::MIN));
    let reference = domain.reference(41_i64).expect("transaction reference");
    let environment = Arc::new(TransactionEnvironment(domain));
    let policy = TransactionPolicy { max_conflicts: NonZeroUsize::MIN };
    success(drive(reference.update::<(), &'static str>(|value| {
        Ok((value + 1, ()))
    }).atomically(policy).run(Arc::clone(&environment))));
    let value = success(drive(reference.read::<&'static str>()
        .atomically(policy).run(environment)));
    assert_eq!(*value, 42);
}

fn transaction_reactivity_bridge() {
    let domain = Arc::new(TransactionDomain::new(NonZeroUsize::MIN));
    let source = TxSubscriptionRef::new(&domain, 41_i64).expect("transactional source");
    let environment = Arc::new(TransactionEnvironment(domain));
    let (initial, subscription) = success(drive(
        source.subscribe().run(Arc::clone(&environment)),
    ));
    assert_eq!(*initial, 41);
    let policy = TransactionPolicy { max_conflicts: NonZeroUsize::MIN };
    success(drive(source.set::<&'static str>(Arc::new(42))
        .atomically(policy).run(Arc::clone(&environment))));
    let change = success(drive(subscription.next().run(environment)));
    assert_eq!(*change, 42);
    assert!(subscription.dispose());
}

fn main() {
    let mut arguments = std::env::args_os().skip(1);
    if let Some(flag) = arguments.next() {
        assert_eq!(flag, "--campaign-config", "unknown consumer argument");
        let path = arguments.next().expect("campaign configuration path");
        assert!(arguments.next().is_none(), "unexpected consumer argument");
        std::fs::read(path).expect("campaign configuration is readable");
    }
    let namespace = KeyNamespace::<Domain>::new("campaign").expect("namespace");
    schema_and_persistence();
    rpc();
    reactivity(&namespace);
    cluster(&namespace);
    workflow(&namespace);
    eventlog(&namespace);
    transactional();
    transaction_reactivity_bridge();
    supplied_runtime_capabilities();
    if let Some(path) = std::env::var_os("EFFECT_CAMPAIGN_REPORT") {
        std::fs::write(
            path,
            r#"{"campaign":"consumer_independent","families":{"schema":1,"persistence":1,"rpc":1,"reactivity":1,"cluster":1,"workflow":1,"eventlog":1,"transaction":1},"bridges":{"INT01":1,"INT02":1,"INT03":1,"INT04":1,"INT05":1}}"#,
        )
        .expect("consumer coverage report is writable");
    }
}
