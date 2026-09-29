use super::{OperationId, OperationIdError, RpcVersion};
use crate::encoding::{FrameDecodeFailure, FrameDecoder, FrameError, FrameParts, FramePolicy};
use crate::{JsonValue, ParseLimits, SchemaIssue, SchemaIssueKind, SchemaResource};
use std::collections::BTreeMap;
use std::num::{NonZeroU32, NonZeroU64};
use std::sync::Arc;

// These field names and tags define the interoperable RPC envelope format.
// They are protocol vocabulary, not application-configured policies.
mod wire {
    pub const OPERATION: &str = "operation";
    pub const REVISION: &str = "revision";
    pub const CALL: &str = "call";
    pub const KIND: &str = "kind";
    pub const PAYLOAD: &str = "payload";
    pub const METADATA: &str = "metadata";
    pub const SEQUENCE: &str = "sequence";
    pub const REQUEST: &str = "request";
    pub const SUCCESS: &str = "success";
    pub const DOMAIN_ERROR: &str = "domain_error";
    pub const STREAM_CHUNK: &str = "stream_chunk";
    pub const STREAM_END: &str = "stream_end";
    pub const ACKNOWLEDGE: &str = "acknowledge";
    pub const CANCEL: &str = "cancel";
    pub const REQUIRED_FIELDS: &[&str] = &[OPERATION, REVISION, CALL, KIND, PAYLOAD, METADATA];
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RpcCallId(Arc<str>);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RpcCallIdError {
    Empty,
    InvalidCharacter,
}

impl RpcCallId {
    pub fn new(value: impl Into<Arc<str>>) -> Result<Self, RpcCallIdError> {
        let value = value.into();
        if value.trim().is_empty() {
            return Err(RpcCallIdError::Empty);
        }
        if value.chars().any(|character| character.is_control()) {
            return Err(RpcCallIdError::InvalidCharacter);
        }
        Ok(Self(value))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RpcEnvelopeKind {
    Request,
    Success,
    DomainError,
    StreamChunk,
    StreamEnd,
    Acknowledge,
    Cancel,
}

impl RpcEnvelopeKind {
    const fn as_wire(self) -> &'static str {
        match self {
            Self::Request => wire::REQUEST,
            Self::Success => wire::SUCCESS,
            Self::DomainError => wire::DOMAIN_ERROR,
            Self::StreamChunk => wire::STREAM_CHUNK,
            Self::StreamEnd => wire::STREAM_END,
            Self::Acknowledge => wire::ACKNOWLEDGE,
            Self::Cancel => wire::CANCEL,
        }
    }

    fn from_wire(value: &str) -> Option<Self> {
        match value {
            wire::REQUEST => Some(Self::Request),
            wire::SUCCESS => Some(Self::Success),
            wire::DOMAIN_ERROR => Some(Self::DomainError),
            wire::STREAM_CHUNK => Some(Self::StreamChunk),
            wire::STREAM_END => Some(Self::StreamEnd),
            wire::ACKNOWLEDGE => Some(Self::Acknowledge),
            wire::CANCEL => Some(Self::Cancel),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RpcEnvelopePolicy {
    pub frame: FramePolicy,
    pub json: ParseLimits,
}

#[derive(Debug)]
pub enum RpcWireError {
    Frame(FrameError),
    Json(SchemaIssue),
    InvalidEnvelope,
    InvalidOperation(OperationIdError),
    InvalidCall(RpcCallIdError),
    InvalidRevision,
    UnknownKind,
    InvalidSequence,
    Poisoned,
}

#[derive(Debug)]
pub struct RpcWireFailure {
    /// Valid earlier frames from the same input chunk remain visible.
    pub completed: Vec<RpcEnvelope>,
    pub error: RpcWireError,
}

/// Metadata is protocol-neutral application data; it is not an HTTP-header
/// map. Payloads keep their Schema JSON representation until a typed
/// operation codec decodes them.
#[derive(Debug)]
pub struct RpcEnvelope {
    pub operation: OperationId,
    pub version: RpcVersion,
    pub call: RpcCallId,
    pub kind: RpcEnvelopeKind,
    /// Stream chunks and their acknowledgements carry a positive sequence.
    pub sequence: Option<NonZeroU64>,
    pub payload: JsonValue,
    pub metadata: BTreeMap<Arc<str>, Arc<str>>,
}

impl RpcEnvelope {
    pub fn encode(self, policy: RpcEnvelopePolicy) -> Result<FrameParts, RpcWireError> {
        validate_sequence(self.kind, self.sequence)?;
        let mut metadata = serde_json::Map::new();
        for (key, value) in self.metadata {
            metadata.insert(key.to_string(), JsonValue::String(value.to_string()));
        }
        let mut fields = serde_json::Map::from_iter([
            (
                wire::OPERATION.to_owned(),
                JsonValue::String(self.operation.as_str().to_owned()),
            ),
            (
                wire::REVISION.to_owned(),
                JsonValue::Number(self.version.value().get().into()),
            ),
            (
                wire::CALL.to_owned(),
                JsonValue::String(self.call.as_str().to_owned()),
            ),
            (
                wire::KIND.to_owned(),
                JsonValue::String(self.kind.as_wire().to_owned()),
            ),
            (wire::PAYLOAD.to_owned(), self.payload),
            (wire::METADATA.to_owned(), JsonValue::Object(metadata)),
        ]);
        if let Some(sequence) = self.sequence {
            fields.insert(
                wire::SEQUENCE.to_owned(),
                JsonValue::Number(sequence.get().into()),
            );
        }
        let value = JsonValue::Object(fields);
        let bytes = crate::schema::internal::parser::encode_json_bytes(&value, &policy.json)
            .map_err(RpcWireError::Json)?;
        FrameParts::new(bytes, policy.frame).map_err(RpcWireError::Frame)
    }

    pub fn decode(payload: &[u8], policy: RpcEnvelopePolicy) -> Result<Self, RpcWireError> {
        if payload.len() > policy.json.bytes.get() {
            return Err(RpcWireError::Json(SchemaIssue::new(
                SchemaIssueKind::LimitExceeded {
                    resource: SchemaResource::Bytes,
                },
            )));
        }
        let value = crate::schema::internal::parser::parse_json_bytes(
            payload,
            crate::schema::internal::parser::strict_wire_policy(policy.json),
        )
        .map_err(RpcWireError::Json)?;
        let JsonValue::Object(mut fields) = value else {
            return Err(RpcWireError::InvalidEnvelope);
        };
        let has_sequence = fields.contains_key(wire::SEQUENCE);
        if fields.len() != wire::REQUIRED_FIELDS.len() + usize::from(has_sequence)
            || fields
                .keys()
                .any(|key| key != wire::SEQUENCE && !wire::REQUIRED_FIELDS.contains(&key.as_str()))
        {
            return Err(RpcWireError::InvalidEnvelope);
        }
        let operation = fields
            .get(wire::OPERATION)
            .and_then(JsonValue::as_str)
            .ok_or(RpcWireError::InvalidEnvelope)
            .and_then(|value| {
                OperationId::new(Arc::<str>::from(value)).map_err(RpcWireError::InvalidOperation)
            })?;
        let version = fields
            .get(wire::REVISION)
            .and_then(JsonValue::as_u64)
            .and_then(|value| u32::try_from(value).ok())
            .and_then(NonZeroU32::new)
            .map(RpcVersion::new)
            .ok_or(RpcWireError::InvalidRevision)?;
        let call = fields
            .get(wire::CALL)
            .and_then(JsonValue::as_str)
            .ok_or(RpcWireError::InvalidEnvelope)
            .and_then(|value| {
                RpcCallId::new(Arc::<str>::from(value)).map_err(RpcWireError::InvalidCall)
            })?;
        let kind = fields
            .get(wire::KIND)
            .and_then(JsonValue::as_str)
            .and_then(RpcEnvelopeKind::from_wire)
            .ok_or(RpcWireError::UnknownKind)?;
        let sequence = fields
            .remove(wire::SEQUENCE)
            .map(|value| {
                value
                    .as_u64()
                    .and_then(NonZeroU64::new)
                    .ok_or(RpcWireError::InvalidSequence)
            })
            .transpose()?;
        validate_sequence(kind, sequence)?;
        let metadata = match fields.remove(wire::METADATA) {
            Some(JsonValue::Object(metadata)) => metadata
                .into_iter()
                .map(|(key, value)| match value {
                    JsonValue::String(value) => Ok((Arc::from(key), Arc::from(value))),
                    JsonValue::Null
                    | JsonValue::Bool(_)
                    | JsonValue::Number(_)
                    | JsonValue::Array(_)
                    | JsonValue::Object(_) => Err(RpcWireError::InvalidEnvelope),
                })
                .collect::<Result<BTreeMap<_, _>, _>>()?,
            _ => return Err(RpcWireError::InvalidEnvelope),
        };
        let payload = fields
            .remove(wire::PAYLOAD)
            .ok_or(RpcWireError::InvalidEnvelope)?;
        Ok(Self {
            operation,
            version,
            call,
            kind,
            sequence,
            payload,
            metadata,
        })
    }
}

fn validate_sequence(
    kind: RpcEnvelopeKind,
    sequence: Option<NonZeroU64>,
) -> Result<(), RpcWireError> {
    match (kind, sequence) {
        (RpcEnvelopeKind::StreamChunk | RpcEnvelopeKind::Acknowledge, Some(_))
        | (
            RpcEnvelopeKind::Request
            | RpcEnvelopeKind::Success
            | RpcEnvelopeKind::DomainError
            | RpcEnvelopeKind::StreamEnd
            | RpcEnvelopeKind::Cancel,
            None,
        ) => Ok(()),
        _ => Err(RpcWireError::InvalidSequence),
    }
}

/// Incremental frames can be split at any byte boundary. A malformed frame
/// poisons the underlying decoder; the caller creates a new session to retry.
pub struct RpcEnvelopeDecoder {
    frames: FrameDecoder,
    policy: RpcEnvelopePolicy,
    poisoned: bool,
}

impl RpcEnvelopeDecoder {
    #[must_use]
    pub const fn new(policy: RpcEnvelopePolicy) -> Self {
        Self {
            frames: FrameDecoder::new(policy.frame),
            policy,
            poisoned: false,
        }
    }

    pub fn push(&mut self, bytes: &[u8]) -> Result<Vec<RpcEnvelope>, RpcWireFailure> {
        if self.poisoned {
            return Err(RpcWireFailure {
                completed: Vec::new(),
                error: RpcWireError::Poisoned,
            });
        }
        let mut completed = Vec::new();
        match self.frames.push(bytes) {
            Ok(frames) => {
                for frame in frames {
                    match RpcEnvelope::decode(&frame, self.policy) {
                        Ok(envelope) => completed.push(envelope),
                        Err(error) => {
                            self.poisoned = true;
                            return Err(RpcWireFailure { completed, error });
                        }
                    }
                }
                Ok(completed)
            }
            Err(FrameDecodeFailure {
                completed: frames,
                error,
            }) => {
                for frame in frames {
                    match RpcEnvelope::decode(&frame, self.policy) {
                        Ok(envelope) => completed.push(envelope),
                        Err(error) => {
                            self.poisoned = true;
                            return Err(RpcWireFailure { completed, error });
                        }
                    }
                }
                self.poisoned = true;
                Err(RpcWireFailure {
                    completed,
                    error: RpcWireError::Frame(error),
                })
            }
        }
    }
}
