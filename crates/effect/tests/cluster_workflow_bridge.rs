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
fn named_deadline_reschedule_fences_stale_wake_and_recovers_at_boundary() {
    let engine = Arc::new(MemoryWorkflowEngine::new());
    let environment = Arc::new(Environment(Arc::clone(&engine)));
    let namespace = KeyNamespace::<Domain>::new("deadline-runs").unwrap();
    let key = |name: &'static str| {
        StableKey::new(&namespace, [KeySegment::Text(Arc::from(name))]).unwrap()
    };
    let run = WorkflowRunId::from_stable(key("run"));
    let definition = WorkflowDefinitionId::new("deadline-workflow").unwrap();
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
        identity: WorkflowDeadlineId::from_stable(key("first")),
        at: UtcMillis::new(10),
    };
    let second = RecordedDeadline {
        identity: WorkflowDeadlineId::from_stable(key("second")),
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
    .expect("earlier restoration is valid");
    assert!(matches!(
        block_on(early.inspect(run.clone()).run(Arc::new(()))),
        Exit::Success(Some(snapshot)) if snapshot.status == WorkflowStatus::Suspended
    ));
    let due = Arc::new(
        MemoryWorkflowEngine::restore_at(definitions, snapshots, UtcMillis::new(20))
            .expect("due restoration is valid"),
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
}

struct Environment(Arc<MemoryWorkflowEngine>);
impl HasWorkflowEngine for Environment {
    type Engine = MemoryWorkflowEngine;
    fn workflow_engine(&self) -> Arc<Self::Engine> {
        Arc::clone(&self.0)
    }
}

#[test]
fn cluster_workflow_commands_use_the_same_engine_records() {
    let engine = Arc::new(MemoryWorkflowEngine::new());
    let environment = Arc::new(Environment(Arc::clone(&engine)));
    let namespace = KeyNamespace::<Domain>::new("runs").unwrap();
    let key = StableKey::new(&namespace, [KeySegment::Text(Arc::from("one"))]).unwrap();
    let run = WorkflowRunId::from_stable(key);
    let definition = WorkflowDefinitionId::new("workflow").unwrap();
    let version = WorkflowVersion::new(NonZeroU32::new(1).unwrap());
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
            .unwrap(),
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
}
