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
pub enum PersistedAtomError<Backend> {
    Hydration(HydrationError),
    Read(Backend),
    /// CAS may have committed before acknowledgement was lost.
    UncertainWrite {
        key: Arc<[u8]>,
        source: Backend,
    },
    InputConsumed,
}

pub enum PersistedAtomLoad {
    Missing,
    Expired {
        store_revision: StoreRevision,
        expires_at: UtcMillis,
    },
    Imported {
        store_revision: StoreRevision,
        source: HydrationImportOutcome,
    },
}

pub enum PersistedAtomSave {
    Written(StoreRevision),
    Conflict { actual: Option<StoreRevision> },
}

/// Source-atom snapshots use the same typed schema/version record as hydration.
/// Derived atoms recompute from sources. The local observation graph remains
/// distinct from the backing store's authority and notification guarantees.
pub struct PersistedAtom<A: Send + Sync + 'static> {
    source: AtomRef<A>,
    schema: HydrationSchema<A>,
    key: Arc<[u8]>,
}

impl<A: Send + Sync + 'static> Clone for PersistedAtom<A> {
    fn clone(&self) -> Self {
        Self {
            source: self.source.clone(),
            schema: self.schema.clone(),
            key: Arc::clone(&self.key),
        }
    }
}

impl<A: Send + Sync + 'static> PersistedAtom<A> {
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
