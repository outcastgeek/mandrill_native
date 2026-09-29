#[path = "support/campaign_report.rs"]
mod campaign_report;

use effect::reactivity::{Atom, AtomId, AtomRef, AtomRefError, AtomRegistry};
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

fn id(namespace: &KeyNamespace<CampaignDomain>, name: &'static str) -> AtomId {
    AtomId::from_stable(
        StableKey::new(namespace, [KeySegment::Text(Arc::from(name))]).expect("atom identity"),
    )
}

fn next(state: &mut u64) -> u64 {
    *state ^= *state << 13;
    *state ^= *state >> 7;
    *state ^= *state << 17;
    *state
}

fn success<A: Send + 'static, E: Send + 'static>(program: Effect<A, E>) -> A {
    match block_on(program.run(Arc::new(()))) {
        Exit::Success(value) => value,
        _ => panic!("reactive operation failed"),
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

#[test]
fn configured_dynamic_graph_histories_match_independent_model() {
    let configured = std::env::var_os("EFFECT_CAMPAIGN_CONFIG")
        .map(|path| std::fs::read_to_string(path).expect("campaign configuration"))
        .unwrap_or_else(|| include_str!("feature_campaigns.json").to_owned());
    let config: JsonValue = serde_json::from_str(&configured).expect("valid campaign JSON");
    let seeds = number(&config, &["campaigns", "reactivity", "seeds"]);
    let transitions = number(&config, &["campaigns", "reactivity", "transitions"]);
    let namespace = KeyNamespace::<CampaignDomain>::new("graph").expect("namespace");
    let options = PubSubOptions {
        capacity: NonZeroUsize::MIN,
        overflow: PubSubOverflow::Backpressure,
        replay: ReplayPolicy::None,
    };
    let mut operations = [0_u64; 5];

    for seed in 0..seeds {
        let registry = AtomRegistry::new(options).expect("registry");
        let left_id = id(&namespace, "left");
        let right_id = id(&namespace, "right");
        let selector_id = id(&namespace, "selector");
        let derived_id = id(&namespace, "selected");
        let left = AtomRef::new(
            registry.clone(),
            left_id.clone(),
            0_i64,
            NonZeroUsize::MIN,
            PubSubOverflow::Backpressure,
        )
        .expect("left source");
        let right = AtomRef::new(
            registry.clone(),
            right_id.clone(),
            0_i64,
            NonZeroUsize::MIN,
            PubSubOverflow::Backpressure,
        )
        .expect("right source");
        let selector = AtomRef::new(
            registry.clone(),
            selector_id.clone(),
            false,
            NonZeroUsize::MIN,
            PubSubOverflow::Backpressure,
        )
        .expect("selector source");
        let derived = Atom::<i64, AtomRefError, ()>::new(
            registry.clone(),
            derived_id.clone(),
            NonZeroUsize::MIN,
            PubSubOverflow::Backpressure,
            {
                let left = left.clone();
                let right = right.clone();
                let selector = selector.clone();
                move |context| {
                    let branches = context.clone();
                    let left = left.clone();
                    let right = right.clone();
                    context.read(&selector).flat_map(move |selected| {
                        if *selected {
                            branches.read(&right).map(|value| *value)
                        } else {
                            branches.read(&left).map(|value| *value)
                        }
                    })
                }
            },
        )
        .expect("derived atom");
        let mut generator = number(&config, &["seed"]).wrapping_add(seed).max(1);
        let (mut expected_left, mut expected_right, mut selected) = (0_i64, 0_i64, false);
        let mut settled = None;

        for _ in 0..transitions {
            let choice = (next(&mut generator) % operations.len() as u64) as usize;
            operations[choice] += 1;
            match choice {
                0 => {
                    expected_left += 1;
                    success(left.set::<()>(expected_left));
                }
                1 => {
                    expected_right += 1;
                    success(right.set::<()>(expected_right));
                }
                2 => {
                    selected = !selected;
                    success(selector.set::<()>(selected));
                }
                3 => {
                    success(derived.refresh());
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
                        registry.dependencies_of(&derived_id).expect("dependencies"),
                        expected_dependencies,
                    );
                }
                4 => {
                    let status = success(derived.read());
                    let observed = status
                        .current_success()
                        .map(|value| value.with(|number| *number).expect("value inspection"));
                    assert_eq!(observed, settled);
                }
                _ => unreachable!("operation chosen from configured set"),
            }
            assert_eq!(*success(left.read::<()>()), expected_left);
            assert_eq!(*success(right.read::<()>()), expected_right);
            assert_eq!(*success(selector.read::<()>()), selected);
        }

        success(derived.dispose(PubSubShutdown::Discard));
        success(left.dispose::<()>(PubSubShutdown::Discard));
        success(right.dispose::<()>(PubSubShutdown::Discard));
        success(selector.dispose::<()>(PubSubShutdown::Discard));
        success(registry.dispose::<()>(PubSubShutdown::Discard));
    }

    assert!(operations.into_iter().all(|count| count > 0));
    campaign_report::completed(
        "reactivity",
        serde_json::json!({
            "seeds": seeds, "transitions": transitions, "operations": operations,
        }),
    );
}
