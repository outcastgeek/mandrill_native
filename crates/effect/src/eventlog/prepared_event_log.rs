//! Typed EventLog execution with preadmitted event and outcome backing.

use super::event_log::recorded_outcome_error;
use super::{
    EventCodecError, EventCommitPolicy, EventInvocation, EventOutcomeKind, EventRunError,
    EventRunOutcome, EventShape, JournalCursor, JournalStream, PreparedAppendDecision,
    PreparedEventHeader, PreparedEventInspection, PreparedJournalEntry, PreparedJournalError,
    PreparedMemoryEventJournal,
};
use crate::PureCodec;
use crate::encoding::{BinaryError, SchemaBinary};
use crate::memory::{Buffer, Bytes, MemoryError};
use crate::schema::{ParseContext, PreparedJsonArrayPool, SharedPreparedJsonValue};
use std::future::Future;
use std::sync::Arc;

/// A writer borrowed from one reserved journal record.
pub struct PreparedEventWriter<'a>(&'a mut Buffer<'static>);

impl PreparedEventWriter<'_> {
    #[must_use]
    pub fn capacity(&self) -> Bytes {
        self.0.capacity()
    }

    pub fn append(&mut self, bytes: &[u8]) -> Result<(), MemoryError> {
        self.0.extend_from_slice(bytes)
    }

    /// Use the existing prepared schema-binary codec with the admitted bytes.
    pub fn write_binary<A: Send + 'static>(
        &mut self,
        binary: &SchemaBinary,
        codec: &PureCodec<A, SharedPreparedJsonValue>,
        value: &A,
        context: Option<&ParseContext>,
        errors: &PreparedJsonArrayPool<BinaryError>,
    ) -> Result<(), EventCodecError> {
        binary
            .encode_prepared_typed_into(codec, value, context, self.0, errors)
            .map_err(EventCodecError::Binary)
    }
}

/// The typed codec roles of a prepared event. Application codec scratch is
/// independently supplied by the selected codec implementation.
pub trait PreparedEventCodec: Send + Sync {
    type Payload: Send + Sync + 'static;
    type Success: Send + 'static;
    type DomainError: Send + 'static;

    fn write_payload(
        &self,
        value: &Self::Payload,
        output: &mut PreparedEventWriter<'_>,
    ) -> Result<(), EventCodecError>;
    fn write_outcome(
        &self,
        value: &Result<Self::Success, Self::DomainError>,
        output: &mut PreparedEventWriter<'_>,
    ) -> Result<(), EventCodecError>;
    fn read_outcome(
        &self,
        kind: EventOutcomeKind,
        input: &[u8],
    ) -> Result<Result<Self::Success, Self::DomainError>, EventCodecError>;
}

pub struct PreparedEventLog<C> {
    shape: EventShape,
    commit: EventCommitPolicy,
    codec: C,
}

impl<C: PreparedEventCodec> PreparedEventLog<C> {
    #[must_use]
    pub const fn new(shape: EventShape, commit: EventCommitPolicy, codec: C) -> Self {
        Self {
            shape,
            commit,
            codec,
        }
    }

    /// Inspect duplicates before running the handler and hold both payload
    /// buffers through append. Append remains the authoritative decision.
    pub async fn execute<H: Send, F>(
        &self,
        journal: &PreparedMemoryEventJournal,
        invocation: EventInvocation<C::Payload>,
        handler: impl FnOnce(Arc<C::Payload>) -> F + Send,
    ) -> Result<EventRunOutcome<C::Success, C::DomainError>, EventRunError<PreparedJournalError, H>>
    where
        F: Future<Output = Result<Result<C::Success, C::DomainError>, H>> + Send,
    {
        let header = PreparedEventHeader {
            journal: invocation.journal,
            aggregate: invocation.aggregate,
            id: invocation.event,
            group: self.shape.group.clone(),
            tag: self.shape.tag.clone(),
            version: self.shape.version,
            occurred_at: invocation.occurred_at,
        };
        let stream = JournalStream {
            journal: header.journal.clone(),
            aggregate: header.aggregate.clone(),
        };
        let existing = match journal
            .inspect_prepared(&stream, &header.id)
            .map_err(EventRunError::Journal)?
        {
            PreparedEventInspection::Compacted => return Ok(EventRunOutcome::CompactedEventId),
            PreparedEventInspection::Present(entry) => Some(entry),
            PreparedEventInspection::Missing => None,
        };
        let mut bytes = journal
            .reserve_append(existing.is_none())
            .map_err(EventRunError::Journal)?;
        let payload = bytes.payload_mut();
        self.codec
            .write_payload(&invocation.payload, &mut PreparedEventWriter(payload))
            .map_err(EventRunError::Codec)?;
        if let Some(entry) = existing {
            if entry.header != header || entry.payload.as_ref() != payload.as_slice() {
                return Ok(EventRunOutcome::ConflictingEventId);
            }
            return self.decode_duplicate(entry);
        }
        let output = bytes
            .outcome_mut()
            .ok_or(EventRunError::RecordedWithoutOutcome)?;
        let result = handler(invocation.payload)
            .await
            .map_err(EventRunError::HandlerBeforeAppend)?;
        let result = match self.commit {
            EventCommitPolicy::SuccessOnly => match result {
                Err(error) => return Ok(EventRunOutcome::Rejected(error)),
                Ok(value) => Ok(value),
            },
            EventCommitPolicy::SuccessAndDomainFailure => result,
        };
        self.codec
            .write_outcome(&result, &mut PreparedEventWriter(output))
            .map_err(EventRunError::Codec)?;
        let kind = if result.is_ok() {
            EventOutcomeKind::Succeeded
        } else {
            EventOutcomeKind::Failed
        };
        let id = header.id.clone();
        let decision = journal
            .append_prepared(header, invocation.expected, bytes, Some(kind))
            .map_err(|source| EventRunError::UncertainAppend { event: id, source })?;
        match decision {
            PreparedAppendDecision::Committed(entry) => Ok(EventRunOutcome::Committed {
                result,
                cursor: JournalCursor::new(entry.sequence.get()),
            }),
            PreparedAppendDecision::Duplicate(entry) => self.decode_duplicate(entry),
            PreparedAppendDecision::ConflictingEventId => Ok(EventRunOutcome::ConflictingEventId),
            PreparedAppendDecision::CompactedEventId => Ok(EventRunOutcome::CompactedEventId),
            PreparedAppendDecision::StaleCursor { actual } => {
                Ok(EventRunOutcome::StaleCursor { actual })
            }
        }
    }

    fn decode_duplicate<H>(
        &self,
        entry: PreparedJournalEntry,
    ) -> Result<EventRunOutcome<C::Success, C::DomainError>, EventRunError<PreparedJournalError, H>>
    {
        let outcome = entry.outcome.ok_or(EventRunError::RecordedWithoutOutcome)?;
        let result = self
            .codec
            .read_outcome(outcome.kind, outcome.payload.as_ref())
            .map_err(recorded_outcome_error)?;
        Ok(EventRunOutcome::Duplicate {
            result,
            cursor: JournalCursor::new(entry.sequence.get()),
        })
    }
}
