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
use effect::{
    effect, effect_cpu, effect_map_error, effect_provide_some, effect_sync, effect_try_sync,
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

fn key(
    namespace: &KeyNamespace<Domain>,
    part: &'static str,
) -> Result<StableKey<Domain>, effect::KeyError> {
    StableKey::new(namespace, [KeySegment::Text(part.into())])
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
    effect_cpu! { once; body: move |_| {
        executions.fetch_add(1, Ordering::AcqRel);
        SavedReply {
            kind: ReplyKind::Success,
            payload,
        }
    }, }
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
    effect! {
    bind;
    program: effect_provide_some! { program: effect_map_error! { program: store
           .abandon(old_owner, UtcMillis::new(2)), map: ExecutionError::Store, }, project: |_: &Environment<R>| (), },
    body: move |abandoned| {
               if !abandoned {
                   return effect_try_sync! { body: { let __dsl_error = || ExecutionError::UnexpectedActivation; move || Err(__dsl_error()) }, on_error: ::core::convert::identity, };
               }
               effect! {
    bind;
    program: effect_provide_some! { program: effect_map_error! { program: activating
                   .activate(
                       address.clone(),
                       replacement.clone(),
                       UtcMillis::new(2),
                       UtcMillis::new(4),
                   ), map: ExecutionError::Store, }, project: |_: &Environment<R>| (), },
    body: |activation| {
                       if matches!(activation, ActivationOutcome::Activated { .. }) {
                           effect_sync! { body: || (), }
                       } else {
                           effect_try_sync! { body: { let __dsl_error = || ExecutionError::UnexpectedActivation; move || Err(__dsl_error()) }, on_error: ::core::convert::identity, }
                       }
                   },
    }
           },
    }
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
        effect! {
        bind;
        program: effect_map_error! { program: ClusterWorkflowEngine::dispatch::<Environment<R>>(WorkflowCommand::Start {
                   handle,
                   input: Arc::from(&b"workflow-input"[..]),
               }), map: ExecutionError::Workflow, },
        body: move |started| {
                   if !matches!(started, WorkflowReply::Snapshot(Some(snapshot))
                       if snapshot.status == WorkflowStatus::Ready)
                   {
                       return effect_try_sync! { body: { let __dsl_error = || ExecutionError::UnexpectedWorkflowReply; move || Err(__dsl_error()) }, on_error: ::core::convert::identity, };
                   }
                   if takeover {
                       let payload = Arc::clone(&payload);
                       let executions = Arc::clone(&executions);
                       effect! {
        bind;
        program: change_owner::<R>(
                           Arc::clone(&store),
                           old_owner.clone(),
                           address.clone(),
                           replacement.clone(),
                       ),
        body: move |_| finish_reply::<R>(Arc::clone(&payload), Arc::clone(&executions)),
        }
                   } else {
                       finish_reply::<R>(Arc::clone(&payload), Arc::clone(&executions))
                   }
               },
        }
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
    let Ok(namespace) = KeyNamespace::<Domain>::new("handoff") else {
        return false;
    };
    let Ok(run_key) = key(&namespace, "run") else {
        return false;
    };
    let Ok(shard_key) = key(&namespace, "shard") else {
        return false;
    };
    let Ok(request_key) = key(&namespace, "request") else {
        return false;
    };
    let Ok(old_key) = key(&namespace, "old") else {
        return false;
    };
    let Ok(new_key) = key(&namespace, "new") else {
        return false;
    };
    let Ok(definition) = WorkflowDefinitionId::new("provider-handoff-workflow") else {
        return false;
    };
    let Ok(entity_type) = ClusterEntityType::new("provider-workflow") else {
        return false;
    };
    let run = WorkflowRunId::from_stable(run_key.clone());
    let handle = WorkflowHandle {
        run: run.clone(),
        definition,
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
        entity_type,
        entity: ClusterEntityId::from_stable(run_key),
        shard: ClusterShardId::from_stable(shard_key),
    };
    let request = ClusterRequestId::from_stable(request_key);
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
                maximum_attempts: NonZeroU32::MIN.saturating_add(1),
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
            ActivationId::from_stable(old_key),
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
        replacement: ActivationId::from_stable(new_key),
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
