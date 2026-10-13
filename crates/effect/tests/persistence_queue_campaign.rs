use serde_json::json;
#[path = "support/campaign_report.rs"]
mod campaign_report;

use effect::persistence::{
    MemoryPersistedQueueStore, PersistedQueueName, PersistedQueueStore, QueueClaimOptions,
    QueueClaimToken, QueueDeadLetterReason, QueueDisposition, QueueMessageId, QueueMessagePolicy,
    QueueOffer, QueueOfferOutcome, QueueOwner, QueueRecordState, QueueRequeue,
    QueueRequeueAttempts, QueueRequeueOutcome, QueueRetry, QueueSettlement, QueueSettlementAction,
    QueueStoreCapacity,
};
use effect::{Effect, Exit, JsonValue, KeyDomain, KeyNamespace, KeySegment, StableKey, UtcMillis};
use futures_executor::block_on;
use std::io::Write;
use std::num::{NonZeroU32, NonZeroU64, NonZeroUsize};
use std::sync::Arc;
use std::time::{Duration, Instant};

struct CampaignDomain;
impl KeyDomain for CampaignDomain {
    const TAG: &'static str = "persistence-queue-campaign";
}

fn next(state: &mut u64) -> u64 {
    *state ^= *state << 13;
    *state ^= *state >> 7;
    *state ^= *state << 17;
    *state
}

fn success<A: Send + 'static, E: std::fmt::Debug + Send + 'static>(
    effect: Effect<A, E>,
) -> Result<A, String> {
    match block_on(effect.run(Arc::new(()))) {
        Exit::Success(value) => Ok(value),
        Exit::Fail(error) => Err(format!("queue failure: {error:?}")),
        Exit::Die(defect) => Err(format!("queue defect: {defect:?}")),
        Exit::Interrupted => Err("queue interrupted".into()),
        Exit::FinalizationFailed(error) => Err(format!("queue finalization: {error:?}")),
    }
}

enum Phase {
    Ready,
    Claimed {
        token: QueueClaimToken,
        at: i64,
        until: i64,
    },
    Acknowledged {
        retained_until: i64,
    },
    DeadLetter,
}

impl Phase {
    fn disposition(&self) -> QueueDisposition {
        match self {
            Self::Ready => QueueDisposition::Ready,
            Self::Claimed { .. } => QueueDisposition::Claimed,
            Self::Acknowledged { .. } => QueueDisposition::Acknowledged,
            Self::DeadLetter => QueueDisposition::DeadLetter,
        }
    }
}

struct Model {
    payload: u8,
    sequence: u64,
    attempts: u32,
    available: i64,
    phase: Phase,
}

fn inspect(
    store: &MemoryPersistedQueueStore,
    queue: &PersistedQueueName,
    id: &QueueMessageId,
    model: &Option<Model>,
) -> Result<(), String> {
    let observed = success(store.inspect(queue.clone(), id.clone()))?;
    match (model, observed) {
        (None, None) => {}
        (Some(expected), Some(record)) => {
            assert_eq!(record.payload.as_ref(), &[expected.payload]);
            assert_eq!(record.sequence.get(), expected.sequence);
            assert_eq!(record.attempts, expected.attempts);
            assert_eq!(record.available_at, UtcMillis::new(expected.available));
            assert_eq!(record.state.disposition(), expected.phase.disposition());
            match (&expected.phase, &record.state) {
                (
                    Phase::Claimed { token, at, until },
                    QueueRecordState::Claimed {
                        token: actual,
                        claimed_at,
                        lease_until,
                    },
                ) => {
                    assert_eq!(actual, token);
                    assert_eq!(*claimed_at, UtcMillis::new(*at));
                    assert_eq!(*lease_until, UtcMillis::new(*until));
                }
                (
                    Phase::Acknowledged { retained_until },
                    QueueRecordState::Acknowledged {
                        retained_until: actual,
                    },
                ) => assert_eq!(*actual, UtcMillis::new(*retained_until)),
                _ => {}
            }
        }
        _ => return Err("queue inspection disagrees with the independent history model".into()),
    }
    Ok(())
}

#[test]
fn configured_queue_histories_match_independent_state_model()
-> Result<(), Box<dyn std::error::Error>> {
    let configured = std::env::var_os("EFFECT_CAMPAIGN_CONFIG")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/feature_campaigns.json")
        });
    let config: JsonValue = std::fs::read_to_string(configured)?.parse()?;
    let number = |path: &[&str]| -> Result<u64, String> {
        path.iter()
            .try_fold(&config, |value, field| {
                value
                    .get(*field)
                    .ok_or_else(|| format!("missing campaign field {field}"))
            })?
            .as_u64()
            .ok_or_else(|| format!("campaign integer: {path:?}"))
    };
    let seeds = number(&["campaigns", "persistence", "seeds"])?;
    let transitions = number(&["campaigns", "persistence", "transitions"])?;
    let maximum_attempts = NonZeroU32::new(u32::try_from(number(&[
        "campaigns",
        "persistence",
        "queue",
        "maximum_attempts",
    ])?)?)
    .ok_or("zero attempt policy")?;
    let lease = i64::try_from(number(&[
        "campaigns",
        "persistence",
        "queue",
        "lease_millis",
    ])?)?;
    let retention = i64::try_from(number(&[
        "campaigns",
        "persistence",
        "queue",
        "retention_millis",
    ])?)?;
    let seconds = number(&["budgets", "seconds"])?;
    assert!(seeds >= 1_000 && transitions >= 100 && maximum_attempts.get() > 1);
    assert!(lease > 0 && retention > 0 && seconds > 0);
    let policy = QueueMessagePolicy {
        maximum_attempts,
        acknowledged_retention: Duration::from_millis(retention as u64),
    };
    let namespace = KeyNamespace::<CampaignDomain>::new("queue")
        .map_err(|error| format!("namespace: {error:?}"))?;
    let id = QueueMessageId::from_key(
        StableKey::new(&namespace, [KeySegment::Text(Arc::from("message"))])
            .map_err(|error| format!("campaign key: {error:?}"))?,
    );
    let queue =
        PersistedQueueName::new("campaign").map_err(|error| format!("queue name: {error:?}"))?;
    let owners = [
        QueueOwner::new("first").map_err(|error| format!("first owner: {error:?}"))?,
        QueueOwner::new("second").map_err(|error| format!("second owner: {error:?}"))?,
    ];
    let started = Instant::now();
    let mut operations = [0_u64; 5];
    for seed in 0..seeds {
        let store = MemoryPersistedQueueStore::new(QueueStoreCapacity::Unbounded);
        let mut generator = number(&["seed"])?.wrapping_add(seed).max(1);
        let mut model: Option<Model> = None;
        let mut sequence = 0_u64;
        let mut last_generation = 0_u64;
        let mut now = 0_i64;
        for _ in 0..transitions {
            let choice = (next(&mut generator) % operations.len() as u64) as usize;
            *operations
                .get_mut(choice)
                .ok_or("invalid operation index")? += 1;
            match choice {
                0 => {
                    let payload = next(&mut generator) as u8;
                    let outcome = success(store.offer(QueueOffer::new(
                        queue.clone(),
                        id.clone(),
                        Arc::from([payload]),
                        policy,
                        UtcMillis::new(now),
                        UtcMillis::new(now),
                    )))?;
                    let replace = model.as_ref().is_none_or(|record| {
                        matches!(
                            record.phase,
                            Phase::Acknowledged { retained_until } if retained_until <= now
                        )
                    });
                    if replace {
                        sequence += 1;
                        assert_eq!(
                            outcome,
                            QueueOfferOutcome::Accepted {
                                sequence: NonZeroU64::new(sequence).ok_or("zero sequence")?
                            }
                        );
                        model = Some(Model {
                            payload,
                            sequence,
                            attempts: 0,
                            available: now,
                            phase: Phase::Ready,
                        });
                    } else {
                        let record = model.as_ref().ok_or("missing model record")?;
                        let expected = if record.payload == payload {
                            QueueOfferOutcome::Duplicate {
                                disposition: record.phase.disposition(),
                            }
                        } else {
                            QueueOfferOutcome::ConflictingId
                        };
                        assert_eq!(outcome, expected);
                    }
                }
                1 => {
                    let owner = owners
                        .get((next(&mut generator) as usize) % owners.len())
                        .ok_or("invalid owner index")?
                        .clone();
                    let claims = success(
                        store.claim(
                            queue.clone(),
                            QueueClaimOptions::new(
                                owner.clone(),
                                UtcMillis::new(now),
                                UtcMillis::new(now + lease),
                                NonZeroUsize::MIN,
                            )
                            .map_err(|error| format!("claim interval: {error:?}"))?,
                        ),
                    )?;
                    let eligible = model.as_ref().is_some_and(|record| {
                        record.available <= now
                            && match &record.phase {
                                Phase::Ready => true,
                                Phase::Claimed { until, .. } => *until <= now,
                                Phase::Acknowledged { .. } | Phase::DeadLetter => false,
                            }
                    });
                    if !eligible {
                        assert!(claims.is_empty());
                    } else {
                        let record = model.as_mut().ok_or("missing eligible record")?;
                        if record.attempts >= maximum_attempts.get() {
                            assert!(claims.is_empty());
                            record.phase = Phase::DeadLetter;
                        } else {
                            assert_eq!(claims.len(), 1);
                            let claim = claims.first().ok_or("missing bounded claim")?;
                            last_generation += 1;
                            assert_eq!(claim.token.generation().get(), last_generation);
                            assert_eq!(claim.token.owner(), &owner);
                            assert_eq!(claim.payload.as_ref(), &[record.payload]);
                            assert_eq!(claim.attempt.get(), record.attempts + 1);
                            record.attempts += 1;
                            record.phase = Phase::Claimed {
                                token: claim.token.clone(),
                                at: now,
                                until: now + lease,
                            };
                        }
                    }
                }
                2 => {
                    let current = model.as_ref().and_then(|record| match &record.phase {
                        Phase::Claimed { token, until, .. } if now < *until => Some(token.clone()),
                        Phase::Claimed { .. }
                        | Phase::Ready
                        | Phase::Acknowledged { .. }
                        | Phase::DeadLetter => None,
                    });
                    let token = current.clone().map_or_else(
                        || -> Result<_, &'static str> {
                            Ok(QueueClaimToken::new(
                                queue.clone(),
                                id.clone(),
                                owners.first().ok_or("missing first owner")?.clone(),
                                NonZeroU64::MAX,
                            ))
                        },
                        Ok,
                    )?;
                    let action = next(&mut generator) % 3;
                    let settlement = success(
                        store.settle(
                            token,
                            match action {
                                0 => QueueSettlementAction::Acknowledge {
                                    at: UtcMillis::new(now),
                                },
                                1 => QueueSettlementAction::Retry(
                                    QueueRetry::new(
                                        UtcMillis::new(now),
                                        UtcMillis::new(now + lease),
                                    )
                                    .map_err(|error| format!("retry interval: {error:?}"))?,
                                ),
                                _ => QueueSettlementAction::DeadLetter {
                                    at: UtcMillis::new(now),
                                    reason: QueueDeadLetterReason::Administrative(Arc::from(
                                        "campaign",
                                    )),
                                },
                            },
                        ),
                    )?;
                    let expected = if current.is_some() {
                        QueueSettlement::Applied
                    } else if model.is_some() {
                        QueueSettlement::StaleClaim
                    } else {
                        QueueSettlement::Missing
                    };
                    assert_eq!(settlement, expected);
                    if current.is_some() {
                        let record = model.as_mut().ok_or("missing claimed model record")?;
                        record.phase = match action {
                            0 => Phase::Acknowledged {
                                retained_until: now + retention,
                            },
                            1 => {
                                record.available = now + lease;
                                Phase::Ready
                            }
                            _ => Phase::DeadLetter,
                        };
                    }
                }
                3 => {
                    let reset = next(&mut generator) & 1 == 0;
                    let outcome = success(
                        store.requeue(
                            queue.clone(),
                            id.clone(),
                            QueueRequeue::new(
                                UtcMillis::new(now),
                                UtcMillis::new(now + lease),
                                if reset {
                                    QueueRequeueAttempts::Reset
                                } else {
                                    QueueRequeueAttempts::Preserve
                                },
                            )
                            .map_err(|error| format!("requeue interval: {error:?}"))?,
                        ),
                    )?;
                    let expected = match model.as_mut() {
                        None => QueueRequeueOutcome::Missing,
                        Some(record) if matches!(record.phase, Phase::DeadLetter) => {
                            if !reset && record.attempts >= maximum_attempts.get() {
                                QueueRequeueOutcome::AttemptsExhausted
                            } else {
                                if reset {
                                    record.attempts = 0;
                                }
                                record.available = now + lease;
                                record.phase = Phase::Ready;
                                QueueRequeueOutcome::Requeued
                            }
                        }
                        Some(_) => QueueRequeueOutcome::NotDeadLetter,
                    };
                    assert_eq!(outcome, expected);
                }
                4 => {
                    let removed = success(store.cleanup(queue.clone(), UtcMillis::new(now)))?;
                    let expired = model.as_ref().is_some_and(|record| {
                        matches!(
                            record.phase,
                            Phase::Acknowledged { retained_until } if retained_until <= now
                        )
                    });
                    assert_eq!(removed, usize::from(expired));
                    if expired {
                        model = None;
                    }
                }
                _ => return Err("invalid operation index".into()),
            }
            inspect(&store, &queue, &id, &model)?;
            let advance = next(&mut generator) % (lease as u64 + 2);
            now += i64::try_from(advance)?;
            assert!(
                started.elapsed().as_secs() <= seconds,
                "queue campaign timed out"
            );
        }
    }
    assert!(operations.into_iter().all(|count| count > 0));
    campaign_report::completed(
        "persistence_queue",
        json!({
            "seeds": seeds, "transitions": transitions, "operations": operations,
        }),
    )?;
    writeln!(
        std::io::stderr().lock(),
        "queue model completed {seeds} seeds x {transitions} transitions; operations={operations:?}"
    )?;
    Ok(())
}
