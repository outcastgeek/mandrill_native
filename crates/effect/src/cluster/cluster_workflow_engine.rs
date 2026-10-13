//! Workflow control through typed Cluster messages.

use super::{ClusterEntityType, EntityDispatcher, EntityRegistrationError};
use crate::Effect;
use crate::rpc::{RpcHandler, RpcOperation};
use crate::workflow::{
    DeadlineWake, HasWorkflowEngine, WorkflowCommand, WorkflowEngine, WorkflowReply, WorkflowStatus,
};
use crate::{
    effect, effect_map, effect_map_error, effect_provide_some, effect_sync, effect_try_sync,
    service,
};
use std::sync::Mutex;

#[derive(Debug)]
/// A failure translating an addressed cluster command to a workflow engine operation.
///
/// # Type parameters
/// - `EngineError`: Error returned by the workflow engine.
pub enum ClusterWorkflowError<EngineError> {
    /// The workflow engine rejected or could not complete the operation. Retains the `EngineError` detail.
    Engine(EngineError),
    /// An owned input was already consumed.
    InputConsumed,
}

/// Cluster delivery adapter over the independent WorkflowEngine authority.
/// The caller registers the returned handler with its Cluster entity type and
/// drives it through ClusterRunner; no second workflow journal is created.
pub struct ClusterWorkflowEngine;

impl ClusterWorkflowEngine {
    /// Borrow the typed entity dispatcher registered with this adapter.
    ///
    /// # Type parameters
    /// - `R`: Environment type. Additional bound: `HasWorkflowEngine + Send + Sync + 'static`.
    ///
    /// # Parameters
    /// - `policy` (`crate::rpc::RpcEnvelopePolicy`): The caller-supplied behavior policy.
    /// - `entity_type` (`ClusterEntityType`): Stable type identity used for addressed dispatch.
    /// - `operation` (`RpcOperation<WorkflowCommand, WorkflowReply, ClusterWorkflowError<<R::Engine as WorkflowEngine>::Error>>`): Typed operation definition or callback selected for execution.
    ///
    /// # Returns
    /// Type: `Result<EntityDispatcher<R>, EntityRegistrationError>`.
    pub fn dispatcher<R>(
        policy: crate::rpc::RpcEnvelopePolicy,
        entity_type: ClusterEntityType,
        operation: RpcOperation<
            WorkflowCommand,
            WorkflowReply,
            ClusterWorkflowError<<R::Engine as WorkflowEngine>::Error>,
        >,
    ) -> Result<EntityDispatcher<R>, EntityRegistrationError>
    where
        R: HasWorkflowEngine + Send + Sync + 'static,
    {
        EntityDispatcher::from_addressed_handler(
            policy,
            entity_type,
            Self::handler::<R>(operation),
            |address, command| address.entity.as_bytes() == command.run().as_bytes(),
        )
    }

    /// Register the supplied definition, rejecting an identity or compatibility conflict.
    ///
    /// # Type parameters
    /// - `R`: Environment type. Additional bound: `HasWorkflowEngine + Send + Sync + 'static`.
    ///
    /// # Parameters
    /// - `dispatcher` (`&mut EntityDispatcher<R>`): The dispatcher supplied to this operation.
    /// - `entity_type` (`ClusterEntityType`): Stable type identity used for addressed dispatch.
    /// - `operation` (`RpcOperation<WorkflowCommand, WorkflowReply, ClusterWorkflowError<<R::Engine as WorkflowEngine>::Error>>`): Typed operation definition or callback selected for execution.
    ///
    /// # Returns
    /// Type: `Result<(), EntityRegistrationError>`.
    pub fn register<R>(
        dispatcher: &mut EntityDispatcher<R>,
        entity_type: ClusterEntityType,
        operation: RpcOperation<
            WorkflowCommand,
            WorkflowReply,
            ClusterWorkflowError<<R::Engine as WorkflowEngine>::Error>,
        >,
    ) -> Result<(), EntityRegistrationError>
    where
        R: HasWorkflowEngine + Send + Sync + 'static,
    {
        dispatcher.register_addressed(
            entity_type,
            Self::handler::<R>(operation),
            |address, command| address.entity.as_bytes() == command.run().as_bytes(),
        )
    }

    /// Return the typed handler registered for this definition.
    ///
    /// # Type parameters
    /// - `R`: Environment type. Additional bound: `HasWorkflowEngine + Send + Sync + 'static`.
    ///
    /// # Parameters
    /// - `operation` (`RpcOperation<WorkflowCommand, WorkflowReply, ClusterWorkflowError<<R::Engine as WorkflowEngine>::Error>>`): Typed operation definition or callback selected for execution.
    ///
    /// # Returns
    /// Type: `RpcHandler<WorkflowCommand, WorkflowReply, ClusterWorkflowError<<R::Engine as WorkflowEngine>::Error>, R>`.
    /// Returns the value described by this method's return type.
    pub fn handler<R>(
        operation: RpcOperation<
            WorkflowCommand,
            WorkflowReply,
            ClusterWorkflowError<<R::Engine as WorkflowEngine>::Error>,
        >,
    ) -> RpcHandler<
        WorkflowCommand,
        WorkflowReply,
        ClusterWorkflowError<<R::Engine as WorkflowEngine>::Error>,
        R,
    >
    where
        R: HasWorkflowEngine + Send + Sync + 'static,
    {
        RpcHandler::new(operation, Self::dispatch::<R>)
    }

    /// Lazily dispatch one owned workflow command through R's existing engine.
    ///
    /// Resume honors the optional exact reset checkpoint: absence resumes only
    /// Suspended state, and stale fences cannot reset newer Running executions.
    /// Completed runs return recorded snapshots. This is a single-use command effect;
    /// no secondary journal or storage durability is supplied by the bridge.
    pub fn dispatch<R>(
        command: WorkflowCommand,
    ) -> Effect<WorkflowReply, ClusterWorkflowError<<R::Engine as WorkflowEngine>::Error>, R>
    where
        R: HasWorkflowEngine + Send + Sync + 'static,
    {
        let command = Mutex::new(Some(command));
        let command = effect_try_sync! {
            body: move || command
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take()
                .ok_or(ClusterWorkflowError::InputConsumed),
        };
        effect! {
            let command = command.clone();
            let engine = service! {
                env: |env: &R| env.workflow_engine(),
            }?;
            let command = command?;
            let response = match command {
                WorkflowCommand::Start { handle, input } => effect_map! { program: effect_provide_some! { program: effect_map_error! { program: engine
                    .start(handle, input), map: ClusterWorkflowError::Engine, }, project: |_: &R| (), }, map: |snapshot| WorkflowReply::snapshot(Some(snapshot)), },
                WorkflowCommand::Activity { run, record } => effect_map! { program: effect_provide_some! { program: effect_map_error! { program: engine
                    .record_activity(
                        run,
                        record.identity,
                        record.attempt,
                        record.encoded_input,
                        record.outcome,
                    ), map: ClusterWorkflowError::Engine, }, project: |_: &R| (), }, map: |_| WorkflowReply::Accepted, },
                WorkflowCommand::Child { parent, record } => effect_map! { program: effect_provide_some! { program: effect_map_error! { program: engine
                    .link_child(parent, record), map: ClusterWorkflowError::Engine, }, project: |_: &R| (), }, map: |_| WorkflowReply::Accepted, },
                WorkflowCommand::WaitChild { parent, child } => effect_map! { program: effect_provide_some! { program: effect_map_error! { program: engine
                    .wait_child(parent, child), map: ClusterWorkflowError::Engine, }, project: |_: &R| (), }, map: WorkflowReply::ChildWait, },
                WorkflowCommand::InspectActivity {
                    run,
                    activity,
                    attempt,
                } => effect_map! { program: effect_provide_some! { program: effect_map_error! { program: engine
                    .inspect_activity(run, activity, attempt), map: ClusterWorkflowError::Engine, }, project: |_: &R| (), }, map: WorkflowReply::Activity, },
                WorkflowCommand::Signal { run, record } => effect_map! { program: effect_provide_some! { program: effect_map_error! { program: engine
                    .deliver_signal(run, record), map: ClusterWorkflowError::Engine, }, project: |_: &R| (), }, map: |_| WorkflowReply::Accepted, },
                WorkflowCommand::InspectSignal {
                    run,
                    signal,
                    version,
                } => effect_map! { program: effect_provide_some! { program: effect_map_error! { program: engine
                    .inspect_signal(run, signal, version), map: ClusterWorkflowError::Engine, }, project: |_: &R| (), }, map: WorkflowReply::Signal, },
                WorkflowCommand::WaitSignal {
                    run,
                    signal,
                    version,
                } => effect_map! { program: effect_provide_some! { program: effect_map_error! { program: engine
                    .wait_signal(run, signal, version), map: ClusterWorkflowError::Engine, }, project: |_: &R| (), }, map: WorkflowReply::SignalWait, },
                WorkflowCommand::Resume { run, checkpoint } => {
                    let resume_engine = engine.clone();
                    let inspection = effect_provide_some! { program: effect_map_error! { program: engine
                        .inspect(run.clone()), map: ClusterWorkflowError::Engine, }, project: |_: &R| (), };
                    effect! {
                        let run = run.clone();
                        let checkpoint = checkpoint.clone();
                        let resume_engine = resume_engine.clone();
                        let snapshot = inspection.clone()?;
                        let response = match snapshot {
                            Some(snapshot)
                                if snapshot.status == WorkflowStatus::Suspended
                                    || (snapshot.status == WorkflowStatus::Running
                                        && checkpoint.is_some()) =>
                            {
                                effect_map! { program: effect_provide_some! { program: effect_map_error! { program: resume_engine
                                    .transition(
                                        run.clone(),
                                        checkpoint.clone().unwrap_or_else(|| snapshot.checkpoint()),
                                        WorkflowStatus::Ready,
                                    ), map: ClusterWorkflowError::Engine, }, project: |_: &R| (), }, map: |snapshot| WorkflowReply::snapshot(Some(snapshot)), }
                            }
                            other => effect_sync! { once; body: move || WorkflowReply::snapshot(other), },
                        };
                        response?
                    }
                }
                WorkflowCommand::Transition {
                    run,
                    expected,
                    next,
                } => effect_map! { program: effect_provide_some! { program: effect_map_error! { program: engine
                    .transition(run, expected, next), map: ClusterWorkflowError::Engine, }, project: |_: &R| (), }, map: |snapshot| WorkflowReply::snapshot(Some(snapshot)), },
                WorkflowCommand::Cancel { run } => effect_map! { program: effect_provide_some! { program: effect_map_error! { program: engine
                    .cancel(run), map: ClusterWorkflowError::Engine, }, project: |_: &R| (), }, map: WorkflowReply::Cancellation, },
                WorkflowCommand::ForceInterrupt { run } => effect_map! { program: effect_provide_some! { program: effect_map_error! { program: engine
                    .force_interrupt(run), map: ClusterWorkflowError::Engine, }, project: |_: &R| (), }, map: WorkflowReply::Cancellation, },
                WorkflowCommand::Deadline { run, deadline } => effect_map! { program: effect_provide_some! { program: effect_map_error! { program: engine
                    .set_deadline(run, deadline), map: ClusterWorkflowError::Engine, }, project: |_: &R| (), }, map: |snapshot| WorkflowReply::snapshot(Some(snapshot)), },
                WorkflowCommand::ClearDeadline { run, expected } => effect_map! { program: effect_provide_some! { program: effect_map_error! { program: engine
                    .clear_deadline(run, expected), map: ClusterWorkflowError::Engine, }, project: |_: &R| (), }, map: |outcome| WorkflowReply::DeadlineClear(Box::new(outcome)), },
                WorkflowCommand::WaitDeadline { run, deadline, now } => effect_map! { program: effect_provide_some! { program: effect_map_error! { program: engine
                    .wait_deadline(run, deadline, now), map: ClusterWorkflowError::Engine, }, project: |_: &R| (), }, map: WorkflowReply::DeadlineWait, },
                WorkflowCommand::WakeDue { run, expected, now } => effect_map! { program: effect_provide_some! { program: effect_map_error! { program: engine
                    .wake_deadline(run, expected, now), map: ClusterWorkflowError::Engine, }, project: |_: &R| (), }, map: |wake| match wake {
                        DeadlineWake::Woken(snapshot) | DeadlineWake::Unchanged(snapshot) => {
                            WorkflowReply::snapshot(Some(snapshot))
                        }
                    }, },
                WorkflowCommand::RegisterCompensation {
                    run,
                    identity,
                    order,
                } => effect_map! { program: effect_provide_some! { program: effect_map_error! { program: engine
                    .register_compensation(run, identity, order), map: ClusterWorkflowError::Engine, }, project: |_: &R| (), }, map: WorkflowReply::Compensation, },
                WorkflowCommand::FinishCompensation {
                    run,
                    identity,
                    outcome,
                } => effect_map! { program: effect_provide_some! { program: effect_map_error! { program: engine
                    .finish_compensation(run, identity, outcome), map: ClusterWorkflowError::Engine, }, project: |_: &R| (), }, map: WorkflowReply::Compensation, },
                WorkflowCommand::PendingCompensations { run } => effect_map! { program: effect_provide_some! { program: effect_map_error! { program: engine
                    .pending_compensations(run), map: ClusterWorkflowError::Engine, }, project: |_: &R| (), }, map: WorkflowReply::Compensations, },
                WorkflowCommand::InspectRace { run, identity } => effect_map! { program: effect_provide_some! { program: effect_map_error! { program: engine
                    .inspect_race(run, identity), map: ClusterWorkflowError::Engine, }, project: |_: &R| (), }, map: WorkflowReply::Race, },
                WorkflowCommand::RecordRace { run, record } => effect_map! { program: effect_provide_some! { program: effect_map_error! { program: engine
                    .record_race(run, record), map: ClusterWorkflowError::Engine, }, project: |_: &R| (), }, map: |record| WorkflowReply::Race(Some(record)), },
                WorkflowCommand::Inspect { run } => effect_map! { program: effect_provide_some! { program: effect_map_error! { program: engine
                    .inspect(run), map: ClusterWorkflowError::Engine, }, project: |_: &R| (), }, map: WorkflowReply::snapshot, },
                WorkflowCommand::Complete { run, outcome } => effect_map! { program: effect_provide_some! { program: effect_map_error! { program: engine
                    .complete(run, outcome), map: ClusterWorkflowError::Engine, }, project: |_: &R| (), }, map: |snapshot| WorkflowReply::snapshot(Some(snapshot)), },
            };
            response?
        }
    }
}
