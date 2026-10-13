use effect::persistable;
use serde_json::json;
#[path = "support/campaign_report.rs"]
mod campaign_report;

use effect::persistence::{
    CompareExchange, ConditionalKeyValueStore, HasKeyValueStore, KeyValueStore,
    MemoryKeyValueStore, Persistable, PersistenceError, RecordVersion, StoreCapacity, StoreEntry,
    StoreRead, StoreRevision, StoredLookup, StoredValue, read_many,
};
use effect::{
    Effect, Exit, JsonValue, KeyDomain, KeyNamespace, KeySegment, ParseLimits, StoredOutcome,
    UtcMillis, json_i64,
};
use futures_executor::block_on;
use std::io::Write;
use std::num::NonZeroUsize;
use std::sync::Arc;
use std::time::Instant;

struct Domain;
impl KeyDomain for Domain {
    const TAG: &'static str = "persistence-record-campaign";
}

struct Environment(Arc<MemoryKeyValueStore>);
impl HasKeyValueStore for Environment {
    type Store = MemoryKeyValueStore;
    fn key_value_store(&self) -> Arc<Self::Store> {
        Arc::clone(&self.0)
    }
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
        Exit::Fail(error) => Err(format!("record failure: {error:?}")),
        Exit::Die(defect) => Err(format!("record defect: {defect:?}")),
        Exit::Interrupted => Err("record interrupted".into()),
        Exit::FinalizationFailed(error) => Err(format!("record finalization: {error:?}")),
    }
}

#[derive(Clone)]
struct ModelEntry {
    bytes: Arc<[u8]>,
    expires_at: Option<UtcMillis>,
    revision: StoreRevision,
    decoded: Option<Result<i64, i64>>,
}

impl ModelEntry {
    fn stored(&self) -> StoreEntry {
        StoreEntry {
            value: StoredValue {
                bytes: Arc::clone(&self.bytes),
                expires_at: self.expires_at,
            },
            revision: self.revision,
        }
    }

    fn read(&self, now: UtcMillis) -> StoreRead {
        if self.expires_at.is_some_and(|expires| now >= expires) {
            StoreRead::Expired(self.stored())
        } else {
            StoreRead::Found(self.stored())
        }
    }
}

fn verify_typed_batch(
    environment: Arc<Environment>,
    definition: &Persistable<usize, i64, i64, Domain>,
    model: &[Option<ModelEntry>],
    now: UtcMillis,
) -> Result<(), String> {
    let count = model.len();
    let Exit::Success(actual) = block_on(
        read_many::<Environment, usize, i64, i64, Domain>(
            definition.clone(),
            move || (0..count).collect(),
            now,
        )
        .run(environment),
    ) else {
        return Err("typed batch availability failure is not injected in this model".into());
    };
    assert_eq!(actual.len(), model.len());
    for (expected, result) in model.iter().zip(actual) {
        match (expected, result) {
            (None, Ok(StoredLookup::Missing)) => {}
            (
                Some(entry),
                Ok(StoredLookup::Expired {
                    revision,
                    expires_at,
                }),
            ) if entry.expires_at == Some(expires_at)
                && entry.revision == revision
                && now >= expires_at => {}
            (Some(entry), Ok(StoredLookup::Found { revision, outcome }))
                if entry.revision == revision && entry.expires_at.is_none_or(|at| now < at) =>
            {
                match (entry.decoded, outcome) {
                    (Some(Ok(expected)), StoredOutcome::Success(actual)) => {
                        assert_eq!(actual, expected)
                    }
                    (Some(Err(expected)), StoredOutcome::Failure(actual)) => {
                        assert_eq!(actual, expected)
                    }
                    _ => return Err("typed outcome differs from independent record model".into()),
                }
            }
            (Some(entry), Err(PersistenceError::Corrupt(_)))
                if entry.decoded.is_none() && entry.expires_at.is_none_or(|at| now < at) => {}
            _ => return Err("typed batch outcome differs from independent record model".into()),
        }
    }
    Ok(())
}

#[test]
fn configured_record_histories_match_expiry_and_batch_model()
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
    let keys_count = usize::try_from(number(&["campaigns", "persistence", "records", "keys"])?)?;
    let ttl = i64::try_from(number(&[
        "campaigns",
        "persistence",
        "records",
        "ttl_millis",
    ])?)?;
    let seconds = number(&["budgets", "seconds"])?;
    assert!(seeds >= 1_000 && transitions >= 100 && keys_count >= 2 && ttl > 0);
    let nonzero = |value| {
        assert!(value > 0, "zero campaign budget");
        NonZeroUsize::MIN.saturating_add(value - 1)
    };
    let limits = ParseLimits {
        bytes: nonzero(number(&["budgets", "bytes"])? as usize),
        depth: nonzero(number(&["budgets", "depth"])? as usize),
        nodes: nonzero(number(&["budgets", "steps"])? as usize),
        collection: nonzero(number(&["budgets", "width"])? as usize),
        issues: nonzero(number(&["budgets", "width"])? as usize),
    };
    let namespace =
        KeyNamespace::<Domain>::new("campaign").map_err(|error| format!("namespace: {error:?}"))?;
    let definition = persistable! { native; namespace: namespace, request: usize, version: RecordVersion::new("campaign-v1").map_err(|error| format!("record version: {error:?}"))?, key: |index| Ok(vec![KeySegment::Text(Arc::from(index.to_string()))]), success: json_i64(), error: json_i64(), limits: limits, };
    let keys: Vec<Arc<[u8]>> = (0..keys_count)
        .map(|index| definition.key(&index).map(effect::StableKey::into_bytes))
        .collect::<Result<_, _>>()
        .map_err(|error| format!("record key: {error:?}"))?;
    let started = Instant::now();
    let mut operations = [0_u64; 6];
    let mut corrupt = 0_u64;
    for seed in 0..seeds {
        let store = Arc::new(MemoryKeyValueStore::new(StoreCapacity::Unbounded));
        let environment = Arc::new(Environment(Arc::clone(&store)));
        let mut model = vec![None::<ModelEntry>; keys_count];
        let mut revision = 0_u64;
        let mut generator = number(&["seed"])?.wrapping_add(seed).max(1);
        let mut now = 0_i64;
        for _ in 0..transitions {
            let choice = (next(&mut generator) % operations.len() as u64) as usize;
            let slot = (next(&mut generator) % keys_count as u64) as usize;
            *operations
                .get_mut(choice)
                .ok_or("invalid operation index")? += 1;
            let key = keys.get(slot).ok_or("invalid key slot")?;
            match choice {
                0 => {
                    let value = next(&mut generator) as i64;
                    let kind = next(&mut generator) % 5;
                    let decoded = if kind == 0 {
                        None
                    } else if kind & 1 == 0 {
                        Some(Ok(value))
                    } else {
                        Some(Err(value))
                    };
                    let bytes = match decoded {
                        Some(Ok(value)) => definition
                            .encode(&StoredOutcome::Success(value))
                            .map_err(|error| format!("success encoding: {error:?}"))?,
                        Some(Err(value)) => definition
                            .encode(&StoredOutcome::Failure(value))
                            .map_err(|error| format!("error encoding: {error:?}"))?,
                        None => {
                            corrupt += 1;
                            Arc::from(&b"{"[..])
                        }
                    };
                    let expires_at = if next(&mut generator) & 1 == 0 {
                        Some(UtcMillis::new(now + ttl))
                    } else {
                        None
                    };
                    let actual = success(store.put(
                        Arc::clone(key),
                        StoredValue {
                            bytes: Arc::clone(&bytes),
                            expires_at,
                        },
                    ))?;
                    revision += 1;
                    assert_eq!(actual.value(), revision);
                    *model.get_mut(slot).ok_or("invalid model slot")? = Some(ModelEntry {
                        bytes,
                        expires_at,
                        revision: actual,
                        decoded,
                    });
                }
                1 => {
                    let actual = success(store.get(Arc::clone(key), UtcMillis::new(now)))?;
                    let expected = model
                        .get(slot)
                        .ok_or("invalid model slot")?
                        .as_ref()
                        .map_or(StoreRead::Missing, |entry| entry.read(UtcMillis::new(now)));
                    assert_eq!(actual, expected);
                }
                2 => {
                    let actual = success(store.get_many(keys.clone(), UtcMillis::new(now)))?;
                    assert_eq!(actual.len(), keys_count);
                    for ((item, key), model_entry) in actual.into_iter().zip(&keys).zip(&model) {
                        assert_eq!(&item.key, key);
                        let expected = model_entry
                            .as_ref()
                            .map_or(StoreRead::Missing, |entry| entry.read(UtcMillis::new(now)));
                        assert_eq!(
                            item.result
                                .map_err(|error| format!("memory batch item: {error:?}"))?,
                            expected
                        );
                    }
                    verify_typed_batch(
                        Arc::clone(&environment),
                        &definition,
                        &model,
                        UtcMillis::new(now),
                    )?;
                }
                3 => {
                    let expected = if next(&mut generator) & 1 == 0 {
                        model
                            .get(slot)
                            .ok_or("invalid model slot")?
                            .as_ref()
                            .map(|entry| entry.revision)
                    } else {
                        None
                    };
                    let replace = next(&mut generator) & 1 == 0;
                    let value = next(&mut generator) as i64;
                    let bytes = definition
                        .encode(&StoredOutcome::Success(value))
                        .map_err(|error| format!("CAS encoding: {error:?}"))?;
                    let replacement = replace.then(|| StoredValue {
                        bytes: Arc::clone(&bytes),
                        expires_at: Some(UtcMillis::new(now + ttl)),
                    });
                    let actual =
                        success(store.compare_exchange(Arc::clone(key), expected, replacement))?;
                    let current = model
                        .get(slot)
                        .ok_or("invalid model slot")?
                        .as_ref()
                        .map(|entry| entry.revision);
                    if expected != current {
                        assert_eq!(actual, CompareExchange::Conflict { actual: current });
                    } else {
                        revision += 1;
                        let next_revision = StoreRevision::from_recorded(revision)
                            .map_err(|error| format!("model revision: {error:?}"))?;
                        assert_eq!(actual, CompareExchange::Written(next_revision));
                        *model.get_mut(slot).ok_or("invalid model slot")? =
                            replace.then_some(ModelEntry {
                                bytes,
                                expires_at: Some(UtcMillis::new(now + ttl)),
                                revision: next_revision,
                                decoded: Some(Ok(value)),
                            });
                    }
                }
                4 => {
                    let actual = success(store.remove(Arc::clone(key)))?;
                    let expected = model
                        .get_mut(slot)
                        .ok_or("invalid model slot")?
                        .take()
                        .map(|entry| entry.stored());
                    assert_eq!(actual, expected);
                }
                5 => {
                    let actual = success(store.cleanup_expired(UtcMillis::new(now)))?;
                    let mut removed = 0;
                    for entry in &mut model {
                        if entry.as_ref().is_some_and(|value| {
                            value.expires_at.is_some_and(|at| UtcMillis::new(now) >= at)
                        }) {
                            *entry = None;
                            removed += 1;
                        }
                    }
                    assert_eq!(actual, removed);
                }
                _ => return Err("invalid operation index".into()),
            }
            let actual = success(store.get(Arc::clone(key), UtcMillis::new(now)))?;
            let expected = model
                .get(slot)
                .ok_or("invalid model slot")?
                .as_ref()
                .map_or(StoreRead::Missing, |entry| entry.read(UtcMillis::new(now)));
            assert_eq!(actual, expected);
            now += i64::try_from(next(&mut generator) % (ttl as u64 + 2))?;
            assert!(
                started.elapsed().as_secs() <= seconds,
                "record campaign timed out"
            );
        }
    }
    assert!(operations.into_iter().all(|count| count > 0));
    assert!(corrupt > 0);
    let boundary_store = MemoryKeyValueStore::new(StoreCapacity::Unbounded);
    let boundary_key = Arc::clone(keys.first().ok_or("missing boundary key")?);
    let boundary_bytes = definition
        .encode(&StoredOutcome::Success(1))
        .map_err(|error| format!("boundary record: {error:?}"))?;
    success(boundary_store.put(
        Arc::clone(&boundary_key),
        StoredValue {
            bytes: boundary_bytes,
            expires_at: Some(UtcMillis::new(ttl)),
        },
    ))?;
    assert!(matches!(
        success(boundary_store.get(Arc::clone(&boundary_key), UtcMillis::new(ttl - 1)))?,
        StoreRead::Found(_)
    ));
    assert!(matches!(
        success(boundary_store.get(Arc::clone(&boundary_key), UtcMillis::new(ttl)))?,
        StoreRead::Expired(_)
    ));
    assert!(matches!(
        success(boundary_store.get(boundary_key, UtcMillis::new(ttl + 1)))?,
        StoreRead::Expired(_)
    ));
    campaign_report::completed(
        "persistence_record",
        json!({
            "seeds": seeds, "transitions": transitions,
            "operations": operations, "corrupt": corrupt,
        }),
    )?;
    writeln!(
        std::io::stderr().lock(),
        "record model completed {seeds} seeds x {transitions} transitions; operations={operations:?}; corrupt={corrupt}"
    )?;
    Ok(())
}
