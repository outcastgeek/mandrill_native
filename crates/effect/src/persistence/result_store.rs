//! Typed stored outcomes over encoded key-value operations.

use super::{
    CompareExchange, ConditionalKeyValueStore, HasKeyValueStore, KeyValueStore, Persistable,
    PersistedRecordError, PersistenceError, PersistenceMutation, StoreRead, StoreRevision,
    StoredValue,
};
use crate::{Effect, KeyDomain, StoredOutcome, UtcMillis};
use crate::{
    effect, effect_map, effect_map_error, effect_provide_some, effect_sync, effect_try_sync,
    service,
};
use std::sync::Arc;

/// Type alias for backend error in persistence result store.
///
/// # Type parameters
/// - `R`: Environment type.
type BackendError<R> = <<R as HasKeyValueStore>::Store as KeyValueStore>::Error;
/// Type alias for stored read many effect in persistence result store.
///
/// # Type parameters
/// - `A`: Application value type.
/// - `E`: Domain-error type.
/// - `R`: Environment type.
pub type StoredReadManyEffect<A, E, R> = Effect<
    Vec<Result<StoredLookup<A, E>, PersistenceError<BackendError<R>>>>,
    PersistenceError<BackendError<R>>,
    R,
>;

/// A read does not collapse absence, expiry, corruption, and provider failure.
/// Corruption and provider failure use the Effect error channel.
#[derive(Debug)]
///
/// # Type parameters
/// - `A`: Application value type.
/// - `E`: Domain-error type.
pub enum StoredLookup<A, E> {
    /// No matching value exists.
    Missing,
    /// The value exists but has passed its expiry instant.
    Expired {
        /// The recorded storage revision. Type: `StoreRevision`.
        revision: StoreRevision,
        /// The time at which the expires occurred. Type: `UtcMillis`.
        expires_at: UtcMillis,
    },
    /// A matching value was found.
    Found {
        /// The recorded storage revision. Type: `StoreRevision`.
        revision: StoreRevision,
        /// The recorded outcome of this operation. Type: `StoredOutcome<A, E>`.
        outcome: StoredOutcome<A, E>,
    },
}

///
/// # Type parameters
/// - `I`: Workflow or codec input type.
/// - `A`: Bounds: `Send + 'static`.
/// - `E`: Bounds: `Send + 'static`.
/// - `D`: Bounds: `KeyDomain`.
/// - `Backend`: Backend-specific error or provider type.
///
/// # Parameters
/// - `definition` (`&Persistable<I, A, E, D>`): The definition supplied to this operation.
/// - `read` (`StoreRead`): The read supplied to this operation.
///
/// # Returns
/// Type: `Result<StoredLookup<A, E>, PersistenceError<Backend>>`.
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
///
/// # Type parameters
/// - `R`: Environment type. Additional bound: `HasKeyValueStore + Send + Sync + 'static`.
/// - `I`: Input type. Additional bound: `Send + 'static`.
/// - `A`: Application value type. Additional bound: `Send + 'static`.
/// - `E`: Domain-error type. Additional bound: `Send + 'static`.
/// - `D`: Identity-domain type. Additional bound: `KeyDomain`.
///
/// # Parameters
/// - `definition` (`Persistable<I, A, E, D>`): The typed definition used by this operation.
/// - `input` (`impl Fn() -> I + Send + Sync + 'static`): The input value or factory consumed by this operation.
/// - `now` (`UtcMillis`): The caller-supplied current time.
///
/// # Returns
/// Type: `Effect<StoredLookup<A, E>, PersistenceError<<<R as HasKeyValueStore>::Store as KeyValueStore>::Error>, R>`.
/// Returns a lazy `Effect` with the declared success, error, and environment types.
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
    {
        let __dsl_program = effect_try_sync! {
        body: move || {
            let request = input();
            key_definition.key(&request).map(|key| key.into_bytes())
        }, on_error: PersistenceError::Identity, };
        effect! {
                let definition = definition.clone(); let key = __dsl_program.clone()?; {
            let definition = definition.clone();
            {
                let __dsl_program = service! { env: |environment: &R| environment.key_value_store(), }; effect! {
                let definition = definition.clone(); let key = key.clone(); let store = __dsl_program.clone()?; {
                let definition = definition.clone();
                let key = Arc::clone(&key);
                {
                    let __dsl_program = effect_provide_some! {
                    program: effect_map_error! {
                    program: store
                    .get(key, now), map: PersistenceError::Backend, }, project: |_: &R| (), }; effect! { let definition = definition.clone(); let record = __dsl_program.clone()?; effect_try_sync! { once; body: { let __dsl_result = decode_entry(&definition, record); move || __dsl_result }, on_error: ::core::convert::identity, }? } }
            }? } }
        }? }
    }
}

/// Batch reads preserve request order and key association. Identity, backend
/// and decode failures remain at their respective result slots; a malformed
/// batch response or unavailable batch is an outer failure.
///
/// # Type parameters
/// - `R`: Environment type. Additional bound: `HasKeyValueStore + Send + Sync + 'static`.
/// - `I`: Input type. Additional bound: `Send + 'static`.
/// - `A`: Application value type. Additional bound: `Send + 'static`.
/// - `E`: Domain-error type. Additional bound: `Send + 'static`.
/// - `D`: Identity-domain type. Additional bound: `KeyDomain`.
///
/// # Parameters
/// - `definition` (`Persistable<I, A, E, D>`): The typed definition used by this operation.
/// - `inputs` (`impl Fn() -> Vec<I> + Send + Sync + 'static`): The inputs supplied to this operation.
/// - `now` (`UtcMillis`): The caller-supplied current time.
///
/// # Returns
/// Type: `StoredReadManyEffect<A, E, R>`.
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
    {
        let __dsl_program = effect_sync! {
            body: move || {
            inputs()
                .iter()
                .map(|input| key_definition.key(input).map(|key| key.into_bytes()))
                .collect::<Vec<_>>()
        }, };
        effect! {
            let __dsl_program = __dsl_program.clone(); let definition = definition.clone(); let slots = __dsl_program.clone()?; {
            let keys = Arc::new(slots);
            let definition = definition.clone();
            {
                let __dsl_program = service! { env: |environment: &R| environment.key_value_store(), }; effect! {
                let __dsl_program = __dsl_program.clone(); let definition = definition.clone(); let keys = keys.clone(); let store = __dsl_program.clone()?; (move || {
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
                    return effect_sync! { once; body: move || failures, };
                }
                {
                    let __dsl_program = effect_provide_some! {
                    program: effect_map_error! {
                    program: store
                    .get_many(valid, now), map: PersistenceError::Backend, }, project: |_: &R| (), }; effect! {
                        let __dsl_program = __dsl_program.clone(); let definition = definition.clone(); let keys = keys.clone(); let records = __dsl_program.clone()?; (move || {
                        let mut records = records.into_iter();
                        let mut decoded = Vec::with_capacity(keys.len());
                        for slot in keys.iter() {
                            match slot {
                                Err(error) => decoded.push(Err(PersistenceError::Identity(*error))),
                                Ok(expected) => {
                                    let Some(record) = records.next() else {
                                        return effect_try_sync! { body: { let __dsl_error = || PersistenceError::BatchMismatch; move || Err(__dsl_error()) }, on_error: ::core::convert::identity, };
                                    };
                                    if expected.as_ref() != record.key.as_ref() {
                                        return effect_try_sync! { body: { let __dsl_error = || PersistenceError::BatchMismatch; move || Err(__dsl_error()) }, on_error: ::core::convert::identity, };
                                    }
                                    decoded.push(match record.result {
                                        Ok(read) => decode_entry(&definition, read),
                                        Err(error) => Err(PersistenceError::Backend(error)),
                                    });
                                }
                            }
                        }
                        if records.next().is_some() {
                            return effect_try_sync! { body: { let __dsl_error = || PersistenceError::BatchMismatch; move || Err(__dsl_error()) }, on_error: ::core::convert::identity, };
                        }
                        effect_sync! { once; body: move || decoded, }
                    })()? } }
            })()? } }
        }? }
    }
}

/// A write error from the provider is uncertain because the provider may have
/// committed before its acknowledgement was lost. Identity/codec errors occur
/// before submission and remain distinguishable.
///
/// # Type parameters
/// - `R`: Environment type. Additional bound: `HasKeyValueStore + Send + Sync + 'static`.
/// - `I`: Input type. Additional bound: `Send + 'static`.
/// - `A`: Application value type. Additional bound: `Send + 'static`.
/// - `E`: Domain-error type. Additional bound: `Send + 'static`.
/// - `D`: Identity-domain type. Additional bound: `KeyDomain`.
///
/// # Parameters
/// - `definition` (`Persistable<I, A, E, D>`): The typed definition used by this operation.
/// - `input` (`impl Fn() -> I + Send + Sync + 'static`): The input value or factory consumed by this operation.
/// - `outcome` (`impl Fn() -> StoredOutcome<A, E> + Send + Sync + 'static`): Complete terminal outcome, including expected errors and cleanup failures.
/// - `expires_at` (`Option<UtcMillis>`): The optional expiry instant.
///
/// # Returns
/// Type: `Effect<StoreRevision, PersistenceError<<<R as HasKeyValueStore>::Store as KeyValueStore>::Error>, R>`.
/// Returns a lazy `Effect` with the declared success, error, and environment types.
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
    {
        let __dsl_program = effect_try_sync! {
        body: move || {
            let request = input();
            let key = definition
                .key(&request)
                .map_err(PersistenceError::Identity)?;
            let value = outcome();
            let bytes = definition.encode(&value).map_err(PersistenceError::Codec)?;
            Ok((key.into_bytes(), StoredValue { bytes, expires_at }))
        }, on_error: std::convert::identity, };
        effect! {
                let (key , value) = __dsl_program.clone()?; {
            let value = Arc::new(std::sync::Mutex::new(Some(value)));
            {
                let __dsl_program = service! { env: |environment: &R| environment.key_value_store(), }; effect! {
                let key = key.clone(); let value = value.clone(); let store = __dsl_program.clone()?; {
                let key = Arc::clone(&key);
                let value = Arc::clone(&value);
                {
                    let __dsl_factory = move || {
                    let value = value
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .take();
                    match value {
                        Some(value) => {
                            let __dsl_program = effect_provide_some! {
                            program: effect_map_error! {
                            program: store
                            .put(Arc::clone(&key), value), map: {
                                let key = Arc::clone(&key);
                                move |source| PersistenceError::Uncertain {
                                    operation: PersistenceMutation::Write,
                                    key: Arc::clone(&key),
                                    source,
                                }
                            }, }, project: |_: &R| (), }; let key = Arc::clone(&key); effect! {
                                let __dsl_program = __dsl_program.clone(); let key = key.clone(); let revision = __dsl_program?; {
                                    if revision.value() == 0 {
                                        let key = Arc::clone(&key);
                                        effect_try_sync! {
                                            body: {
                                            let __dsl_error = move || {
                                            PersistenceError::InvalidAcknowledgement {
                                                operation: PersistenceMutation::Write,
                                                key: Arc::clone(&key),
                                            }
                                        }; move || Err(__dsl_error()) }, on_error: ::core::convert::identity, }
                                    } else {
                                        effect_sync! { once; body: move || revision, }
                                    }
                                }? } },
                        None => effect_try_sync! {
                            body: {
                            let __dsl_error = || {
                            PersistenceError::Codec(crate::SchemaIssue::new(
                                crate::SchemaIssueKind::InputConsumed,
                            ))
                        }; move || Err(__dsl_error()) }, on_error: ::core::convert::identity, },
                    }
                }; effect! { __dsl_factory()? } }
            }? } }
        }? }
    }
}

/// Remove the selected entry through this collection or store contract.
///
/// # Type parameters
/// - `R`: Environment type. Additional bound: `HasKeyValueStore + Send + Sync + 'static`.
/// - `I`: Input type. Additional bound: `Send + 'static`.
/// - `A`: Application value type. Additional bound: `Send + 'static`.
/// - `E`: Domain-error type. Additional bound: `Send + 'static`.
/// - `D`: Identity-domain type. Additional bound: `KeyDomain`.
///
/// # Parameters
/// - `definition` (`Persistable<I, A, E, D>`): The typed definition used by this operation.
/// - `input` (`impl Fn() -> I + Send + Sync + 'static`): The input value or factory consumed by this operation.
///
/// # Returns
/// Type: `Effect<bool, PersistenceError<<<R as HasKeyValueStore>::Store as KeyValueStore>::Error>, R>`.
/// Returns a lazy `Effect` with the declared success, error, and environment types.
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
    {
        let __dsl_program = effect_try_sync! {
        body: move || {
            let request = input();
            definition.key(&request).map(|key| key.into_bytes())
        }, on_error: PersistenceError::Identity, };
        effect! {
                let key = __dsl_program.clone()?; {
            {
                let __dsl_program = service! { env: |environment: &R| environment.key_value_store(), }; effect! {
                let key = key.clone(); let store = __dsl_program.clone()?; {
                effect_provide_some! {
                    program: effect_map_error! {
                    program: effect_map! {
                    program: store
                    .remove(Arc::clone(&key)), map: |removed| removed.is_some(), }, map: {
                        let key = Arc::clone(&key);
                        move |source| PersistenceError::Uncertain {
                            operation: PersistenceMutation::Remove,
                            key: Arc::clone(&key),
                            source,
                        }
                    }, }, project: |_: &R| (), }
            }? } }
        }? }
    }
}

/// Conditional mutation is available only when R's store supplies the
/// stronger per-key compare-and-exchange guarantee. A plain key/value store
/// cannot acquire it through a callback or name alone.
///
/// # Type parameters
/// - `R`: Environment type. Additional bound: `HasKeyValueStore + Send + Sync + 'static`. Additional bound: `:Store: ConditionalKeyValueStore`.
///
/// # Parameters
/// - `key` (`Arc<[u8]>`): Stable key used to locate or identify the selected value.
/// - `expected` (`Option<StoreRevision>`): Value or revision required for the proposed state transition.
/// - `replacement` (`Option<StoredValue>`): The replacement supplied to this operation.
///
/// # Returns
/// Type: `Effect<CompareExchange, PersistenceError<<<R as HasKeyValueStore>::Store as KeyValueStore>::Error>, R>`.
/// Returns a lazy `Effect` with the declared success, error, and environment types.
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
    effect! {
        bind;
        program: service! { env: |environment: &R| environment.key_value_store(), },
        body: move |store| {
        let key = Arc::clone(&key);
        let replacement = replacement
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        let Some(replacement) = replacement else {
            return effect_try_sync! {
                body: {
                let __dsl_error = || {
                PersistenceError::Codec(crate::SchemaIssue::new(
                    crate::SchemaIssueKind::InputConsumed,
                ))
            }; move || Err(__dsl_error()) }, on_error: ::core::convert::identity, };
        };
        {
            let __dsl_program = effect_provide_some! {
            program: effect_map_error! {
            program: store
            .compare_exchange(Arc::clone(&key), expected, replacement), map: {
                let key = Arc::clone(&key);
                move |source| PersistenceError::Uncertain {
                    operation: PersistenceMutation::CompareExchange,
                    key: Arc::clone(&key),
                    source,
                }
            }, }, project: |_: &R| (), }; effect! {
                let key = key.clone(); let result = __dsl_program.clone()?; {
                if matches!(result, CompareExchange::Written(revision) if revision.value() == 0) {
                    let key = Arc::clone(&key);
                    effect_try_sync! {
                        body: {
                        let __dsl_error = move || PersistenceError::InvalidAcknowledgement {
                        operation: PersistenceMutation::CompareExchange,
                        key: Arc::clone(&key),
                    }; move || Err(__dsl_error()) }, on_error: ::core::convert::identity, }
                } else {
                    effect_sync! { once; body: move || result, }
                }
            }? } }
        },
    }
}
