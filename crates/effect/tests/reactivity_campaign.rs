use effect::{effect, effect_map, reactive_derived, reactive_source};
use serde_json::json;
#[path = "support/campaign_report.rs"]
mod campaign_report;

use effect::reactivity::{Atom, AtomId, AtomRefError, AtomRegistry};
use effect::{
    Effect, Exit, JsonValue, KeyDomain, KeyNamespace, KeySegment, PubSubOptions, PubSubOverflow,
    PubSubShutdown, ReplayPolicy, StableKey,
};
use futures_executor::block_on;
use std::collections::BTreeSet;
use std::num::NonZeroUsize;
use std::sync::Arc;

struct CampaignDomain;

impl KeyDomain for CampaignDomain {
    const TAG: &'static str = "reactivity-campaign";
}

fn id(namespace: &KeyNamespace<CampaignDomain>, name: &'static str) -> Result<AtomId, String> {
    StableKey::new(namespace, [KeySegment::Text(Arc::from(name))])
        .map(AtomId::from_stable)
        .map_err(|error| format!("atom identity: {error:?}"))
}

fn next(state: &mut u64) -> u64 {
    *state ^= *state << 13;
    *state ^= *state >> 7;
    *state ^= *state << 17;
    *state
}

fn success<A: Send + 'static, E: std::fmt::Debug + Send + 'static>(
    program: Effect<A, E>,
) -> Result<A, String> {
    match block_on(program.run(Arc::new(()))) {
        Exit::Success(value) => Ok(value),
        Exit::Fail(error) => Err(format!("reactive failure: {error:?}")),
        Exit::Die(defect) => Err(format!("reactive defect: {defect:?}")),
        Exit::Interrupted => Err("reactive operation interrupted".into()),
        Exit::FinalizationFailed(error) => Err(format!("reactive finalization: {error:?}")),
    }
}

fn number(config: &JsonValue, path: &[&str]) -> Result<u64, String> {
    path.iter()
        .try_fold(config, |value, field| {
            value
                .get(*field)
                .ok_or_else(|| format!("missing campaign field: {field}"))
        })?
        .as_u64()
        .ok_or_else(|| format!("campaign integer: {path:?}"))
}

#[test]
fn configured_dynamic_graph_histories_match_independent_model() -> Result<(), String> {
    let configured = std::env::var_os("EFFECT_CAMPAIGN_CONFIG")
        .map(std::fs::read_to_string)
        .transpose()
        .map_err(|error| format!("campaign configuration: {error}"))?
        .unwrap_or_else(|| include_str!("feature_campaigns.json").to_owned());
    let config: JsonValue =
        serde_json::from_str(&configured).map_err(|error| format!("campaign JSON: {error}"))?;
    let seeds = number(&config, &["campaigns", "reactivity", "seeds"])?;
    let transitions = number(&config, &["campaigns", "reactivity", "transitions"])?;
    let namespace = KeyNamespace::<CampaignDomain>::new("graph")
        .map_err(|error| format!("namespace: {error:?}"))?;
    let options = PubSubOptions {
        capacity: NonZeroUsize::MIN,
        overflow: PubSubOverflow::Backpressure,
        replay: ReplayPolicy::None,
    };
    let mut operations = [0_u64; 5];

    for seed in 0..seeds {
        let registry =
            AtomRegistry::new(options).map_err(|error| format!("registry: {error:?}"))?;
        let left_id = id(&namespace, "left")?;
        let right_id = id(&namespace, "right")?;
        let selector_id = id(&namespace, "selector")?;
        let derived_id = id(&namespace, "selected")?;
        let left = reactive_source! { registry: registry.clone(), id: left_id.clone(), initial: 0_i64, capacity: NonZeroUsize::MIN, overflow: PubSubOverflow::Backpressure,  }
        .map_err(|error| format!("left source: {error:?}"))?;
        let right = reactive_source! { registry: registry.clone(), id: right_id.clone(), initial: 0_i64, capacity: NonZeroUsize::MIN, overflow: PubSubOverflow::Backpressure,  }
        .map_err(|error| format!("right source: {error:?}"))?;
        let selector = reactive_source! { registry: registry.clone(), id: selector_id.clone(), initial: false, capacity: NonZeroUsize::MIN, overflow: PubSubOverflow::Backpressure,  }
        .map_err(|error| format!("selector source: {error:?}"))?;
        let derived = { let __dsl_program: Result<Atom<i64, AtomRefError, ()>, effect::reactivity::AtomRegistryError> = reactive_derived! { registry: registry.clone(), id: derived_id.clone(), capacity: NonZeroUsize::MIN, overflow: PubSubOverflow::Backpressure, evaluate: {
                let left = left.clone();
                let right = right.clone();
                let selector = selector.clone();
                move |context| {
                    let branches = context.clone();
                    let left = left.clone();
                    let right = right.clone();
                    effect! {
 bind;
 program: context.read(&selector),
 body: move |selected| {
                        if *selected {
                            effect_map! { program: branches.read(&right), map: |value| *value, }
                        } else {
                            effect_map! { program: branches.read(&left), map: |value| *value, }
                        }
                    },
 }
                }
            },  }; __dsl_program }
        .map_err(|error| format!("derived atom: {error:?}"))?;
        let mut generator = number(&config, &["seed"])?.wrapping_add(seed).max(1);
        let (mut expected_left, mut expected_right, mut selected) = (0_i64, 0_i64, false);
        let mut settled = None;

        for _ in 0..transitions {
            let choice = (next(&mut generator) % operations.len() as u64) as usize;
            *operations
                .get_mut(choice)
                .ok_or("operation outside configured set")? += 1;
            match choice {
                0 => {
                    expected_left += 1;
                    success(left.set::<()>(expected_left))?;
                }
                1 => {
                    expected_right += 1;
                    success(right.set::<()>(expected_right))?;
                }
                2 => {
                    selected = !selected;
                    success(selector.set::<()>(selected))?;
                }
                3 => {
                    success(derived.refresh())?;
                    settled = Some(if selected {
                        expected_right
                    } else {
                        expected_left
                    });
                    let expected_dependencies = BTreeSet::from([
                        selector_id.clone(),
                        if selected {
                            right_id.clone()
                        } else {
                            left_id.clone()
                        },
                    ]);
                    assert_eq!(
                        registry
                            .dependencies_of(&derived_id)
                            .map_err(|error| format!("dependencies: {error:?}"))?,
                        expected_dependencies,
                    );
                }
                4 => {
                    let status = success(derived.read())?;
                    let observed = status
                        .current_success()
                        .map(|value| value.with(|number| *number))
                        .transpose()
                        .map_err(|error| format!("value inspection: {error:?}"))?;
                    assert_eq!(observed, settled);
                }
                _ => return Err("operation outside configured set".into()),
            }
            assert_eq!(*success(left.read::<()>())?, expected_left);
            assert_eq!(*success(right.read::<()>())?, expected_right);
            assert_eq!(*success(selector.read::<()>())?, selected);
        }

        success(derived.dispose(PubSubShutdown::Discard))?;
        success(left.dispose::<()>(PubSubShutdown::Discard))?;
        success(right.dispose::<()>(PubSubShutdown::Discard))?;
        success(selector.dispose::<()>(PubSubShutdown::Discard))?;
        success(registry.dispose::<()>(PubSubShutdown::Discard))?;
    }

    assert!(operations.into_iter().all(|count| count > 0));
    campaign_report::completed(
        "reactivity",
        json!({
            "seeds": seeds, "transitions": transitions, "operations": operations,
        }),
    )?;
    Ok(())
}
