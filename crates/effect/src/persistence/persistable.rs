use crate::{
    IssueSegment, JsonValue, KeyDomain, KeyError, KeyNamespace, KeySegment, ParseLimits, PureCodec,
    SchemaIssue, SchemaIssueKind, StableKey, StoredOutcome, ValueKind, stored_outcome_codec,
};
use std::sync::Arc;

type KeySegments<I> = dyn Fn(&I) -> Result<Vec<KeySegment>, KeyError> + Send + Sync;

/// Application-selected compatibility identity, separate from key identity and
/// the stored-outcome wire format version. Changing it requires a migration.
pub struct RecordVersion(Arc<str>);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecordVersionError {
    Empty,
}

impl RecordVersion {
    pub fn new(value: impl Into<Arc<str>>) -> Result<Self, RecordVersionError> {
        let value = value.into();
        if value.trim().is_empty() {
            return Err(RecordVersionError::Empty);
        }
        Ok(Self(value))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Clone for RecordVersion {
    fn clone(&self) -> Self {
        Self(Arc::clone(&self.0))
    }
}

/// The application chooses both schema compatibility and parser budgets.
pub struct RecordPolicy {
    version: RecordVersion,
    limits: ParseLimits,
}

impl RecordPolicy {
    #[must_use]
    pub fn new(version: RecordVersion, limits: ParseLimits) -> Self {
        Self { version, limits }
    }

    #[must_use]
    pub fn version(&self) -> &RecordVersion {
        &self.version
    }

    #[must_use]
    pub const fn limits(&self) -> ParseLimits {
        self.limits
    }
}

/// A typed record ties request identity to the complete success/error outcome
/// codec. It captures no store, clock, executor, or runtime provider.
pub struct Persistable<I, A: Send + 'static, E: Send + 'static, D: KeyDomain> {
    namespace: KeyNamespace<D>,
    version: RecordVersion,
    key_segments: Arc<KeySegments<I>>,
    outcome: PureCodec<StoredOutcome<A, E>, JsonValue>,
    limits: ParseLimits,
}

impl<I, A: Send + 'static, E: Send + 'static, D: KeyDomain> Clone for Persistable<I, A, E, D> {
    fn clone(&self) -> Self {
        Self {
            namespace: self.namespace.clone(),
            version: self.version.clone(),
            key_segments: Arc::clone(&self.key_segments),
            outcome: self.outcome.clone(),
            limits: self.limits,
        }
    }
}

#[derive(Debug)]
pub enum PersistedRecordError {
    Codec(SchemaIssue),
    VersionMismatch,
}

/// Outer record format version 1 is the JSON object
/// `{ "record_version": string, "outcome": StoredOutcomeV1 }`. The version
/// string is application supplied; the nested outcome has its own wire version.
mod wire {
    pub(super) const VERSION: &str = "record_version";
    pub(super) const OUTCOME: &str = "outcome";
}

impl<I, A: Send + 'static, E: Send + 'static, D: KeyDomain> Persistable<I, A, E, D> {
    #[must_use]
    pub fn with_policy(
        namespace: KeyNamespace<D>,
        policy: RecordPolicy,
        key_segments: impl Fn(&I) -> Result<Vec<KeySegment>, KeyError> + Send + Sync + 'static,
        success: PureCodec<A, JsonValue>,
        error: PureCodec<E, JsonValue>,
    ) -> Self {
        Self::new(
            namespace,
            policy.version,
            key_segments,
            success,
            error,
            policy.limits,
        )
    }

    #[must_use]
    pub fn new(
        namespace: KeyNamespace<D>,
        version: RecordVersion,
        key_segments: impl Fn(&I) -> Result<Vec<KeySegment>, KeyError> + Send + Sync + 'static,
        success: PureCodec<A, JsonValue>,
        error: PureCodec<E, JsonValue>,
        limits: ParseLimits,
    ) -> Self {
        Self {
            namespace,
            version,
            key_segments: Arc::new(key_segments),
            outcome: stored_outcome_codec(success, error, limits),
            limits,
        }
    }

    pub fn key(&self, input: &I) -> Result<StableKey<D>, KeyError> {
        StableKey::new(&self.namespace, (self.key_segments)(input)?)
    }

    pub fn encode(&self, value: &StoredOutcome<A, E>) -> Result<Arc<[u8]>, SchemaIssue> {
        let outcome = self.outcome.encode_ref(value)?;
        let encoded = JsonValue::Object(serde_json::Map::from_iter([
            (
                wire::VERSION.to_owned(),
                JsonValue::String(self.version.as_str().to_owned()),
            ),
            (wire::OUTCOME.to_owned(), outcome),
        ]));
        let bytes = crate::schema::internal::parser::encode_json_bytes(&encoded, &self.limits)?;
        Ok(Arc::from(bytes))
    }

    pub fn decode(&self, bytes: &[u8]) -> Result<StoredOutcome<A, E>, PersistedRecordError> {
        if bytes.len() > self.limits.bytes.get() {
            return Err(PersistedRecordError::Codec(SchemaIssue::new(
                SchemaIssueKind::LimitExceeded {
                    resource: crate::SchemaResource::Bytes,
                },
            )));
        }
        let encoded = crate::schema::internal::parser::parse_json_bytes(
            bytes,
            crate::schema::internal::parser::strict_wire_policy(self.limits),
        )
        .map_err(PersistedRecordError::Codec)?;
        let object = encoded.as_object().ok_or_else(|| {
            PersistedRecordError::Codec(SchemaIssue::new(SchemaIssueKind::TypeMismatch {
                expected: ValueKind::Record,
                actual: crate::value_kind(&encoded),
            }))
        })?;
        for name in object.keys() {
            if name != wire::VERSION && name != wire::OUTCOME {
                return Err(PersistedRecordError::Codec(
                    SchemaIssue::new(SchemaIssueKind::UnknownField(Arc::from(name.as_str())))
                        .at(IssueSegment::Field(Arc::from(name.as_str()))),
                ));
            }
        }
        let version = object.get(wire::VERSION).ok_or_else(|| {
            PersistedRecordError::Codec(SchemaIssue::new(SchemaIssueKind::MissingField(Arc::from(
                wire::VERSION,
            ))))
        })?;
        if version.as_str() != Some(self.version.as_str()) {
            return Err(PersistedRecordError::VersionMismatch);
        }
        let outcome = object.get(wire::OUTCOME).ok_or_else(|| {
            PersistedRecordError::Codec(SchemaIssue::new(SchemaIssueKind::MissingField(Arc::from(
                wire::OUTCOME,
            ))))
        })?;
        self.outcome
            .decode_ref(outcome)
            .map_err(PersistedRecordError::Codec)
    }
}
