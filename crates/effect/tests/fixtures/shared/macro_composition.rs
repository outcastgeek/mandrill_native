//! One representative composable-macro scenario shared by independent and mixed providers.

use fx::cluster::*;
use fx::encoding::{BinaryIdentity, BinaryLimits, FrameParts, FramePolicy};
use fx::eventlog::*;
use fx::persistence::*;
use fx::reactivity::*;
use fx::rpc::*;
use fx::transactional::*;
use fx::workflow::*;
use fx::*;
use std::cell::Cell;
use std::collections::{BTreeMap, VecDeque};
use std::num::{NonZeroU32, NonZeroU64, NonZeroUsize};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};
use std::time::Duration;

/// Fixture bounds are explicit inputs, independent of production defaults.
fn nz(value: usize) -> NonZeroUsize {
    NonZeroUsize::new(value).expect("positive composition fixture bound")
}
/// Retain explicit parsing bounds, including enough text width for the fixture stable identities.
fn limits() -> ParseLimits {
    ParseLimits {
        bytes: nz(4096),
        depth: nz(16),
        nodes: nz(128),
        collection: nz(128),
        issues: nz(8),
    }
}
/// Binary bounds match the fixture's small typed event and queue records.
fn binary_limits() -> BinaryLimits {
    BinaryLimits {
        bytes: nz(4096),
        depth: nz(16),
        nodes: nz(128),
        collection: nz(16),
        steps: nz(512),
    }
}
/// Explicit RPC framing policy used by unary, streaming and entity registration.
fn rpc_policy() -> RpcEnvelopePolicy {
    RpcEnvelopePolicy {
        frame: FramePolicy {
            maximum_payload: nz(4096),
        },
        json: limits(),
    }
}
/// Pure fixture codec policy retains rejection/first-error semantics.
fn parse_policy() -> ParsePolicy {
    ParsePolicy {
        limits: limits(),
        errors: SchemaErrorMode::First,
        unknown_fields: UnknownFields::Reject,
        duplicate_fields: DuplicateFields::Reject,
    }
}
/// Only successful primary outcomes satisfy these representative execution assertions.
#[track_caller]
fn success<A, E>(exit: Exit<A, E>) -> A {
    match exit {
        Exit::Success(value) => value,
        _ => panic!("composition fixture expected success"),
    }
}
/// Stable fixture key domain; keys never derive from Rust source positions.
struct Domain;
impl KeyDomain for Domain {
    /// Explicit stable namespace tag, shared by the fixture encoded identities.
    const TAG: &'static str = "macro-composition";
}
/// Names and segments are explicit application vocabulary for the fixture's stored identities.
fn key(name: &str) -> StableKey<Domain> {
    StableKey::new(
        &KeyNamespace::<Domain>::new("macro-composition").expect("fixture namespace"),
        [KeySegment::Text(name.into())],
    )
    .expect("fixture stable key")
}
/// Binary wire identities remain explicit and independent of Rust type names.
fn binary(name: &str) -> BinaryIdentity {
    BinaryIdentity::new(name, NonZeroU64::MIN).expect("fixture binary identity")
}
/// Metadata retained from each span creation for parentage/annotation assertions.
#[derive(Clone)]
struct Start {
    /// Name chosen in the traced domain expression.
    name: Arc<str>,
    /// Actual parent identity captured by the existing tracer boundary.
    parent: Option<SpanId>,
    /// Inherited annotations visible before the span starts.
    annotations: Annotations,
}
/// Domain capabilities coexist with whichever async/CPU/blocking providers the consumer supplies.
struct Environment<R> {
    /// Existing consumer providers, borrowed through capability delegation.
    base: Arc<R>,
    /// Fixture tracer with observable starts/outcomes.
    tracer: Tracer,
    /// Explicit wall-clock authority for activation and stored occurrence times.
    wall: TestWallClock,
    /// Explicit interruption capability used by the transactional loop fixture.
    cancellation: Cancellation,
    /// One transaction journal domain for all attempt reconstruction.
    transactions: Arc<TransactionDomain>,
    /// Existing in-memory activation authority.
    activation: Arc<MemoryClusterMessageStore>,
    /// Existing journal used by handled events and projection decoding.
    journal: Arc<MemoryEventJournal>,
    /// Atomic projection state/checkpoint store.
    projection: Arc<MemoryProjectionStore<i64>>,
    /// Existing queue store retaining worker fences and acknowledgement state.
    queues: Arc<MemoryPersistedQueueStore>,
    /// Existing workflow engine retaining durable deferred results.
    workflow: Arc<MemoryWorkflowEngine>,
}
impl<R: HasTaskSpawner> HasTaskSpawner for Environment<R> {
    /// Reuse the consumer's independent task provider.
    type Tasks = R::Tasks;
    /// Delegate without installing another runtime.
    fn task_spawner(&self) -> &Self::Tasks {
        self.base.task_spawner()
    }
}
impl<R: HasCpuExecutor> HasCpuExecutor for Environment<R> {
    /// Reuse the consumer's CPU provider in the same environment.
    type Executor = R::Executor;
    /// Delegate the existing selected executor.
    fn cpu_executor(&self) -> &Self::Executor {
        self.base.cpu_executor()
    }
}
impl<R: HasBlockingExecutor> HasBlockingExecutor for Environment<R> {
    /// Reuse the consumer's blocking provider.
    type Executor = R::Executor;
    /// Delegate to the selected blocking capability.
    fn blocking_executor(&self) -> &Self::Executor {
        self.base.blocking_executor()
    }
}
impl<R: HasClock> HasClock for Environment<R> {
    /// Monotonic timing stays with the consumer's clock provider.
    type Clock = R::Clock;
    /// Delegate the existing timing capability.
    fn clock(&self) -> &Self::Clock {
        self.base.clock()
    }
}
impl<R> HasWallClock for Environment<R> {
    /// Explicit test wall clock supports reproducible lease checks.
    type Clock = TestWallClock;
    /// Borrow the same authority for all activation operations.
    fn wall_clock(&self) -> &Self::Clock {
        &self.wall
    }
}
impl<R> HasTracer for Environment<R> {
    /// Return the fixture tracer used by every nested domain body.
    fn tracer(&self) -> &Tracer {
        &self.tracer
    }
}
impl<R> HasCancellation for Environment<R> {
    /// Borrow the explicit fixture cancellation authority.
    fn cancellation(&self) -> &Cancellation {
        &self.cancellation
    }
}
impl<R> HasTransactionDomain for Environment<R> {
    /// Share the same journal domain across macro-created transaction attempts.
    fn transaction_domain(&self) -> Arc<TransactionDomain> {
        Arc::clone(&self.transactions)
    }
}
impl<R> HasClusterActivationStore for Environment<R> {
    /// Keep provider failures in the existing concrete activation error channel.
    type Store = MemoryClusterMessageStore;
    /// Share only the existing store handle.
    fn cluster_activation_store(&self) -> Arc<Self::Store> {
        Arc::clone(&self.activation)
    }
}
impl<R> HasEventJournal for Environment<R> {
    /// Use the existing bounded in-memory event journal.
    type Journal = MemoryEventJournal;
    /// Retain one authoritative journal instance.
    fn event_journal(&self) -> Arc<Self::Journal> {
        Arc::clone(&self.journal)
    }
}
impl<R> HasAtomicProjectionStore<i64> for Environment<R> {
    /// Keep state and cursor atomic under the existing store contract.
    type Store = MemoryProjectionStore<i64>;
    /// Retain the shared projection authority.
    fn atomic_projection_store(&self) -> Arc<Self::Store> {
        Arc::clone(&self.projection)
    }
}
impl<R> HasPersistedQueueStore for Environment<R> {
    /// Existing queue provider keeps its original typed settlement errors.
    type Store = MemoryPersistedQueueStore;
    /// Return the configured bounded queue store.
    fn persisted_queue_store(&self) -> Arc<Self::Store> {
        Arc::clone(&self.queues)
    }
}
impl<R> HasWorkflowEngine for Environment<R> {
    /// The workflow engine remains the existing logical recording/replay authority.
    type Engine = MemoryWorkflowEngine;
    /// Return the shared engine selected through this environment.
    fn workflow_engine(&self) -> Arc<Self::Engine> {
        Arc::clone(&self.workflow)
    }
}
/// A value that is neither Clone nor Sync verifies moves through async/CPU/blocking binds.
struct Owned(
    /// Owned interior-mutable payload verifies sendable movement without sharing or cloning.
    Cell<i64>,
);
/// A mapped error payload moves once through primary and cleanup channels.
struct Failure {
    /// Original provider code retained by explicit mapping.
    code: u8,
}
/// Extracted ordinary Effect helper participates in nested domain bodies under the same R.
fn owned<R: Send + Sync + 'static>() -> Effect<Owned, (), R> {
    Effect::from_future_once(|_| async { Owned(Cell::new(4)) })
}
/// Extracted tracked helper retains the caller's evaluation context instead of reading directly.
fn tracked<R: Send + Sync + 'static>(source: AtomRef<i64>) -> AtomProgram<i64, AtomRefError, R> {
    atom! {let source=source.clone();let value=AtomProgram::read(source)?;*value}
}
/// Existing stream-session fixture shape, with no runtime or transport engine added.
struct Session {
    /// Preencoded request frames consumed in protocol order.
    input: Arc<Mutex<VecDeque<Vec<u8>>>>,
    /// Responses retained for the existing server's framing assertions.
    output: Arc<Mutex<Vec<FrameParts>>>,
    /// Session closes must precede middleware resource release.
    closed: Arc<AtomicUsize>,
}
impl RpcStreamSession for Session {
    /// Concrete fixture backend error; no application error erasure.
    type Error = ();
    /// Consume one queued frame through the existing Effect transport contract.
    fn receive_frame(&self) -> Effect<Option<Vec<u8>>, Self::Error> {
        {
            let input = Arc::clone(&self.input);
            Effect::sync_once(move || input.lock().expect("session input").pop_front())
        }
    }
    /// Retain one transferred response frame without cloning its payload.
    fn send(&self, frame: FrameParts) -> Effect<(), Self::Error> {
        let output = Arc::clone(&self.output);
        Effect::sync_once(move || {
            output.lock().expect("session output").push(frame);
        })
    }
    /// Record close during the existing protected server cleanup.
    fn close(&self) -> Effect<(), Self::Error> {
        let closed = Arc::clone(&self.closed);
        Effect::sync_once(move || {
            closed.fetch_add(1, Ordering::SeqCst);
        })
    }
}
/// Record used by the schema macro to construct and decode an existing native Rust type.
struct FixtureRecord {
    /// Typed decoded fixture value.
    value: i64,
}
/// Typed reusable unary callback alternative to inline lowering.
fn unary<R: Send + Sync + 'static>(request: i64) -> Effect<i64, i64, R> {
    Effect::sync_once(move || request + 1)
}

/// Exercise the same composition and lifecycle contracts under either consumer's providers.
/// Bounds are supplied exclusively by R; no scenario code creates a task/CPU runtime.
pub async fn exercise<R>(base: Arc<R>)
where
    R: HasTaskSpawner + HasCpuExecutor + HasBlockingExecutor + HasClock + Send + Sync + 'static,
{
    let starts = Arc::new(Mutex::new(Vec::<Start>::new()));
    let finishes = Arc::new(Mutex::new(Vec::<SpanOutcome>::new()));
    let start_log = Arc::clone(&starts);
    let finish_log = Arc::clone(&finishes);
    let tracer = Tracer::new(
        Diagnostics::new(
            DiagnosticCapacity::Bounded(nz(16)),
            DiagnosticOverflow::DiscardNewest,
        ),
        |annotations| Ok(annotations.clone()),
        move |start| {
            let mut log = start_log.lock().expect("span starts");
            let id = SpanId::new(format!("fixture-span-{}", log.len()));
            log.push(Start {
                name: Arc::clone(&start.name),
                parent: start.parent.clone(),
                annotations: start.annotations.as_ref().clone(),
            });
            Ok(Some(id))
        },
        move |finish| {
            finish_log
                .lock()
                .expect("span finishes")
                .push(finish.outcome);
            Ok(())
        },
    );
    let environment = Arc::new(Environment {
        base,
        tracer,
        cancellation: Cancellation::new(),
        wall: TestWallClock::new(UtcMillis::new(0)),
        transactions: Arc::new(TransactionDomain::new(nz(16))),
        activation: Arc::new(MemoryClusterMessageStore::new(
            MessageStoreCapacity::Bounded(nz(16)),
        )),
        journal: Arc::new(MemoryEventJournal::new(MemoryJournalCapacity {
            streams: nz(8),
            entries_per_stream: nz(8),
            tombstones_per_stream: nz(8),
        })),
        projection: Arc::new(MemoryProjectionStore::new(nz(8))),
        queues: Arc::new(MemoryPersistedQueueStore::new(QueueStoreCapacity::Bounded(
            nz(8),
        ))),
        workflow: Arc::new(MemoryWorkflowEngine::new()),
    });

    let program: Effect<i64, (), Environment<R>> = span! {
        name:"outer",annotations:BTreeMap::from([(Arc::from("component"),Arc::from("composition"))]),
        body {
            service! {select:|_:&Environment<R>|3i64,run |selected| {
                let offset=*selected;
                let value=owned()?;
                let value=Effect::cpu_once(move|_|value)?;
                let value=Effect::blocking_once(move|_|value)?;
                span! {name:"inner",effect:Effect::sync_once(move||value.0.get()+offset)}?
            }}?
        },
    };
    assert!(starts.lock().expect("starts").is_empty());
    assert_eq!(success(program.run(Arc::clone(&environment)).await), 7);
    let log = starts.lock().expect("starts");
    assert_eq!(log.len(), 2);
    assert_eq!(&*log[0].name, "outer");
    assert!(log[0].parent.is_none());
    assert_eq!(
        log[1].parent.as_ref().map(SpanId::as_str),
        Some("fixture-span-0")
    );
    assert_eq!(
        log[1].annotations.get("component").map(AsRef::as_ref),
        Some("composition")
    );
    drop(log);
    assert!(
        finishes
            .lock()
            .expect("finishes")
            .iter()
            .all(|outcome| *outcome == SpanOutcome::Succeeded)
    );
    let restored: Effect<TraceContext, (), Environment<R>> = Effect::trace_context();
    assert!(
        success(restored.run(Arc::clone(&environment)).await)
            .span
            .is_none()
    );
    let mapped: Effect<i64, i64, Environment<R>> = service! {select:|_:&Environment<R>|5i64,on_error:|error:u8|i64::from(error),run |selected| {
        let ordinary=Effect::sync(||2i64)?;
        let provider=ServiceOperation::new(|receiver:&i64|Effect::<i64,u8,Environment<R>>::sync_once({let value=*receiver;move||value}))?;
        assert_eq!(*selected,5);ordinary+provider
    }};
    assert_eq!(success(mapped.run(Arc::clone(&environment)).await), 7);
    let provider_maps = Arc::new(AtomicUsize::new(0));
    let mappings = Arc::clone(&provider_maps);
    let mapped_failure: Effect<(), i64, Environment<R>> = service! {select:|_:&Environment<R>|5i64,on_error:move|error:u8|{mappings.fetch_add(1,Ordering::SeqCst);i64::from(error)},run |_selected| {
        ServiceOperation::new(|_:&i64|Effect::<(),u8,Environment<R>>::fail(7))?;
    }};
    assert!(matches!(
        mapped_failure.run(Arc::clone(&environment)).await,
        Exit::Fail(7)
    ));
    assert_eq!(provider_maps.load(Ordering::SeqCst), 1);
    let mappings = Arc::clone(&provider_maps);
    let disposals = Arc::new(AtomicUsize::new(0));
    let released = Arc::clone(&disposals);
    let selected_resource: Effect<(), i64, Environment<R>> = service! {handle:5i64,on_error:move|error:u8|{mappings.fetch_add(1,Ordering::SeqCst);i64::from(error)},run |_selected| {
        let disposals=Arc::clone(&released);
        let _resource=ServiceResource::new(|value:&i64|{let value=*value;Effect::<i64,u8,Environment<R>>::sync_once(move||value)},move|_:i64|{let disposals=Arc::clone(&disposals);Effect::sync_once(move||{disposals.fetch_add(1,Ordering::SeqCst);}).flat_map(|()|Effect::fail(9u8))})?;
        Effect::<(),i64,Environment<R>>::fail(11)?;
    }};
    assert!(matches!(
        selected_resource.run(Arc::clone(&environment)).await,
        Exit::FinalizationFailed(Finalization {
            primary: PrimaryOutcome::Failed(11),
            first: CleanupFailure::ReleaseFailure(9),
            ..
        })
    ));
    assert_eq!(provider_maps.load(Ordering::SeqCst), 2);
    assert_eq!(disposals.load(Ordering::SeqCst), 1);
    let failed_span: Effect<(), (), Environment<R>> =
        span! {name:"failed-span",effect:Effect::fail(())};
    assert!(matches!(
        failed_span.run(Arc::clone(&environment)).await,
        Exit::Fail(())
    ));
    let defect_span: Effect<(), (), Environment<R>> =
        span! {name:"defect-span",effect:Effect::die(Defect::new("fixture defect"))};
    assert!(matches!(
        defect_span.run(Arc::clone(&environment)).await,
        Exit::Die(_)
    ));
    assert!(
        finishes
            .lock()
            .expect("span outcomes")
            .contains(&SpanOutcome::Failed)
    );
    assert!(
        finishes
            .lock()
            .expect("span outcomes")
            .contains(&SpanOutcome::Defected)
    );
    let projected: Effect<i64, (), Environment<R>> = service! {project:|_:&Environment<R>|9i64,effect:Effect::<i64,(),i64>::service(|value|*value)};
    assert_eq!(success(projected.run(Arc::clone(&environment)).await), 9);

    let released = Arc::new(AtomicUsize::new(0));
    let release_log = Arc::clone(&released);
    let records: Stream<i64, (), Environment<R>> = stream! {
        let released=Arc::clone(&release_log);
        let resource=ResourceBinding::new(Effect::sync(||String::from("owned")),move|_,_|{let released=Arc::clone(&released);Release::sync(move||{released.fetch_add(1,Ordering::SeqCst);})})?;
        yield resource.access(|value|value.len() as i64)?;
        for value in 0..2 {match value {value if value==0=>{yield Effect::sync_once(move||value)?;},_=>{yield value;}}}
        yield_from!(Stream::from_iterable(||[3i64].into_iter()).flat_map(|value|stream!{yield value+1;}));
    };
    assert_eq!(released.load(Ordering::SeqCst), 0);
    assert_eq!(
        success(
            records
                .run_collect(Some(4))
                .run(Arc::clone(&environment))
                .await
        ),
        vec![5, 0, 1, 4]
    );
    assert_eq!(released.load(Ordering::SeqCst), 1);
    assert!(matches!(
        records
            .run_for_each(|_| Effect::fail(()))
            .run(Arc::clone(&environment))
            .await,
        Exit::Fail(())
    ));
    assert_eq!(released.load(Ordering::SeqCst), 2);

    let registry = AtomRegistry::new(PubSubOptions {
        capacity: nz(8),
        overflow: PubSubOverflow::Backpressure,
        replay: ReplayPolicy::None,
    })
    .expect("registry");
    let source=reactive_source! {registry:registry.clone(),id:AtomId::from_stable(key("source")),initial:4i64,capacity:nz(8),overflow:PubSubOverflow::Backpressure}.expect("source");
    let captured = source.clone();
    let derived=reactive_derived! {registry:registry.clone(),id:AtomId::from_stable(key("view")),capacity:nz(8),overflow:PubSubOverflow::Backpressure,program:atom! {
        let captured=captured.clone();
        let value=if true {tracked::<Environment<R>>(captured)?}else{0};
        Effect::<i64,AtomRefError,Environment<R>>::sync_once(move||value+1)?
    }}.expect("derived");
    success(derived.refresh().run(Arc::clone(&environment)).await);
    assert!(
        registry
            .dependencies_of(derived.id())
            .expect("tracked dependencies")
            .contains(source.id())
    );
    let hydration_schema = HydrationSchema::new(
        source.id().clone(),
        RecordVersion::new("composition-v1").expect("hydration version"),
        json_i64(),
        limits(),
    );
    let order = Arc::new(Mutex::new(Vec::new()));
    let set:HydrationSet<Environment<R>>=hydration! {
        bindings:[{order.lock().expect("order").push("source");source.clone()}=>hydration_schema.clone()],
        registry:{order.lock().expect("order").push("registry");registry.clone()},limits:limits(),
    }.expect("hydration");
    assert_eq!(*order.lock().expect("order"), vec!["source", "registry"]);
    let duplicate: Result<HydrationSet<Environment<R>>, HydrationError> = hydration! {registry:registry.clone(),limits:limits(),bindings:[source.clone()=>hydration_schema.clone(),source.clone()=>hydration_schema]};
    assert!(matches!(duplicate, Err(HydrationError::DuplicateAtom)));
    let snapshot = match set.export_all().run(Arc::clone(&environment)).await {
        Exit::Success(snapshot) => snapshot,
        other => panic!("hydration export: {other:?}"),
    };
    assert_eq!(snapshot.len(), 1);
    success(
        source
            .set::<Environment<R>>(8)
            .run(Arc::clone(&environment))
            .await,
    );
    success(
        set.import_all(snapshot, HydrationConflictPolicy::ForceMonotonic)
            .run(Arc::clone(&environment))
            .await,
    );
    success(
        derived
            .dispose(PubSubShutdown::Discard)
            .run(Arc::clone(&environment))
            .await,
    );

    let cache_program: Effect<i64, SharedValue<()>, Environment<R>> = Scope::scoped(|scope| {
        effect! {
            let cached=cache! {scope:scope.clone(),config:CacheConfig{equal:Arc::new(|left:&i64,right|left==right),ttl:None,capacity:CacheCapacity::Bounded(nz(2)),cache_failure:Arc::new(|_:&()|false)},options:CacheOptions{ttl:CacheTtl::Fixed(None),admission:CacheAdmission::Bounded{maximum:nz(2),capacity_error:Arc::new(||())}},load |key:Arc<i64>| {{let value=*key;span!{name:"cache-load",body{Effect::<i64,(),Environment<R>>::cpu_once(move|_|value+1)?}}?}}}?;
            let value=cached.get(3)?;value.with(|value|*value).expect("cache borrow")
        }
    });
    assert_eq!(
        success(cache_program.run(Arc::clone(&environment)).await),
        4
    );
    let dependency: Effect<(), SharedValue<DependencyCacheError<()>>, Environment<R>> =
        Scope::scoped(|scope| {
            effect! {
                let cached=dependency_cache!{scope:scope.clone(),policy:DependencyCachePolicy{retention:nz(2),maximum_active:nz(2),maximum_dependencies:nz(2),key_equal:Arc::new(|left:&i64,right|left==right),dependency_equal:Arc::new(|left:&i64,right|left==right),cache_failure:Arc::new(|_:&()|false),ttl:CacheTtl::Fixed(None)},load |key:Arc<i64>|{Effect::<(i64,Vec<i64>),(),Environment<R>>::sync_once(move||(*key,vec![1]))?}}?;
                let _=cached.get(3)?;cached.invalidate_dependency(1)?;
            }
        });
    success(dependency.run(Arc::clone(&environment)).await);

    let callback_calls = Arc::new(AtomicUsize::new(0));
    let call_log = Arc::clone(&callback_calls);
    let callback_prefixes = Arc::new(AtomicUsize::new(0));
    let prefix_log = Arc::clone(&callback_prefixes);
    let captured_handler = rpc_operation! {id:OperationId::new("captured-helper").expect("operation"),version:RpcVersion::new(NonZeroU32::MIN),request:json_i64(),success:json_i64(),error:json_i64(),limits:limits(),handler |request| {
        let calls=Arc::clone(&call_log);let prefix=Arc::clone(&prefix_log);
        let _fresh={prefix.fetch_add(1,Ordering::SeqCst);String::from("fresh-callback-local")};
        Effect::<i64,i64,Environment<R>>::cpu_once(move|_|{calls.fetch_add(1,Ordering::SeqCst);request+1})?
    }};
    let first = captured_handler.call(1);
    assert_eq!(callback_calls.load(Ordering::SeqCst), 0);
    assert_eq!(callback_prefixes.load(Ordering::SeqCst), 0);
    assert_eq!(success(first.run(Arc::clone(&environment)).await), 2);
    assert_eq!(
        success(captured_handler.call(2).run(Arc::clone(&environment)).await),
        3
    );
    assert_eq!(callback_calls.load(Ordering::SeqCst), 2);
    assert_eq!(callback_prefixes.load(Ordering::SeqCst), 2);
    let handler = rpc_operation! {id:OperationId::new("lookup").expect("operation"),version:RpcVersion::new(NonZeroU32::MIN),request:json_i64(),success:json_i64(),error:json_i64(),limits:limits(),handler |request| {Effect::<i64,i64,Environment<R>>::cpu_once(move|_|request+1)?}};
    assert_eq!(
        success(handler.call(3).run(Arc::clone(&environment)).await),
        4
    );
    let callback = rpc_operation! {id:OperationId::new("callback").expect("operation"),version:RpcVersion::new(NonZeroU32::MIN),request:json_i64(),success:json_i64(),error:json_i64(),limits:limits(),handler:unary::<Environment<R>>};
    assert_eq!(
        success(callback.call(4).run(Arc::clone(&environment)).await),
        5
    );
    let stream_handler = rpc_stream_operation! {id:OperationId::new("stream").expect("operation"),version:RpcVersion::new(NonZeroU32::MIN),request:json_i64(),chunk:json_i64(),terminal:json_i64(),error:json_i64(),limits:limits(),handler |request,context:RpcStreamContext| {
        assert!(context.metadata.is_empty());
        Effect::<_,i64,Environment<R>>::sync_once(move||RpcStreamProducer::new(move||Effect::sync_once(move||RpcProduced::Terminal(request+1))))?
    }};
    let _group: RpcProtocolGroup<
        RpcProtocol<i64, Environment<R>>,
        RpcStreamProtocol<i64, (), Environment<R>, Session>,
    > = rpc_protocol! {policy:rpc_policy(),unary:[handler],streaming:[stream_handler.clone()]}
        .expect("grouped protocols");
    let middleware_released = Arc::new(AtomicUsize::new(0));
    let acquisition_releases = Arc::clone(&middleware_released);
    let authorization: RpcServiceMiddleware<ResourceValue<i64>, (), Environment<R>> = rpc_middleware! {acquire {
        let releases=Arc::clone(&acquisition_releases);
        ResourceBinding::new(Effect::sync(||7i64),move|_,_|{let releases=Arc::clone(&releases);Release::sync(move||{releases.fetch_add(1,Ordering::SeqCst);})})?
    }};
    let result = authorization.apply(
        |service, _: &Environment<R>| service,
        |input: i64| {
            Effect::<i64, (), Arc<ResourceValue<i64>>>::service(move |service| {
                service
                    .with(|value| *value + input)
                    .expect("live middleware resource")
            })
        },
        1,
    );
    assert_eq!(middleware_released.load(Ordering::SeqCst), 0);
    assert_eq!(success(result.run(Arc::clone(&environment)).await), 8);
    assert_eq!(middleware_released.load(Ordering::SeqCst), 1);
    let failure_releases = Arc::new(AtomicUsize::new(0));
    let failure_log = Arc::clone(&failure_releases);
    let failing: RpcServiceMiddleware<ResourceValue<i64>, i64, Environment<R>> = rpc_middleware! {acquire {
        let log=Arc::clone(&failure_log);
        ResourceBinding::new(Effect::sync(||1i64),move|_,_|{let log=Arc::clone(&log);Release::try_sync(move||{log.fetch_add(1,Ordering::SeqCst);Err(17)})})?
    }};
    let failed = failing.apply(
        |service, _: &Environment<R>| service,
        |_: ()| Effect::<(), i64, Arc<ResourceValue<i64>>>::fail(23),
        (),
    );
    assert!(matches!(
        failed.run(Arc::clone(&environment)).await,
        Exit::FinalizationFailed(Finalization {
            primary: PrimaryOutcome::Failed(RpcServiceMiddlewareError::Handler(23)),
            first: CleanupFailure::ReleaseFailure(RpcServiceMiddlewareError::Acquire(17)),
            ..
        })
    ));
    assert_eq!(failure_releases.load(Ordering::SeqCst), 1);

    let attempts = Arc::new(AtomicUsize::new(0));
    let counts = Arc::clone(&attempts);
    let reference = environment
        .transactions
        .reference(0i64)
        .expect("transaction reference");
    let read = reference.clone();
    let transaction: Transaction<String, ()> = transaction! {
        let owned=String::from("attempt-local");counts.fetch_add(1,Ordering::SeqCst);
        let value=read.read()?;
        match *value {value if value==0=>Transaction::<(),()>::retry()?,_=>()};
        let mut index=0;while index<3 {Transaction::<(),()>::succeed_with(||())?;index+=1;}
        owned
    };
    assert_eq!(attempts.load(Ordering::SeqCst), 0);
    let writer_counts = Arc::clone(&attempts);
    let write = reference.clone();
    let retried: Effect<String, TransactionError<()>, Environment<R>> = Scope::scoped(
        move |scope| {
            let write = write.clone();
            let writer_counts = Arc::clone(&writer_counts);
            let transaction = transaction.clone();
            effect! {
                let transaction=transaction.clone();let write=write.clone();let counts=Arc::clone(&writer_counts);
                let writer=scope.fork(Effect::cpu_once(move|_|{while counts.load(Ordering::SeqCst)==0 {std::thread::yield_now();}}).flat_map(move|()|write.write::<()>(Arc::new(1)).atomically(TransactionPolicy{max_conflicts:nz(8)})))?;
                let result=transaction.clone().atomically(TransactionPolicy{max_conflicts:nz(8)})?;
                writer.join()?;result
            }
        },
    );
    assert_eq!(
        success(retried.run(Arc::clone(&environment)).await),
        "attempt-local"
    );
    assert!(attempts.load(Ordering::SeqCst) >= 2);
    let loop_iterations = Arc::new(AtomicUsize::new(0));
    let iterations = Arc::clone(&loop_iterations);
    let cancelled: Transaction<(), ()> = transaction! {loop {iterations.fetch_add(1,Ordering::SeqCst);Transaction::succeed_with(||())?;}};
    let token = environment.cancellation.clone();
    let observed = Arc::clone(&loop_iterations);
    let interruption: Effect<(), TransactionError<()>, Environment<R>> = Scope::scoped(
        move |scope| {
            let cancelled = cancelled.clone();
            let token = token.clone();
            let observed = Arc::clone(&observed);
            effect! {
                let cancelled=cancelled.clone();let token=token.clone();let observed=Arc::clone(&observed);
                let running=scope.fork(cancelled.atomically(TransactionPolicy{max_conflicts:nz(8)}))?;
                Effect::cpu_once(move|_|{while observed.load(Ordering::SeqCst)==0 {std::thread::yield_now();}assert!(token.request());})?;
                running.join()?;
            }
        },
    );
    assert!(matches!(
        interruption.run_cancellable(Arc::clone(&environment)).await,
        Exit::Interrupted
    ));
    assert!(loop_iterations.load(Ordering::SeqCst) > 0);

    let resource_releases = Arc::new(AtomicUsize::new(0));
    let resource_log = Arc::clone(&resource_releases);
    let address = ClusterAddress {
        entity_type: ClusterEntityType::new("composition-entity").expect("entity type"),
        entity: ClusterEntityId::from_stable(key("entity")),
        shard: ClusterShardId::from_stable(key("shard")),
    };
    let activation = ActivationId::from_stable(key("activation"));
    let owner = match success(
        environment
            .activation
            .activate(
                address.clone(),
                activation,
                UtcMillis::new(0),
                UtcMillis::new(100),
            )
            .run(Arc::new(()))
            .await,
    ) {
        ActivationOutcome::Activated { lease, .. } => lease.owner,
        _ => panic!("fixture activation"),
    };
    let partial_releases = Arc::new(AtomicUsize::new(0));
    let partial_log = Arc::clone(&partial_releases);
    let partial: EntityResourcePlan<(), u8, Environment<R>> = entity_resource! {
        let log=Arc::clone(&partial_log);
        let _resource=ResourceBinding::new(Effect::sync(||String::from("partial")),move|_,_|{let log=Arc::clone(&log);Release::try_sync(move||{log.fetch_add(1,Ordering::SeqCst);Err(9u8)})})?;
        Effect::<(),u8,Environment<R>>::fail(7)?;
    };
    let mapped = partial.map_error(|code| Failure { code });
    let partial_owner = owner.clone();
    let partial: Effect<(), Failure, Environment<R>> =
        Scope::scoped(move |scope| mapped.acquire(scope, partial_owner.clone()));
    assert!(matches!(
        partial.run(Arc::clone(&environment)).await,
        Exit::FinalizationFailed(Finalization {
            primary: PrimaryOutcome::Failed(Failure { code: 7 }),
            first: CleanupFailure::ReleaseFailure(Failure { code: 9 }),
            ..
        })
    ));
    assert_eq!(partial_releases.load(Ordering::SeqCst), 1);
    let resources: EntityResourcePlan<ResourceValue<String>, (), Environment<R>> = entity_resource! {
        let releases=Arc::clone(&resource_log);
        ResourceBinding::new(Effect::sync(||String::from("state")),move|_,_|{let releases=Arc::clone(&releases);Release::sync(move||{releases.fetch_add(1,Ordering::SeqCst);})})?
    };
    let definition = entity! {entity_type:address.entity_type.clone(),resources:resources,handlers:[rpc_operation! {id:OperationId::new("entity-length").expect("operation"),version:RpcVersion::new(NonZeroU32::MIN),request:json_i64(),success:json_i64(),error:json_i64(),limits:limits()}=>|state:Arc<ResourceValue<String>>,request:i64| {let size=state.access(|value|value.len() as i64)?;Effect::<i64,i64,Environment<R>>::sync_once(move||size+request)?}]};
    let catalog: ClusterDefinition<(), Environment<R>> =
        cluster! {policy:rpc_policy(),entities:[definition]}.expect("catalog");
    assert_eq!(resource_releases.load(Ordering::SeqCst), 0);
    let wall = environment.wall.clone();
    let dispatch_address = address.clone();
    let release_check = Arc::clone(&resource_releases);
    let acquired: Effect<(), (), Environment<R>> = Scope::scoped(move |scope| {
        let releases = Arc::clone(&resource_releases);
        let wall = wall.clone();
        let address = dispatch_address.clone();
        catalog
            .activate(scope, owner.clone())
            .map_error(|_| ())
            .flat_map(move |live| {
                let releases = Arc::clone(&releases);
                let wall = wall.clone();
                let address = address.clone();
                Effect::sync_once(move || {
                    assert_eq!(releases.load(Ordering::SeqCst), 0);
                    wall.set(UtcMillis::new(100));
                    live
                })
                .flat_map(move |live| {
                    live.execute(address.clone(), Arc::from(&b""[..]))
                        .materialize_exit()
                        .map_error(|never| match never {})
                })
                .map(|exit| {
                    assert!(matches!(
                        exit,
                        Exit::Fail(EntityActivationError::StaleOwnership)
                    ))
                })
            })
    });
    success(acquired.run(Arc::clone(&environment)).await);
    assert_eq!(release_check.load(Ordering::SeqCst), 1);
    environment.wall.set(UtcMillis::new(0));
    let singleton_address = ClusterAddress {
        entity_type: ClusterEntityType::new("composition-singleton").expect("entity type"),
        entity: ClusterEntityId::from_stable(key("singleton")),
        shard: ClusterShardId::from_stable(key("shard")),
    };
    let single = singleton! {singleton:ClusterSingleton::new(singleton_address),activation:ActivationId::from_stable(key("singleton-activation")),lease_until:UtcMillis::new(100),run |owner:ClusterOwner| {assert_eq!(owner.address.entity_type.as_str(),"composition-singleton");Effect::<i64,(),Environment<R>>::cpu_once(|_|9)?}};
    assert!(matches!(
        success(single.run(Arc::clone(&environment)).await),
        SingletonOutcome::Ran(9)
    ));
    let schedule = cluster_schedule! {id:ClusterScheduleId::from_stable(key("schedule")),cron:Cron::parse("* * * * * *",DayMatch::Both).expect("cron"),transitions:CalendarTransitionPolicy{gap:GapPolicy::Skip,overlap:OverlapPolicy::Earlier},catch_up:MissedOccurrencePolicy::Latest,concurrent:ConcurrentOccurrencePolicy::Skip,deadline_after:Duration::from_secs(10)};
    assert_eq!(
        schedule.id(),
        &ClusterScheduleId::from_stable(key("schedule"))
    );

    let event_definition = event! {group:EventGroupId::new("composition-events").expect("group"),tag:EventTag::new("incremented").expect("tag"),version:EventVersion::new(NonZeroU32::MIN),payload:json_i64(),success:json_i64(),error:json_i64(),payload_identity:binary("composition-payload"),success_identity:binary("composition-success"),error_identity:binary("composition-error"),limits:binary_limits(),aggregate:|_:&i64|AggregateId::from_stable(key("aggregate"))};
    let handled = handled_event! {definition:event_definition,commit:EventCommitPolicy::SuccessOnly,handler |context:EventHandlerContext,payload:Arc<i64>| {
        assert_eq!(context.expected,JournalCursor::new(0));
        span! {name:"handled-event",effect:Effect::<Result<i64,i64>,(),Environment<R>>::cpu_once(move|_|Ok(*payload+1))}?
    }};
    let grouped=event_group! {group:EventGroupId::new("composition-events").expect("group"),events:[&handled]}.expect("handled grouping");
    assert!(grouped.contains(
        &EventTag::new("incremented").expect("tag"),
        EventVersion::new(NonZeroU32::MIN)
    ));
    assert!(matches!(
        success(
            handled
                .execute(
                    EventJournalId::from_stable(key("journal")),
                    EventId::from_stable(key("event")),
                    JournalCursor::new(0),
                    UtcMillis::new(0),
                    Arc::new(4)
                )
                .run(Arc::clone(&environment))
                .await
        ),
        EventRunOutcome::Committed { result: Ok(5), .. }
    ));
    let projection=projection! {key:ProjectionKey{id:ProjectionId::from_stable(key("projection")),stream:JournalStream{journal:EventJournalId::from_stable(key("journal")),aggregate:AggregateId::from_stable(key("aggregate"))}},version:ProjectionVersion::new(NonZeroU32::MIN),initial:||0i64,cases:[handled.definition()=>|state:&i64,payload:i64,outcome:Result<i64,i64>|Ok::<i64,()>(*state+payload+outcome.expect("successful domain result"))]}.expect("typed projection");
    assert!(
        matches!(success(projection.advance(nz(8)).run(Arc::clone(&environment)).await),ProjectionAdvance::Advanced{snapshot,..} if *snapshot.state==9)
    );

    let failing_projection=projection! {key:ProjectionKey{id:ProjectionId::from_stable(key("projection-failure")),stream:JournalStream{journal:EventJournalId::from_stable(key("journal")),aggregate:AggregateId::from_stable(key("aggregate"))}},version:ProjectionVersion::new(NonZeroU32::MIN),initial:||0i64,cases:[handled.definition()=>|_:&i64,_:i64,_:Result<i64,i64>|Err::<i64,u8>(31)]}.expect("failing typed projection");
    assert!(matches!(
        failing_projection
            .advance(nz(8))
            .run(Arc::clone(&environment))
            .await,
        Exit::Fail(ProjectionAdvanceError::Fold(ProjectionCaseError::Fold(31)))
    ));
    assert!(
        success(
            environment
                .projection
                .load(failing_projection.key.clone())
                .run(Arc::new(()))
                .await
        )
        .is_none()
    );

    let coordination_runs = Arc::new(AtomicUsize::new(0));
    let coordination_log = Arc::clone(&coordination_runs);
    let durable_run = WorkflowRunId::from_stable(key("durable-composition-run"));
    let activity_run = durable_run.clone();
    let work = activity! {name:WorkflowActivityId::from_stable(key("durable-composition-activity")),input:json_i64(),success:json_i64(),error:json_i64(),limits:limits(),body |value:Arc<i64>| {
        let value=*value;
        span!{name:"recorded-activity",effect:Effect::<Result<i64,i64>,(),Environment<R>>::cpu_once(move|_|Ok(value+1))}?
    }};
    let coordination = workflow! {name:WorkflowDefinitionId::new("durable-composition").expect("definition"),version:WorkflowVersion::new(NonZeroU32::MIN),payload:json_i64(),success:json_i64(),error:json_i64(),compatibility:Arc::from(&b"composition-v1"[..]),limits:limits(),run |value| {
        let work=work.clone();let run=activity_run.clone();let log=Arc::clone(&coordination_log);
        let attempt_local={log.fetch_add(1,Ordering::SeqCst);String::from("fresh-durable-local")};
        let value=work.call(run,NonZeroU32::MIN,value)?;
        assert_eq!(attempt_local,"fresh-durable-local");
        match value {Ok(value) if value>0=>Ok::<i64,i64>(value),other=>other}
    }};
    let execution = coordination.run(durable_run, 4);
    assert_eq!(coordination_runs.load(Ordering::SeqCst), 0);
    assert!(matches!(
        success(execution.run(Arc::clone(&environment)).await),
        WorkflowRunResult::Completed(Ok(5))
    ));
    assert!(matches!(
        success(execution.run(Arc::clone(&environment)).await),
        WorkflowRunResult::Completed(Ok(5))
    ));
    assert_eq!(coordination_runs.load(Ordering::SeqCst), 1);

    let run = WorkflowRunId::from_stable(key("queue-run"));
    let version = WorkflowVersion::new(NonZeroU32::MIN);
    let definition =
        WorkflowDefinitionId::new("composition-queue-workflow").expect("workflow identity");
    success(
        environment
            .workflow
            .register(WorkflowRegistration {
                id: definition.clone(),
                version,
                compatibility: Arc::from(&b"composition-v1"[..]),
            })
            .run(Arc::new(()))
            .await,
    );
    success(
        environment
            .workflow
            .start(
                WorkflowHandle {
                    run: run.clone(),
                    definition,
                    version,
                },
                Arc::from(&b"input"[..]),
            )
            .run(Arc::new(()))
            .await,
    );
    let deferred = DurableDeferred {
        run: run.clone(),
        signal: WorkflowSignalId::from_stable(key("queue-result")),
        version,
        codec: json_i64(),
        limits: limits(),
    };
    let queue: PersistedQueue<i64, Domain> = persisted_queue! {name:PersistedQueueName::new("composition-jobs").expect("queue name"),payload:json_i64(),identity:binary("composition-job"),limits:binary_limits(),policy:QueueMessagePolicy{maximum_attempts:NonZeroU32::MIN,acknowledged_retention:Duration::from_secs(10)}};
    let queue = durable_queue! {queue:queue};
    success(
        queue
            .enqueue(
                key("queue-run"),
                || 4,
                &deferred,
                UtcMillis::new(0),
                UtcMillis::new(0),
            )
            .run(Arc::clone(&environment))
            .await,
    );
    let mut claims = success(
        queue
            .claim(
                QueueClaimOptions::new(
                    QueueOwner::new("composition-worker").expect("worker"),
                    UtcMillis::new(0),
                    UtcMillis::new(100),
                    NonZeroUsize::MIN,
                )
                .expect("claim policy"),
            )
            .run(Arc::clone(&environment))
            .await,
    );
    let delivery = queue
        .decode_claim(claims.pop().expect("one queued job"))
        .map_err(|_| ())
        .expect("decoded job");
    let worker = durable_queue! {queue:DurableQueue::new(queue.queue().clone()),delivery:delivery,deferred:deferred.clone(),acknowledged_at:UtcMillis::new(1),handler |payload| {span!{name:"queue-worker",body{Effect::<i64,(),Environment<R>>::cpu_once(move|_|payload+1)?}}?}};
    assert!(matches!(
        success(worker.run(Arc::clone(&environment)).await),
        QueueSettlement::Applied
    ));
    assert!(matches!(
        success(deferred.wait().run(Arc::clone(&environment)).await),
        DurableSignalWait::Ready(5)
    ));
    assert!(
        matches!(success(queue.inspect(key("queue-run")).run(Arc::clone(&environment)).await),Some(record) if matches!(record.state,QueueRecordState::Acknowledged{..}))
    );

    // Session ownership and middleware lifetime are observed in the same representative RPC case.
    let closed = Arc::new(AtomicUsize::new(0));
    let output = Arc::new(Mutex::new(Vec::new()));
    let request = RpcEnvelope {
        operation: stream_handler.operation().spec().id.clone(),
        version: stream_handler.operation().spec().version,
        call: RpcCallId::new("composition-stream-call").expect("call"),
        kind: RpcEnvelopeKind::Request,
        sequence: None,
        payload: JsonValue::from(4),
        metadata: BTreeMap::new(),
    }
    .encode(rpc_policy())
    .expect("request frame")
    .into_bytes()
    .expect("request bytes");
    let session = Arc::new(Session {
        input: Arc::new(Mutex::new(VecDeque::from([request]))),
        output: Arc::clone(&output),
        closed: Arc::clone(&closed),
    });
    let acquisition: RpcServiceMiddleware<(), (), Environment<R>> =
        rpc_middleware! {acquire {Effect::sync(||())?}};
    let serve = rpc_stream_middleware! {middleware:acquisition,connect:|_:Arc<()>,environment:&Environment<R>|Arc::clone(&environment.base),handler:rpc_stream_operation! {id:OperationId::new("stream").expect("operation"),version:RpcVersion::new(NonZeroU32::MIN),request:json_i64(),chunk:json_i64(),terminal:json_i64(),error:json_i64(),limits:limits(),handler |request,_context|{Effect::<_,i64,Arc<R>>::sync_once(move||RpcStreamProducer::new(move||Effect::sync_once(move||RpcProduced::Terminal(request+1))))?}},policy:rpc_policy(),accepted:session};
    success(serve.run(Arc::clone(&environment)).await);
    assert_eq!(closed.load(Ordering::SeqCst), 1);
    assert!(!output.lock().expect("stream responses").is_empty());
    let codec = schema! {struct FixtureRecord{value:json_i64()}policy:parse_policy()}
        .expect("record schema");
    assert_eq!(
        codec
            .decode_ref(&JsonValue::Object(
                [("value".to_owned(), JsonValue::from(3))]
                    .into_iter()
                    .collect()
            ))
            .expect("decoded record")
            .value,
        3
    );
}
