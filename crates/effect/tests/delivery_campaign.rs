#[path = "support/campaign_report.rs"]
mod campaign_report;

use effect::cluster::{
    AcceptancePolicy, AcceptedRequest, ActivationId, ActivationOutcome, ClaimOptions, ClaimToken,
    ClusterActivationStore, ClusterAddress, ClusterEntityId, ClusterEntityType,
    ClusterMessageStore, ClusterOwner, ClusterRequestId, ClusterShardId, ClusterStoreError,
    CompletedReply, MemoryClusterMessageStore, MessageAcceptance, MessageDisposition,
    MessageStoreCapacity, PayloadConflictPolicy, ReplyKind, SavedMessageState, SavedReply,
    StoreCompletion,
};
use effect::{Effect, Exit, JsonValue, KeyDomain, KeyNamespace, KeySegment, StableKey, UtcMillis};
use futures_executor::block_on;
use std::num::{NonZeroU32, NonZeroUsize};
use std::sync::Arc;
use std::time::Duration;

struct CampaignDomain;

impl KeyDomain for CampaignDomain {
    const TAG: &'static str = "cluster-delivery-campaign";
}

fn key(namespace: &KeyNamespace<CampaignDomain>, value: KeySegment) -> StableKey<CampaignDomain> {
    StableKey::new(namespace, [value]).expect("stable campaign identity")
}

fn next(state: &mut u64) -> u64 {
    *state ^= *state << 13;
    *state ^= *state >> 7;
    *state ^= *state << 17;
    *state
}

fn run<A: Send + 'static, E: Send + 'static>(program: Effect<A, E>) -> Exit<A, E> {
    block_on(program.run(Arc::new(())))
}

fn success<A: Send + 'static, E: Send + 'static>(program: Effect<A, E>) -> A {
    match run(program) {
        Exit::Success(value) => value,
        _ => panic!("cluster operation failed"),
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

enum Phase {
    Pending,
    Claimed(ClaimToken),
    Completed { retained_until: i64 },
    DeadLetter,
}

impl Phase {
    fn disposition(&self) -> MessageDisposition {
        match self {
            Self::Pending => MessageDisposition::Pending,
            Self::Claimed(_) => MessageDisposition::Claimed,
            Self::Completed { .. } => MessageDisposition::Completed,
            Self::DeadLetter => MessageDisposition::DeadLetter,
        }
    }
}

struct Model {
    payload: u8,
    attempts: u32,
    accepted_at: i64,
    phase: Phase,
}

fn inspect(
    store: &MemoryClusterMessageStore,
    address: &ClusterAddress,
    request: &ClusterRequestId,
    model: &Option<Model>,
) {
    let observed = success(store.inspect(address.clone(), request.clone()));
    match (model, observed) {
        (None, None) => {}
        (Some(expected), Some(actual)) => {
            assert_eq!(actual.payload.as_ref(), &[expected.payload]);
            assert_eq!(actual.accepted_at, UtcMillis::new(expected.accepted_at));
            let disposition = match &actual.state {
                SavedMessageState::Pending => MessageDisposition::Pending,
                SavedMessageState::Claimed(token) => {
                    assert_eq!(token.attempt.get(), expected.attempts);
                    MessageDisposition::Claimed
                }
                SavedMessageState::Completed { retained_until, .. } => {
                    if let Phase::Completed {
                        retained_until: expected_until,
                        ..
                    } = &expected.phase
                    {
                        assert_eq!(*retained_until, UtcMillis::new(*expected_until));
                    }
                    MessageDisposition::Completed
                }
                SavedMessageState::DeadLetter { attempts } => {
                    assert_eq!(attempts.get(), expected.attempts);
                    MessageDisposition::DeadLetter
                }
            };
            assert_eq!(disposition, expected.phase.disposition());
        }
        _ => panic!("cluster inspection differs from independent model"),
    }
}

#[test]
fn configured_delivery_histories_match_acceptance_and_fencing_model() {
    let configured = std::env::var_os("EFFECT_CAMPAIGN_CONFIG")
        .map(|path| std::fs::read_to_string(path).expect("campaign configuration"))
        .unwrap_or_else(|| include_str!("feature_campaigns.json").to_owned());
    let config: JsonValue = serde_json::from_str(&configured).expect("campaign JSON");
    let seeds = number(&config, &["campaigns", "delivery", "seeds"]);
    let transitions = number(&config, &["campaigns", "delivery", "transitions"]);
    let lease = i64::try_from(number(
        &config,
        &["campaigns", "delivery", "claim_lease_millis"],
    ))
    .expect("claim lease fits time");
    let retention = number(&config, &["campaigns", "delivery", "retention_millis"]);
    let max_attempts = NonZeroU32::new(
        u32::try_from(number(
            &config,
            &["campaigns", "delivery", "maximum_attempts"],
        ))
        .expect("attempt policy fits u32"),
    )
    .expect("positive attempt policy");
    assert!(lease > 0);
    let namespace = KeyNamespace::<CampaignDomain>::new("delivery").expect("namespace");
    let address = ClusterAddress {
        entity_type: ClusterEntityType::new("counter").expect("entity type"),
        entity: ClusterEntityId::from_stable(key(
            &namespace,
            KeySegment::Text(Arc::from("entity")),
        )),
        shard: ClusterShardId::from_stable(key(&namespace, KeySegment::Text(Arc::from("shard")))),
    };
    let request =
        ClusterRequestId::from_stable(key(&namespace, KeySegment::Text(Arc::from("request"))));
    let policy = AcceptancePolicy {
        dedup_retention: Duration::from_millis(retention),
        reply_retention: Duration::from_millis(retention),
        payload_conflict: PayloadConflictPolicy::Reject,
        maximum_attempts: max_attempts,
    };
    let mut operations = [0_u64; 7];

    for seed in 0..seeds {
        let store = MemoryClusterMessageStore::new(MessageStoreCapacity::Unbounded);
        let mut generator = number(&config, &["seed"]).wrapping_add(seed).max(1);
        let mut owner_serial = 0_u64;
        let initial_activation =
            ActivationId::from_stable(key(&namespace, KeySegment::Unsigned(owner_serial)));
        let horizon = i64::try_from(transitions)
            .expect("history fits time")
            .checked_add(lease)
            .expect("activation horizon");
        let activated = success(store.activate(
            address.clone(),
            initial_activation,
            UtcMillis::new(0),
            UtcMillis::new(horizon),
        ));
        let ActivationOutcome::Activated {
            lease: activation, ..
        } = activated
        else {
            panic!("initial activation must acquire ownership");
        };
        let mut owner: ClusterOwner = activation.owner;
        let mut model: Option<Model> = None;
        let mut accepted_count = 0_u64;

        for now in 0..transitions {
            let now = i64::try_from(now).expect("history time fits i64");
            let choice = (next(&mut generator) % operations.len() as u64) as usize;
            operations[choice] += 1;
            match choice {
                0 | 1 => {
                    let payload = if choice == 0 {
                        model.as_ref().map_or(0, |entry| entry.payload)
                    } else {
                        model
                            .as_ref()
                            .map_or(1, |entry| entry.payload.wrapping_add(1))
                    };
                    let offered = AcceptedRequest {
                        address: address.clone(),
                        id: request.clone(),
                        payload: Arc::from([payload]),
                        idempotency_key: None,
                        accepted_at: UtcMillis::new(now),
                        policy,
                    };
                    let replace = model.as_ref().is_none_or(|entry| {
                        matches!(
                            entry.phase, Phase::Completed { retained_until, .. }
                                if retained_until <= now
                        )
                    });
                    match run(store.accept(offered)) {
                        Exit::Success(MessageAcceptance::Accepted { order }) if replace => {
                            accepted_count += 1;
                            assert_eq!(order.get(), accepted_count);
                            model = Some(Model {
                                payload,
                                attempts: 0,
                                accepted_at: now,
                                phase: Phase::Pending,
                            });
                        }
                        Exit::Success(MessageAcceptance::Duplicate { disposition })
                            if choice == 0 && !replace =>
                        {
                            assert_eq!(
                                disposition,
                                model.as_ref().expect("record").phase.disposition()
                            );
                        }
                        Exit::Fail(ClusterStoreError::PayloadConflict)
                            if choice == 1 && !replace => {}
                        _ => panic!("cluster acceptance differs from independent model"),
                    }
                }
                2 => {
                    let mut claims = success(store.claim(ClaimOptions {
                        owner: owner.clone(),
                        now: UtcMillis::new(now),
                        lease_until: UtcMillis::new(now + lease),
                        limit: NonZeroUsize::MIN,
                    }));
                    let eligible = model.as_ref().is_some_and(|entry| match &entry.phase {
                        Phase::Pending => true,
                        Phase::Claimed(token) => {
                            token.owner != owner || now >= token.lease_until.value()
                        }
                        Phase::Completed { .. } | Phase::DeadLetter => false,
                    });
                    if eligible {
                        let entry = model.as_mut().expect("eligible record");
                        if entry.attempts >= max_attempts.get() {
                            assert!(claims.is_empty());
                            entry.phase = Phase::DeadLetter;
                        } else {
                            assert_eq!(claims.len(), 1);
                            let claim = claims.pop().expect("one bounded claim");
                            entry.attempts += 1;
                            assert_eq!(claim.token.attempt.get(), entry.attempts);
                            assert_eq!(claim.token.owner, owner);
                            entry.phase = Phase::Claimed(claim.token);
                        }
                    } else {
                        assert!(claims.is_empty());
                    }
                }
                3 => {
                    if let Some(Model {
                        phase: Phase::Claimed(token),
                        ..
                    }) = &model
                    {
                        let token = token.clone();
                        let reply = SavedReply {
                            kind: ReplyKind::Success,
                            payload: Arc::from([42]),
                        };
                        let completion = CompletedReply {
                            token: token.clone(),
                            reply,
                            completed_at: UtcMillis::new(now),
                        };
                        let expected = if token.owner != owner {
                            Err(ClusterStoreError::WrongOwner)
                        } else if now >= token.lease_until.value() {
                            Err(ClusterStoreError::StaleClaim)
                        } else {
                            Ok(StoreCompletion::Completed)
                        };
                        match (run(store.complete(completion)), expected) {
                            (Exit::Success(actual), Ok(expected)) if actual == expected => {
                                model.as_mut().expect("claimed record").phase = Phase::Completed {
                                    retained_until: now
                                        + i64::try_from(retention).expect("retention fits time"),
                                };
                            }
                            (Exit::Fail(actual), Err(expected)) if actual == expected => {}
                            _ => panic!("cluster completion differs from independent model"),
                        }
                    }
                }
                4 => inspect(&store, &address, &request, &model),
                5 => {
                    let expired = model.as_ref().is_some_and(|entry| {
                        matches!(
                            entry.phase, Phase::Completed { retained_until, .. }
                                if retained_until <= now
                        )
                    });
                    let removed = success(store.cleanup(UtcMillis::new(now)));
                    assert_eq!(removed, usize::from(expired));
                    if expired {
                        model = None;
                    }
                }
                6 => {
                    assert!(success(store.abandon(owner.clone(), UtcMillis::new(now))));
                    owner_serial += 1;
                    let activation = ActivationId::from_stable(key(
                        &namespace,
                        KeySegment::Unsigned(owner_serial),
                    ));
                    let acquired = success(store.activate(
                        address.clone(),
                        activation,
                        UtcMillis::new(now),
                        UtcMillis::new(now + horizon),
                    ));
                    let ActivationOutcome::Activated { lease, .. } = acquired else {
                        panic!("abandoned owner must be replaceable");
                    };
                    assert!(lease.owner.generation.value().get() > owner.generation.value().get());
                    owner = lease.owner;
                }
                _ => unreachable!("operation selected from history table"),
            }
            inspect(&store, &address, &request, &model);
        }
    }
    assert!(operations.into_iter().all(|count| count > 0));
    campaign_report::completed(
        "delivery",
        serde_json::json!({
            "seeds": seeds, "transitions": transitions, "operations": operations,
        }),
    );
}
