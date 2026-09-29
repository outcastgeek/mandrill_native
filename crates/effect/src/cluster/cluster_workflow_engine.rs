use super::{ClusterEntityType, EntityDispatcher, EntityRegistrationError};
use crate::Effect;
use crate::rpc::{RpcHandler, RpcOperation};
use crate::workflow::{
    DeadlineWake, HasWorkflowEngine, WorkflowCommand, WorkflowEngine, WorkflowReply, WorkflowStatus,
};
use std::sync::Mutex;

#[derive(Debug)]
pub enum ClusterWorkflowError<EngineError> {
    Engine(EngineError),
    InputConsumed,
}

/// Cluster delivery adapter over the independent WorkflowEngine authority.
/// The caller registers the returned handler with its Cluster entity type and
/// drives it through ClusterRunner; no second workflow journal is created.
pub struct ClusterWorkflowEngine;

impl ClusterWorkflowEngine {
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

    pub fn dispatch<R>(
        command: WorkflowCommand,
    ) -> Effect<WorkflowReply, ClusterWorkflowError<<R::Engine as WorkflowEngine>::Error>, R>
    where
        R: HasWorkflowEngine + Send + Sync + 'static,
    {
        let command = Mutex::new(Some(command));
        Effect::service(|env: &R| env.workflow_engine()).flat_map(move |engine| {
            let command = command
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take();
            let Some(command) = command else {
                return Effect::from_result_once(Err(ClusterWorkflowError::InputConsumed));
            };
            match command {
                WorkflowCommand::Start { handle, input } => engine
                    .start(handle, input)
                    .map_error(ClusterWorkflowError::Engine)
                    .provide_some(|_: &R| ())
                    .map(|snapshot| WorkflowReply::snapshot(Some(snapshot))),
                WorkflowCommand::Activity { run, record } => engine
                    .record_activity(
                        run,
                        record.identity,
                        record.attempt,
                        record.encoded_input,
                        record.outcome,
                    )
                    .map_error(ClusterWorkflowError::Engine)
                    .provide_some(|_: &R| ())
                    .map(|_| WorkflowReply::Accepted),
                WorkflowCommand::Child { parent, record } => engine
                    .link_child(parent, record)
                    .map_error(ClusterWorkflowError::Engine)
                    .provide_some(|_: &R| ())
                    .map(|_| WorkflowReply::Accepted),
                WorkflowCommand::WaitChild { parent, child } => engine
                    .wait_child(parent, child)
                    .map_error(ClusterWorkflowError::Engine)
                    .provide_some(|_: &R| ())
                    .map(WorkflowReply::ChildWait),
                WorkflowCommand::InspectActivity {
                    run,
                    activity,
                    attempt,
                } => engine
                    .inspect_activity(run, activity, attempt)
                    .map_error(ClusterWorkflowError::Engine)
                    .provide_some(|_: &R| ())
                    .map(WorkflowReply::Activity),
                WorkflowCommand::Signal { run, record } => engine
                    .deliver_signal(run, record)
                    .map_error(ClusterWorkflowError::Engine)
                    .provide_some(|_: &R| ())
                    .map(|_| WorkflowReply::Accepted),
                WorkflowCommand::InspectSignal {
                    run,
                    signal,
                    version,
                } => engine
                    .inspect_signal(run, signal, version)
                    .map_error(ClusterWorkflowError::Engine)
                    .provide_some(|_: &R| ())
                    .map(WorkflowReply::Signal),
                WorkflowCommand::WaitSignal {
                    run,
                    signal,
                    version,
                } => engine
                    .wait_signal(run, signal, version)
                    .map_error(ClusterWorkflowError::Engine)
                    .provide_some(|_: &R| ())
                    .map(WorkflowReply::SignalWait),
                WorkflowCommand::Resume { run } => {
                    let resume_engine = engine.clone();
                    engine
                        .inspect(run.clone())
                        .map_error(ClusterWorkflowError::Engine)
                        .provide_some(|_: &R| ())
                        .flat_map(move |snapshot| match snapshot {
                            Some(snapshot) if snapshot.status == WorkflowStatus::Suspended => {
                                resume_engine
                                    .transition(
                                        run.clone(),
                                        WorkflowStatus::Suspended,
                                        WorkflowStatus::Ready,
                                    )
                                    .map_error(ClusterWorkflowError::Engine)
                                    .provide_some(|_: &R| ())
                                    .map(|snapshot| WorkflowReply::snapshot(Some(snapshot)))
                            }
                            other => Effect::sync_once(move || WorkflowReply::snapshot(other)),
                        })
                }
                WorkflowCommand::Transition {
                    run,
                    expected,
                    next,
                } => engine
                    .transition(run, expected, next)
                    .map_error(ClusterWorkflowError::Engine)
                    .provide_some(|_: &R| ())
                    .map(|snapshot| WorkflowReply::snapshot(Some(snapshot))),
                WorkflowCommand::Cancel { run } => engine
                    .cancel(run)
                    .map_error(ClusterWorkflowError::Engine)
                    .provide_some(|_: &R| ())
                    .map(WorkflowReply::Cancellation),
                WorkflowCommand::ForceInterrupt { run } => engine
                    .force_interrupt(run)
                    .map_error(ClusterWorkflowError::Engine)
                    .provide_some(|_: &R| ())
                    .map(WorkflowReply::Cancellation),
                WorkflowCommand::Deadline { run, deadline } => engine
                    .set_deadline(run, deadline)
                    .map_error(ClusterWorkflowError::Engine)
                    .provide_some(|_: &R| ())
                    .map(|snapshot| WorkflowReply::snapshot(Some(snapshot))),
                WorkflowCommand::ClearDeadline { run, expected } => engine
                    .clear_deadline(run, expected)
                    .map_error(ClusterWorkflowError::Engine)
                    .provide_some(|_: &R| ())
                    .map(|outcome| WorkflowReply::DeadlineClear(Box::new(outcome))),
                WorkflowCommand::WaitDeadline { run, deadline, now } => engine
                    .wait_deadline(run, deadline, now)
                    .map_error(ClusterWorkflowError::Engine)
                    .provide_some(|_: &R| ())
                    .map(WorkflowReply::DeadlineWait),
                WorkflowCommand::WakeDue { run, expected, now } => engine
                    .wake_deadline(run, expected, now)
                    .map_error(ClusterWorkflowError::Engine)
                    .provide_some(|_: &R| ())
                    .map(|wake| match wake {
                        DeadlineWake::Woken(snapshot) | DeadlineWake::Unchanged(snapshot) => {
                            WorkflowReply::snapshot(Some(snapshot))
                        }
                    }),
                WorkflowCommand::RegisterCompensation {
                    run,
                    identity,
                    order,
                } => engine
                    .register_compensation(run, identity, order)
                    .map_error(ClusterWorkflowError::Engine)
                    .provide_some(|_: &R| ())
                    .map(WorkflowReply::Compensation),
                WorkflowCommand::FinishCompensation {
                    run,
                    identity,
                    outcome,
                } => engine
                    .finish_compensation(run, identity, outcome)
                    .map_error(ClusterWorkflowError::Engine)
                    .provide_some(|_: &R| ())
                    .map(WorkflowReply::Compensation),
                WorkflowCommand::PendingCompensations { run } => engine
                    .pending_compensations(run)
                    .map_error(ClusterWorkflowError::Engine)
                    .provide_some(|_: &R| ())
                    .map(WorkflowReply::Compensations),
                WorkflowCommand::InspectRace { run, identity } => engine
                    .inspect_race(run, identity)
                    .map_error(ClusterWorkflowError::Engine)
                    .provide_some(|_: &R| ())
                    .map(WorkflowReply::Race),
                WorkflowCommand::RecordRace { run, record } => engine
                    .record_race(run, record)
                    .map_error(ClusterWorkflowError::Engine)
                    .provide_some(|_: &R| ())
                    .map(|record| WorkflowReply::Race(Some(record))),
                WorkflowCommand::Inspect { run } => engine
                    .inspect(run)
                    .map_error(ClusterWorkflowError::Engine)
                    .provide_some(|_: &R| ())
                    .map(WorkflowReply::snapshot),
                WorkflowCommand::Complete { run, outcome } => engine
                    .complete(run, outcome)
                    .map_error(ClusterWorkflowError::Engine)
                    .provide_some(|_: &R| ())
                    .map(|snapshot| WorkflowReply::snapshot(Some(snapshot))),
            }
        })
    }
}
