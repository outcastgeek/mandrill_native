use effect::cluster::{
    AcceptancePolicy, AcceptedRequest, ActivationId, ActivationOutcome, ClaimOptions,
    ClusterActivationStore, ClusterAddress, ClusterEntityId, ClusterEntityType, ClusterExecutor,
    ClusterMessageStore, ClusterOwner, ClusterRequestId, ClusterRunOutcome, ClusterRunner,
    ClusterRunnerError, ClusterShardId, ClusterStoreError, ClusterWorkflowEngine,
    ClusterWorkflowError, HasClusterExecutor, HasClusterMessageStore, MemoryClusterMessageStore,
    MessageAcceptance, MessageStoreCapacity, PayloadConflictPolicy, ReplyKind, SavedReply,
    StoreCompletion,
};
use effect::workflow::{
    HasWorkflowEngine, MemoryWorkflowEngine, WorkflowCommand, WorkflowDefinitionId, WorkflowEngine,
    WorkflowEngineError, WorkflowHandle, WorkflowRegistration, WorkflowReply, WorkflowRunId,
    WorkflowStatus, WorkflowVersion,
};
use effect::{
    Effect, Exit, HasCpuExecutor, HasTaskSpawner, HasWallClock, KeyDomain, KeyNamespace,
    KeySegment, StableKey, TestWallClock, UtcMillis,
};
use std::marker::PhantomData;
use std::num::{NonZeroU32, NonZeroUsize};
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};
use std::time::Duration;

struct Domain;

impl KeyDomain for Domain {
    const TAG: &'static str = "provider-cluster-workflow-takeover";
}

fn key(namespace: &KeyNamespace<Domain>, part: &'static str) -> StableKey<Domain> {
    StableKey::new(namespace, [KeySegment::Text(part.into())])
        .expect("valid cluster workflow identity")
}

struct Environment<R> {
    base: Arc<R>,
    store: Arc<MemoryClusterMessageStore>,
    executor: Arc<TakeoverExecutor<R>>,
    clock: TestWallClock,
}

impl<R: HasTaskSpawner> HasTaskSpawner for Environment<R> {
    type Tasks = R::Tasks;
    fn task_spawner(&self) -> &Self::Tasks {
        self.base.task_spawner()
    }
}

impl<R: HasCpuExecutor> HasCpuExecutor for Environment<R> {
    type Executor = R::Executor;
    fn cpu_executor(&self) -> &Self::Executor {
        self.base.cpu_executor()
    }
}

impl<R> HasWallClock for Environment<R> {
    type Clock = TestWallClock;
    fn wall_clock(&self) -> &Self::Clock {
        &self.clock
    }
}

impl<R> HasClusterMessageStore for Environment<R> {
    type Store = MemoryClusterMessageStore;
    fn cluster_message_store(&self) -> Arc<Self::Store> {
        Arc::clone(&self.store)
    }
}

impl<R> HasWorkflowEngine for Environment<R>
where
    R: HasWorkflowEngine<Engine = MemoryWorkflowEngine>,
{
    type Engine = MemoryWorkflowEngine;
    fn workflow_engine(&self) -> Arc<Self::Engine> {
        self.base.workflow_engine()
    }
}

impl<R> HasClusterExecutor for Environment<R>
where
    R: HasTaskSpawner
        + HasCpuExecutor
        + HasWorkflowEngine<Engine = MemoryWorkflowEngine>
        + Send
        + Sync
        + 'static,
{
    type Executor = TakeoverExecutor<R>;
    fn cluster_executor(&self) -> Arc<Self::Executor> {
        Arc::clone(&self.executor)
    }
}

#[derive(Debug)]
enum ExecutionError {
    Workflow(ClusterWorkflowError<WorkflowEngineError>),
    Store(ClusterStoreError),
    UnexpectedWorkflowReply,
    UnexpectedActivation,
}

impl std::fmt::Display for ExecutionError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Workflow(error) => write!(formatter, "workflow: {error:?}"),
            Self::Store(error) => write!(formatter, "cluster store: {error:?}"),
            Self::UnexpectedWorkflowReply => formatter.write_str("unexpected workflow reply"),
            Self::UnexpectedActivation => formatter.write_str("unexpected activation"),
        }
    }
}

struct TakeoverExecutor<R> {
    store: Arc<MemoryClusterMessageStore>,
    old_owner: ClusterOwner,
    replacement: ActivationId,
    handle: WorkflowHandle,
    first: AtomicBool,
    executions: Arc<AtomicUsize>,
    _environment: PhantomData<fn() -> R>,
}

fn finish_reply<R>(
    payload: Arc<[u8]>,
    executions: Arc<AtomicUsize>,
) -> Effect<SavedReply, ExecutionError, Environment<R>>
where
    R: HasCpuExecutor + Send + Sync + 'static,
{
    Effect::cpu_once(move |_| {
        executions.fetch_add(1, Ordering::AcqRel);
        SavedReply {
            kind: ReplyKind::Success,
            payload,
        }
    })
}

fn change_owner<R>(
    store: Arc<MemoryClusterMessageStore>,
    old_owner: ClusterOwner,
    address: ClusterAddress,
    replacement: ActivationId,
) -> Effect<(), ExecutionError, Environment<R>>
where
    R: Send + Sync + 'static,
{
    let activating = Arc::clone(&store);
    store
        .abandon(old_owner, UtcMillis::new(2))
        .map_error(ExecutionError::Store)
        .provide_some(|_: &Environment<R>| ())
        .flat_map(move |abandoned| {
            if !abandoned {
                return Effect::fail_with(|| ExecutionError::UnexpectedActivation);
            }
            activating
                .activate(
                    address.clone(),
                    replacement.clone(),
                    UtcMillis::new(2),
                    UtcMillis::new(4),
                )
                .map_error(ExecutionError::Store)
                .provide_some(|_: &Environment<R>| ())
                .flat_map(|activation| match activation {
                    ActivationOutcome::Activated { .. } => Effect::sync(|| ()),
                    _ => Effect::fail_with(|| ExecutionError::UnexpectedActivation),
                })
        })
}

impl<R> ClusterExecutor<Environment<R>> for TakeoverExecutor<R>
where
    R: HasTaskSpawner
        + HasCpuExecutor
        + HasWorkflowEngine<Engine = MemoryWorkflowEngine>
        + Send
        + Sync
        + 'static,
{
    type Error = ExecutionError;

    fn execute(
        &self,
        address: ClusterAddress,
        payload: Arc<[u8]>,
    ) -> Effect<SavedReply, Self::Error, Environment<R>> {
        let takeover = self.first.swap(false, Ordering::AcqRel);
        let store = Arc::clone(&self.store);
        let old_owner = self.old_owner.clone();
        let replacement = self.replacement.clone();
        let handle = self.handle.clone();
        let executions = Arc::clone(&self.executions);
        ClusterWorkflowEngine::dispatch::<Environment<R>>(WorkflowCommand::Start {
            handle,
            input: Arc::from(&b"workflow-input"[..]),
        })
        .map_error(ExecutionError::Workflow)
        .flat_map(move |started| {
            if !matches!(started, WorkflowReply::Snapshot(Some(snapshot))
                if snapshot.status == WorkflowStatus::Ready)
            {
                return Effect::fail_with(|| ExecutionError::UnexpectedWorkflowReply);
            }
            if takeover {
                let payload = Arc::clone(&payload);
                let executions = Arc::clone(&executions);
                change_owner::<R>(
                    Arc::clone(&store),
                    old_owner.clone(),
                    address.clone(),
                    replacement.clone(),
                )
                .flat_map(move |_| finish_reply::<R>(Arc::clone(&payload), Arc::clone(&executions)))
            } else {
                finish_reply::<R>(Arc::clone(&payload), Arc::clone(&executions))
            }
        })
    }
}

type RunnerError = ClusterRunnerError<ClusterStoreError, ExecutionError, std::convert::Infallible>;

pub async fn run<R>(base: Arc<R>) -> bool
where
    R: HasTaskSpawner
        + HasCpuExecutor
        + HasWorkflowEngine<Engine = MemoryWorkflowEngine>
        + Send
        + Sync
        + 'static,
{
    let namespace = KeyNamespace::<Domain>::new("handoff").expect("valid handoff namespace");
    let run = WorkflowRunId::from_stable(key(&namespace, "run"));
    let handle = WorkflowHandle {
        run: run.clone(),
        definition: WorkflowDefinitionId::new("provider-handoff-workflow")
            .expect("valid workflow definition"),
        version: WorkflowVersion::new(NonZeroU32::MIN),
    };
    let registration = WorkflowRegistration {
        id: handle.definition.clone(),
        version: handle.version,
        compatibility: Arc::from(&b"provider-v1"[..]),
    };
    if !matches!(
        base.workflow_engine()
            .register(registration)
            .run(Arc::new(()))
            .await,
        Exit::Success(())
    ) {
        return false;
    }
    let store = Arc::new(MemoryClusterMessageStore::new(
        MessageStoreCapacity::Bounded(NonZeroUsize::MIN),
    ));
    let address = ClusterAddress {
        entity_type: ClusterEntityType::new("provider-workflow").expect("valid entity type"),
        entity: ClusterEntityId::from_stable(key(&namespace, "run")),
        shard: ClusterShardId::from_stable(key(&namespace, "shard")),
    };
    let request = ClusterRequestId::from_stable(key(&namespace, "request"));
    let accepted = store
        .accept(AcceptedRequest {
            address: address.clone(),
            id: request.clone(),
            payload: Arc::from(&b"start"[..]),
            idempotency_key: None,
            accepted_at: UtcMillis::new(1),
            policy: AcceptancePolicy {
                dedup_retention: Duration::ZERO,
                reply_retention: Duration::ZERO,
                payload_conflict: PayloadConflictPolicy::Reject,
                maximum_attempts: NonZeroU32::new(2).expect("original and replacement claims"),
            },
        })
        .run(Arc::new(()))
        .await;
    if !matches!(accepted, Exit::Success(MessageAcceptance::Accepted { .. })) {
        return false;
    }
    let initial = store
        .activate(
            address.clone(),
            ActivationId::from_stable(key(&namespace, "old")),
            UtcMillis::new(1),
            UtcMillis::new(3),
        )
        .run(Arc::new(()))
        .await;
    let Exit::Success(ActivationOutcome::Activated { lease, .. }) = initial else {
        return false;
    };
    let executor = Arc::new(TakeoverExecutor {
        store: Arc::clone(&store),
        old_owner: lease.owner.clone(),
        replacement: ActivationId::from_stable(key(&namespace, "new")),
        handle,
        first: AtomicBool::new(true),
        executions: Arc::new(AtomicUsize::new(0)),
        _environment: PhantomData,
    });
    let environment = Arc::new(Environment {
        base,
        store: Arc::clone(&store),
        executor: Arc::clone(&executor),
        clock: TestWallClock::new(UtcMillis::new(1)),
    });
    let first = ClusterRunner::run_once::<
        Environment<R>,
        ClusterStoreError,
        ExecutionError,
        std::convert::Infallible,
    >(
        ClaimOptions {
            owner: lease.owner,
            now: UtcMillis::new(1),
            lease_until: UtcMillis::new(2),
            limit: NonZeroUsize::MIN,
        },
        NonZeroUsize::MIN,
    )
    .run(Arc::clone(&environment))
    .await;
    if !matches!(first, Exit::Success(outcomes) if matches!(outcomes.as_slice(),
    [Err(RunnerError::UncertainCompletion {
        source: ClusterStoreError::WrongOwner, ..
    })])) {
        return false;
    }
    let committed = environment
        .workflow_engine()
        .inspect(run.clone())
        .run(Arc::new(()))
        .await;
    if !matches!(committed, Exit::Success(Some(snapshot))
        if snapshot.status == WorkflowStatus::Ready)
    {
        return false;
    }
    let replacement = store
        .inspect_activation(address.clone())
        .run(Arc::new(()))
        .await;
    let Exit::Success(Some(status)) = replacement else {
        return false;
    };
    environment.clock.set(UtcMillis::new(2));
    let second = ClusterRunner::run_once::<
        Environment<R>,
        ClusterStoreError,
        ExecutionError,
        std::convert::Infallible,
    >(
        ClaimOptions {
            owner: status.lease.owner,
            now: UtcMillis::new(2),
            lease_until: UtcMillis::new(3),
            limit: NonZeroUsize::MIN,
        },
        NonZeroUsize::MIN,
    )
    .run(Arc::clone(&environment))
    .await;
    matches!(second, Exit::Success(outcomes) if matches!(outcomes.as_slice(),
    [Ok(ClusterRunOutcome {
        completion: StoreCompletion::Completed, ..
    })]))
        && executor.executions.load(Ordering::Acquire) == 2
}
