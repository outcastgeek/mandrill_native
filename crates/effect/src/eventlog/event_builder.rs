use super::{
    AggregateId, EncodedEventOutcome, EventEnvelope, EventGroupId, EventId, EventJournalId,
    EventOutcomeKind, EventTag, EventVersion,
};
use crate::encoding::{BinaryError, BinaryIdentity, BinaryLimits, SchemaBinary};
use crate::{JsonValue, PureCodec, UtcMillis};
use std::sync::Arc;

#[derive(Debug)]
pub enum EventCodecError {
    Binary(BinaryError),
    GroupMismatch,
    TagMismatch,
    VersionMismatch,
    MissingOutcome,
}

pub struct EventShape {
    pub group: EventGroupId,
    pub tag: EventTag,
    pub version: EventVersion,
}

pub struct EventCodecs<P, A, E> {
    pub payload: PureCodec<P, JsonValue>,
    pub success: PureCodec<A, JsonValue>,
    pub error: PureCodec<E, JsonValue>,
}

pub struct EventBinaryFormats {
    pub payload: BinaryIdentity,
    pub success: BinaryIdentity,
    pub error: BinaryIdentity,
    pub limits: BinaryLimits,
}

/// The event payload, successful handler result and domain failure each
/// retain their own schema and binary identity. A caller chooses those
/// identities and resource limits; no Rust type name enters stored data.
pub struct EventDefinition<P, A, E> {
    pub group: EventGroupId,
    pub tag: EventTag,
    pub version: EventVersion,
    payload: PureCodec<P, JsonValue>,
    success: PureCodec<A, JsonValue>,
    error: PureCodec<E, JsonValue>,
    payload_binary: Arc<SchemaBinary>,
    success_binary: Arc<SchemaBinary>,
    error_binary: Arc<SchemaBinary>,
}

impl<P, A, E> Clone for EventDefinition<P, A, E> {
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

    pub fn decode_payload(&self, envelope: &EventEnvelope) -> Result<P, EventCodecError> {
        self.validate(envelope)?;
        self.payload_binary
            .decode_typed(&self.payload, &envelope.payload)
            .map_err(EventCodecError::Binary)
    }

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
