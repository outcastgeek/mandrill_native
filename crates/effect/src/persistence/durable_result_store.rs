//! Result storage requiring an explicitly durable provider.

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
///
/// # Type parameters
/// - `S`: Service or storage type. Bound or type: `DurableKeyValueStore`.
struct DurableStoreView<S: DurableKeyValueStore>(Arc<S>);

impl<S: DurableKeyValueStore> HasKeyValueStore for DurableStoreView<S> {
    /// Type alias for store in persistence durable result store.
    type Store = S;

    ///
    /// # Returns
    /// Type: `Arc<Self::Store>`.
    /// Returns `Arc<Self::Store>`.
    fn key_value_store(&self) -> Arc<Self::Store> {
        Arc::clone(&self.0)
    }
}

/// Type alias for durable backend in persistence durable result store.
///
/// # Type parameters
/// - `R`: Environment type.
type DurableBackend<R> = <<R as HasDurableKeyValueStore>::Store as KeyValueStore>::Error;

/// Read through the provider that explicitly warrants acknowledged-state
/// recovery. A volatile-only environment cannot satisfy this signature.
///
/// # Type parameters
/// - `R`: Environment type. Additional bound: `HasDurableKeyValueStore + Send + Sync + 'static`.
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
/// Type: `Effect<StoredLookup<A, E>, PersistenceError<DurableBackend<R>>, R>`.
/// Returns a lazy `Effect` with the declared success, error, and environment types.
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

type DurableBatch<A, E, R> = Effect<
    Vec<Result<StoredLookup<A, E>, PersistenceError<DurableBackend<R>>>>,
    PersistenceError<DurableBackend<R>>,
    R,
>;

/// Lazily read an ordered batch through R's declared durable key-value provider.
///
/// The input factory supplies fresh owned inputs for each execution. Each slot
/// retains its original lookup or persistence error in input order; an outer error
/// represents batch-level failure. Actual persistence guarantees belong to the
/// provider, not this type alias or adapter.
pub fn read_many_durable<R, I, A, E, D>(
    definition: Persistable<I, A, E, D>,
    inputs: impl Fn() -> Vec<I> + Send + Sync + 'static,
    now: UtcMillis,
) -> DurableBatch<A, E, R>
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
///
/// # Type parameters
/// - `R`: Environment type. Additional bound: `HasDurableKeyValueStore + Send + Sync + 'static`.
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
/// Type: `Effect<StoreRevision, PersistenceError<DurableBackend<R>>, R>`.
/// Returns a lazy `Effect` with the declared success, error, and environment types.
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

/// Remove a recorded result through the durable store contract.
///
/// # Type parameters
/// - `R`: Environment type. Additional bound: `HasDurableKeyValueStore + Send + Sync + 'static`.
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
/// Type: `Effect<bool, PersistenceError<DurableBackend<R>>, R>`.
/// Returns a lazy `Effect` with the declared success, error, and environment types.
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
///
/// # Type parameters
/// - `R`: Environment type. Additional bound: `HasDurableKeyValueStore + Send + Sync + 'static`. Additional bound: `:Store: ConditionalKeyValueStore`.
///
/// # Parameters
/// - `key` (`Arc<[u8]>`): Stable key used to locate or identify the selected value.
/// - `expected` (`Option<StoreRevision>`): Value or revision required for the proposed state transition.
/// - `replacement` (`Option<StoredValue>`): The replacement supplied to this operation.
///
/// # Returns
/// Type: `Effect<CompareExchange, PersistenceError<DurableBackend<R>>, R>`.
/// Returns a lazy `Effect` with the declared success, error, and environment types.
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
