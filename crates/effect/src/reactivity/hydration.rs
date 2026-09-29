use super::{AtomId, AtomRef, AtomRefError, AtomRegistry};
use crate::persistence::RecordVersion;
use crate::{
    Effect, JsonValue, ParseLimits, PubSubError, PublishReport, PureCodec, SchemaIssue,
    SchemaIssueKind, SchemaResource,
};
use std::collections::{BTreeMap, BTreeSet};
use std::num::{NonZeroU64, NonZeroUsize};
use std::sync::{Arc, Mutex};

type HydrationExport<R> = dyn Fn() -> Effect<Arc<[u8]>, HydrationError, R> + Send + Sync;
type HydrationImport<R> = dyn Fn(Arc<[u8]>, HydrationConflictPolicy) -> Effect<HydrationImportOutcome, HydrationError, R>
    + Send
    + Sync;

// These names form the versioned export format. No runtime object, callback,
// subscription, task or service can appear in a serialized record.
mod wire {
    pub const ATOM: &str = "atom";
    pub const VERSION: &str = "version";
    pub const REVISION: &str = "revision";
    pub const VALUE: &str = "value";
    pub const FIELDS: &[&str] = &[ATOM, VERSION, REVISION, VALUE];
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HydrationConflictPolicy {
    /// Apply only if the incoming revision is strictly newer.
    OnlyIfNewer,
    /// Apply an older/equal record but advance the local revision so a stale
    /// completion cannot later appear newer than the imported state.
    ForceMonotonic,
}

#[derive(Debug)]
pub enum HydrationError {
    Source(AtomRefError),
    Schema(SchemaIssue),
    InvalidRecord,
    WrongAtom,
    VersionMismatch,
    Stale {
        current: NonZeroU64,
        incoming: NonZeroU64,
    },
    RevisionExhausted,
    InputConsumed,
    ForeignRegistry,
    DuplicateAtom,
    DuplicateRecord,
    UnknownAtom,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HydrationImportOutcome {
    pub revision: NonZeroU64,
    pub forced: bool,
    pub publication: PublishReport,
}

/// A typed export description; the codec is application-supplied and pure.
/// This record's compatibility version is separate from the atom key and the
/// codec's schema metadata.
pub struct HydrationSchema<A> {
    atom: AtomId,
    version: RecordVersion,
    codec: PureCodec<A, JsonValue>,
    limits: ParseLimits,
}

impl<A> Clone for HydrationSchema<A> {
    fn clone(&self) -> Self {
        Self {
            atom: self.atom.clone(),
            version: self.version.clone(),
            codec: self.codec.clone(),
            limits: self.limits,
        }
    }
}

impl<A: Send + 'static> HydrationSchema<A> {
    #[must_use]
    pub fn new(
        atom: AtomId,
        version: RecordVersion,
        codec: PureCodec<A, JsonValue>,
        limits: ParseLimits,
    ) -> Self {
        Self {
            atom,
            version,
            codec,
            limits,
        }
    }

    #[must_use]
    pub fn atom(&self) -> &AtomId {
        &self.atom
    }

    pub fn encode(&self, value: &A, revision: NonZeroU64) -> Result<Arc<[u8]>, HydrationError> {
        let encoded = self
            .codec
            .encode_ref(value)
            .map_err(HydrationError::Schema)?;
        let atom = JsonValue::Array(
            self.atom
                .as_bytes()
                .iter()
                .copied()
                .map(|byte| JsonValue::Number(u64::from(byte).into()))
                .collect(),
        );
        let record = JsonValue::Object(serde_json::Map::from_iter([
            (wire::ATOM.to_owned(), atom),
            (
                wire::VERSION.to_owned(),
                JsonValue::String(self.version.as_str().to_owned()),
            ),
            (
                wire::REVISION.to_owned(),
                JsonValue::Number(revision.get().into()),
            ),
            (wire::VALUE.to_owned(), encoded),
        ]));
        let bytes = crate::schema::internal::parser::encode_json_bytes(&record, &self.limits)
            .map_err(HydrationError::Schema)?;
        Ok(Arc::from(bytes))
    }

    pub fn decode(&self, bytes: &[u8]) -> Result<(A, NonZeroU64), HydrationError> {
        if bytes.len() > self.limits.bytes.get() {
            return Err(HydrationError::Schema(SchemaIssue::new(
                SchemaIssueKind::LimitExceeded {
                    resource: SchemaResource::Bytes,
                },
            )));
        }
        let record = crate::schema::internal::parser::parse_json_bytes(
            bytes,
            crate::schema::internal::parser::strict_wire_policy(self.limits),
        )
        .map_err(HydrationError::Schema)?;
        let JsonValue::Object(mut fields) = record else {
            return Err(HydrationError::InvalidRecord);
        };
        if fields.len() != wire::FIELDS.len()
            || fields
                .keys()
                .any(|field| !wire::FIELDS.contains(&field.as_str()))
        {
            return Err(HydrationError::InvalidRecord);
        }
        let atom = fields
            .remove(wire::ATOM)
            .ok_or(HydrationError::InvalidRecord)?;
        let JsonValue::Array(atom_bytes) = atom else {
            return Err(HydrationError::InvalidRecord);
        };
        if atom_bytes.len() != self.atom.as_bytes().len()
            || atom_bytes
                .iter()
                .zip(self.atom.as_bytes())
                .any(|(item, expected)| item.as_u64() != Some(u64::from(*expected)))
        {
            return Err(HydrationError::WrongAtom);
        }
        let version = fields
            .remove(wire::VERSION)
            .and_then(|value| value.as_str().map(str::to_owned))
            .ok_or(HydrationError::InvalidRecord)?;
        if version != self.version.as_str() {
            return Err(HydrationError::VersionMismatch);
        }
        let revision = fields
            .remove(wire::REVISION)
            .and_then(|value| value.as_u64())
            .and_then(NonZeroU64::new)
            .ok_or(HydrationError::InvalidRecord)?;
        let encoded = fields
            .remove(wire::VALUE)
            .ok_or(HydrationError::InvalidRecord)?;
        let value = self
            .codec
            .decode_ref(&encoded)
            .map_err(HydrationError::Schema)?;
        Ok((value, revision))
    }
}

pub struct HydrationRecordResult {
    pub atom: AtomId,
    pub outcome: Result<HydrationImportOutcome, HydrationError>,
}

struct Binding<R: Send + Sync + 'static> {
    atom: AtomId,
    export: Arc<HydrationExport<R>>,
    import: Arc<HydrationImport<R>>,
}

impl<R: Send + Sync + 'static> Clone for Binding<R> {
    fn clone(&self) -> Self {
        Self {
            atom: self.atom.clone(),
            export: Arc::clone(&self.export),
            import: Arc::clone(&self.import),
        }
    }
}

/// Heterogeneous registration erases only the already-typed Effect factories.
/// Values are never downcast; each binding retains its own PureCodec<A>.
pub struct HydrationSet<R: Send + Sync + 'static> {
    registry: AtomRegistry,
    routing_limits: ParseLimits,
    bindings: BTreeMap<Arc<[u8]>, Binding<R>>,
}

impl<R: Send + Sync + 'static> HydrationSet<R> {
    #[must_use]
    pub fn new(registry: AtomRegistry, routing_limits: ParseLimits) -> Self {
        Self {
            registry,
            routing_limits,
            bindings: BTreeMap::new(),
        }
    }

    pub fn register<A: Send + Sync + 'static>(
        &mut self,
        source: AtomRef<A>,
        schema: HydrationSchema<A>,
    ) -> Result<(), HydrationError> {
        if !self.registry.same_instance(&source.registry) {
            return Err(HydrationError::ForeignRegistry);
        }
        if source.id != schema.atom {
            return Err(HydrationError::WrongAtom);
        }
        let key: Arc<[u8]> = Arc::from(source.id.as_bytes());
        if self.bindings.contains_key(&key) {
            return Err(HydrationError::DuplicateAtom);
        }
        let export_source = source.clone();
        let import_source = source;
        let export_schema = schema.clone();
        let import_schema = schema;
        self.bindings.insert(
            key,
            Binding {
                atom: export_source.id.clone(),
                export: Arc::new(move || export_source.export(export_schema.clone())),
                import: Arc::new(move |bytes, policy| {
                    import_source.import(import_schema.clone(), bytes, policy)
                }),
            },
        );
        Ok(())
    }

    pub fn export_all(&self) -> Effect<Vec<Arc<[u8]>>, HydrationError, R> {
        let programs = self
            .bindings
            .values()
            .map(|binding| (binding.export)())
            .collect();
        Effect::all(programs)
    }

    /// Validate routing identities and duplicates for the entire batch before
    /// issuing any mutation. After that boundary each record has its own
    /// outcome; there is no invented multi-atom transaction guarantee.
    pub fn import_all(
        &self,
        records: Vec<Arc<[u8]>>,
        policy: HydrationConflictPolicy,
    ) -> Effect<Vec<HydrationRecordResult>, HydrationError, R> {
        let records = Mutex::new(Some(records));
        let bindings = self.bindings.clone();
        let limits = self.routing_limits;
        Effect::try_sync(
            move || {
                let records = records
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .take()
                    .ok_or(HydrationError::InputConsumed)?;
                let mut seen = BTreeSet::new();
                let mut selected = Vec::with_capacity(records.len());
                for record in records {
                    let key = routing_key(&record, limits)?;
                    if !seen.insert(Arc::clone(&key)) {
                        return Err(HydrationError::DuplicateRecord);
                    }
                    let binding = bindings.get(&key).ok_or(HydrationError::UnknownAtom)?;
                    selected.push((binding.clone(), record));
                }
                Ok(selected)
            },
            std::convert::identity,
        )
        .flat_map(move |selected| {
            let programs = selected
                .into_iter()
                .map(|(binding, record)| {
                    let atom = binding.atom.clone();
                    (binding.import)(record, policy)
                        .either()
                        .map(move |outcome| HydrationRecordResult {
                            atom: atom.clone(),
                            outcome,
                        })
                })
                .collect();
            Effect::all(programs)
        })
    }
}

fn routing_key(record: &[u8], limits: ParseLimits) -> Result<Arc<[u8]>, HydrationError> {
    if record.len() > limits.bytes.get() {
        return Err(HydrationError::Schema(SchemaIssue::new(
            SchemaIssueKind::LimitExceeded {
                resource: SchemaResource::Bytes,
            },
        )));
    }
    let value = crate::schema::internal::parser::parse_json_bytes(
        record,
        crate::schema::internal::parser::strict_wire_policy(limits),
    )
    .map_err(HydrationError::Schema)?;
    let fields = value.as_object().ok_or(HydrationError::InvalidRecord)?;
    let items = fields
        .get(wire::ATOM)
        .and_then(JsonValue::as_array)
        .ok_or(HydrationError::InvalidRecord)?;
    let bytes = items
        .iter()
        .map(|item| {
            item.as_u64()
                .and_then(|value| u8::try_from(value).ok())
                .ok_or(HydrationError::InvalidRecord)
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Arc::from(bytes))
}

impl<A: Send + Sync + 'static> AtomRef<A> {
    /// The mutation gate keeps the value and revision from different commits
    /// from being combined into one exported record.
    pub fn export<R: Send + Sync + 'static>(
        &self,
        schema: HydrationSchema<A>,
    ) -> Effect<Arc<[u8]>, HydrationError, R> {
        if schema.atom != self.id {
            return Effect::from_result_once(Err(HydrationError::WrongAtom));
        }
        let source = self.clone();
        self.mutation
            .acquire(NonZeroUsize::MIN)
            .map_error(|error| HydrationError::Source(AtomRefError::Admission(error)))
            .flat_map(move |permit| {
                let source = source.clone();
                let schema = schema.clone();
                let snapshot = source
                    .value
                    .read()
                    .map_error(|error| HydrationError::Source(AtomRefError::Publication(error)))
                    .flat_map(move |value| {
                        let revision = *source
                            .revision
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner);
                        Effect::from_result_once(schema.encode(&value, revision))
                    });
                snapshot.ensuring(Effect::sync_once(move || drop(permit)))
            })
    }

    pub fn import<R: Send + Sync + 'static>(
        &self,
        schema: HydrationSchema<A>,
        bytes: Arc<[u8]>,
        policy: HydrationConflictPolicy,
    ) -> Effect<HydrationImportOutcome, HydrationError, R> {
        if schema.atom != self.id {
            return Effect::from_result_once(Err(HydrationError::WrongAtom));
        }
        let source = self.clone();
        let bytes = Mutex::new(Some(bytes));
        Effect::try_sync(
            move || {
                let bytes = bytes
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .take()
                    .ok_or(HydrationError::InputConsumed)?;
                schema.decode(&bytes)
            },
            std::convert::identity,
        )
        .flat_map(move |(value, incoming)| {
            let source = source.clone();
            let value = Mutex::new(Some(value));
            source
                .mutation
                .acquire(NonZeroUsize::MIN)
                .map_error(|error| HydrationError::Source(AtomRefError::Admission(error)))
                .flat_map(move |permit| {
                    let value = value
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .take();
                    let Some(value) = value else {
                        return Effect::from_result_once(Err(HydrationError::InputConsumed));
                    };
                    let current = *source
                        .revision
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    let (revision, forced) = match policy {
                        HydrationConflictPolicy::OnlyIfNewer if incoming <= current => {
                            return Effect::from_result_once(Err(HydrationError::Stale {
                                current,
                                incoming,
                            }));
                        }
                        HydrationConflictPolicy::OnlyIfNewer => (incoming, false),
                        HydrationConflictPolicy::ForceMonotonic if incoming > current => {
                            (incoming, true)
                        }
                        HydrationConflictPolicy::ForceMonotonic => {
                            let Some(revision) =
                                current.get().checked_add(1).and_then(NonZeroU64::new)
                            else {
                                return Effect::from_result_once(Err(
                                    HydrationError::RevisionExhausted,
                                ));
                            };
                            (revision, true)
                        }
                    };
                    let revision_state = Arc::clone(&source.revision);
                    let registry = source.registry.clone();
                    let id = source.id.clone();
                    let imported = source
                        .value
                        .set_with_commit(value, move |_| {
                            *revision_state
                                .lock()
                                .unwrap_or_else(std::sync::PoisonError::into_inner) = revision;
                        })
                        .map_error(|error: PubSubError| {
                            HydrationError::Source(AtomRefError::Publication(error))
                        })
                        .flat_map(move |publication| {
                            registry
                                .changed(id.clone())
                                .map_error(|error| {
                                    HydrationError::Source(
                                        AtomRefError::CommittedWithoutInvalidation(error),
                                    )
                                })
                                .map(move |_| HydrationImportOutcome {
                                    revision,
                                    forced,
                                    publication,
                                })
                        });
                    imported
                        .uninterruptible()
                        .ensuring(Effect::sync_once(move || drop(permit)))
                })
        })
    }
}
