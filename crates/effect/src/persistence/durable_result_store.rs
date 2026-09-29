use super::{
    CompareExchange, ConditionalKeyValueStore, DurableKeyValueStore, HasDurableKeyValueStore,
    HasKeyValueStore, KeyValueStore, Persistable, PersistenceError, StoreRevision, StoredLookup,
    StoredValue, result_store,
};
use crate::{Effect, KeyDomain, StoredOutcome, UtcMillis};
use std::sync::Arc;

/// Select the *same* provider for every operation in a physically durable
/// result-store path. This private view only shares the provider handle; it
/// does not copy records or invent a second store.
struct DurableStoreView<S: DurableKeyValueStore>(Arc<S>);

impl<S: DurableKeyValueStore> HasKeyValueStore for DurableStoreView<S> {
    type Store = S;

    fn key_value_store(&self) -> Arc<Self::Store> {
        Arc::clone(&self.0)
    }
}

type DurableBackend<R> = <<R as HasDurableKeyValueStore>::Store as KeyValueStore>::Error;

/// Read through the provider that explicitly warrants acknowledged-state
/// recovery. A volatile-only environment cannot satisfy this signature.
pub fn read_durable<R, I, A, E, D>(
    definition: Persistable<I, A, E, D>,
    input: impl Fn() -> I + Send + Sync + 'static,
    now: UtcMillis,
) -> Effect<StoredLookup<A, E>, PersistenceError<DurableBackend<R>>, R>
where
    R: HasDurableKeyValueStore + Send + Sync + 'static,
    I: Send + 'static,
    A: Send + 'static,
    E: Send + 'static,
    D: KeyDomain,
{
    result_store::read::<DurableStoreView<R::Store>, I, A, E, D>(definition, input, now)
        .provide_some(|environment: &R| DurableStoreView(environment.durable_key_value_store()))
}

/// Batch reads retain the original per-slot result and failure ordering.
pub fn read_many_durable<R, I, A, E, D>(
    definition: Persistable<I, A, E, D>,
    inputs: impl Fn() -> Vec<I> + Send + Sync + 'static,
    now: UtcMillis,
) -> Effect<
    Vec<Result<StoredLookup<A, E>, PersistenceError<DurableBackend<R>>>>,
    PersistenceError<DurableBackend<R>>,
    R,
>
where
    R: HasDurableKeyValueStore + Send + Sync + 'static,
    I: Send + 'static,
    A: Send + 'static,
    E: Send + 'static,
    D: KeyDomain,
{
    result_store::read_many::<DurableStoreView<R::Store>, I, A, E, D>(definition, inputs, now)
        .provide_some(|environment: &R| DurableStoreView(environment.durable_key_value_store()))
}

/// Provider acknowledgement is still classified as uncertain when a write
/// error may have occurred after the provider committed it.
pub fn write_durable<R, I, A, E, D>(
    definition: Persistable<I, A, E, D>,
    input: impl Fn() -> I + Send + Sync + 'static,
    outcome: impl Fn() -> StoredOutcome<A, E> + Send + Sync + 'static,
    expires_at: Option<UtcMillis>,
) -> Effect<StoreRevision, PersistenceError<DurableBackend<R>>, R>
where
    R: HasDurableKeyValueStore + Send + Sync + 'static,
    I: Send + 'static,
    A: Send + 'static,
    E: Send + 'static,
    D: KeyDomain,
{
    result_store::write::<DurableStoreView<R::Store>, I, A, E, D>(
        definition, input, outcome, expires_at,
    )
    .provide_some(|environment: &R| DurableStoreView(environment.durable_key_value_store()))
}

pub fn remove_durable<R, I, A, E, D>(
    definition: Persistable<I, A, E, D>,
    input: impl Fn() -> I + Send + Sync + 'static,
) -> Effect<bool, PersistenceError<DurableBackend<R>>, R>
where
    R: HasDurableKeyValueStore + Send + Sync + 'static,
    I: Send + 'static,
    A: Send + 'static,
    E: Send + 'static,
    D: KeyDomain,
{
    result_store::remove::<DurableStoreView<R::Store>, I, A, E, D>(definition, input)
        .provide_some(|environment: &R| DurableStoreView(environment.durable_key_value_store()))
}

/// Atomic per-key mutation requires the provider to warrant both durability
/// and conditional writes. Plain durable key/value storage is insufficient.
pub fn compare_exchange_durable<R>(
    key: Arc<[u8]>,
    expected: Option<StoreRevision>,
    replacement: Option<StoredValue>,
) -> Effect<CompareExchange, PersistenceError<DurableBackend<R>>, R>
where
    R: HasDurableKeyValueStore + Send + Sync + 'static,
    R::Store: ConditionalKeyValueStore,
{
    result_store::compare_exchange::<DurableStoreView<R::Store>>(key, expected, replacement)
        .provide_some(|environment: &R| DurableStoreView(environment.durable_key_value_store()))
}
