//! Runtime and storage wiring for the café stories; these are Effect providers.
//!
//! Tasks and CPU/blocking jobs use independent native threads, supplied by this
//! example application. The core selects no executor. Stores demonstrate logical
//! recording and fencing in one process; they do not survive a process restart.

/// Wire models shared by the two related ticket examples.
pub mod tickets;
/// Finite RPC/socket transport plumbing, independent of the application DSL recipe.
pub mod transport;

use effect::cluster::{HasClusterActivationStore, MemoryClusterMessageStore, MessageStoreCapacity};
use effect::encoding::{BinaryIdentity, BinaryLimits, FramePolicy};
use effect::eventlog::{
    HasAtomicProjectionStore, HasEventJournal, MemoryEventJournal, MemoryJournalCapacity,
    MemoryProjectionStore,
};
use effect::persistence::{
    HasKeyValueStore, HasPersistedQueueStore, MemoryKeyValueStore, MemoryPersistedQueueStore,
    QueueStoreCapacity, StoreCapacity,
};
use effect::rpc::RpcEnvelopePolicy;
use effect::transactional::{HasTransactionDomain, TransactionDomain};
use effect::workflow::{HasWorkflowEngine, MemoryWorkflowEngine};
use effect::{
    BlockingExecutor, Cancellation, CpuExecutor, Defect, DiagnosticCapacity, DiagnosticOverflow,
    Diagnostics, DuplicateFields, HasBlockingExecutor, HasCancellation, HasClock, HasCpuExecutor,
    HasRequestScope, HasTaskSpawner, HasTracer, HasWallClock, KeyDomain, KeyNamespace, KeySegment,
    ParseLimits, ParsePolicy, RequestScope, ScheduledJob, ScheduledTask, SchemaErrorMode, SpanId,
    StableKey, TaskSpawner, TestClock, TestWallClock, Tracer, UnknownFields, UtcMillis,
};
use std::num::{NonZeroU64, NonZeroUsize};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};
use std::thread::JoinHandle;

/// Retain explicit parsing bounds, including enough text width for the example stable identities.
#[must_use]
pub fn limits() -> ParseLimits {
    ParseLimits {
        bytes: std::num::NonZeroUsize::MIN.saturating_add(4095),
        depth: std::num::NonZeroUsize::MIN.saturating_add(15),
        nodes: std::num::NonZeroUsize::MIN.saturating_add(127),
        collection: std::num::NonZeroUsize::MIN.saturating_add(127),
        issues: std::num::NonZeroUsize::MIN.saturating_add(7),
    }
}
/// Binary bounds match the example's small typed event and queue records.
#[must_use]
pub fn binary_limits() -> BinaryLimits {
    BinaryLimits {
        bytes: std::num::NonZeroUsize::MIN.saturating_add(4095),
        depth: std::num::NonZeroUsize::MIN.saturating_add(15),
        nodes: std::num::NonZeroUsize::MIN.saturating_add(127),
        collection: std::num::NonZeroUsize::MIN.saturating_add(15),
        steps: std::num::NonZeroUsize::MIN.saturating_add(511),
    }
}
/// Explicit RPC framing policy used by unary, streaming and entity registration.
#[must_use]
pub fn rpc_policy() -> RpcEnvelopePolicy {
    RpcEnvelopePolicy {
        frame: FramePolicy {
            maximum_payload: std::num::NonZeroUsize::MIN.saturating_add(4095),
        },
        json: limits(),
    }
}
/// Pure example codec policy retains rejection/first-error semantics.
#[must_use]
pub fn parse_policy() -> ParsePolicy {
    ParsePolicy {
        limits: limits(),
        errors: SchemaErrorMode::First,
        unknown_fields: UnknownFields::Reject,
        duplicate_fields: DuplicateFields::Reject,
    }
}
/// Application storage namespace for café draft, order, and receipt identities.
pub struct CafeKeys;
impl KeyDomain for CafeKeys {
    /// Explicit stable namespace tag, shared by the example encoded identities.
    const TAG: &'static str = "neighborhood-cafe";
}
/// Names and segments are explicit application vocabulary for the example's stored identities.
pub fn key(name: &str) -> Result<StableKey<CafeKeys>, Defect> {
    let namespace = KeyNamespace::<CafeKeys>::new("neighborhood-cafe")
        .map_err(|error| Defect::new(format!("example namespace: {error:?}")))?;
    StableKey::new(&namespace, [KeySegment::Text(name.into())])
        .map_err(|error| Defect::new(format!("example stable key: {error:?}")))
}
/// Binary wire identities remain explicit and independent of Rust type names.
pub fn binary(name: &str) -> Result<BinaryIdentity, Defect> {
    BinaryIdentity::new(name, NonZeroU64::MIN)
        .map_err(|error| Defect::new(format!("example binary identity: {error:?}")))
}
/// Café environment: application services beside Effect provider capabilities.
pub struct Cafe<Services: Send + Sync + 'static = (), Sales: Send + Sync + 'static = ()> {
    /// Application-defined café services, named in each customer or staff story.
    pub services: Arc<Services>,
    /// Independent workers explicitly selected by the café application.
    pub base: Arc<Providers>,
    /// CPU and blocking jobs intentionally use distinct concrete provider types.
    cpu: CoffeeCpu,
    /// Blocking work is independently selected through its own R capability.
    blocking: CoffeeBlocking,
    /// Example tracer with observable starts/outcomes.
    pub tracer: Tracer,
    /// Explicit wall-clock authority for activation and stored occurrence times.
    pub wall: TestWallClock,
    /// Cancellation authority shared by tasks and atomic stock reservations.
    pub cancellation: Cancellation,
    /// Process-local resolver identity shared by this café's finite request session.
    pub requests: RequestScope,
    /// Journal authority shared by stock reservations in the café.
    pub transactions: Arc<TransactionDomain>,
    /// Existing in-memory activation authority.
    pub activation: Arc<MemoryClusterMessageStore>,
    /// Existing journal used by handled events and projection decoding.
    pub journal: Arc<MemoryEventJournal>,
    /// Atomic projection state/checkpoint store.
    pub projection: Arc<MemoryProjectionStore<Sales>>,
    /// Existing queue store retaining worker fences and acknowledgement state.
    pub queues: Arc<MemoryPersistedQueueStore>,
    /// Existing workflow engine retaining durable deferred results.
    pub workflow: Arc<MemoryWorkflowEngine>,
    /// Bounded logical record store used by saved coffee drafts.
    pub records: Arc<MemoryKeyValueStore>,
}
impl<Services: Send + Sync + 'static, Sales: Send + Sync + 'static> HasTaskSpawner
    for Cafe<Services, Sales>
{
    /// Reuse the consumer's independent task provider.
    type Tasks = Providers;
    /// Delegate without installing another runtime.
    fn task_spawner(&self) -> &Self::Tasks {
        &self.base
    }
}
impl<Services: Send + Sync + 'static, Sales: Send + Sync + 'static> HasCpuExecutor
    for Cafe<Services, Sales>
{
    /// Reuse the consumer's CPU provider in the same environment.
    type Executor = CoffeeCpu;
    /// Delegate the application-selected CPU capability.
    fn cpu_executor(&self) -> &Self::Executor {
        &self.cpu
    }
}
impl<Services: Send + Sync + 'static, Sales: Send + Sync + 'static> HasBlockingExecutor
    for Cafe<Services, Sales>
{
    /// Reuse the consumer's blocking provider.
    type Executor = CoffeeBlocking;
    /// Delegate to the independently selected blocking capability.
    fn blocking_executor(&self) -> &Self::Executor {
        &self.blocking
    }
}
impl<Services: Send + Sync + 'static, Sales: Send + Sync + 'static> HasClock
    for Cafe<Services, Sales>
{
    /// Monotonic timing stays with the consumer's clock provider.
    type Clock = TestClock;
    /// Delegate the existing timing capability.
    fn clock(&self) -> &Self::Clock {
        &self.base.clock
    }
}
impl<Services: Send + Sync + 'static, Sales: Send + Sync + 'static> HasWallClock
    for Cafe<Services, Sales>
{
    /// Explicit test wall clock supports reproducible lease checks.
    type Clock = TestWallClock;
    /// Borrow the same authority for all activation operations.
    fn wall_clock(&self) -> &Self::Clock {
        &self.wall
    }
}
impl<Services: Send + Sync + 'static, Sales: Send + Sync + 'static> HasTracer
    for Cafe<Services, Sales>
{
    /// Return the example tracer used by every nested domain body.
    fn tracer(&self) -> &Tracer {
        &self.tracer
    }
}
impl<Services: Send + Sync + 'static, Sales: Send + Sync + 'static> HasCancellation
    for Cafe<Services, Sales>
{
    /// Borrow the explicit example cancellation authority.
    fn cancellation(&self) -> &Cancellation {
        &self.cancellation
    }
}
impl<Services: Send + Sync + 'static, Sales: Send + Sync + 'static> HasRequestScope
    for Cafe<Services, Sales>
{
    /// Resolver sharing uses the consumer-selected session identity, not a global key.
    fn request_scope(&self) -> &RequestScope {
        &self.requests
    }
}
impl<Services: Send + Sync + 'static, Sales: Send + Sync + 'static> HasTransactionDomain
    for Cafe<Services, Sales>
{
    /// Share the same journal domain across macro-created transaction attempts.
    fn transaction_domain(&self) -> Arc<TransactionDomain> {
        Arc::clone(&self.transactions)
    }
}
impl<Services: Send + Sync + 'static, Sales: Send + Sync + 'static> HasClusterActivationStore
    for Cafe<Services, Sales>
{
    /// Keep provider failures in the existing concrete activation error channel.
    type Store = MemoryClusterMessageStore;
    /// Share only the existing store handle.
    fn cluster_activation_store(&self) -> Arc<Self::Store> {
        Arc::clone(&self.activation)
    }
}
impl<Services: Send + Sync + 'static, Sales: Send + Sync + 'static> HasEventJournal
    for Cafe<Services, Sales>
{
    /// Use the existing bounded in-memory event journal.
    type Journal = MemoryEventJournal;
    /// Retain one authoritative journal instance.
    fn event_journal(&self) -> Arc<Self::Journal> {
        Arc::clone(&self.journal)
    }
}
impl<Services: Send + Sync + 'static, Sales: Send + Sync + 'static> HasAtomicProjectionStore<Sales>
    for Cafe<Services, Sales>
{
    /// Keep state and cursor atomic under the existing store contract.
    type Store = MemoryProjectionStore<Sales>;
    /// Retain the shared projection authority.
    fn atomic_projection_store(&self) -> Arc<Self::Store> {
        Arc::clone(&self.projection)
    }
}
impl<Services: Send + Sync + 'static, Sales: Send + Sync + 'static> HasPersistedQueueStore
    for Cafe<Services, Sales>
{
    /// Existing queue provider keeps its original typed settlement errors.
    type Store = MemoryPersistedQueueStore;
    /// Return the configured bounded queue store.
    fn persisted_queue_store(&self) -> Arc<Self::Store> {
        Arc::clone(&self.queues)
    }
}
impl<Services: Send + Sync + 'static, Sales: Send + Sync + 'static> HasWorkflowEngine
    for Cafe<Services, Sales>
{
    /// The workflow engine remains the existing logical recording/replay authority.
    type Engine = MemoryWorkflowEngine;
    /// Return the shared engine selected through this environment.
    fn workflow_engine(&self) -> Arc<Self::Engine> {
        Arc::clone(&self.workflow)
    }
}
/// Application-selected worker provider for these finite demonstrations.
pub struct Providers {
    /// Explicit maximum concurrently accepted workers.
    maximum: NonZeroUsize,
    /// Owned workers, joined after the domain scope closes.
    workers: Mutex<Vec<JoinHandle<Result<(), Defect>>>>,
    /// Deterministic monotonic clock; examples using timers advance it explicitly.
    clock: TestClock,
}
impl Providers {
    /// Accept an independent worker, or reject it before transferring the task.
    fn submit(
        &self,
        work: impl FnOnce() -> Result<(), Defect> + Send + 'static,
    ) -> Result<(), Defect> {
        let mut workers = self
            .workers
            .lock()
            .map_err(|error| Defect::new(error.to_string()))?;
        let active = workers
            .iter()
            .filter(|worker| !worker.is_finished())
            .count();
        if active >= self.maximum.get() {
            return Err(Defect::new("example worker capacity"));
        }
        workers.push(
            std::thread::Builder::new()
                .spawn(work)
                .map_err(Defect::from_io)?,
        );
        Ok(())
    }
    /// Join every worker after effects have settled their scopes.
    pub fn finish(&self) -> Result<(), Defect> {
        let workers = std::mem::take(
            &mut *self
                .workers
                .lock()
                .map_err(|error| Defect::new(error.to_string()))?,
        );
        let mut failure = None;
        for worker in workers {
            if let Err(error) = worker
                .join()
                .map_err(Defect::from_panic)
                .and_then(|result| result)
            {
                failure.get_or_insert(error);
            }
        }
        failure.map_or(Ok(()), Err)
    }
}
impl TaskSpawner for Providers {
    /// Drive accepted tasks with the example's executor on an independent thread.
    fn spawn(&self, task: ScheduledTask) -> Result<(), Defect> {
        self.submit(move || {
            futures_executor::block_on(task);
            Ok(())
        })
    }
}
/// Concrete CPU provider, distinct from the task spawner and blocking provider.
pub struct CoffeeCpu {
    /// Shared worker ownership/admission, without installing a core executor.
    workers: Arc<Providers>,
}
impl CpuExecutor for CoffeeCpu {
    /// Run the CPU job on an application-owned worker.
    fn submit(&self, job: ScheduledJob) -> Result<(), Defect> {
        self.workers.submit(move || job.run())
    }
}
/// Concrete blocking provider selected separately from CPU execution through R.
pub struct CoffeeBlocking {
    /// Application worker ownership shared only for admission and final joining.
    workers: Arc<Providers>,
}
impl BlockingExecutor for CoffeeBlocking {
    /// Run the blocking job on an application-owned worker.
    fn submit(&self, job: ScheduledJob) -> Result<(), Defect> {
        self.workers.submit(move || job.run())
    }
}
impl<Services: Send + Sync + 'static, Sales: Send + Sync + 'static> Cafe<Services, Sales> {
    /// Open this local café with explicit order/storage capacity and worker admission.
    #[must_use]
    pub fn open(services: Services, capacity: NonZeroUsize, workers: NonZeroUsize) -> Arc<Self> {
        let ids = Arc::new(AtomicUsize::new(0));
        let tracer = Tracer::new(
            Diagnostics::new(
                DiagnosticCapacity::Bounded(capacity),
                DiagnosticOverflow::DiscardNewest,
            ),
            |annotations| Ok(annotations.clone()),
            move |start| {
                use std::io::Write;
                writeln!(
                    std::io::stdout().lock(),
                    "[trace] {} parent={:?}",
                    start.name,
                    start.parent
                )
                .map_err(Defect::from_io)?;
                Ok(Some(SpanId::new(format!(
                    "cafe-span-{}",
                    ids.fetch_add(1, Ordering::SeqCst)
                ))))
            },
            |finish| {
                use std::io::Write;
                writeln!(
                    std::io::stdout().lock(),
                    "[trace] finished {:?}",
                    finish.outcome
                )
                .map_err(Defect::from_io)?;
                Ok(())
            },
        );
        let providers = Arc::new(Providers {
            maximum: workers,
            workers: Mutex::new(Vec::new()),
            clock: TestClock::new(std::time::Duration::ZERO),
        });
        Arc::new(Self {
            services: Arc::new(services),
            cpu: CoffeeCpu {
                workers: providers.clone(),
            },
            blocking: CoffeeBlocking {
                workers: providers.clone(),
            },
            base: providers,
            tracer,
            wall: TestWallClock::new(UtcMillis::new(0)),
            cancellation: Cancellation::new(),
            requests: RequestScope::new(),
            transactions: Arc::new(TransactionDomain::new(capacity)),
            activation: Arc::new(MemoryClusterMessageStore::new(
                MessageStoreCapacity::Bounded(capacity),
            )),
            journal: Arc::new(MemoryEventJournal::new(MemoryJournalCapacity {
                streams: capacity,
                entries_per_stream: capacity,
                tombstones_per_stream: capacity,
            })),
            projection: Arc::new(MemoryProjectionStore::new(capacity)),
            queues: Arc::new(MemoryPersistedQueueStore::new(QueueStoreCapacity::Bounded(
                capacity,
            ))),
            workflow: Arc::new(MemoryWorkflowEngine::new()),
            records: Arc::new(MemoryKeyValueStore::new(StoreCapacity::Bounded(capacity))),
        })
    }
}

impl<Services: Send + Sync + 'static, Sales: Send + Sync + 'static> HasKeyValueStore
    for Cafe<Services, Sales>
{
    /// Logical store, without a physical durability warranty.
    type Store = MemoryKeyValueStore;
    /// Return the consumer-selected record provider.
    fn key_value_store(&self) -> Arc<Self::Store> {
        self.records.clone()
    }
}
