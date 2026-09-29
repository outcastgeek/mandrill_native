use super::{
    CancellationOutcome, ChildWait, DeadlineClear, DeadlineWait, EncodedWorkflowOutcome,
    RecordedActivity, RecordedChild, RecordedCompensation, RecordedDeadline, RecordedRace,
    RecordedSignal, SignalWait, WorkflowActivityId, WorkflowCompensationId, WorkflowHandle,
    WorkflowRaceId, WorkflowRunId, WorkflowSignalId, WorkflowSnapshot, WorkflowStatus,
    WorkflowVersion,
};
use crate::cluster::{
    AcceptancePolicy, ClusterClientError, ClusterMessageStore, ClusterObserverId, ClusterPoll,
    ClusterRequestId, EntityProxy, HasClusterMessageStore, MessageAcceptance, StoreMutation,
};
use crate::rpc::RpcCallId;
use crate::{Effect, UtcMillis};
use std::num::NonZeroU32;
use std::sync::Arc;

/// Stable, typed workflow control protocol. It is transport-neutral and can
/// be paired with application-provided RPC codecs and a Cluster entity.
pub enum WorkflowCommand {
    Start {
        handle: WorkflowHandle,
        input: Arc<[u8]>,
    },
    Activity {
        run: WorkflowRunId,
        record: RecordedActivity,
    },
    Child {
        parent: WorkflowRunId,
        record: RecordedChild,
    },
    WaitChild {
        parent: WorkflowRunId,
        child: WorkflowRunId,
    },
    InspectActivity {
        run: WorkflowRunId,
        activity: WorkflowActivityId,
        attempt: NonZeroU32,
    },
    Signal {
        run: WorkflowRunId,
        record: RecordedSignal,
    },
    InspectSignal {
        run: WorkflowRunId,
        signal: WorkflowSignalId,
        version: WorkflowVersion,
    },
    WaitSignal {
        run: WorkflowRunId,
        signal: WorkflowSignalId,
        version: WorkflowVersion,
    },
    Resume {
        run: WorkflowRunId,
    },
    Transition {
        run: WorkflowRunId,
        expected: WorkflowStatus,
        next: WorkflowStatus,
    },
    Cancel {
        run: WorkflowRunId,
    },
    ForceInterrupt {
        run: WorkflowRunId,
    },
    Deadline {
        run: WorkflowRunId,
        deadline: RecordedDeadline,
    },
    ClearDeadline {
        run: WorkflowRunId,
        expected: RecordedDeadline,
    },
    WaitDeadline {
        run: WorkflowRunId,
        deadline: RecordedDeadline,
        now: UtcMillis,
    },
    /// Scheduler reconciliation for one addressed run. The engine's recorded
    /// deadline remains the authority; `now` is an explicit wall-clock input.
    WakeDue {
        run: WorkflowRunId,
        expected: RecordedDeadline,
        now: UtcMillis,
    },
    RegisterCompensation {
        run: WorkflowRunId,
        identity: WorkflowCompensationId,
        order: NonZeroU32,
    },
    FinishCompensation {
        run: WorkflowRunId,
        identity: WorkflowCompensationId,
        outcome: EncodedWorkflowOutcome,
    },
    PendingCompensations {
        run: WorkflowRunId,
    },
    InspectRace {
        run: WorkflowRunId,
        identity: WorkflowRaceId,
    },
    RecordRace {
        run: WorkflowRunId,
        record: RecordedRace,
    },
    Inspect {
        run: WorkflowRunId,
    },
    Complete {
        run: WorkflowRunId,
        outcome: EncodedWorkflowOutcome,
    },
}

impl WorkflowCommand {
    #[must_use]
    pub fn run(&self) -> &WorkflowRunId {
        match self {
            Self::Start { handle, .. } => &handle.run,
            Self::Activity { run, .. }
            | Self::Signal { run, .. }
            | Self::Child { parent: run, .. }
            | Self::WaitChild { parent: run, .. }
            | Self::InspectActivity { run, .. }
            | Self::InspectSignal { run, .. }
            | Self::WaitSignal { run, .. }
            | Self::Resume { run }
            | Self::Cancel { run }
            | Self::Transition { run, .. }
            | Self::ForceInterrupt { run }
            | Self::Deadline { run, .. }
            | Self::ClearDeadline { run, .. }
            | Self::WaitDeadline { run, .. }
            | Self::WakeDue { run, .. }
            | Self::RegisterCompensation { run, .. }
            | Self::FinishCompensation { run, .. }
            | Self::PendingCompensations { run }
            | Self::InspectRace { run, .. }
            | Self::RecordRace { run, .. }
            | Self::Inspect { run }
            | Self::Complete { run, .. } => run,
        }
    }
}

pub enum WorkflowReply {
    Accepted,
    Snapshot(Option<Box<WorkflowSnapshot>>),
    Activity(Option<RecordedActivity>),
    Signal(Option<RecordedSignal>),
    SignalWait(SignalWait),
    ChildWait(ChildWait),
    DeadlineWait(DeadlineWait),
    DeadlineClear(Box<DeadlineClear>),
    Compensation(RecordedCompensation),
    Compensations(Vec<RecordedCompensation>),
    Race(Option<RecordedRace>),
    Cancellation(CancellationOutcome),
}

impl WorkflowReply {
    /// Keep the common control replies small while allocating only when a
    /// complete workflow snapshot is actually returned.
    pub fn snapshot(snapshot: Option<WorkflowSnapshot>) -> Self {
        Self::Snapshot(snapshot.map(Box::new))
    }
}

#[derive(Debug)]
pub enum WorkflowProxyError<Backend> {
    EntityIdentityMismatch,
    Cluster(ClusterClientError<Backend>),
}

pub struct WorkflowProxy<DomainError, Backend>
where
    DomainError: Send + 'static,
    Backend: Send + 'static,
{
    entity: EntityProxy<WorkflowCommand, WorkflowReply, DomainError, Backend>,
}

impl<DomainError, Backend> WorkflowProxy<DomainError, Backend>
where
    DomainError: Send + 'static,
    Backend: Send + 'static,
{
    #[must_use]
    pub fn new(entity: EntityProxy<WorkflowCommand, WorkflowReply, DomainError, Backend>) -> Self {
        Self { entity }
    }

    pub fn submit<R>(
        &self,
        request: ClusterRequestId,
        command: WorkflowCommand,
        call: RpcCallId,
        accepted_at: UtcMillis,
        acceptance: AcceptancePolicy,
    ) -> Effect<MessageAcceptance, WorkflowProxyError<Backend>, R>
    where
        R: HasClusterMessageStore + Send + Sync + 'static,
        R::Store: ClusterMessageStore<Error = Backend>,
    {
        if self.entity.address().entity.as_bytes() != command.run().as_bytes() {
            return Effect::fail_with(|| WorkflowProxyError::EntityIdentityMismatch);
        }
        self.entity
            .submit(request, command, call, accepted_at, acceptance)
            .map_error(WorkflowProxyError::Cluster)
    }

    pub fn poll<R>(
        &self,
        request: ClusterRequestId,
        observer: ClusterObserverId,
        call: RpcCallId,
    ) -> Effect<ClusterPoll<WorkflowReply, DomainError>, WorkflowProxyError<Backend>, R>
    where
        R: HasClusterMessageStore + Send + Sync + 'static,
        R::Store: ClusterMessageStore<Error = Backend>,
    {
        self.entity
            .poll(request, observer, call)
            .map_error(WorkflowProxyError::Cluster)
    }

    pub fn acknowledge_reply<R>(
        &self,
        request: ClusterRequestId,
        observer: ClusterObserverId,
    ) -> Effect<StoreMutation, WorkflowProxyError<Backend>, R>
    where
        R: HasClusterMessageStore + Send + Sync + 'static,
        R::Store: ClusterMessageStore<Error = Backend>,
    {
        self.entity
            .acknowledge_reply(request, observer)
            .map_error(WorkflowProxyError::Cluster)
    }
}
