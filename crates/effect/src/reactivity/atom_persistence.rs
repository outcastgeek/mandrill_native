//! Acknowledged persistence of reactive values.

use super::{
    AtomRef, HydrationConflictPolicy, HydrationError, HydrationImportOutcome, HydrationSchema,
};
use crate::persistence::{
    CompareExchange, ConditionalKeyValueStore, HasKeyValueStore, KeyValueStore, StoreRead,
    StoreRevision, StoredValue,
};
use crate::{Effect, UtcMillis};
use std::sync::Arc;

#[derive(Debug)]
/// A schema, store, or publication failure in a persisted reactive atom.
///
/// # Type parameters
/// - `Backend`: Backend-specific error or provider type.
pub enum PersistedAtomError<Backend> {
    /// Importing or exporting reactive state failed. Retains the `HydrationError` detail.
    Hydration(HydrationError),
    /// Reading the selected source failed. Retains the `Backend` detail.
    Read(Backend),
    /// CAS may have committed before acknowledgement was lost.
    UncertainWrite {
        /// Key used to address the stored value. Type: `Arc<[u8]>`.
        key: Arc<[u8]>,
        /// The underlying failure or provider cause. Type: `Backend`.
        source: Backend,
    },
    /// An owned input was already consumed.
    InputConsumed,
}

/// The decoded state and version obtained when loading a persisted atom.
pub enum PersistedAtomLoad {
    /// No matching value exists.
    Missing,
    /// The value exists but has passed its expiry instant.
    Expired {
        /// Revision used to fence conditional storage updates. Type: `StoreRevision`.
        store_revision: StoreRevision,
        /// The time at which the expires occurred. Type: `UtcMillis`.
        expires_at: UtcMillis,
    },
    /// The incoming state was accepted and applied.
    Imported {
        /// Revision used to fence conditional storage updates. Type: `StoreRevision`.
        store_revision: StoreRevision,
        /// The underlying failure or provider cause. Type: `HydrationImportOutcome`.
        source: HydrationImportOutcome,
    },
}

/// The version and publication result after saving a persisted atom.
pub enum PersistedAtomSave {
    /// The conditional write committed. Carries `StoreRevision`.
    Written(StoreRevision),
    /// A conditional atom write lost to a newer stored revision.
    Conflict {
        /// Revision now stored, or absence. Type: `Option<StoreRevision>`.
        actual: Option<StoreRevision>,
    },
}

/// Source-atom snapshots use the same typed schema/version record as hydration.
/// Derived atoms recompute from sources. The local observation graph remains
/// distinct from the backing store's authority and notification guarantees.
///
/// # Type parameters
/// - `A`: Application value type. Bound or type: `Send + Sync + 'static`.
pub struct PersistedAtom<A: Send + Sync + 'static> {
    source: AtomRef<A>,
    schema: HydrationSchema<A>,
    key: Arc<[u8]>,
}

impl<A: Send + Sync + 'static> Clone for PersistedAtom<A> {
    ///
    /// # Returns
    /// Type: `Self`.
    fn clone(&self) -> Self {
        Self {
            source: self.source.clone(),
            schema: self.schema.clone(),
            key: Arc::clone(&self.key),
        }
    }
}

impl<A: Send + Sync + 'static> PersistedAtom<A> {
    /// Construct a `PersistedAtom` from its required inputs and explicit policies.
    ///
    /// # Type parameters
    /// - `A`: Success or application-value type.
    ///
    /// # Parameters
    /// - `source` (`AtomRef<A>`): Underlying provider or input from which values are read.
    /// - `schema` (`HydrationSchema<A>`): Codec and validation rules for the application value.
    ///
    /// # Returns
    /// Type: `Result<Self, HydrationError>`.
    pub fn new(source: AtomRef<A>, schema: HydrationSchema<A>) -> Result<Self, HydrationError> {
        if source.id() != schema.atom() {
            return Err(HydrationError::WrongAtom);
        }
        let key = Arc::from(source.id().as_bytes());
        Ok(Self {
            source,
            schema,
            key,
        })
    }

    /// Read and decode the selected value from its backing store.
    ///
    /// # Type parameters
    /// - `A`: Application value type. Bound: `Send + Sync + 'static`.
    /// - `R`: Environment type. Additional bound: `HasKeyValueStore + Send + Sync + 'static`. Additional bound: `:Store: KeyValueStore<Error = Backend>`.
    /// - `Backend`: Backend-specific error or provider type. Additional bound: `Send + 'static`.
    ///
    /// # Parameters
    /// - `now` (`UtcMillis`): The caller-supplied current time.
    /// - `policy` (`HydrationConflictPolicy`): The caller-supplied behavior policy.
    ///
    /// # Returns
    /// Type: `Effect<PersistedAtomLoad, PersistedAtomError<Backend>, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    pub fn load<R, Backend>(
        &self,
        now: UtcMillis,
        policy: HydrationConflictPolicy,
    ) -> Effect<PersistedAtomLoad, PersistedAtomError<Backend>, R>
    where
        R: HasKeyValueStore + Send + Sync + 'static,
        R::Store: KeyValueStore<Error = Backend>,
        Backend: Send + 'static,
    {
        let atom = self.clone();
        Effect::service(|env: &R| env.key_value_store()).flat_map(move |store| {
            let atom = atom.clone();
            store
                .get(Arc::clone(&atom.key), now)
                .map_error(PersistedAtomError::Read)
                .provide_some(|_: &R| ())
                .flat_map(move |read| match read {
                    StoreRead::Missing => Effect::sync(|| PersistedAtomLoad::Missing),
                    StoreRead::Expired(entry) => match entry.value.expires_at {
                        Some(expires_at) => Effect::sync_once(move || PersistedAtomLoad::Expired {
                            store_revision: entry.revision,
                            expires_at,
                        }),
                        None => Effect::from_result_once(Err(PersistedAtomError::Hydration(
                            HydrationError::InvalidRecord,
                        ))),
                    },
                    StoreRead::Found(entry) => {
                        let revision = entry.revision;
                        atom.source
                            .import(atom.schema.clone(), entry.value.bytes, policy)
                            .map_error(PersistedAtomError::Hydration)
                            .map(move |source| PersistedAtomLoad::Imported {
                                store_revision: revision,
                                source,
                            })
                    }
                })
        })
    }

    /// The caller supplies the observed backing revision. A provider with only
    /// get/put cannot satisfy this operation's conditional mutation contract.
    ///
    /// # Type parameters
    /// - `A`: Application value type. Bound: `Send + Sync + 'static`.
    /// - `R`: Environment type. Additional bound: `HasKeyValueStore + Send + Sync + 'static`. Additional bound: `:Store: ConditionalKeyValueStore<Error = Backend>`.
    /// - `Backend`: Backend-specific error or provider type. Additional bound: `Send + 'static`.
    ///
    /// # Parameters
    /// - `expected` (`Option<StoreRevision>`): Value or revision required for the proposed state transition.
    /// - `expires_at` (`Option<UtcMillis>`): The optional expiry instant.
    ///
    /// # Returns
    /// Type: `Effect<PersistedAtomSave, PersistedAtomError<Backend>, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    pub fn save<R, Backend>(
        &self,
        expected: Option<StoreRevision>,
        expires_at: Option<UtcMillis>,
    ) -> Effect<PersistedAtomSave, PersistedAtomError<Backend>, R>
    where
        R: HasKeyValueStore + Send + Sync + 'static,
        R::Store: ConditionalKeyValueStore<Error = Backend>,
        Backend: Send + 'static,
    {
        let atom = self.clone();
        self.source
            .export(self.schema.clone())
            .map_error(PersistedAtomError::Hydration)
            .flat_map(move |bytes| {
                let atom = atom.clone();
                let value = std::sync::Mutex::new(Some(StoredValue { bytes, expires_at }));
                Effect::service(|env: &R| env.key_value_store()).flat_map(move |store| {
                    let value = value
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .take();
                    let Some(value) = value else {
                        return Effect::from_result_once(Err(PersistedAtomError::InputConsumed));
                    };
                    let key = Arc::clone(&atom.key);
                    let error_key = Arc::clone(&atom.key);
                    store
                        .compare_exchange(key, expected, Some(value))
                        .map_error(move |source| PersistedAtomError::UncertainWrite {
                            key: Arc::clone(&error_key),
                            source,
                        })
                        .provide_some(|_: &R| ())
                        .map(|result| match result {
                            CompareExchange::Written(revision) => {
                                PersistedAtomSave::Written(revision)
                            }
                            CompareExchange::Conflict { actual } => {
                                PersistedAtomSave::Conflict { actual }
                            }
                        })
                })
            })
    }
}
