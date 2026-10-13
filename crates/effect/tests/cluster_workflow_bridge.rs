use effect::cluster::{ClusterWorkflowEngine, ClusterWorkflowError};
use effect::workflow::{
    DeadlineClear, DeadlineWait, DeadlineWake, DurableClock, EncodedWorkflowOutcome,
    HasWorkflowEngine, MemoryWorkflowEngine, RecordedDeadline, WorkflowCommand, WorkflowDeadlineId,
    WorkflowDefinitionId, WorkflowEngine, WorkflowEngineError, WorkflowHandle, WorkflowOutcomeKind,
    WorkflowRegistration, WorkflowReply, WorkflowRunId, WorkflowStatus, WorkflowVersion,
    WorkflowWait, WorkflowWakeReason,
};
use effect::{Exit, KeyDomain, KeyNamespace, KeySegment, StableKey, UtcMillis};
use futures_executor::block_on;
use std::num::NonZeroU32;
use std::sync::Arc;

struct Domain;
impl KeyDomain for Domain {
    const TAG: &'static str = "cluster-workflow-test";
}

#[test]
fn named_deadline_reschedule_fences_stale_wake_and_recovers_at_boundary() -> Result<(), String> {
    let engine = Arc::new(MemoryWorkflowEngine::new());
    let environment = Arc::new(Environment(Arc::clone(&engine)));
    let namespace = KeyNamespace::<Domain>::new("deadline-runs")
        .map_err(|error| format!("deadline namespace: {error:?}"))?;
    let key = |name: &'static str| {
        StableKey::new(&namespace, [KeySegment::Text(Arc::from(name))])
            .map_err(|error| format!("{name} identity: {error:?}"))
    };
    let run = WorkflowRunId::from_stable(key("run")?);
    let definition = WorkflowDefinitionId::new("deadline-workflow")
        .map_err(|error| format!("deadline workflow: {error:?}"))?;
    let version = WorkflowVersion::new(NonZeroU32::MIN);
    let handle = WorkflowHandle {
        run: run.clone(),
        definition: definition.clone(),
        version,
    };
    assert!(matches!(
        block_on(
            engine
                .register(WorkflowRegistration {
                    id: definition,
                    version,
                    compatibility: Arc::from(&b"deadline-v1"[..]),
                })
                .run(Arc::new(()))
        ),
        Exit::Success(())
    ));
    assert!(matches!(
        block_on(
            engine
                .start(handle, Arc::from(&b"input"[..]))
                .run(Arc::new(()))
        ),
        Exit::Success(_)
    ));
    let first = RecordedDeadline {
        identity: WorkflowDeadlineId::from_stable(key("first")?),
        at: UtcMillis::new(10),
    };
    let second = RecordedDeadline {
        identity: WorkflowDeadlineId::from_stable(key("second")?),
        at: UtcMillis::new(20),
    };
    assert!(matches!(
        block_on(
            engine
                .wait_deadline(run.clone(), first.clone(), UtcMillis::new(0))
                .run(Arc::new(()))
        ),
        Exit::Success(DeadlineWait::Waiting)
    ));
    assert!(matches!(
        block_on(
            DurableClock::schedule(run.clone(), second.clone())
                .run(Arc::clone(&environment))
        ),
        Exit::Success(snapshot) if snapshot.waiting_on == Some(WorkflowWait::Deadline(second.clone()))
    ));
    assert!(matches!(
        block_on(
            engine
                .wake_deadline(run.clone(), first.clone(), UtcMillis::new(20))
                .run(Arc::new(()))
        ),
        Exit::Success(DeadlineWake::Unchanged(snapshot)) if snapshot.status == WorkflowStatus::Suspended
    ));
    assert!(matches!(
        block_on(engine.clear_deadline(run.clone(), first).run(Arc::new(()))),
        Exit::Success(DeadlineClear::Unchanged(snapshot))
            if snapshot.waiting_on == Some(WorkflowWait::Deadline(second.clone()))
    ));
    let stale_occurrence = RecordedDeadline {
        identity: second.identity.clone(),
        at: UtcMillis::new(10),
    };
    assert!(matches!(
        block_on(
            engine
                .clear_deadline(run.clone(), stale_occurrence)
                .run(Arc::new(()))
        ),
        Exit::Success(DeadlineClear::Unchanged(snapshot))
            if snapshot.waiting_on == Some(WorkflowWait::Deadline(second.clone()))
    ));
    let definitions = engine.definitions();
    let snapshots = engine.snapshot_all();
    let early = MemoryWorkflowEngine::restore_at(
        definitions.clone(),
        snapshots.clone(),
        UtcMillis::new(19),
    )
    .map_err(|error| format!("early restoration: {error:?}"))?;
    assert!(matches!(
        block_on(early.inspect(run.clone()).run(Arc::new(()))),
        Exit::Success(Some(snapshot)) if snapshot.status == WorkflowStatus::Suspended
    ));
    let due = Arc::new(
        MemoryWorkflowEngine::restore_at(definitions, snapshots, UtcMillis::new(20))
            .map_err(|error| format!("due restoration: {error:?}"))?,
    );
    assert!(matches!(
        block_on(due.inspect(run.clone()).run(Arc::new(()))),
        Exit::Success(Some(snapshot))
            if snapshot.status == WorkflowStatus::Ready
                && snapshot.wake_reason == Some(WorkflowWakeReason::Deadline(second.clone()))
    ));
    assert!(matches!(
        block_on(
            DurableClock::clear(run, second).run(Arc::new(Environment(Arc::clone(&due))))
        ),
        Exit::Success(DeadlineClear::Cleared(snapshot)) if snapshot.deadline.is_none()
    ));
    Ok(())
}

struct Environment(Arc<MemoryWorkflowEngine>);
impl HasWorkflowEngine for Environment {
    type Engine = MemoryWorkflowEngine;
    fn workflow_engine(&self) -> Arc<Self::Engine> {
        Arc::clone(&self.0)
    }
}

#[test]
fn cluster_workflow_commands_use_the_same_engine_records() -> Result<(), String> {
    let engine = Arc::new(MemoryWorkflowEngine::new());
    let environment = Arc::new(Environment(Arc::clone(&engine)));
    let namespace = KeyNamespace::<Domain>::new("runs")
        .map_err(|error| format!("workflow namespace: {error:?}"))?;
    let key = StableKey::new(&namespace, [KeySegment::Text(Arc::from("one"))])
        .map_err(|error| format!("workflow run: {error:?}"))?;
    let run = WorkflowRunId::from_stable(key);
    let definition = WorkflowDefinitionId::new("workflow")
        .map_err(|error| format!("workflow definition: {error:?}"))?;
    let version = WorkflowVersion::new(NonZeroU32::MIN);
    let handle = WorkflowHandle {
        run: run.clone(),
        definition: definition.clone(),
        version,
    };

    let unregistered = ClusterWorkflowEngine::dispatch::<Environment>(WorkflowCommand::Start {
        handle: handle.clone(),
        input: Arc::from(&b"input"[..]),
    });
    assert!(matches!(
        block_on(unregistered.run(Arc::clone(&environment))),
        Exit::Fail(ClusterWorkflowError::Engine(
            WorkflowEngineError::DefinitionMismatch
        ))
    ));

    let registration = WorkflowRegistration {
        id: definition,
        version,
        compatibility: Arc::from(&b"v1"[..]),
    };
    assert!(matches!(
        block_on(engine.register(registration).run(Arc::new(()))),
        Exit::Success(())
    ));
    let started = ClusterWorkflowEngine::dispatch::<Environment>(WorkflowCommand::Start {
        handle,
        input: Arc::from(&b"input"[..]),
    });
    assert!(
        matches!(block_on(started.run(Arc::clone(&environment))), Exit::Success(WorkflowReply::Snapshot(Some(snapshot))) if snapshot.status == WorkflowStatus::Ready)
    );

    let inspected = ClusterWorkflowEngine::dispatch::<Environment>(WorkflowCommand::Inspect {
        run: run.clone(),
    });
    assert!(
        matches!(block_on(inspected.run(Arc::clone(&environment))), Exit::Success(WorkflowReply::Snapshot(Some(snapshot))) if snapshot.handle.run == run)
    );
    assert!(
        matches!(block_on(engine.inspect(run.clone()).run(Arc::new(()))), Exit::Success(Some(snapshot)) if snapshot.status == WorkflowStatus::Ready)
    );
    let deadline = RecordedDeadline {
        identity: WorkflowDeadlineId::from_stable(
            StableKey::new(
                &namespace,
                [KeySegment::Text(Arc::from("cluster-deadline"))],
            )
            .map_err(|error| format!("cluster deadline: {error:?}"))?,
        ),
        at: UtcMillis::new(10),
    };
    assert!(matches!(
        block_on(
            ClusterWorkflowEngine::dispatch::<Environment>(WorkflowCommand::WaitDeadline {
                run: run.clone(),
                deadline: deadline.clone(),
                now: UtcMillis::new(0),
            })
            .run(Arc::clone(&environment))
        ),
        Exit::Success(WorkflowReply::DeadlineWait(DeadlineWait::Waiting))
    ));
    assert!(matches!(
        block_on(
            ClusterWorkflowEngine::dispatch::<Environment>(WorkflowCommand::WakeDue {
                run: run.clone(),
                expected: deadline.clone(),
                now: deadline.at,
            })
            .run(Arc::clone(&environment))
        ),
        Exit::Success(WorkflowReply::Snapshot(Some(snapshot)))
            if snapshot.status == WorkflowStatus::Ready
                && snapshot.wake_reason == Some(WorkflowWakeReason::Deadline(deadline.clone()))
    ));
    assert!(matches!(
        block_on(
            ClusterWorkflowEngine::dispatch::<Environment>(WorkflowCommand::ClearDeadline {
                run: run.clone(),
                expected: deadline.clone(),
            })
            .run(Arc::clone(&environment))
        ),
        Exit::Success(WorkflowReply::DeadlineClear(outcome))
            if matches!(outcome.as_ref(), DeadlineClear::Cleared(snapshot) if snapshot.deadline.is_none())
    ));
    let outcome = EncodedWorkflowOutcome {
        kind: WorkflowOutcomeKind::Succeeded,
        payload: Arc::from(&b"result"[..]),
    };
    let completed = ClusterWorkflowEngine::dispatch::<Environment>(WorkflowCommand::Complete {
        run: run.clone(),
        outcome: outcome.clone(),
    });
    assert!(
        matches!(block_on(completed.run(Arc::clone(&environment))), Exit::Success(WorkflowReply::Snapshot(Some(snapshot))) if snapshot.status == WorkflowStatus::Completed(outcome.clone()))
    );
    assert!(
        matches!(block_on(engine.inspect(run).run(Arc::new(()))), Exit::Success(Some(snapshot)) if snapshot.status == WorkflowStatus::Completed(outcome))
    );
    Ok(())
}

#[test]
fn cluster_resume_retries_only_the_supplied_execution_checkpoint_and_acknowledges_completion()
-> Result<(), String> {
    let engine = Arc::new(MemoryWorkflowEngine::new());
    let env = Arc::new(Environment(Arc::clone(&engine)));
    let namespace = KeyNamespace::<Domain>::new("fenced-resume")
        .map_err(|error| format!("resume namespace: {error:?}"))?;
    let run = WorkflowRunId::from_stable(
        StableKey::new(&namespace, [KeySegment::Text("run".into())])
            .map_err(|error| format!("resume identity: {error:?}"))?,
    );
    let definition = WorkflowDefinitionId::new("fenced-resume")
        .map_err(|error| format!("resume definition: {error:?}"))?;
    let version = WorkflowVersion::new(NonZeroU32::MIN);
    assert!(matches!(
        block_on(
            engine
                .register(WorkflowRegistration {
                    id: definition.clone(),
                    version,
                    compatibility: Arc::from(&b"resume"[..])
                })
                .run(Arc::new(()))
        ),
        Exit::Success(())
    ));
    let Exit::Success(ready) = block_on(
        engine
            .start(
                WorkflowHandle {
                    run: run.clone(),
                    definition,
                    version,
                },
                Arc::from([]),
            )
            .run(Arc::new(())),
    ) else {
        return Err(String::from("workflow start must succeed"));
    };
    let Exit::Success(first) = block_on(
        engine
            .transition(run.clone(), ready.checkpoint(), WorkflowStatus::Running)
            .run(Arc::new(())),
    ) else {
        return Err(String::from("workflow must enter running state"));
    };
    assert!(
        matches!(block_on(ClusterWorkflowEngine::dispatch::<Environment>(WorkflowCommand::Resume{run:run.clone(),checkpoint:None}).run(Arc::clone(&env))),Exit::Success(WorkflowReply::Snapshot(Some(snapshot))) if snapshot.checkpoint()==first.checkpoint())
    );
    assert!(
        matches!(block_on(ClusterWorkflowEngine::dispatch::<Environment>(WorkflowCommand::Resume{run:run.clone(),checkpoint:Some(first.checkpoint())}).run(Arc::clone(&env))),Exit::Success(WorkflowReply::Snapshot(Some(snapshot))) if snapshot.status==WorkflowStatus::Ready)
    );
    let Exit::Success(Some(ready)) = block_on(engine.inspect(run.clone()).run(Arc::new(()))) else {
        return Err(String::from("workflow inspection must return the run"));
    };
    let Exit::Success(second) = block_on(
        engine
            .transition(run.clone(), ready.checkpoint(), WorkflowStatus::Running)
            .run(Arc::new(())),
    ) else {
        return Err(String::from("workflow must enter a new running checkpoint"));
    };
    assert!(matches!(
        block_on(
            ClusterWorkflowEngine::dispatch::<Environment>(WorkflowCommand::Resume {
                run: run.clone(),
                checkpoint: Some(first.checkpoint())
            })
            .run(Arc::clone(&env))
        ),
        Exit::Fail(ClusterWorkflowError::Engine(
            WorkflowEngineError::StateConflict
        ))
    ));
    assert!(
        matches!(block_on(engine.inspect(run.clone()).run(Arc::new(()))),Exit::Success(Some(snapshot)) if snapshot.checkpoint()==second.checkpoint())
    );
    let outcome = EncodedWorkflowOutcome {
        kind: WorkflowOutcomeKind::Succeeded,
        payload: Arc::from(&b"done"[..]),
    };
    assert!(matches!(
        block_on(
            engine
                .complete(run.clone(), outcome.clone())
                .run(Arc::new(()))
        ),
        Exit::Success(_)
    ));
    assert!(
        matches!(block_on(ClusterWorkflowEngine::dispatch::<Environment>(WorkflowCommand::Resume{run,checkpoint:Some(first.checkpoint())}).run(env)),Exit::Success(WorkflowReply::Snapshot(Some(snapshot))) if snapshot.status==WorkflowStatus::Completed(outcome.clone()))
    );
    Ok(())
}
