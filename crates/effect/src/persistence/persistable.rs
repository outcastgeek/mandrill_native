//! Stable record identities, codecs, and version policy.

use crate::{
    IssueSegment, JsonValue, KeyDomain, KeyError, KeyNamespace, KeySegment, ParseLimits, PureCodec,
    SchemaIssue, SchemaIssueKind, StableKey, StoredOutcome, ValueKind, stored_outcome_codec,
};
use std::sync::Arc;

/// A process-local callback dispatched through a Rust vtable. Persisted keys
/// contain its resulting segments, never the callback or its vtable.
///
/// # Type parameters
/// - `I`: Input type.
type KeySegments<I> = dyn Fn(&I) -> Result<Vec<KeySegment>, KeyError> + Send + Sync;

/// Application-selected compatibility identity, separate from key identity and
/// the stored-outcome wire format version. Changing it requires a migration.
pub struct RecordVersion(Arc<str>);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// A stored record version incompatible with its registered schema.
pub enum RecordVersionError {
    /// No value is present.
    Empty,
}

impl RecordVersion {
    /// Construct a `RecordVersion` from its required inputs and explicit policies.
    ///
    /// # Parameters
    /// - `value` (`impl Into<Arc<str>>`): Value read, transformed, encoded, or stored by this operation.
    ///
    /// # Returns
    /// Type: `Result<Self, RecordVersionError>`.
    pub fn new(value: impl Into<Arc<str>>) -> Result<Self, RecordVersionError> {
        let value = value.into();
        if value.trim().is_empty() {
            return Err(RecordVersionError::Empty);
        }
        Ok(Self(value))
    }

    #[must_use]
    /// Borrow the textual representation without allocating a new string.
    ///
    /// # Returns
    /// Type: `&str`.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Clone for RecordVersion {
    ///
    /// # Returns
    /// Type: `Self`.
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
    /// Construct a `RecordPolicy` from its required inputs and explicit policies.
    ///
    /// # Parameters
    /// - `version` (`RecordVersion`): Registered schema, event, or workflow compatibility version.
    /// - `limits` (`ParseLimits`): Explicit resource bounds enforced while processing this input.
    ///
    /// # Returns
    /// Type: `Self`.
    pub fn new(version: RecordVersion, limits: ParseLimits) -> Self {
        Self { version, limits }
    }

    #[must_use]
    /// Return the schema version used to interpret persisted records.
    ///
    /// # Returns
    /// Type: `&RecordVersion`.
    pub fn version(&self) -> &RecordVersion {
        &self.version
    }

    #[must_use]
    /// # Returns
    /// Type: `ParseLimits`.
    pub const fn limits(&self) -> ParseLimits {
        self.limits
    }
}

/// A typed record ties request identity to the complete success/error outcome
/// codec. It captures no store, clock, executor, or runtime provider.
///
/// # Type parameters
/// - `I`: Input type.
/// - `A`: Application value type. Bound or type: `Send + 'static`.
/// - `E`: Domain-error type. Bound or type: `Send + 'static`.
/// - `D`: Identity-domain type. Bound or type: `KeyDomain`.
pub struct Persistable<I, A: Send + 'static, E: Send + 'static, D: KeyDomain> {
    namespace: KeyNamespace<D>,
    version: RecordVersion,
    key_segments: Arc<KeySegments<I>>,
    outcome: PureCodec<StoredOutcome<A, E>, JsonValue>,
    limits: ParseLimits,
}

impl<I, A: Send + 'static, E: Send + 'static, D: KeyDomain> Clone for Persistable<I, A, E, D> {
    ///
    /// # Returns
    /// Type: `Self`.
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
/// A codec, key, version, or store failure for a typed persisted record.
pub enum PersistedRecordError {
    /// Encoding or decoding the typed value failed. Retains the `SchemaIssue` detail.
    Codec(SchemaIssue),
    /// The recorded version differs from the declared version.
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
    /// Apply explicit parse and representation limits to this persisted definition.
    ///
    /// # Type parameters
    /// - `A`: Success or application-value type.
    /// - `D`: Stable-key domain type.
    /// - `E`: Domain-error type.
    /// - `I`: Input type.
    ///
    /// # Parameters
    /// - `namespace` (`KeyNamespace<D>`): The namespace supplied to this operation.
    /// - `policy` (`RecordPolicy`): The caller-supplied behavior policy.
    /// - `key_segments` (`impl Fn(&I) -> Result<Vec<KeySegment>, KeyError> + Send + Sync + 'static`): The key segments supplied to this operation.
    /// - `success` (`PureCodec<A, JsonValue>`): Successful application value or its codec.
    /// - `error` (`PureCodec<E, JsonValue>`): The error to propagate or record.
    ///
    /// # Returns
    /// Type: `Self`.
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
    /// Construct a `Persistable` from its required inputs and explicit policies.
    ///
    /// # Type parameters
    /// - `A`: Success or application-value type.
    /// - `D`: Stable-key domain type.
    /// - `E`: Domain-error type.
    /// - `I`: Input type.
    ///
    /// # Parameters
    /// - `namespace` (`KeyNamespace<D>`): The namespace supplied to this operation.
    /// - `version` (`RecordVersion`): Registered schema, event, or workflow compatibility version.
    /// - `key_segments` (`impl Fn(&I) -> Result<Vec<KeySegment>, KeyError> + Send + Sync + 'static`): The key segments supplied to this operation.
    /// - `success` (`PureCodec<A, JsonValue>`): Successful application value or its codec.
    /// - `error` (`PureCodec<E, JsonValue>`): The error to propagate or record.
    /// - `limits` (`ParseLimits`): Explicit resource bounds enforced while processing this input.
    ///
    /// # Returns
    /// Type: `Self`.
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

    /// Borrow the stable key identifying this value.
    ///
    /// # Type parameters
    /// - `A`: Application value type. Bound: `Send + 'static`.
    /// - `E`: Domain-error type. Bound: `Send + 'static`.
    /// - `D`: Stable-key domain type.
    /// - `I`: Input type.
    ///
    /// # Parameters
    /// - `input` (`&I`): The input value or factory consumed by this operation.
    ///
    /// # Returns
    /// Type: `Result<StableKey<D>, KeyError>`.
    pub fn key(&self, input: &I) -> Result<StableKey<D>, KeyError> {
        StableKey::new(&self.namespace, (self.key_segments)(input)?)
    }

    /// Encode the typed value using this definition’s declared representation.
    ///
    /// # Type parameters
    /// - `I`: Input type.
    /// - `D`: Identity-domain type. Bound: `KeyDomain`.
    /// - `A`: Success or application-value type.
    /// - `E`: Domain-error type.
    ///
    /// # Parameters
    /// - `value` (`&StoredOutcome<A, E>`): Value read, transformed, encoded, or stored by this operation.
    ///
    /// # Returns
    /// Type: `Result<Arc<[u8]>, SchemaIssue>`.
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

    /// Validate the encoded input and reconstruct its typed value.
    ///
    /// # Type parameters
    /// - `I`: Input type.
    /// - `D`: Identity-domain type. Bound: `KeyDomain`.
    /// - `A`: Success or application-value type.
    /// - `E`: Domain-error type.
    ///
    /// # Parameters
    /// - `bytes` (`&[u8]`): Encoded bytes to validate, decode, write, or transfer.
    ///
    /// # Returns
    /// Type: `Result<StoredOutcome<A, E>, PersistedRecordError>`.
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
