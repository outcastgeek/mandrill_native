#[path = "../../shared/provider_program.rs"]
mod provider_program;
#[path = "../../shared/cluster_workflow_takeover.rs"]
mod cluster_workflow_takeover;
#[path = "../../shared/transactional_collections.rs"]
mod transactional_collections;
#[path = "../../capability_consumer/src/persistence_provider.rs"]
mod persistence_provider;

use effect::{
    BlockingExecutor, CpuExecutor, Defect, Exit, HasBlockingExecutor, HasCpuExecutor,
    HasTaskSpawner, LibraryDefect, ScheduledJob, ScheduledTask, TaskSpawner,
};
use effect::eventlog::{EventRunOutcome, HasEventJournal, MemoryEventJournal,
    MemoryJournalCapacity};
use effect::workflow::{HasWorkflowEngine, MemoryWorkflowEngine, WorkflowRunResult};
use effect::persistence::{HasKeyValueStore, HasPersistedQueueStore,
    MemoryPersistedQueueStore, QueueStoreCapacity, StoredLookup};
use effect::StoredOutcome;
use effect::transactional::{HasTransactionDomain, TransactionDomain, TxSubscriptionRef};
use effect::cluster::{HasClusterExecutor, HasClusterMessageStore,
    MemoryClusterMessageStore};
use effect::{Cancellation, Clock, ClockError, ClockFuture, ClockTick, HasClock,
    HasWallClock, TestWallClock, UtcMillis};
use persistence_provider::ExternalKeyValueStore;
use rayon::ThreadPool;
use std::num::NonZeroUsize;
use std::time::Duration;
use std::sync::{Arc, Condvar, Mutex};
use tokio::runtime::{Builder, Handle, Runtime};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tokio::task::JoinHandle;

struct TokioTasks {
    handle: Handle,
    admission: Arc<Semaphore>,
    joins: Mutex<Vec<JoinHandle<()>>>,
}

impl TokioTasks {
    fn new(handle: Handle, capacity: NonZeroUsize) -> Self {
        Self {
            handle,
            admission: Arc::new(Semaphore::new(capacity.get())),
            joins: Mutex::new(Vec::new()),
        }
    }

    async fn join(&self) {
        let joins = std::mem::take(&mut *self.joins.lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner));
        for join in joins {
            join.await.expect("Tokio task completed without panic");
        }
    }
}

impl TaskSpawner for TokioTasks {
    fn spawn(&self, task: ScheduledTask) -> Result<(), Defect> {
        let permit = Arc::clone(&self.admission).try_acquire_owned()
            .map_err(|_| Defect::from(LibraryDefect::ProviderCapacityFull))?;
        let join = self.handle.spawn(async move {
            let _permit = permit;
            task.await;
        });
        self.joins.lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(join);
        Ok(())
    }
}

struct RayonCpu {
    pool: ThreadPool,
    capacity: NonZeroUsize,
    state: Arc<(Mutex<CpuState>, Condvar)>,
}

#[derive(Default)]
struct CpuState {
    pending: usize,
    failures: Vec<Defect>,
}

impl RayonCpu {
    fn new(workers: NonZeroUsize, capacity: NonZeroUsize) -> Self {
        Self {
            pool: rayon::ThreadPoolBuilder::new().num_threads(workers.get())
                .build().expect("Rayon pool builds"),
            capacity,
            state: Arc::new((Mutex::new(CpuState::default()), Condvar::new())),
        }
    }

    fn join(&self) {
        let (lock, changed) = &*self.state;
        let mut state = lock.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        while state.pending != 0 {
            state = changed.wait(state)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
        }
        assert!(state.failures.is_empty(), "Rayon job failed");
    }
}

impl CpuExecutor for RayonCpu {
    fn submit(&self, job: ScheduledJob) -> Result<(), Defect> {
        {
            let (lock, _) = &*self.state;
            let mut state = lock.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            if state.pending >= self.capacity.get() {
                return Err(LibraryDefect::ProviderCapacityFull.into());
            }
            state.pending += 1;
        }
        let state = Arc::clone(&self.state);
        self.pool.spawn(move || {
            let result = job.run();
            let (lock, changed) = &*state;
            let mut state = lock.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            if let Err(error) = result {
                state.failures.push(error);
            }
            state.pending -= 1;
            changed.notify_all();
        });
        Ok(())
    }
}

struct TokioBlocking {
    handle: Handle,
    admission: Arc<Semaphore>,
    joins: Mutex<Vec<JoinHandle<Result<(), Defect>>>>,
}

impl TokioBlocking {
    fn new(handle: Handle, capacity: NonZeroUsize) -> Self {
        Self {
            handle,
            admission: Arc::new(Semaphore::new(capacity.get())),
            joins: Mutex::new(Vec::new()),
        }
    }

    async fn join(&self) {
        let joins = std::mem::take(&mut *self.joins.lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner));
        for join in joins {
            join.await.expect("Tokio blocking job joined")
                .expect("Tokio blocking job completed without defect");
        }
    }
}

impl BlockingExecutor for TokioBlocking {
    fn submit(&self, job: ScheduledJob) -> Result<(), Defect> {
        let permit: OwnedSemaphorePermit = Arc::clone(&self.admission)
            .try_acquire_owned()
            .map_err(|_| Defect::from(LibraryDefect::ProviderCapacityFull))?;
        let join = self.handle.spawn_blocking(move || {
            let _permit = permit;
            job.run()
        });
        self.joins.lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(join);
        Ok(())
    }
}

struct MixedEnvironment {
    tasks: TokioTasks,
    cpu: RayonCpu,
    blocking: TokioBlocking,
    journal: Arc<MemoryEventJournal>,
    engine: Arc<MemoryWorkflowEngine>,
    store: Arc<ExternalKeyValueStore>,
    queue: Arc<MemoryPersistedQueueStore>,
    transaction_domain: Arc<TransactionDomain>,
    cluster_store: Arc<MemoryClusterMessageStore>,
    cluster_executor: Arc<provider_program::CpuClusterExecutor<MixedEnvironment>>,
    wall_clock: TestWallClock,
    monotonic_clock: TokioClock,
}

struct TokioClock {
    started: tokio::time::Instant,
}

impl Clock for TokioClock {
    fn now(&self) -> Duration { self.started.elapsed() }

    fn sleep(&self, duration: Duration, cancellation: Cancellation) -> ClockFuture {
        Box::pin(async move {
            tokio::select! {
                _ = tokio::time::sleep(duration) => Ok(ClockTick::elapsed()),
                _ = cancellation.cancelled() => Err(ClockError::Interrupted),
            }
        })
    }
}

impl HasClock for MixedEnvironment {
    type Clock = TokioClock;
    fn clock(&self) -> &Self::Clock { &self.monotonic_clock }
}

impl HasClusterMessageStore for MixedEnvironment {
    type Store = MemoryClusterMessageStore;
    fn cluster_message_store(&self) -> Arc<Self::Store> {
        Arc::clone(&self.cluster_store)
    }
}

impl HasClusterExecutor for MixedEnvironment {
    type Executor = provider_program::CpuClusterExecutor<Self>;
    fn cluster_executor(&self) -> Arc<Self::Executor> {
        Arc::clone(&self.cluster_executor)
    }
}

impl HasWallClock for MixedEnvironment {
    type Clock = TestWallClock;
    fn wall_clock(&self) -> &Self::Clock { &self.wall_clock }
}

impl HasTransactionDomain for MixedEnvironment {
    fn transaction_domain(&self) -> Arc<TransactionDomain> {
        Arc::clone(&self.transaction_domain)
    }
}

impl HasKeyValueStore for MixedEnvironment {
    type Store = ExternalKeyValueStore;
    fn key_value_store(&self) -> Arc<Self::Store> { Arc::clone(&self.store) }
}

impl HasPersistedQueueStore for MixedEnvironment {
    type Store = MemoryPersistedQueueStore;
    fn persisted_queue_store(&self) -> Arc<Self::Store> { Arc::clone(&self.queue) }
}

impl HasEventJournal for MixedEnvironment {
    type Journal = MemoryEventJournal;
    fn event_journal(&self) -> Arc<Self::Journal> { Arc::clone(&self.journal) }
}

impl HasWorkflowEngine for MixedEnvironment {
    type Engine = MemoryWorkflowEngine;
    fn workflow_engine(&self) -> Arc<Self::Engine> { Arc::clone(&self.engine) }
}

impl HasTaskSpawner for MixedEnvironment {
    type Tasks = TokioTasks;
    fn task_spawner(&self) -> &Self::Tasks { &self.tasks }
}

impl HasCpuExecutor for MixedEnvironment {
    type Executor = RayonCpu;
    fn cpu_executor(&self) -> &Self::Executor { &self.cpu }
}

impl HasBlockingExecutor for MixedEnvironment {
    type Executor = TokioBlocking;
    fn blocking_executor(&self) -> &Self::Executor { &self.blocking }
}

fn runtime() -> Runtime {
    Builder::new_multi_thread().worker_threads(2).enable_time().build()
        .expect("Tokio runtime builds")
}

fn main() {
    let mut arguments = std::env::args_os().skip(1);
    if let Some(flag) = arguments.next() {
        assert_eq!(flag, "--campaign-config", "unknown consumer argument");
        let path = arguments.next().expect("campaign configuration path");
        assert!(arguments.next().is_none(), "unexpected consumer argument");
        std::fs::read(path).expect("campaign configuration is readable");
    }
    let runtime = runtime();
    let handle = runtime.handle().clone();
    let capacity = NonZeroUsize::new(2).expect("two overlapping branches");
    let environment = Arc::new(MixedEnvironment {
        tasks: TokioTasks::new(handle.clone(), capacity),
        cpu: RayonCpu::new(capacity, capacity),
        blocking: TokioBlocking::new(handle, capacity),
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
        wall_clock: TestWallClock::new(UtcMillis::new(1)),
        monotonic_clock: TokioClock { started: tokio::time::Instant::now() },
    });
    let outcome = runtime.block_on(provider_program::run::<MixedEnvironment>()
        .run(Arc::clone(&environment)));
    assert!(matches!(outcome, Exit::Success(42)));
    let cancelled = runtime.block_on(provider_program::cancel_and_join::<MixedEnvironment>()
        .run(Arc::clone(&environment)));
    assert!(matches!(cancelled, Exit::Success(true)));
    let limits = provider_program::rpc_limits();
    let frame = provider_program::rpc_frame();
    let response = runtime.block_on(provider_program::rpc_cpu::<MixedEnvironment>(limits, frame)
        .run(Arc::clone(&environment)));
    let Exit::Success(response) = response else {
        panic!("mixed RPC CPU dispatch failed");
    };
    provider_program::assert_rpc_response(response, limits, frame);
    let event = runtime.block_on(provider_program::event_cpu::<MixedEnvironment>(
        provider_program::event_limits()).run(Arc::clone(&environment)));
    assert!(matches!(event,
        Exit::Success(EventRunOutcome::Committed { result: Ok(42), .. })));
    assert!(runtime.block_on(provider_program::rpc_event_notification_loss(
        Arc::clone(&environment))));
    assert!(runtime.block_on(provider_program::replica_observer_restart(
        Arc::clone(&environment))));
    let workflow = runtime.block_on(provider_program::workflow_cpu::<MixedEnvironment>()
        .run(Arc::clone(&environment)));
    assert!(matches!(workflow,
        Exit::Success(WorkflowRunResult::Completed(Ok(42)))));
    assert!(runtime.block_on(provider_program::queue_workflow_race(
        Arc::clone(&environment))));
    let schema = runtime.block_on(provider_program::schema_cpu::<MixedEnvironment>()
        .run(Arc::clone(&environment)));
    assert!(matches!(schema, Exit::Success(42)));
    let reactivity = runtime.block_on(provider_program::reactivity_cpu::<MixedEnvironment>()
        .run(Arc::clone(&environment)));
    assert!(matches!(reactivity, Exit::Success(true)));
    let persistence = runtime.block_on(provider_program::persistence_cpu::<MixedEnvironment>()
        .run(Arc::clone(&environment)));
    assert!(matches!(persistence, Exit::Success(StoredLookup::Found {
        outcome: StoredOutcome::Success(42), ..
    })));
    let reference = environment.transaction_domain.reference(41_i64)
        .expect("transaction fixture reference");
    let transaction = runtime.block_on(provider_program::transaction_cpu::<MixedEnvironment>(
        reference).run(Arc::clone(&environment)));
    assert!(matches!(transaction, Exit::Success(42)));
    let source = TxSubscriptionRef::new(&environment.transaction_domain, 41_i64)
        .expect("transactional reactive fixture source");
    let reactive = runtime.block_on(
        provider_program::transactional_reactive::<MixedEnvironment>(source)
            .run(Arc::clone(&environment)));
    assert!(matches!(reactive, Exit::Success(true)));
    let source = TxSubscriptionRef::new(&environment.transaction_domain, 41_i64)
        .expect("notification loss fixture source");
    let notification = runtime.block_on(
        provider_program::notification_loss_after_commit::<MixedEnvironment>(source)
            .run(Arc::clone(&environment)));
    assert!(matches!(notification, Exit::Success(true)));
    assert!(runtime.block_on(transactional_collections::run(
        Arc::clone(&environment))));
    let claim = runtime.block_on(provider_program::cluster_prepare(
        Arc::clone(&environment.cluster_store)).run(Arc::new(())));
    let Exit::Success(claim) = claim else {
        panic!("cluster fixture setup failed");
    };
    let deliveries = runtime.block_on(provider_program::cluster_cpu::<MixedEnvironment>(claim)
        .run(Arc::clone(&environment)));
    let Exit::Success(deliveries) = deliveries else {
        panic!("cluster fixture dispatch failed");
    };
    assert!(provider_program::completed_cluster_run(&deliveries));
    assert!(runtime.block_on(provider_program::entity_macro(
        Arc::clone(&environment))));
    assert!(runtime.block_on(cluster_workflow_takeover::run(
        Arc::clone(&environment))));
    let elapsed = runtime.block_on(provider_program::monotonic_timer::<MixedEnvironment>(
        Duration::from_millis(1)).run(Arc::clone(&environment)));
    assert!(matches!(elapsed, Exit::Success(duration) if !duration.is_zero()));
    runtime.block_on(environment.tasks.join());
    environment.cpu.join();
    runtime.block_on(environment.blocking.join());
    if let Some(path) = std::env::var_os("EFFECT_CAMPAIGN_REPORT") {
        std::fs::write(
            path,
            r#"{"campaign":"consumer_mixed","families":{"schema":1,"persistence":1,"rpc":1,"reactivity":1,"cluster":1,"workflow":1,"eventlog":1,"transaction":1},"bridges":{"INT01":1,"INT02":1,"INT03":1,"INT04":1,"INT05":1}}"#,
        )
        .expect("consumer coverage report is writable");
    }
}
