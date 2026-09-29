use super::{
    CompareExchange, ConditionalKeyValueStore, HasKeyValueStore, KeyValueStore, Persistable,
    PersistedRecordError, PersistenceError, PersistenceMutation, StoreRead, StoreRevision,
    StoredValue,
};
use crate::{Effect, KeyDomain, StoredOutcome, UtcMillis};
use std::sync::Arc;

type BackendError<R> = <<R as HasKeyValueStore>::Store as KeyValueStore>::Error;
pub type StoredReadManyEffect<A, E, R> = Effect<
    Vec<Result<StoredLookup<A, E>, PersistenceError<BackendError<R>>>>,
    PersistenceError<BackendError<R>>,
    R,
>;

/// A read does not collapse absence, expiry, corruption, and provider failure.
/// Corruption and provider failure use the Effect error channel.
#[derive(Debug)]
pub enum StoredLookup<A, E> {
    Missing,
    Expired {
        revision: StoreRevision,
        expires_at: UtcMillis,
    },
    Found {
        revision: StoreRevision,
        outcome: StoredOutcome<A, E>,
    },
}

fn decode_entry<I, A: Send + 'static, E: Send + 'static, D: KeyDomain, Backend>(
    definition: &Persistable<I, A, E, D>,
    read: StoreRead,
) -> Result<StoredLookup<A, E>, PersistenceError<Backend>> {
    match read {
        StoreRead::Missing => Ok(StoredLookup::Missing),
        StoreRead::Expired(entry) => {
            if entry.revision.value() == 0 {
                return Err(PersistenceError::Corrupt(crate::SchemaIssue::new(
                    crate::SchemaIssueKind::InvalidDefinition,
                )));
            }
            let Some(expires_at) = entry.value.expires_at else {
                return Err(PersistenceError::Corrupt(crate::SchemaIssue::new(
                    crate::SchemaIssueKind::InvalidDefinition,
                )));
            };
            Ok(StoredLookup::Expired {
                revision: entry.revision,
                expires_at,
            })
        }
        StoreRead::Found(entry) => {
            if entry.revision.value() == 0 {
                return Err(PersistenceError::Corrupt(crate::SchemaIssue::new(
                    crate::SchemaIssueKind::InvalidDefinition,
                )));
            }
            let outcome = definition
                .decode(&entry.value.bytes)
                .map_err(|error| match error {
                    PersistedRecordError::Codec(issue) => PersistenceError::Corrupt(issue),
                    PersistedRecordError::VersionMismatch => PersistenceError::VersionMismatch,
                })?;
            Ok(StoredLookup::Found {
                revision: entry.revision,
                outcome,
            })
        }
    }
}

/// Read a typed record using only the store supplied by R. The input factory
/// supplies an owned request for each run; no request or outcome Clone is needed.
pub fn read<R, I, A, E, D>(
    definition: Persistable<I, A, E, D>,
    input: impl Fn() -> I + Send + Sync + 'static,
    now: UtcMillis,
) -> Effect<
    StoredLookup<A, E>,
    PersistenceError<<<R as HasKeyValueStore>::Store as KeyValueStore>::Error>,
    R,
>
where
    R: HasKeyValueStore + Send + Sync + 'static,
    I: Send + 'static,
    A: Send + 'static,
    E: Send + 'static,
    D: KeyDomain,
{
    let key_definition = definition.clone();
    Effect::try_sync(
        move || {
            let request = input();
            key_definition.key(&request).map(|key| key.into_bytes())
        },
        PersistenceError::Identity,
    )
    .flat_map(move |key| {
        let definition = definition.clone();
        Effect::service(|environment: &R| environment.key_value_store()).flat_map(move |store| {
            let definition = definition.clone();
            let key = Arc::clone(&key);
            store
                .get(key, now)
                .map_error(PersistenceError::Backend)
                .provide_some(|_: &R| ())
                .flat_map(move |record| Effect::from_result_once(decode_entry(&definition, record)))
        })
    })
}

/// Batch reads preserve request order and key association. Identity, backend
/// and decode failures remain at their respective result slots; a malformed
/// batch response or unavailable batch is an outer failure.
pub fn read_many<R, I, A, E, D>(
    definition: Persistable<I, A, E, D>,
    inputs: impl Fn() -> Vec<I> + Send + Sync + 'static,
    now: UtcMillis,
) -> StoredReadManyEffect<A, E, R>
where
    R: HasKeyValueStore + Send + Sync + 'static,
    I: Send + 'static,
    A: Send + 'static,
    E: Send + 'static,
    D: KeyDomain,
{
    let key_definition = definition.clone();
    Effect::sync(move || {
        inputs()
            .iter()
            .map(|input| key_definition.key(input).map(|key| key.into_bytes()))
            .collect::<Vec<_>>()
    })
    .flat_map(move |slots| {
        let keys = Arc::new(slots);
        let definition = definition.clone();
        Effect::service(|environment: &R| environment.key_value_store()).flat_map(move |store| {
            let keys = Arc::clone(&keys);
            let definition = definition.clone();
            let valid: Vec<_> = keys
                .iter()
                .filter_map(|key| key.as_ref().ok().cloned())
                .collect();
            if valid.is_empty() {
                let failures = keys
                    .iter()
                    .filter_map(|slot| {
                        slot.as_ref()
                            .err()
                            .copied()
                            .map(|error| Err(PersistenceError::Identity(error)))
                    })
                    .collect::<Vec<_>>();
                return Effect::sync_once(move || failures);
            }
            store
                .get_many(valid, now)
                .map_error(PersistenceError::Backend)
                .provide_some(|_: &R| ())
                .flat_map(move |records| {
                    let mut records = records.into_iter();
                    let mut decoded = Vec::with_capacity(keys.len());
                    for slot in keys.iter() {
                        match slot {
                            Err(error) => decoded.push(Err(PersistenceError::Identity(*error))),
                            Ok(expected) => {
                                let Some(record) = records.next() else {
                                    return Effect::fail_with(|| PersistenceError::BatchMismatch);
                                };
                                if expected.as_ref() != record.key.as_ref() {
                                    return Effect::fail_with(|| PersistenceError::BatchMismatch);
                                }
                                decoded.push(match record.result {
                                    Ok(read) => decode_entry(&definition, read),
                                    Err(error) => Err(PersistenceError::Backend(error)),
                                });
                            }
                        }
                    }
                    if records.next().is_some() {
                        return Effect::fail_with(|| PersistenceError::BatchMismatch);
                    }
                    Effect::sync_once(move || decoded)
                })
        })
    })
}

/// A write error from the provider is uncertain because the provider may have
/// committed before its acknowledgement was lost. Identity/codec errors occur
/// before submission and remain distinguishable.
pub fn write<R, I, A, E, D>(
    definition: Persistable<I, A, E, D>,
    input: impl Fn() -> I + Send + Sync + 'static,
    outcome: impl Fn() -> StoredOutcome<A, E> + Send + Sync + 'static,
    expires_at: Option<UtcMillis>,
) -> Effect<
    StoreRevision,
    PersistenceError<<<R as HasKeyValueStore>::Store as KeyValueStore>::Error>,
    R,
>
where
    R: HasKeyValueStore + Send + Sync + 'static,
    I: Send + 'static,
    A: Send + 'static,
    E: Send + 'static,
    D: KeyDomain,
{
    Effect::try_sync(
        move || {
            let request = input();
            let key = definition
                .key(&request)
                .map_err(PersistenceError::Identity)?;
            let value = outcome();
            let bytes = definition.encode(&value).map_err(PersistenceError::Codec)?;
            Ok((key.into_bytes(), StoredValue { bytes, expires_at }))
        },
        std::convert::identity,
    )
    .flat_map(move |(key, value)| {
        let value = Arc::new(std::sync::Mutex::new(Some(value)));
        Effect::service(|environment: &R| environment.key_value_store()).flat_map(move |store| {
            let key = Arc::clone(&key);
            let value = Arc::clone(&value);
            Effect::defer(move || {
                let value = value
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .take();
                match value {
                    Some(value) => store
                        .put(Arc::clone(&key), value)
                        .map_error({
                            let key = Arc::clone(&key);
                            move |source| PersistenceError::Uncertain {
                                operation: PersistenceMutation::Write,
                                key: Arc::clone(&key),
                                source,
                            }
                        })
                        .provide_some(|_: &R| ())
                        .flat_map({
                            let key = Arc::clone(&key);
                            move |revision| {
                                if revision.value() == 0 {
                                    let key = Arc::clone(&key);
                                    Effect::fail_with(move || {
                                        PersistenceError::InvalidAcknowledgement {
                                            operation: PersistenceMutation::Write,
                                            key: Arc::clone(&key),
                                        }
                                    })
                                } else {
                                    Effect::sync_once(move || revision)
                                }
                            }
                        }),
                    None => Effect::fail_with(|| {
                        PersistenceError::Codec(crate::SchemaIssue::new(
                            crate::SchemaIssueKind::InputConsumed,
                        ))
                    }),
                }
            })
        })
    })
}

pub fn remove<R, I, A, E, D>(
    definition: Persistable<I, A, E, D>,
    input: impl Fn() -> I + Send + Sync + 'static,
) -> Effect<bool, PersistenceError<<<R as HasKeyValueStore>::Store as KeyValueStore>::Error>, R>
where
    R: HasKeyValueStore + Send + Sync + 'static,
    I: Send + 'static,
    A: Send + 'static,
    E: Send + 'static,
    D: KeyDomain,
{
    Effect::try_sync(
        move || {
            let request = input();
            definition.key(&request).map(|key| key.into_bytes())
        },
        PersistenceError::Identity,
    )
    .flat_map(move |key| {
        Effect::service(|environment: &R| environment.key_value_store()).flat_map(move |store| {
            store
                .remove(Arc::clone(&key))
                .map(|removed| removed.is_some())
                .map_error({
                    let key = Arc::clone(&key);
                    move |source| PersistenceError::Uncertain {
                        operation: PersistenceMutation::Remove,
                        key: Arc::clone(&key),
                        source,
                    }
                })
                .provide_some(|_: &R| ())
        })
    })
}

/// Conditional mutation is available only when R's store supplies the
/// stronger per-key compare-and-exchange guarantee. A plain key/value store
/// cannot acquire it through a callback or name alone.
pub fn compare_exchange<R>(
    key: Arc<[u8]>,
    expected: Option<StoreRevision>,
    replacement: Option<StoredValue>,
) -> Effect<
    CompareExchange,
    PersistenceError<<<R as HasKeyValueStore>::Store as KeyValueStore>::Error>,
    R,
>
where
    R: HasKeyValueStore + Send + Sync + 'static,
    R::Store: ConditionalKeyValueStore,
{
    let replacement = std::sync::Mutex::new(Some(replacement));
    Effect::service(|environment: &R| environment.key_value_store()).flat_map(move |store| {
        let key = Arc::clone(&key);
        let replacement = replacement
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        let Some(replacement) = replacement else {
            return Effect::fail_with(|| {
                PersistenceError::Codec(crate::SchemaIssue::new(
                    crate::SchemaIssueKind::InputConsumed,
                ))
            });
        };
        store
            .compare_exchange(Arc::clone(&key), expected, replacement)
            .map_error({
                let key = Arc::clone(&key);
                move |source| PersistenceError::Uncertain {
                    operation: PersistenceMutation::CompareExchange,
                    key: Arc::clone(&key),
                    source,
                }
            })
            .provide_some(|_: &R| ())
            .flat_map(move |result| {
                if matches!(result, CompareExchange::Written(revision) if revision.value() == 0) {
                    let key = Arc::clone(&key);
                    Effect::fail_with(move || PersistenceError::InvalidAcknowledgement {
                        operation: PersistenceMutation::CompareExchange,
                        key: Arc::clone(&key),
                    })
                } else {
                    Effect::sync_once(move || result)
                }
            })
    })
}
