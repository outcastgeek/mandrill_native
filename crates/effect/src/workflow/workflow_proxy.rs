//! Typed workflow control protocol derivation.

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
use crate::{effect_map_error, effect_try_sync};
use std::num::NonZeroU32;
use std::sync::Arc;

/// Stable, typed workflow control protocol. It is transport-neutral and can
/// be paired with application-provided RPC codecs and a Cluster entity.
pub enum WorkflowCommand {
    /// Begin the selected workflow or operation.
    Start {
        /// Handle used to observe or settle this operation. Type: `WorkflowHandle`.
        handle: WorkflowHandle,
        /// The input supplied to this operation. Type: `Arc<[u8]>`.
        input: Arc<[u8]>,
    },
    /// The workflow is waiting for or executing a named activity.
    Activity {
        /// Stable identity of the workflow run. Type: `WorkflowRunId`.
        run: WorkflowRunId,
        /// Activity record returned by the workflow engine. Type: `RecordedActivity`.
        record: RecordedActivity,
    },
    /// Execution is waiting for a child workflow.
    Child {
        /// Identity of the parent operation or span. Type: `WorkflowRunId`.
        parent: WorkflowRunId,
        /// Child workflow record returned by the engine. Type: `RecordedChild`.
        record: RecordedChild,
    },
    /// Suspend until a child workflow settles.
    WaitChild {
        /// Identity of the parent operation or span. Type: `WorkflowRunId`.
        parent: WorkflowRunId,
        /// Stable identity of the child workflow run. Type: `WorkflowRunId`.
        child: WorkflowRunId,
    },
    /// Read the recorded state of a named activity.
    InspectActivity {
        /// Stable identity of the workflow run. Type: `WorkflowRunId`.
        run: WorkflowRunId,
        /// Identity of the registered workflow activity. Type: `WorkflowActivityId`.
        activity: WorkflowActivityId,
        /// Number of the current delivery or execution attempt. Type: `NonZeroU32`.
        attempt: NonZeroU32,
    },
    /// The workflow is waiting for or recording a named signal.
    Signal {
        /// Stable identity of the workflow run. Type: `WorkflowRunId`.
        run: WorkflowRunId,
        /// Signal record returned by the workflow engine. Type: `RecordedSignal`.
        record: RecordedSignal,
    },
    /// Read the recorded state of a named signal.
    InspectSignal {
        /// Stable identity of the workflow run. Type: `WorkflowRunId`.
        run: WorkflowRunId,
        /// Identity of the workflow signal. Type: `WorkflowSignalId`.
        signal: WorkflowSignalId,
        /// The declared compatibility version. Type: `WorkflowVersion`.
        version: WorkflowVersion,
    },
    /// Suspend until a named signal is recorded.
    WaitSignal {
        /// Stable identity of the workflow run. Type: `WorkflowRunId`.
        run: WorkflowRunId,
        /// Identity of the workflow signal. Type: `WorkflowSignalId`.
        signal: WorkflowSignalId,
        /// The declared compatibility version. Type: `WorkflowVersion`.
        version: WorkflowVersion,
    },
    /// Resume a suspended run or retry a previously failed exact reset.
    ///
    /// Some(checkpoint) carries the failed reset fence; None permits ordinary Suspended
    /// resume and cannot reset a Running body. Completed runs acknowledge recorded
    /// state. A stale fence cannot reset a replacement execution.
    Resume {
        /// Stable identity of the workflow run. Type: `WorkflowRunId`.
        run: WorkflowRunId,
        /// Exact failed-reset ownership, or None for ordinary suspended resume.
        checkpoint: Option<super::WorkflowCheckpoint>,
    },
    /// Apply the selected state transition.
    Transition {
        /// Stable identity of the workflow run. Type: `WorkflowRunId`.
        run: WorkflowRunId,
        /// Status and execution counter that must both still match; not a status-only comparison.
        expected: super::WorkflowCheckpoint,
        /// Next position after the current record. Type: `WorkflowStatus`.
        next: WorkflowStatus,
    },
    /// Cancel the selected work under its ownership rules.
    Cancel {
        /// Stable identity of the workflow run. Type: `WorkflowRunId`.
        run: WorkflowRunId,
    },
    /// Request interruption even when work is currently active.
    ForceInterrupt {
        /// Stable identity of the workflow run. Type: `WorkflowRunId`.
        run: WorkflowRunId,
    },
    /// Execution is waiting for a recorded deadline.
    Deadline {
        /// Stable identity of the workflow run. Type: `WorkflowRunId`.
        run: WorkflowRunId,
        /// The absolute deadline for this operation. Type: `RecordedDeadline`.
        deadline: RecordedDeadline,
    },
    /// Remove a recorded deadline wait.
    ClearDeadline {
        /// Stable identity of the workflow run. Type: `WorkflowRunId`.
        run: WorkflowRunId,
        /// The value required by the requested condition. Type: `RecordedDeadline`.
        expected: RecordedDeadline,
    },
    /// Suspend until the recorded deadline fires.
    WaitDeadline {
        /// Stable identity of the workflow run. Type: `WorkflowRunId`.
        run: WorkflowRunId,
        /// The absolute deadline for this operation. Type: `RecordedDeadline`.
        deadline: RecordedDeadline,
        /// Caller-supplied current instant used for the decision. Type: `UtcMillis`.
        now: UtcMillis,
    },
    /// Scheduler reconciliation for one addressed run. The engine's recorded
    /// deadline remains the authority; `now` is an explicit wall-clock input.
    WakeDue {
        /// Stable identity of the workflow run. Type: `WorkflowRunId`.
        run: WorkflowRunId,
        /// The value required by the requested condition. Type: `RecordedDeadline`.
        expected: RecordedDeadline,
        /// Caller-supplied current instant used for the decision. Type: `UtcMillis`.
        now: UtcMillis,
    },
    /// Record a recovery action before continuing.
    RegisterCompensation {
        /// Stable identity of the workflow run. Type: `WorkflowRunId`.
        run: WorkflowRunId,
        /// Stable identity used for deduplication or lookup. Type: `WorkflowCompensationId`.
        identity: WorkflowCompensationId,
        /// Stable acceptance or delivery order. Type: `NonZeroU32`.
        order: NonZeroU32,
    },
    /// Record the terminal result of a recovery action.
    FinishCompensation {
        /// Stable identity of the workflow run. Type: `WorkflowRunId`.
        run: WorkflowRunId,
        /// Stable identity used for deduplication or lookup. Type: `WorkflowCompensationId`.
        identity: WorkflowCompensationId,
        /// The recorded outcome of this operation. Type: `EncodedWorkflowOutcome`.
        outcome: EncodedWorkflowOutcome,
    },
    /// Registered recovery actions have not all settled.
    PendingCompensations {
        /// Stable identity of the workflow run. Type: `WorkflowRunId`.
        run: WorkflowRunId,
    },
    /// Read the recorded winner of a durable race.
    InspectRace {
        /// Stable identity of the workflow run. Type: `WorkflowRunId`.
        run: WorkflowRunId,
        /// Stable identity used for deduplication or lookup. Type: `WorkflowRaceId`.
        identity: WorkflowRaceId,
    },
    /// Persist the selected race winner for replay.
    RecordRace {
        /// Stable identity of the workflow run. Type: `WorkflowRunId`.
        run: WorkflowRunId,
        /// Race decision recorded for replay. Type: `RecordedRace`.
        record: RecordedRace,
    },
    /// Return recorded state without changing it.
    Inspect {
        /// Stable identity of the workflow run. Type: `WorkflowRunId`.
        run: WorkflowRunId,
    },
    /// The operation has a recorded terminal result.
    Complete {
        /// Stable identity of the workflow run. Type: `WorkflowRunId`.
        run: WorkflowRunId,
        /// The recorded outcome of this operation. Type: `EncodedWorkflowOutcome`.
        outcome: EncodedWorkflowOutcome,
    },
}

impl WorkflowCommand {
    #[must_use]
    /// Execute or describe one run using the supplied environment and inputs.
    ///
    /// # Returns
    /// Type: `&WorkflowRunId`.
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
            | Self::Resume { run, .. }
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

/// The typed result returned for a workflow command addressed through a proxy.
pub enum WorkflowReply {
    /// The operation was accepted for processing.
    Accepted,
    /// The state was reconstructed from a recorded snapshot. Contains `Option<Box<WorkflowSnapshot>>`.
    Snapshot(Option<Box<WorkflowSnapshot>>),
    /// The workflow is waiting for or executing a named activity. Contains `Option<RecordedActivity>`.
    Activity(Option<RecordedActivity>),
    /// The workflow is waiting for or recording a named signal. Contains `Option<RecordedSignal>`.
    Signal(Option<RecordedSignal>),
    /// Suspend until the named signal is recorded. Contains `SignalWait`.
    SignalWait(SignalWait),
    /// The workflow is suspended until a child run settles. Contains `ChildWait`.
    ChildWait(ChildWait),
    /// Suspend the run until its recorded deadline. Contains `DeadlineWait`.
    DeadlineWait(DeadlineWait),
    /// Clear a deadline previously registered with the engine. Contains `Box<DeadlineClear>`.
    DeadlineClear(Box<DeadlineClear>),
    /// Run the registered recovery action. Contains `RecordedCompensation`.
    Compensation(RecordedCompensation),
    /// One or more recovery actions remain recorded. Contains `Vec<RecordedCompensation>`.
    Compensations(Vec<RecordedCompensation>),
    /// The workflow is coordinating a durable race. Contains `Option<RecordedRace>`.
    Race(Option<RecordedRace>),
    /// The operation ended because cancellation was requested. Contains `CancellationOutcome`.
    Cancellation(CancellationOutcome),
}

impl WorkflowReply {
    /// Keep the common control replies small while allocating only when a
    /// complete workflow snapshot is actually returned.
    ///
    /// # Parameters
    /// - `snapshot` (`Option<WorkflowSnapshot>`): The snapshot supplied to this operation.
    ///
    /// # Returns
    /// Type: `Self`.
    pub fn snapshot(snapshot: Option<WorkflowSnapshot>) -> Self {
        Self::Snapshot(snapshot.map(Box::new))
    }
}

#[derive(Debug)]
/// A typed failure encoding, dispatching, or decoding a proxied workflow command.
///
/// # Type parameters
/// - `Backend`: Backend-specific error or provider type.
pub enum WorkflowProxyError<Backend> {
    /// The entity identity differs from the required or recorded value.
    EntityIdentityMismatch,
    /// Cluster delivery or ownership coordination failed. Retains the `ClusterClientError<Backend>` detail.
    Cluster(ClusterClientError<Backend>),
}

/// A client façade for sending typed workflow commands through an addressed transport.
///
/// # Type parameters
/// - `DomainError`: Expected domain-error type.
/// - `Backend`: Backend-specific error or provider type.
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
    /// Construct a `WorkflowProxy` from its required inputs and explicit policies.
    ///
    /// # Type parameters
    /// - `DomainError`: Expected domain-error type.
    /// - `Backend`: Backend-specific error or provider type.
    ///
    /// # Parameters
    /// - `entity` (`EntityProxy<WorkflowCommand, WorkflowReply, DomainError, Backend>`): The entity supplied to this operation.
    ///
    /// # Returns
    /// Type: `Self`.
    pub fn new(entity: EntityProxy<WorkflowCommand, WorkflowReply, DomainError, Backend>) -> Self {
        Self { entity }
    }

    /// Transfer accepted work to the selected provider for independent execution.
    ///
    /// # Type parameters
    /// - `DomainError`: Expected domain-error type.
    /// - `Backend`: Backend-specific error or provider type.
    /// - `R`: Environment type. Additional bound: `HasClusterMessageStore + Send + Sync + 'static`. Additional bound: `:Store: ClusterMessageStore<Error = Backend>`.
    ///
    /// # Parameters
    /// - `request` (`ClusterRequestId`): The typed request to process.
    /// - `command` (`WorkflowCommand`): The command supplied to this operation.
    /// - `call` (`RpcCallId`): Identity and contents of the selected RPC call.
    /// - `accepted_at` (`UtcMillis`): The accepted at supplied to this operation.
    /// - `acceptance` (`AcceptancePolicy`): The acceptance supplied to this operation.
    ///
    /// # Returns
    /// Type: `Effect<MessageAcceptance, WorkflowProxyError<Backend>, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
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
            return effect_try_sync! { body: { let __dsl_error = || WorkflowProxyError::EntityIdentityMismatch; move || Err(__dsl_error()) }, on_error: ::core::convert::identity, };
        }
        effect_map_error! { program: self.entity
        .submit(request, command, call, accepted_at, acceptance), map: WorkflowProxyError::Cluster, }
    }

    /// Inspect the current state without waiting for a later change.
    ///
    /// # Type parameters
    /// - `DomainError`: Expected domain-error type.
    /// - `Backend`: Backend-specific error or provider type.
    /// - `R`: Environment type. Additional bound: `HasClusterMessageStore + Send + Sync + 'static`. Additional bound: `:Store: ClusterMessageStore<Error = Backend>`.
    ///
    /// # Parameters
    /// - `request` (`ClusterRequestId`): The typed request to process.
    /// - `observer` (`ClusterObserverId`): Stable identity of the client observing recorded results.
    /// - `call` (`RpcCallId`): Identity and contents of the selected RPC call.
    ///
    /// # Returns
    /// Type: `Effect<ClusterPoll<WorkflowReply, DomainError>, WorkflowProxyError<Backend>, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
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
        effect_map_error! { program: self.entity
        .poll(request, observer, call), map: WorkflowProxyError::Cluster, }
    }

    /// Record that the selected observer has consumed the reply.
    ///
    /// # Type parameters
    /// - `DomainError`: Expected domain-error type.
    /// - `Backend`: Backend-specific error or provider type.
    /// - `R`: Environment type. Additional bound: `HasClusterMessageStore + Send + Sync + 'static`. Additional bound: `:Store: ClusterMessageStore<Error = Backend>`.
    ///
    /// # Parameters
    /// - `request` (`ClusterRequestId`): The typed request to process.
    /// - `observer` (`ClusterObserverId`): Stable identity of the client observing recorded results.
    ///
    /// # Returns
    /// Type: `Effect<StoreMutation, WorkflowProxyError<Backend>, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    pub fn acknowledge_reply<R>(
        &self,
        request: ClusterRequestId,
        observer: ClusterObserverId,
    ) -> Effect<StoreMutation, WorkflowProxyError<Backend>, R>
    where
        R: HasClusterMessageStore + Send + Sync + 'static,
        R::Store: ClusterMessageStore<Error = Backend>,
    {
        effect_map_error! { program: self.entity
        .acknowledge_reply(request, observer), map: WorkflowProxyError::Cluster, }
    }
}
