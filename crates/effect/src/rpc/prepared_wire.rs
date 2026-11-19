//! Prepared JSON envelope owners over the existing versioned framing format.

use super::envelope::{validate_sequence, wire};
use super::{OperationId, RpcCallId, RpcEnvelopeKind, RpcEnvelopePolicy, RpcVersion, RpcWireError};
use crate::encoding::{CompleteFrames, PreparedFrameError, complete_frames, encode_frame_into};
use crate::memory::{Buffer, MemoryError};
use crate::schema::PreparedJsonLimitsReservation;
use crate::schema::internal::parser::{PreparedJsonParseReservation, strict_wire_policy};
use crate::{
    JsonWriter, JsonWriterPolicy, ParseLimits, PreparedJsonDocument, PreparedJsonKind,
    PreparedJsonParserPool, PreparedJsonValidationPool, PreparedJsonValue, PreparedJsonWriterPool,
    SchemaIssue, SchemaIssueKind,
};
use std::num::{NonZeroU32, NonZeroU64};
use std::sync::Arc;

/// A refusal at the physical, framing, envelope or operation-schema boundary.
#[derive(Debug)]
pub enum PreparedRpcWireError {
    Memory(MemoryError),
    Wire(RpcWireError),
    Payload(SchemaIssue),
    MissingFrame,
    ExtraFrames,
}

impl From<RpcWireError> for PreparedRpcWireError {
    fn from(error: RpcWireError) -> Self {
        Self::Wire(error)
    }
}

impl From<PreparedFrameError> for PreparedRpcWireError {
    fn from(error: PreparedFrameError) -> Self {
        match error {
            PreparedFrameError::Memory(error) => Self::Memory(error),
            PreparedFrameError::Framing(error) => Self::Wire(RpcWireError::Frame(error)),
        }
    }
}

/// Borrowed correlation and framing metadata for one message.
#[derive(Clone, Copy, Debug)]
pub struct PreparedRpcHeader<'a> {
    pub operation: &'a str,
    pub version: RpcVersion,
    pub call: &'a str,
    pub kind: RpcEnvelopeKind,
    pub sequence: Option<NonZeroU64>,
}

/// Fixed pools used by the selected RPC wire path.
pub struct PreparedRpcWire {
    writers: Arc<PreparedJsonWriterPool>,
    parser: Arc<PreparedJsonParserPool>,
    validation: Arc<PreparedJsonValidationPool>,
}

impl PreparedRpcWire {
    #[must_use]
    pub const fn new(
        writers: Arc<PreparedJsonWriterPool>,
        parser: Arc<PreparedJsonParserPool>,
        validation: Arc<PreparedJsonValidationPool>,
    ) -> Self {
        Self {
            writers,
            parser,
            validation,
        }
    }

    /// Claim the operation and envelope writer/parser owners before calling a codec.
    pub fn reserve_encode(
        &self,
        policy: RpcEnvelopePolicy,
        limits: ParseLimits,
    ) -> Result<PreparedRpcEncoder, MemoryError> {
        let payload = self.writers.admit_serde_buffered(writer_policy(limits))?;
        let payload_parse = self.parser.reserve_parse(strict_wire_policy(limits))?;
        let envelope = self
            .writers
            .admit_serde_buffered(writer_policy(policy.json))?;
        let envelope_parse = self.parser.reserve_parse(strict_wire_policy(policy.json))?;
        Ok(PreparedRpcEncoder {
            policy,
            payload,
            payload_parse,
            envelope,
            envelope_parse,
        })
    }

    /// Claim borrowed envelope decoding and the later typed payload limit check.
    pub fn reserve_decode(
        &self,
        policy: RpcEnvelopePolicy,
        limits: ParseLimits,
    ) -> Result<PreparedRpcDecoder, MemoryError> {
        let envelope = self.parser.reserve_parse(strict_wire_policy(policy.json))?;
        let payload = self.validation.reserve_parse_limits(limits)?;
        Ok(PreparedRpcDecoder {
            policy,
            envelope,
            payload,
        })
    }
}

const fn writer_policy(limits: ParseLimits) -> JsonWriterPolicy {
    JsonWriterPolicy {
        bytes: limits.bytes,
        depth: limits.depth,
    }
}

/// The actual structural and byte owners for one admitted frame write.
pub struct PreparedRpcEncoder {
    policy: RpcEnvelopePolicy,
    payload: JsonWriter,
    payload_parse: PreparedJsonParseReservation,
    envelope: JsonWriter,
    envelope_parse: PreparedJsonParseReservation,
}

impl PreparedRpcEncoder {
    /// Write into an admitted destination, preserving its prefix on failure.
    /// The codec writes one payload into the selected operation writer.
    pub fn encode<'a>(
        self,
        header: PreparedRpcHeader<'_>,
        metadata: impl IntoIterator<Item = (&'a str, &'a str)>,
        output: &mut Buffer<'static>,
        payload: impl FnOnce(&mut JsonWriter) -> Result<(), SchemaIssue>,
    ) -> Result<(), PreparedRpcWireError> {
        let Self {
            policy,
            payload: mut writer,
            payload_parse,
            mut envelope,
            envelope_parse,
        } = self;
        validate_sequence(header.kind, header.sequence)?;
        OperationId::validate(header.operation).map_err(RpcWireError::InvalidOperation)?;
        RpcCallId::validate(header.call).map_err(RpcWireError::InvalidCall)?;
        payload(&mut writer).map_err(payload_issue)?;
        let payload = writer.finish_prepared().map_err(payload_issue)?;
        let document = payload_parse
            .parse(payload.as_bytes())
            .map_err(payload_issue)?;
        drop(payload);

        write_envelope(&mut envelope, header, metadata, document.root()).map_err(envelope_issue)?;
        drop(document);
        let envelope = envelope.finish_prepared().map_err(envelope_issue)?;
        // Apply the same strict envelope limits as ordinary encoded RPC before
        // the completed wire frame becomes visible to the transport.
        drop(
            envelope_parse
                .parse(envelope.as_bytes())
                .map_err(envelope_issue)?,
        );
        encode_frame_into(envelope.as_bytes(), output, policy.frame)?;
        Ok(())
    }
}

fn write_envelope<'a>(
    writer: &mut JsonWriter,
    header: PreparedRpcHeader<'_>,
    metadata: impl IntoIterator<Item = (&'a str, &'a str)>,
    payload: PreparedJsonValue<'_>,
) -> Result<(), SchemaIssue> {
    writer.begin_object()?;
    writer.property(wire::CALL)?;
    writer.string(header.call)?;
    writer.property(wire::KIND)?;
    writer.string(header.kind.as_wire())?;
    writer.property(wire::METADATA)?;
    writer.begin_object()?;
    for (key, value) in metadata {
        writer.property(key)?;
        writer.string(value)?;
    }
    writer.end_object()?;
    writer.property(wire::OPERATION)?;
    writer.string(header.operation)?;
    writer.property(wire::PAYLOAD)?;
    payload.write_to(writer)?;
    writer.property(wire::REVISION)?;
    writer.u64(u64::from(header.version.value().get()))?;
    if let Some(sequence) = header.sequence {
        writer.property(wire::SEQUENCE)?;
        writer.u64(sequence.get())?;
    }
    writer.end_object()
}

fn payload_issue(issue: SchemaIssue) -> PreparedRpcWireError {
    if let SchemaIssueKind::Memory(error) = issue.kind() {
        PreparedRpcWireError::Memory(*error)
    } else {
        PreparedRpcWireError::Payload(issue)
    }
}

fn envelope_issue(issue: SchemaIssue) -> PreparedRpcWireError {
    if let SchemaIssueKind::Memory(error) = issue.kind() {
        PreparedRpcWireError::Memory(*error)
    } else {
        PreparedRpcWireError::Wire(RpcWireError::Json(issue))
    }
}

/// Physical owners for decoding one framed response or request.
pub struct PreparedRpcDecoder {
    policy: RpcEnvelopePolicy,
    envelope: PreparedJsonParseReservation,
    payload: PreparedJsonLimitsReservation,
}

impl PreparedRpcDecoder {
    /// Decode one complete message without materializing a frame vector or a native JSON DOM.
    pub fn decode(self, frame: &[u8]) -> Result<PreparedRpcEnvelope, PreparedRpcWireError> {
        let payload =
            match complete_frames(frame, self.policy.frame).map_err(RpcWireError::Frame)? {
                CompleteFrames::Missing => return Err(PreparedRpcWireError::MissingFrame),
                CompleteFrames::One(payload) => payload,
                CompleteFrames::Extra => return Err(PreparedRpcWireError::ExtraFrames),
            };
        let document = self.envelope.parse(payload).map_err(envelope_issue)?;
        let header = read_header(document.root())?;
        let metadata = document
            .root()
            .get(wire::METADATA)
            .ok_or(RpcWireError::InvalidEnvelope)?;
        validate_metadata(metadata)?;
        Ok(PreparedRpcEnvelope {
            version: header.version,
            kind: header.kind,
            sequence: header.sequence,
            document,
            payload: Some(self.payload),
        })
    }
}

/// A parsed envelope retaining its regional node and decoded text backing.
pub struct PreparedRpcEnvelope {
    document: PreparedJsonDocument,
    version: RpcVersion,
    kind: RpcEnvelopeKind,
    sequence: Option<NonZeroU64>,
    payload: Option<PreparedJsonLimitsReservation>,
}

impl PreparedRpcEnvelope {
    /// Borrow validated header fields without creating correlation owners.
    #[must_use]
    #[expect(
        clippy::expect_used,
        reason = "decode checks both string fields before exposing the envelope"
    )]
    pub fn header(&self) -> PreparedRpcHeader<'_> {
        let root = self.document.root();
        PreparedRpcHeader {
            operation: root
                .get(wire::OPERATION)
                .and_then(PreparedJsonValue::as_str)
                .expect("validated operation"),
            call: root
                .get(wire::CALL)
                .and_then(PreparedJsonValue::as_str)
                .expect("validated call"),
            version: self.version,
            kind: self.kind,
            sequence: self.sequence,
        }
    }

    /// Borrow the payload from the admitted document.
    #[must_use]
    #[expect(
        clippy::expect_used,
        reason = "envelope validation requires a payload field"
    )]
    pub fn payload(&self) -> PreparedJsonValue<'_> {
        self.document
            .root()
            .get(wire::PAYLOAD)
            .expect("validated payload")
    }

    /// Borrow metadata in wire order without collecting or copying entries.
    pub fn metadata(&self) -> impl Iterator<Item = (&str, &str)> {
        let mut next = self
            .document
            .root()
            .get(wire::METADATA)
            .and_then(PreparedJsonValue::first_child);
        std::iter::from_fn(move || {
            let child = next?;
            next = child.next_sibling();
            Some((child.property_name()?, child.as_str()?))
        })
    }

    /// Use the preadmitted payload limit check after operation and correlation validation.
    pub fn check_payload(&mut self) -> Result<(), SchemaIssue> {
        let check = self
            .payload
            .take()
            .ok_or_else(|| SchemaIssue::new(SchemaIssueKind::InvalidDefinition))?;
        check.check(self.payload())
    }
}

fn read_header(root: PreparedJsonValue<'_>) -> Result<PreparedRpcHeader<'_>, RpcWireError> {
    if root.kind() != Some(PreparedJsonKind::Object) {
        return Err(RpcWireError::InvalidEnvelope);
    }
    let sequence_field = root.get(wire::SEQUENCE);
    if root.len() != wire::REQUIRED_FIELDS.len() + usize::from(sequence_field.is_some()) {
        return Err(RpcWireError::InvalidEnvelope);
    }
    let mut field = root.first_child();
    while let Some(child) = field {
        let name = child.property_name().ok_or(RpcWireError::InvalidEnvelope)?;
        if name != wire::SEQUENCE && !wire::REQUIRED_FIELDS.contains(&name) {
            return Err(RpcWireError::InvalidEnvelope);
        }
        field = child.next_sibling();
    }
    let operation = root
        .get(wire::OPERATION)
        .and_then(PreparedJsonValue::as_str)
        .ok_or(RpcWireError::InvalidEnvelope)?;
    OperationId::validate(operation).map_err(RpcWireError::InvalidOperation)?;
    let version = root
        .get(wire::REVISION)
        .and_then(unsigned)
        .and_then(|value| u32::try_from(value).ok())
        .and_then(NonZeroU32::new)
        .map(RpcVersion::new)
        .ok_or(RpcWireError::InvalidRevision)?;
    let call = root
        .get(wire::CALL)
        .and_then(PreparedJsonValue::as_str)
        .ok_or(RpcWireError::InvalidEnvelope)?;
    RpcCallId::validate(call).map_err(RpcWireError::InvalidCall)?;
    let kind = root
        .get(wire::KIND)
        .and_then(PreparedJsonValue::as_str)
        .and_then(RpcEnvelopeKind::from_wire)
        .ok_or(RpcWireError::UnknownKind)?;
    let sequence = sequence_field
        .map(|value| {
            unsigned(value)
                .and_then(NonZeroU64::new)
                .ok_or(RpcWireError::InvalidSequence)
        })
        .transpose()?;
    validate_sequence(kind, sequence)?;
    Ok(PreparedRpcHeader {
        operation,
        version,
        call,
        kind,
        sequence,
    })
}

fn unsigned(value: PreparedJsonValue<'_>) -> Option<u64> {
    value.number_token()?.parse().ok()
}

fn validate_metadata(value: PreparedJsonValue<'_>) -> Result<(), RpcWireError> {
    if value.kind() != Some(PreparedJsonKind::Object) {
        return Err(RpcWireError::InvalidEnvelope);
    }
    let mut field = value.first_child();
    while let Some(child) = field {
        if child.as_str().is_none() {
            return Err(RpcWireError::InvalidEnvelope);
        }
        field = child.next_sibling();
    }
    Ok(())
}
