//! Checked construction of event definitions.

use super::{
    AggregateId, EncodedEventOutcome, EventEnvelope, EventGroupId, EventId, EventJournalId,
    EventOutcomeKind, EventTag, EventVersion,
};
use crate::encoding::{BinaryError, BinaryIdentity, BinaryLimits, SchemaBinary};
use crate::{JsonValue, PureCodec, UtcMillis};
use std::sync::Arc;

#[derive(Debug)]
/// A typed failure encoding or decoding an event payload or result.
pub enum EventCodecError {
    /// The binary identity, framing, or payload is invalid. Retains the `BinaryError` detail.
    Binary(BinaryError),
    /// The group differs from the required or recorded value.
    GroupMismatch,
    /// The tag differs from the required or recorded value.
    TagMismatch,
    /// The recorded version differs from the declared version.
    VersionMismatch,
    /// The required outcome is missing.
    MissingOutcome,
}

/// The stable group, tag, and version that identify a typed event.
pub struct EventShape {
    /// Stable event or request group identity. Type: `EventGroupId`.
    pub group: EventGroupId,
    /// The event or protocol tag. Type: `EventTag`.
    pub tag: EventTag,
    /// The declared compatibility version. Type: `EventVersion`.
    pub version: EventVersion,
}

/// The request, success, and domain-error codecs for a typed event.
///
/// # Type parameters
/// - `P`: Payload type.
/// - `A`: Application value type.
/// - `E`: Domain-error type.
pub struct EventCodecs<P, A, E> {
    /// The payload carried by this request or record. Type: `PureCodec<P, JsonValue>`.
    pub payload: PureCodec<P, JsonValue>,
    /// Codec or representation identity for successful results. Type: `PureCodec<A, JsonValue>`.
    pub success: PureCodec<A, JsonValue>,
    /// The typed error carried by this result. Type: `PureCodec<E, JsonValue>`.
    pub error: PureCodec<E, JsonValue>,
}

/// The selected binary format identity and limits for event records.
pub struct EventBinaryFormats {
    /// The payload carried by this request or record. Type: `BinaryIdentity`.
    pub payload: BinaryIdentity,
    /// Codec or representation identity for successful results. Type: `BinaryIdentity`.
    pub success: BinaryIdentity,
    /// The typed error carried by this result. Type: `BinaryIdentity`.
    pub error: BinaryIdentity,
    /// Explicit resource bounds applied to this operation. Type: `BinaryLimits`.
    pub limits: BinaryLimits,
}

/// The event payload, successful handler result and domain failure each
/// retain their own schema and binary identity. A caller chooses those
/// identities and resource limits; no Rust type name enters stored data.
///
/// # Type parameters
/// - `P`: Payload type.
/// - `A`: Application value type.
/// - `E`: Domain-error type.
pub struct EventDefinition<P, A, E> {
    /// Stable event or request group identity. Type: `EventGroupId`.
    pub group: EventGroupId,
    /// The event or protocol tag. Type: `EventTag`.
    pub tag: EventTag,
    /// The declared compatibility version. Type: `EventVersion`.
    pub version: EventVersion,
    payload: PureCodec<P, JsonValue>,
    success: PureCodec<A, JsonValue>,
    error: PureCodec<E, JsonValue>,
    payload_binary: Arc<SchemaBinary>,
    success_binary: Arc<SchemaBinary>,
    error_binary: Arc<SchemaBinary>,
}

impl<P, A, E> Clone for EventDefinition<P, A, E> {
    ///
    /// # Returns
    /// Type: `Self`.
    fn clone(&self) -> Self {
        Self {
            group: self.group.clone(),
            tag: self.tag.clone(),
            version: self.version,
            payload: self.payload.clone(),
            success: self.success.clone(),
            error: self.error.clone(),
            payload_binary: Arc::clone(&self.payload_binary),
            success_binary: Arc::clone(&self.success_binary),
            error_binary: Arc::clone(&self.error_binary),
        }
    }
}

impl<P: Send + 'static, A: Send + 'static, E: Send + 'static> EventDefinition<P, A, E> {
    #[must_use]
    /// Construct a `EventDefinition` from its required inputs and explicit policies.
    ///
    /// # Type parameters
    /// - `A`: Success or application-value type.
    /// - `E`: Domain-error type.
    /// - `P`: Payload type.
    ///
    /// # Parameters
    /// - `shape` (`EventShape`): Encoded-side structure required by this schema.
    /// - `codecs` (`EventCodecs<P, A, E>`): The codecs supplied to this operation.
    /// - `formats` (`EventBinaryFormats`): The formats supplied to this operation.
    ///
    /// # Returns
    /// Type: `Self`.
    pub fn new(
        shape: EventShape,
        codecs: EventCodecs<P, A, E>,
        formats: EventBinaryFormats,
    ) -> Self {
        let EventShape {
            group,
            tag,
            version,
        } = shape;
        let EventCodecs {
            payload,
            success,
            error,
        } = codecs;
        let EventBinaryFormats {
            payload: payload_identity,
            success: success_identity,
            error: error_identity,
            limits,
        } = formats;
        let payload_binary = Arc::new(SchemaBinary::new(
            payload.ast().clone(),
            payload_identity,
            limits,
        ));
        let success_binary = Arc::new(SchemaBinary::new(
            success.ast().clone(),
            success_identity,
            limits,
        ));
        let error_binary = Arc::new(SchemaBinary::new(
            error.ast().clone(),
            error_identity,
            limits,
        ));
        Self {
            group,
            tag,
            version,
            payload,
            success,
            error,
            payload_binary,
            success_binary,
            error_binary,
        }
    }

    /// Encode a typed event into its stable identity, version, and payload envelope.
    ///
    /// # Type parameters
    /// - `A`: Success or application-value type.
    /// - `E`: Domain-error type.
    /// - `P`: Payload type.
    ///
    /// # Parameters
    /// - `journal` (`EventJournalId`): The journal supplied to this operation.
    /// - `aggregate` (`AggregateId`): The aggregate supplied to this operation.
    /// - `id` (`EventId`): Stable identity of the selected record or operation.
    /// - `payload` (`&P`): Application data moved into the message or stored record.
    /// - `outcome` (`Option<&Result<A, E>>`): Complete terminal outcome, including expected errors and cleanup failures.
    /// - `occurred_at` (`UtcMillis`): The occurred at supplied to this operation.
    ///
    /// # Returns
    /// Type: `Result<EventEnvelope, EventCodecError>`.
    pub fn envelope(
        &self,
        journal: EventJournalId,
        aggregate: AggregateId,
        id: EventId,
        payload: &P,
        outcome: Option<&Result<A, E>>,
        occurred_at: UtcMillis,
    ) -> Result<EventEnvelope, EventCodecError> {
        let payload = self
            .payload_binary
            .encode_typed(&self.payload, payload)
            .map_err(EventCodecError::Binary)?;
        let outcome = outcome
            .map(|outcome| self.encode_outcome(outcome))
            .transpose()?;
        Ok(EventEnvelope {
            journal,
            aggregate,
            id,
            group: self.group.clone(),
            tag: self.tag.clone(),
            version: self.version,
            payload: Arc::from(payload),
            outcome,
            occurred_at,
        })
    }

    /// Validate an event envelope and decode its typed payload.
    ///
    /// # Type parameters
    /// - `A`: Application value type. Bound: `Send + 'static`.
    /// - `E`: Domain-error type. Bound: `Send + 'static`.
    /// - `P`: Payload type.
    ///
    /// # Parameters
    /// - `envelope` (`&EventEnvelope`): Versioned encoded message crossing the transport boundary.
    ///
    /// # Returns
    /// Type: `Result<P, EventCodecError>`.
    pub fn decode_payload(&self, envelope: &EventEnvelope) -> Result<P, EventCodecError> {
        self.validate(envelope)?;
        self.payload_binary
            .decode_typed(&self.payload, &envelope.payload)
            .map_err(EventCodecError::Binary)
    }

    /// Decode the recorded terminal outcome through its registered codecs.
    ///
    /// # Type parameters
    /// - `P`: Payload type. Bound: `Send + 'static`.
    /// - `A`: Success or application-value type.
    /// - `E`: Domain-error type.
    ///
    /// # Parameters
    /// - `envelope` (`&EventEnvelope`): Versioned encoded message crossing the transport boundary.
    ///
    /// # Returns
    /// Type: `Result<Result<A, E>, EventCodecError>`.
    pub fn decode_outcome(
        &self,
        envelope: &EventEnvelope,
    ) -> Result<Result<A, E>, EventCodecError> {
        self.validate(envelope)?;
        let outcome = envelope
            .outcome
            .as_ref()
            .ok_or(EventCodecError::MissingOutcome)?;
        match outcome.kind {
            EventOutcomeKind::Succeeded => self
                .success_binary
                .decode_typed(&self.success, &outcome.payload)
                .map(Ok)
                .map_err(EventCodecError::Binary),
            EventOutcomeKind::Failed => self
                .error_binary
                .decode_typed(&self.error, &outcome.payload)
                .map(Err)
                .map_err(EventCodecError::Binary),
        }
    }

    ///
    /// # Type parameters
    /// - `A`: Success or application-value type.
    /// - `E`: Domain-error type.
    ///
    /// # Parameters
    /// - `outcome` (`&Result<A, E>`): Complete terminal outcome, including expected errors and cleanup failures.
    ///
    /// # Returns
    /// Type: `Result<EncodedEventOutcome, EventCodecError>`.
    fn encode_outcome(
        &self,
        outcome: &Result<A, E>,
    ) -> Result<EncodedEventOutcome, EventCodecError> {
        let (kind, payload) = match outcome {
            Ok(value) => (
                EventOutcomeKind::Succeeded,
                self.success_binary.encode_typed(&self.success, value),
            ),
            Err(error) => (
                EventOutcomeKind::Failed,
                self.error_binary.encode_typed(&self.error, error),
            ),
        };
        Ok(EncodedEventOutcome {
            kind,
            payload: Arc::from(payload.map_err(EventCodecError::Binary)?),
        })
    }

    ///
    /// # Parameters
    /// - `envelope` (`&EventEnvelope`): Versioned encoded message crossing the transport boundary.
    ///
    /// # Returns
    /// Type: `Result<(), EventCodecError>`.
    fn validate(&self, envelope: &EventEnvelope) -> Result<(), EventCodecError> {
        if envelope.group != self.group {
            return Err(EventCodecError::GroupMismatch);
        }
        if envelope.tag != self.tag {
            return Err(EventCodecError::TagMismatch);
        }
        if envelope.version != self.version {
            return Err(EventCodecError::VersionMismatch);
        }
        Ok(())
    }
}
