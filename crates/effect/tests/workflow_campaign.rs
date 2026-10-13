use serde_json::json;
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

fn key(
    namespace: &KeyNamespace<CampaignDomain>,
    segment: u64,
) -> Result<StableKey<CampaignDomain>, String> {
    StableKey::new(namespace, [KeySegment::Unsigned(segment)])
        .map_err(|error| format!("stable identity: {error:?}"))
}

fn run<A: Send + 'static, E: Send + 'static>(
    program: Effect<A, E>,
) -> Result<Result<A, E>, String> {
    match block_on(program.run(Arc::new(()))) {
        Exit::Success(value) => Ok(Ok(value)),
        Exit::Fail(error) => Ok(Err(error)),
        Exit::Die(defect) => Err(format!("workflow defect: {defect:?}")),
        Exit::Interrupted => Err("workflow interrupted".into()),
        Exit::FinalizationFailed(_) => Err("workflow finalization failed".into()),
    }
}

fn number(config: &JsonValue, path: &[&str]) -> Result<u64, String> {
    path.iter()
        .try_fold(config, |value, field| {
            value
                .get(*field)
                .ok_or_else(|| format!("campaign field: {field}"))
        })?
        .as_u64()
        .ok_or_else(|| format!("campaign integer: {path:?}"))
}

fn next(state: &mut u64) -> u64 {
    *state ^= *state << 13;
    *state ^= *state >> 7;
    *state ^= *state << 17;
    *state
}

#[test]
fn configured_workflow_histories_match_replay_and_transition_model() -> Result<(), String> {
    let configured = std::env::var_os("EFFECT_CAMPAIGN_CONFIG")
        .map(std::fs::read_to_string)
        .transpose()
        .map_err(|error| format!("campaign configuration: {error}"))?
        .unwrap_or_else(|| include_str!("feature_campaigns.json").to_owned());
    let config: JsonValue =
        serde_json::from_str(&configured).map_err(|error| format!("campaign JSON: {error}"))?;
    let seeds = number(&config, &["campaigns", "workflow", "seeds"])?;
    let transitions = number(&config, &["campaigns", "workflow", "transitions"])?;
    let namespace = KeyNamespace::<CampaignDomain>::new("workflow")
        .map_err(|error| format!("namespace: {error:?}"))?;
    let definition =
        WorkflowDefinitionId::new("campaign").map_err(|error| format!("definition: {error:?}"))?;
    let version = WorkflowVersion::new(NonZeroU32::MIN);
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
        }))?
        .map_err(|error| format!("register definition: {error:?}"))?;
        let handle = WorkflowHandle {
            run: WorkflowRunId::from_stable(key(&namespace, seed)?),
            definition: definition.clone(),
            version,
        };
        let first = run(engine.start(handle.clone(), Arc::clone(&input)))?
            .map_err(|error| format!("start run: {error:?}"))?;
        assert_eq!(first.status, WorkflowStatus::Ready);
        let mut expected_status = WorkflowStatus::Ready;
        let mut activity_count = 0_usize;
        let mut generator = number(&config, &["seed"])?.wrapping_add(seed).max(1);

        for step in 0..transitions {
            match next(&mut generator) % operations.len() as u64 {
                0 => {
                    let next_status = match expected_status {
                        WorkflowStatus::Ready => WorkflowStatus::Running,
                        WorkflowStatus::Running => WorkflowStatus::Suspended,
                        WorkflowStatus::Suspended => WorkflowStatus::Ready,
                        WorkflowStatus::Completed(_)
                        | WorkflowStatus::Cancelled
                        | WorkflowStatus::Abandoned => {
                            return Err("terminal status before completion".into());
                        }
                    };
                    let snapshot = run(engine.transition(
                        handle.run.clone(),
                        run(engine.inspect(handle.run.clone()))?
                            .map_err(|error| format!("inspect: {error:?}"))?
                            .ok_or("missing run")?
                            .checkpoint(),
                        next_status.clone(),
                    ))?
                    .map_err(|error| format!("transition: {error:?}"))?;
                    assert_eq!(snapshot.status, next_status);
                    expected_status = next_status;
                    operations[0] += 1;
                }
                1 => {
                    let replay = run(engine.start(handle.clone(), Arc::clone(&input)))?
                        .map_err(|error| format!("start replay: {error:?}"))?;
                    assert_eq!(replay.status, expected_status);
                    assert_eq!(replay.activities.len(), activity_count);
                    operations[1] += 1;
                }
                2 => {
                    let conflict = run(engine.start(handle.clone(), Arc::from(&b"other"[..])))?;
                    assert!(matches!(conflict, Err(WorkflowEngineError::RunConflict)));
                    operations[2] += 1;
                }
                3 => {
                    let activity = WorkflowActivityId::from_stable(key(&namespace, step)?);
                    let attempt = NonZeroU32::MIN;
                    let record = run(engine.record_activity(
                        handle.run.clone(),
                        activity.clone(),
                        attempt,
                        Arc::clone(&input),
                        outcome.clone(),
                    ))?
                    .map_err(|error| format!("record activity: {error:?}"))?;
                    assert_eq!(record.identity, activity);
                    activity_count += 1;
                    operations[3] += 1;
                }
                4 => {
                    let activity = WorkflowActivityId::from_stable(key(&namespace, step)?);
                    let attempt = NonZeroU32::MIN;
                    let record = run(engine.record_activity(
                        handle.run.clone(),
                        activity.clone(),
                        attempt,
                        Arc::clone(&input),
                        outcome.clone(),
                    ))?
                    .map_err(|error| format!("record activity: {error:?}"))?;
                    let replay = run(engine.record_activity(
                        handle.run.clone(),
                        activity,
                        attempt,
                        Arc::clone(&input),
                        outcome.clone(),
                    ))?
                    .map_err(|error| format!("activity replay: {error:?}"))?;
                    assert_eq!(record, replay);
                    activity_count += 1;
                    operations[4] += 1;
                }
                5 => {
                    let activity = WorkflowActivityId::from_stable(key(&namespace, step)?);
                    let attempt = NonZeroU32::MIN;
                    run(engine.record_activity(
                        handle.run.clone(),
                        activity.clone(),
                        attempt,
                        Arc::clone(&input),
                        outcome.clone(),
                    ))?
                    .map_err(|error| format!("record activity: {error:?}"))?;
                    let conflict = run(engine.record_activity(
                        handle.run.clone(),
                        activity,
                        attempt,
                        Arc::from(&b"other"[..]),
                        outcome.clone(),
                    ))?;
                    assert!(matches!(
                        conflict,
                        Err(WorkflowEngineError::ActivityConflict)
                    ));
                    activity_count += 1;
                    operations[5] += 1;
                }
                _ => return Err("operation outside configured set".into()),
            }
            let snapshot = run(engine.inspect(handle.run.clone()))?
                .map_err(|error| format!("inspect run: {error:?}"))?
                .ok_or("run no longer present")?;
            assert_eq!(snapshot.status, expected_status);
            assert_eq!(snapshot.activities.len(), activity_count);
        }
        let completed = run(engine.complete(handle.run.clone(), outcome.clone()))?
            .map_err(|error| format!("complete run: {error:?}"))?;
        assert_eq!(completed.status, WorkflowStatus::Completed(outcome.clone()));
        assert_eq!(completed.activities.len(), activity_count);
        let replay = run(engine.complete(handle.run, outcome.clone()))?
            .map_err(|error| format!("completion replay: {error:?}"))?;
        assert_eq!(replay, completed);
    }

    assert!(operations.iter().all(|count| *count > 0));
    campaign_report::completed(
        "workflow",
        json!({
            "seeds": seeds,
            "transitions": transitions,
            "operations": operations,
        }),
    )?;
    Ok(())
}
