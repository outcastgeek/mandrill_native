#[path = "support/campaign_report.rs"]
mod campaign_report;

use effect::workflow::{
    EncodedWorkflowOutcome, MemoryWorkflowEngine, WorkflowActivityId, WorkflowDefinitionId,
    WorkflowEngine, WorkflowEngineError, WorkflowHandle, WorkflowOutcomeKind, WorkflowRegistration,
    WorkflowRunId, WorkflowStatus, WorkflowVersion,
};
use effect::{Effect, Exit, JsonValue, KeyDomain, KeyNamespace, KeySegment, StableKey};
use futures_executor::block_on;
use std::num::NonZeroU32;
use std::sync::Arc;

struct CampaignDomain;
impl KeyDomain for CampaignDomain {
    const TAG: &'static str = "workflow-campaign";
}

fn key(namespace: &KeyNamespace<CampaignDomain>, segment: u64) -> StableKey<CampaignDomain> {
    StableKey::new(namespace, [KeySegment::Unsigned(segment)]).expect("stable identity")
}

fn run<A: Send + 'static, E: Send + 'static>(program: Effect<A, E>) -> Result<A, E> {
    match block_on(program.run(Arc::new(()))) {
        Exit::Success(value) => Ok(value),
        Exit::Fail(error) => Err(error),
        Exit::Die(defect) => panic!("workflow defect: {defect:?}"),
        Exit::Interrupted => panic!("workflow interrupted"),
        Exit::FinalizationFailed(_) => panic!("workflow finalization failed"),
    }
}

fn number(config: &JsonValue, path: &[&str]) -> u64 {
    path.iter()
        .fold(config, |value, field| {
            value.get(*field).expect("campaign field")
        })
        .as_u64()
        .expect("campaign integer")
}

fn next(state: &mut u64) -> u64 {
    *state ^= *state << 13;
    *state ^= *state >> 7;
    *state ^= *state << 17;
    *state
}

#[test]
fn configured_workflow_histories_match_replay_and_transition_model() {
    let configured = std::env::var_os("EFFECT_CAMPAIGN_CONFIG")
        .map(|path| std::fs::read_to_string(path).expect("campaign configuration"))
        .unwrap_or_else(|| include_str!("feature_campaigns.json").to_owned());
    let config: JsonValue = serde_json::from_str(&configured).expect("campaign JSON");
    let seeds = number(&config, &["campaigns", "workflow", "seeds"]);
    let transitions = number(&config, &["campaigns", "workflow", "transitions"]);
    let namespace = KeyNamespace::<CampaignDomain>::new("workflow").expect("namespace");
    let definition = WorkflowDefinitionId::new("campaign").expect("definition");
    let version = WorkflowVersion::new(NonZeroU32::new(1).expect("version"));
    let input: Arc<[u8]> = Arc::from(&b"workflow-input"[..]);
    let outcome = EncodedWorkflowOutcome {
        kind: WorkflowOutcomeKind::Succeeded,
        payload: Arc::from(&b"workflow-output"[..]),
    };
    let mut operations = [0_u64; 6];

    for seed in 0..seeds {
        let engine = MemoryWorkflowEngine::new();
        run(engine.register(WorkflowRegistration {
            id: definition.clone(),
            version,
            compatibility: Arc::from(&b"campaign-compatible"[..]),
        }))
        .expect("register definition");
        let handle = WorkflowHandle {
            run: WorkflowRunId::from_stable(key(&namespace, seed)),
            definition: definition.clone(),
            version,
        };
        let first = run(engine.start(handle.clone(), Arc::clone(&input))).expect("start run");
        assert_eq!(first.status, WorkflowStatus::Ready);
        let mut expected_status = WorkflowStatus::Ready;
        let mut activity_count = 0_usize;
        let mut generator = number(&config, &["seed"]).wrapping_add(seed).max(1);

        for step in 0..transitions {
            match next(&mut generator) % operations.len() as u64 {
                0 => {
                    let next_status = match expected_status {
                        WorkflowStatus::Ready => WorkflowStatus::Running,
                        WorkflowStatus::Running => WorkflowStatus::Suspended,
                        WorkflowStatus::Suspended => WorkflowStatus::Ready,
                        _ => unreachable!("terminal status before completion"),
                    };
                    let snapshot = run(engine.transition(
                        handle.run.clone(),
                        run(engine.inspect(handle.run.clone()))
                            .expect("inspect")
                            .expect("run")
                            .checkpoint(),
                        next_status.clone(),
                    ))
                    .expect("valid transition");
                    assert_eq!(snapshot.status, next_status);
                    expected_status = next_status;
                    operations[0] += 1;
                }
                1 => {
                    let replay = run(engine.start(handle.clone(), Arc::clone(&input)))
                        .expect("same-input start replay");
                    assert_eq!(replay.status, expected_status);
                    assert_eq!(replay.activities.len(), activity_count);
                    operations[1] += 1;
                }
                2 => {
                    let conflict = run(engine.start(handle.clone(), Arc::from(&b"other"[..])));
                    assert!(matches!(conflict, Err(WorkflowEngineError::RunConflict)));
                    operations[2] += 1;
                }
                3 => {
                    let activity = WorkflowActivityId::from_stable(key(&namespace, step));
                    let attempt = NonZeroU32::new(1).expect("attempt");
                    let record = run(engine.record_activity(
                        handle.run.clone(),
                        activity.clone(),
                        attempt,
                        Arc::clone(&input),
                        outcome.clone(),
                    ))
                    .expect("record activity");
                    assert_eq!(record.identity, activity);
                    activity_count += 1;
                    operations[3] += 1;
                }
                4 => {
                    let activity = WorkflowActivityId::from_stable(key(&namespace, step));
                    let attempt = NonZeroU32::new(1).expect("attempt");
                    let record = run(engine.record_activity(
                        handle.run.clone(),
                        activity.clone(),
                        attempt,
                        Arc::clone(&input),
                        outcome.clone(),
                    ))
                    .expect("record activity");
                    let replay = run(engine.record_activity(
                        handle.run.clone(),
                        activity,
                        attempt,
                        Arc::clone(&input),
                        outcome.clone(),
                    ))
                    .expect("exact activity replay");
                    assert_eq!(record, replay);
                    activity_count += 1;
                    operations[4] += 1;
                }
                5 => {
                    let activity = WorkflowActivityId::from_stable(key(&namespace, step));
                    let attempt = NonZeroU32::new(1).expect("attempt");
                    run(engine.record_activity(
                        handle.run.clone(),
                        activity.clone(),
                        attempt,
                        Arc::clone(&input),
                        outcome.clone(),
                    ))
                    .expect("record activity");
                    let conflict = run(engine.record_activity(
                        handle.run.clone(),
                        activity,
                        attempt,
                        Arc::from(&b"other"[..]),
                        outcome.clone(),
                    ));
                    assert!(matches!(
                        conflict,
                        Err(WorkflowEngineError::ActivityConflict)
                    ));
                    activity_count += 1;
                    operations[5] += 1;
                }
                _ => unreachable!(),
            }
            let snapshot = run(engine.inspect(handle.run.clone()))
                .expect("inspect run")
                .expect("run remains present");
            assert_eq!(snapshot.status, expected_status);
            assert_eq!(snapshot.activities.len(), activity_count);
        }
        let completed =
            run(engine.complete(handle.run.clone(), outcome.clone())).expect("complete run");
        assert_eq!(completed.status, WorkflowStatus::Completed(outcome.clone()));
        assert_eq!(completed.activities.len(), activity_count);
        let replay = run(engine.complete(handle.run, outcome.clone())).expect("completion replay");
        assert_eq!(replay, completed);
    }

    assert!(operations.iter().all(|count| *count > 0));
    campaign_report::completed(
        "workflow",
        serde_json::json!({
            "seeds": seeds,
            "transitions": transitions,
            "operations": operations,
        }),
    );
}
