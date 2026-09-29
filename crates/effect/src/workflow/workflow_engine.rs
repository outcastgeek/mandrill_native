use super::{
    EncodedWorkflowOutcome, RecordedActivity, RecordedChild, RecordedCompensation,
    RecordedDeadline, RecordedRace, RecordedSignal, WorkflowActivityId, WorkflowCompensationId,
    WorkflowHandle, WorkflowRaceId, WorkflowRegistration, WorkflowRunId, WorkflowSignalId,
    WorkflowSnapshot, WorkflowStatus, WorkflowVersion,
};
use crate::{Effect, UtcMillis};
use std::num::NonZeroU32;
use std::sync::Arc;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WorkflowEngineError {
    DefinitionConflict,
    RunConflict,
    MissingRun,
    DefinitionMismatch,
    TerminalRun,
    ActivityConflict,
    ActivityAttemptConflict,
    SignalConflict,
    CompensationConflict,
    RaceConflict,
    RaceWinnerMissing,
    ChildConflict,
    StateConflict,
    InputConsumed,
    InvalidTransition,
    EngineShutdown,
    MigrationConflict,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CancellationOutcome {
    Cancelled,
    AlreadyCancelled,
    Active,
    AlreadyCompleted,
}

pub enum SignalWait {
    Ready(RecordedSignal),
    Waiting,
}

pub enum ChildWait {
    Terminal(WorkflowStatus),
    Waiting,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeadlineWait {
    Due,
    Waiting,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DeadlineWake {
    Woken(WorkflowSnapshot),
    Unchanged(WorkflowSnapshot),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DeadlineClear {
    Cleared(WorkflowSnapshot),
    Unchanged(WorkflowSnapshot),
}

/// A runtime-neutral authority for execution records. All operations return
/// Effects, so the concrete engine and its runtime services live only in R.
/// The memory implementation is useful without Cluster or a database.
pub trait WorkflowEngine: Send + Sync + 'static {
    type Error: Send + 'static;

    fn register(&self, definition: WorkflowRegistration) -> Effect<(), Self::Error>;
    fn start(
        &self,
        handle: WorkflowHandle,
        encoded_input: Arc<[u8]>,
    ) -> Effect<WorkflowSnapshot, Self::Error>;
    fn inspect(&self, run: WorkflowRunId) -> Effect<Option<WorkflowSnapshot>, Self::Error>;
    fn transition(
        &self,
        run: WorkflowRunId,
        expected: WorkflowStatus,
        next: WorkflowStatus,
    ) -> Effect<WorkflowSnapshot, Self::Error>;
    fn record_activity(
        &self,
        run: WorkflowRunId,
        activity: WorkflowActivityId,
        attempt: NonZeroU32,
        encoded_input: Arc<[u8]>,
        outcome: EncodedWorkflowOutcome,
    ) -> Effect<RecordedActivity, Self::Error>;
    fn inspect_activity(
        &self,
        run: WorkflowRunId,
        activity: WorkflowActivityId,
        attempt: NonZeroU32,
    ) -> Effect<Option<RecordedActivity>, Self::Error>;
    fn link_child(
        &self,
        parent: WorkflowRunId,
        child: RecordedChild,
    ) -> Effect<RecordedChild, Self::Error>;
    fn wait_child(
        &self,
        parent: WorkflowRunId,
        child: WorkflowRunId,
    ) -> Effect<ChildWait, Self::Error>;
    fn set_deadline(
        &self,
        run: WorkflowRunId,
        deadline: RecordedDeadline,
    ) -> Effect<WorkflowSnapshot, Self::Error>;
    /// Clear only the exact named occurrence that was scheduled. A stale
    /// cancellation cannot remove a rescheduled deadline.
    fn clear_deadline(
        &self,
        run: WorkflowRunId,
        expected: RecordedDeadline,
    ) -> Effect<DeadlineClear, Self::Error>;
    fn wake_due(&self, now: UtcMillis) -> Effect<Vec<WorkflowRunId>, Self::Error>;
    /// Atomically wake one run only if it still waits on this exact deadline.
    /// A stale scheduler command cannot undo a reschedule or cancellation.
    fn wake_deadline(
        &self,
        run: WorkflowRunId,
        expected: RecordedDeadline,
        now: UtcMillis,
    ) -> Effect<DeadlineWake, Self::Error>;
    fn wait_deadline(
        &self,
        run: WorkflowRunId,
        deadline: RecordedDeadline,
        now: UtcMillis,
    ) -> Effect<DeadlineWait, Self::Error>;
    fn deliver_signal(
        &self,
        run: WorkflowRunId,
        signal: RecordedSignal,
    ) -> Effect<RecordedSignal, Self::Error>;
    fn inspect_signal(
        &self,
        run: WorkflowRunId,
        signal: WorkflowSignalId,
        version: WorkflowVersion,
    ) -> Effect<Option<RecordedSignal>, Self::Error>;
    fn wait_signal(
        &self,
        run: WorkflowRunId,
        signal: WorkflowSignalId,
        version: WorkflowVersion,
    ) -> Effect<SignalWait, Self::Error>;
    fn register_compensation(
        &self,
        run: WorkflowRunId,
        identity: WorkflowCompensationId,
        order: NonZeroU32,
    ) -> Effect<RecordedCompensation, Self::Error>;
    fn finish_compensation(
        &self,
        run: WorkflowRunId,
        identity: WorkflowCompensationId,
        outcome: EncodedWorkflowOutcome,
    ) -> Effect<RecordedCompensation, Self::Error>;
    fn pending_compensations(
        &self,
        run: WorkflowRunId,
    ) -> Effect<Vec<RecordedCompensation>, Self::Error>;
    fn inspect_race(
        &self,
        run: WorkflowRunId,
        race: WorkflowRaceId,
    ) -> Effect<Option<RecordedRace>, Self::Error>;
    fn record_race(
        &self,
        run: WorkflowRunId,
        race: RecordedRace,
    ) -> Effect<RecordedRace, Self::Error>;

    fn complete(
        &self,
        run: WorkflowRunId,
        outcome: EncodedWorkflowOutcome,
    ) -> Effect<WorkflowSnapshot, Self::Error>;
    fn cancel(&self, run: WorkflowRunId) -> Effect<CancellationOutcome, Self::Error>;
    /// Force terminal cancellation even while code is running. The external
    /// side effect may still finish; its outcome is uncertain to this engine.
    fn force_interrupt(&self, run: WorkflowRunId) -> Effect<CancellationOutcome, Self::Error>;
}

pub trait HasWorkflowEngine {
    type Engine: WorkflowEngine;
    fn workflow_engine(&self) -> Arc<Self::Engine>;
}

/// A provider warrants that acknowledged workflow records survive its
/// documented restart boundary and can be recovered by a new engine instance.
/// The process-local memory engine does not implement this capability.
pub trait DurableWorkflowEngine: WorkflowEngine {}
