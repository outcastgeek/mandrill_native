//! Workflow engine control and storage contracts.

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
/// A typed rejection or storage failure in the workflow engine contract.
pub enum WorkflowEngineError {
    /// An incompatible definition is already recorded.
    DefinitionConflict,
    /// An incompatible run is already recorded.
    RunConflict,
    /// The required run is missing.
    MissingRun,
    /// The definition differs from the required or recorded value.
    DefinitionMismatch,
    /// The workflow run already has a terminal outcome.
    TerminalRun,
    /// An incompatible activity is already recorded.
    ActivityConflict,
    /// An incompatible activity attempt is already recorded.
    ActivityAttemptConflict,
    /// An incompatible signal is already recorded.
    SignalConflict,
    /// An incompatible compensation is already recorded.
    CompensationConflict,
    /// An incompatible race is already recorded.
    RaceConflict,
    /// No winning branch was recorded for this durable race.
    RaceWinnerMissing,
    /// An incompatible child is already recorded.
    ChildConflict,
    /// No further execution owner can be allocated.
    ExecutionExhausted,
    /// An incompatible state is already recorded.
    StateConflict,
    /// An owned input was already consumed.
    InputConsumed,
    /// The transition failed validation.
    InvalidTransition,
    /// The workflow engine has stopped admitting new work.
    EngineShutdown,
    /// An incompatible migration is already recorded.
    MigrationConflict,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// The engine decision after a request to cancel a workflow run.
pub enum CancellationOutcome {
    /// Execution was cancelled.
    Cancelled,
    /// A cancellation request was already recorded.
    AlreadyCancelled,
    /// The owner or operation is currently active.
    Active,
    /// The record already contains a terminal outcome.
    AlreadyCompleted,
}

/// The registered wait for a named signal on one workflow run.
pub enum SignalWait {
    /// The operation is ready to continue. Carries `RecordedSignal`.
    Ready(RecordedSignal),
    /// Execution is suspended until the recorded condition changes.
    Waiting,
}

/// The registered wait for a child workflow’s terminal outcome.
pub enum ChildWait {
    /// The stream or operation has reached its terminal state. Contains `WorkflowStatus`.
    Terminal(WorkflowStatus),
    /// Execution is suspended until the recorded condition changes.
    Waiting,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// The registered wait for a durable deadline.
pub enum DeadlineWait {
    /// The recorded deadline has become due.
    Due,
    /// Execution is suspended until the recorded condition changes.
    Waiting,
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// The decision to resume a run whose recorded deadline fired.
pub enum DeadlineWake {
    /// The suspended operation became runnable. Contains `WorkflowSnapshot`.
    Woken(WorkflowSnapshot),
    /// The proposed transition left authoritative state unchanged. Contains `WorkflowSnapshot`.
    Unchanged(WorkflowSnapshot),
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// The decision to remove a recorded deadline wait.
pub enum DeadlineClear {
    /// The recorded wait or state was removed. Contains `WorkflowSnapshot`.
    Cleared(WorkflowSnapshot),
    /// The proposed transition left authoritative state unchanged. Contains `WorkflowSnapshot`.
    Unchanged(WorkflowSnapshot),
}

/// A runtime-neutral authority for execution records. All operations return
/// Effects, so the concrete engine and its runtime services live only in R.
/// The memory implementation is useful without Cluster or a database.
pub trait WorkflowEngine: Send + Sync + 'static {
    /// Provider-specific failure type returned by this capability.
    type Error: Send + 'static;

    /// # Parameters
    /// - `definition` (`WorkflowRegistration`): The typed definition used by this operation.
    ///
    /// # Returns
    /// Type: `Effect<(), Self::Error>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    fn register(&self, definition: WorkflowRegistration) -> Effect<(), Self::Error>;
    /// # Parameters
    /// - `handle` (`WorkflowHandle`): Owned or borrowed capability handle kept live through this operation.
    /// - `encoded_input` (`Arc<[u8]>`): The encoded input supplied to this operation.
    ///
    /// # Returns
    /// Type: `Effect<WorkflowSnapshot, Self::Error>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    fn start(
        &self,
        handle: WorkflowHandle,
        encoded_input: Arc<[u8]>,
    ) -> Effect<WorkflowSnapshot, Self::Error>;
    /// # Parameters
    /// - `run` (`WorkflowRunId`): Stable identity of the workflow run being inspected or changed.
    ///
    /// # Returns
    /// Type: `Effect<Option<WorkflowSnapshot>, Self::Error>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    fn inspect(&self, run: WorkflowRunId) -> Effect<Option<WorkflowSnapshot>, Self::Error>;
    /// Lazily conditionally transition this run using the complete expected checkpoint.
    ///
    /// Providers must atomically compare status and execution ownership with `expected`
    /// before writing `next`; stale checkpoints must not reset or replace a newer
    /// execution. Return the authoritative snapshot or the provider's typed rejection.
    /// Entering a new Running execution advances its ownership counter. Storage
    /// durability depends on the provider, not on this method's return type.
    fn transition(
        &self,
        run: WorkflowRunId,
        expected: super::WorkflowCheckpoint,
        next: WorkflowStatus,
    ) -> Effect<WorkflowSnapshot, Self::Error>;
    /// # Parameters
    /// - `run` (`WorkflowRunId`): Stable identity of the workflow run being inspected or changed.
    /// - `activity` (`WorkflowActivityId`): The activity supplied to this operation.
    /// - `attempt` (`NonZeroU32`): Ordinal execution or delivery attempt being recorded.
    /// - `encoded_input` (`Arc<[u8]>`): The encoded input supplied to this operation.
    /// - `outcome` (`EncodedWorkflowOutcome`): Complete terminal outcome, including expected errors and cleanup failures.
    ///
    /// # Returns
    /// Type: `Effect<RecordedActivity, Self::Error>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    fn record_activity(
        &self,
        run: WorkflowRunId,
        activity: WorkflowActivityId,
        attempt: NonZeroU32,
        encoded_input: Arc<[u8]>,
        outcome: EncodedWorkflowOutcome,
    ) -> Effect<RecordedActivity, Self::Error>;
    /// # Parameters
    /// - `run` (`WorkflowRunId`): Stable identity of the workflow run being inspected or changed.
    /// - `activity` (`WorkflowActivityId`): The activity supplied to this operation.
    /// - `attempt` (`NonZeroU32`): Ordinal execution or delivery attempt being recorded.
    ///
    /// # Returns
    /// Type: `Effect<Option<RecordedActivity>, Self::Error>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    fn inspect_activity(
        &self,
        run: WorkflowRunId,
        activity: WorkflowActivityId,
        attempt: NonZeroU32,
    ) -> Effect<Option<RecordedActivity>, Self::Error>;
    /// # Parameters
    /// - `parent` (`WorkflowRunId`): Parent scope, span, or workflow identity.
    /// - `child` (`RecordedChild`): Child workflow or scope-owned task being observed.
    ///
    /// # Returns
    /// Type: `Effect<RecordedChild, Self::Error>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    fn link_child(
        &self,
        parent: WorkflowRunId,
        child: RecordedChild,
    ) -> Effect<RecordedChild, Self::Error>;
    /// # Parameters
    /// - `parent` (`WorkflowRunId`): Parent scope, span, or workflow identity.
    /// - `child` (`WorkflowRunId`): Child workflow or scope-owned task being observed.
    ///
    /// # Returns
    /// Type: `Effect<ChildWait, Self::Error>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    fn wait_child(
        &self,
        parent: WorkflowRunId,
        child: WorkflowRunId,
    ) -> Effect<ChildWait, Self::Error>;
    /// # Parameters
    /// - `run` (`WorkflowRunId`): Stable identity of the workflow run being inspected or changed.
    /// - `deadline` (`RecordedDeadline`): The deadline for the pending operation.
    ///
    /// # Returns
    /// Type: `Effect<WorkflowSnapshot, Self::Error>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    fn set_deadline(
        &self,
        run: WorkflowRunId,
        deadline: RecordedDeadline,
    ) -> Effect<WorkflowSnapshot, Self::Error>;
    /// Clear only the exact named occurrence that was scheduled. A stale
    /// cancellation cannot remove a rescheduled deadline.
    ///
    /// # Parameters
    /// - `run` (`WorkflowRunId`): Stable identity of the workflow run being inspected or changed.
    /// - `expected` (`RecordedDeadline`): Value or revision required for the proposed state transition.
    ///
    /// # Returns
    /// Type: `Effect<DeadlineClear, Self::Error>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    fn clear_deadline(
        &self,
        run: WorkflowRunId,
        expected: RecordedDeadline,
    ) -> Effect<DeadlineClear, Self::Error>;
    /// # Parameters
    /// - `now` (`UtcMillis`): The caller-supplied current time.
    ///
    /// # Returns
    /// Type: `Effect<Vec<WorkflowRunId>, Self::Error>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    fn wake_due(&self, now: UtcMillis) -> Effect<Vec<WorkflowRunId>, Self::Error>;
    /// Atomically wake one run only if it still waits on this exact deadline.
    /// A stale scheduler command cannot undo a reschedule or cancellation.
    ///
    /// # Parameters
    /// - `run` (`WorkflowRunId`): Stable identity of the workflow run being inspected or changed.
    /// - `expected` (`RecordedDeadline`): Value or revision required for the proposed state transition.
    /// - `now` (`UtcMillis`): The caller-supplied current time.
    ///
    /// # Returns
    /// Type: `Effect<DeadlineWake, Self::Error>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    fn wake_deadline(
        &self,
        run: WorkflowRunId,
        expected: RecordedDeadline,
        now: UtcMillis,
    ) -> Effect<DeadlineWake, Self::Error>;
    /// # Parameters
    /// - `run` (`WorkflowRunId`): Stable identity of the workflow run being inspected or changed.
    /// - `deadline` (`RecordedDeadline`): The deadline for the pending operation.
    /// - `now` (`UtcMillis`): The caller-supplied current time.
    ///
    /// # Returns
    /// Type: `Effect<DeadlineWait, Self::Error>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    fn wait_deadline(
        &self,
        run: WorkflowRunId,
        deadline: RecordedDeadline,
        now: UtcMillis,
    ) -> Effect<DeadlineWait, Self::Error>;
    /// # Parameters
    /// - `run` (`WorkflowRunId`): Stable identity of the workflow run being inspected or changed.
    /// - `signal` (`RecordedSignal`): Named value delivered to a waiting workflow.
    ///
    /// # Returns
    /// Type: `Effect<RecordedSignal, Self::Error>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    fn deliver_signal(
        &self,
        run: WorkflowRunId,
        signal: RecordedSignal,
    ) -> Effect<RecordedSignal, Self::Error>;
    /// # Parameters
    /// - `run` (`WorkflowRunId`): Stable identity of the workflow run being inspected or changed.
    /// - `signal` (`WorkflowSignalId`): Named value delivered to a waiting workflow.
    /// - `version` (`WorkflowVersion`): Registered schema, event, or workflow compatibility version.
    ///
    /// # Returns
    /// Type: `Effect<Option<RecordedSignal>, Self::Error>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    fn inspect_signal(
        &self,
        run: WorkflowRunId,
        signal: WorkflowSignalId,
        version: WorkflowVersion,
    ) -> Effect<Option<RecordedSignal>, Self::Error>;
    /// # Parameters
    /// - `run` (`WorkflowRunId`): Stable identity of the workflow run being inspected or changed.
    /// - `signal` (`WorkflowSignalId`): Named value delivered to a waiting workflow.
    /// - `version` (`WorkflowVersion`): Registered schema, event, or workflow compatibility version.
    ///
    /// # Returns
    /// Type: `Effect<SignalWait, Self::Error>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    fn wait_signal(
        &self,
        run: WorkflowRunId,
        signal: WorkflowSignalId,
        version: WorkflowVersion,
    ) -> Effect<SignalWait, Self::Error>;
    /// # Parameters
    /// - `run` (`WorkflowRunId`): Stable identity of the workflow run being inspected or changed.
    /// - `identity` (`WorkflowCompensationId`): Stable identity used to deduplicate or find this record.
    /// - `order` (`NonZeroU32`): The order supplied to this operation.
    ///
    /// # Returns
    /// Type: `Effect<RecordedCompensation, Self::Error>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    fn register_compensation(
        &self,
        run: WorkflowRunId,
        identity: WorkflowCompensationId,
        order: NonZeroU32,
    ) -> Effect<RecordedCompensation, Self::Error>;
    /// # Parameters
    /// - `run` (`WorkflowRunId`): Stable identity of the workflow run being inspected or changed.
    /// - `identity` (`WorkflowCompensationId`): Stable identity used to deduplicate or find this record.
    /// - `outcome` (`EncodedWorkflowOutcome`): Complete terminal outcome, including expected errors and cleanup failures.
    ///
    /// # Returns
    /// Type: `Effect<RecordedCompensation, Self::Error>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    fn finish_compensation(
        &self,
        run: WorkflowRunId,
        identity: WorkflowCompensationId,
        outcome: EncodedWorkflowOutcome,
    ) -> Effect<RecordedCompensation, Self::Error>;
    /// # Parameters
    /// - `run` (`WorkflowRunId`): Stable identity of the workflow run being inspected or changed.
    ///
    /// # Returns
    /// Type: `Effect<Vec<RecordedCompensation>, Self::Error>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    fn pending_compensations(
        &self,
        run: WorkflowRunId,
    ) -> Effect<Vec<RecordedCompensation>, Self::Error>;
    /// # Parameters
    /// - `run` (`WorkflowRunId`): Stable identity of the workflow run being inspected or changed.
    /// - `race` (`WorkflowRaceId`): Stable identity or definition of the durable race.
    ///
    /// # Returns
    /// Type: `Effect<Option<RecordedRace>, Self::Error>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    fn inspect_race(
        &self,
        run: WorkflowRunId,
        race: WorkflowRaceId,
    ) -> Effect<Option<RecordedRace>, Self::Error>;
    /// # Parameters
    /// - `run` (`WorkflowRunId`): Stable identity of the workflow run being inspected or changed.
    /// - `race` (`RecordedRace`): Stable identity or definition of the durable race.
    ///
    /// # Returns
    /// Type: `Effect<RecordedRace, Self::Error>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    fn record_race(
        &self,
        run: WorkflowRunId,
        race: RecordedRace,
    ) -> Effect<RecordedRace, Self::Error>;

    /// # Parameters
    /// - `run` (`WorkflowRunId`): Stable identity of the workflow run being inspected or changed.
    /// - `outcome` (`EncodedWorkflowOutcome`): Complete terminal outcome, including expected errors and cleanup failures.
    ///
    /// # Returns
    /// Type: `Effect<WorkflowSnapshot, Self::Error>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    fn complete(
        &self,
        run: WorkflowRunId,
        outcome: EncodedWorkflowOutcome,
    ) -> Effect<WorkflowSnapshot, Self::Error>;
    /// # Parameters
    /// - `run` (`WorkflowRunId`): Stable identity of the workflow run being inspected or changed.
    ///
    /// # Returns
    /// Type: `Effect<CancellationOutcome, Self::Error>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    fn cancel(&self, run: WorkflowRunId) -> Effect<CancellationOutcome, Self::Error>;
    /// Force terminal cancellation even while code is running. The external
    /// side effect may still finish; its outcome is uncertain to this engine.
    ///
    /// # Parameters
    /// - `run` (`WorkflowRunId`): Stable identity of the workflow run being inspected or changed.
    ///
    /// # Returns
    /// Type: `Effect<CancellationOutcome, Self::Error>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    fn force_interrupt(&self, run: WorkflowRunId) -> Effect<CancellationOutcome, Self::Error>;
}

/// Selects the workflow engine in an application environment.
pub trait HasWorkflowEngine {
    /// Workflow engine selected by this environment.
    type Engine: WorkflowEngine;
    /// # Returns
    /// Type: `Arc<Self::Engine>`.
    fn workflow_engine(&self) -> Arc<Self::Engine>;
}

/// A provider warrants that acknowledged workflow records survive its
/// documented restart boundary and can be recovered by a new engine instance.
/// The process-local memory engine does not implement this capability.
pub trait DurableWorkflowEngine: WorkflowEngine {}
