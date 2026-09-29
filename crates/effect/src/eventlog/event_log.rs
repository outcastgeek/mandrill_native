use super::{
    AggregateId, AppendDecision, AppendReceipt, EventCodecError, EventDefinition, EventEnvelope,
    EventId, EventInspection, EventJournal, EventJournalId, HasEventJournal, JournalCursor,
    JournalStream,
};
use crate::{Effect, UtcMillis};
use std::sync::{Arc, Mutex};

type JournalError<R> = <<R as HasEventJournal>::Journal as EventJournal>::Error;
pub type EventRunEffect<A, E, HandlerError, R> =
    Effect<EventRunOutcome<A, E>, EventRunError<JournalError<R>, HandlerError>, R>;

pub struct EventInvocation<P> {
    pub journal: EventJournalId,
    pub aggregate: AggregateId,
    pub event: EventId,
    pub expected: JournalCursor,
    pub occurred_at: UtcMillis,
    pub payload: Arc<P>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EventCommitPolicy {
    SuccessOnly,
    SuccessAndDomainFailure,
}

pub enum EventRunOutcome<A, E> {
    Committed {
        result: Result<A, E>,
        cursor: JournalCursor,
    },
    Duplicate {
        result: Result<A, E>,
        cursor: JournalCursor,
    },
    Accepted,
    Rejected(E),
    ConflictingEventId,
    CompactedEventId,
    StaleCursor {
        actual: JournalCursor,
    },
}

#[derive(Debug)]
pub enum EventRunError<JournalError, HandlerError> {
    Journal(JournalError),
    HandlerBeforeAppend(HandlerError),
    Codec(EventCodecError),
    RecordedWithoutOutcome,
    InvalidCommitReceipt {
        requested: EventId,
        recorded: EventId,
    },
    UncertainAppend {
        event: EventId,
        source: JournalError,
    },
    InputConsumed,
}

pub struct EventLog<P, A, E> {
    definition: EventDefinition<P, A, E>,
    commit: EventCommitPolicy,
}

fn same_event_intent(actual: &EventEnvelope, expected: &EventEnvelope) -> bool {
    actual.journal == expected.journal
        && actual.aggregate == expected.aggregate
        && actual.id == expected.id
        && actual.group == expected.group
        && actual.tag == expected.tag
        && actual.version == expected.version
        && actual.payload == expected.payload
        && actual.occurred_at == expected.occurred_at
}

fn recorded_outcome_error<JournalError, HandlerError>(
    error: EventCodecError,
) -> EventRunError<JournalError, HandlerError> {
    match error {
        EventCodecError::MissingOutcome => EventRunError::RecordedWithoutOutcome,
        EventCodecError::Binary(_)
        | EventCodecError::GroupMismatch
        | EventCodecError::TagMismatch
        | EventCodecError::VersionMismatch => EventRunError::Codec(error),
    }
}

impl<P, A, E> Clone for EventLog<P, A, E> {
    fn clone(&self) -> Self {
        Self {
            definition: self.definition.clone(),
            commit: self.commit,
        }
    }
}

impl<P: Send + Sync + 'static, A: Send + 'static, E: Send + 'static> EventLog<P, A, E> {
    #[must_use]
    pub fn new(definition: EventDefinition<P, A, E>, commit: EventCommitPolicy) -> Self {
        Self { definition, commit }
    }

    pub fn execute<R, HandlerError>(
        &self,
        invocation: EventInvocation<P>,
        handler: impl Fn(Arc<P>) -> Effect<Result<A, E>, HandlerError, R> + Send + Sync + 'static,
    ) -> EventRunEffect<A, E, HandlerError, R>
    where
        R: HasEventJournal + Send + Sync + 'static,
        HandlerError: Send + 'static,
    {
        let EventInvocation {
            journal,
            aggregate,
            event,
            expected,
            occurred_at,
            payload,
        } = invocation;
        let definition = self.definition.clone();
        let commit = self.commit;
        let handler = Arc::new(handler);
        let stream = JournalStream {
            journal: journal.clone(),
            aggregate: aggregate.clone(),
        };
        Effect::service(|env: &R| env.event_journal()).flat_map(move |store| {
            let definition = definition.clone();
            let handler = Arc::clone(&handler);
            let stream = stream.clone();
            let event = event.clone();
            let journal = journal.clone();
            let aggregate = aggregate.clone();
            let payload = Arc::clone(&payload);
            store
                .inspect(stream, event.clone())
                .map_error(EventRunError::Journal)
                .provide_some(|_: &R| ())
                .flat_map(move |existing| {
                    match existing {
                        EventInspection::Present(existing) => {
                            let intent = match definition.envelope(
                                journal.clone(),
                                aggregate.clone(),
                                event.clone(),
                                &payload,
                                None,
                                occurred_at,
                            ) {
                                Ok(intent) => intent,
                                Err(error) => {
                                    return Effect::from_result_once(Err(EventRunError::Codec(
                                        error,
                                    )));
                                }
                            };
                            if !same_event_intent(&existing.event, &intent) {
                                return Effect::sync(|| EventRunOutcome::ConflictingEventId);
                            }
                            let result = definition
                                .decode_outcome(&existing.event)
                                .map_err(recorded_outcome_error)
                                .map(|result| EventRunOutcome::Duplicate {
                                    result,
                                    cursor: JournalCursor::new(existing.sequence.get()),
                                });
                            return Effect::from_result_once(result);
                        }
                        EventInspection::Compacted => {
                            return Effect::sync(|| EventRunOutcome::CompactedEventId);
                        }
                        EventInspection::Missing => {}
                    }
                    let definition = definition.clone();
                    let store = Arc::clone(&store);
                    let event = event.clone();
                    let journal = journal.clone();
                    let aggregate = aggregate.clone();
                    let payload_for_handler = Arc::clone(&payload);
                    let payload_for_envelope = Arc::clone(&payload);
                    let definition_for_result = definition.clone();
                    handler(payload_for_handler)
                        .map_error(EventRunError::HandlerBeforeAppend)
                        .flat_map(move |result| {
                            let definition = definition_for_result.clone();
                            let result = if matches!(commit, EventCommitPolicy::SuccessOnly) {
                                match result {
                                    Err(error) => {
                                        return Effect::sync_once(move || {
                                            EventRunOutcome::Rejected(error)
                                        });
                                    }
                                    Ok(value) => Ok(value),
                                }
                            } else {
                                result
                            };
                            let envelope = match definition.envelope(
                                journal.clone(),
                                aggregate.clone(),
                                event.clone(),
                                &payload_for_envelope,
                                Some(&result),
                                occurred_at,
                            ) {
                                Ok(value) => value,
                                Err(error) => {
                                    return Effect::from_result_once(Err(EventRunError::Codec(
                                        error,
                                    )));
                                }
                            };
                            let result = Arc::new(Mutex::new(Some(result)));
                            let error_event = event.clone();
                            let definition_for_receipt = definition.clone();
                            let expected_envelope = envelope.clone();
                            store
                                .append(envelope, expected)
                                .map_error(move |source| EventRunError::UncertainAppend {
                                    event: error_event.clone(),
                                    source,
                                })
                                .provide_some(|_: &R| ())
                                .flat_map(move |receipt| match receipt {
                                    AppendReceipt::Accepted { event } => {
                                        if event != expected_envelope.id {
                                            Effect::from_result_once(Err(
                                                EventRunError::InvalidCommitReceipt {
                                                    requested: expected_envelope.id.clone(),
                                                    recorded: event,
                                                },
                                            ))
                                        } else {
                                            Effect::sync(|| EventRunOutcome::Accepted)
                                        }
                                    }
                                    AppendReceipt::Decided(AppendDecision::Committed(entry)) => {
                                        if entry.event != expected_envelope
                                            || expected.value().checked_add(1)
                                                != Some(entry.sequence.get())
                                        {
                                            return Effect::from_result_once(Err(
                                                EventRunError::InvalidCommitReceipt {
                                                    requested: expected_envelope.id.clone(),
                                                    recorded: entry.event.id.clone(),
                                                },
                                            ));
                                        }
                                        let result = result
                                            .lock()
                                            .unwrap_or_else(std::sync::PoisonError::into_inner)
                                            .take()
                                            .ok_or(EventRunError::InputConsumed)
                                            .map(|result| EventRunOutcome::Committed {
                                                result,
                                                cursor: JournalCursor::new(entry.sequence.get()),
                                            });
                                        Effect::from_result_once(result)
                                    }
                                    AppendReceipt::Decided(AppendDecision::Duplicate(entry)) => {
                                        if entry.event != expected_envelope {
                                            return Effect::sync(|| {
                                                EventRunOutcome::ConflictingEventId
                                            });
                                        }
                                        let result = definition_for_receipt
                                            .decode_outcome(&entry.event)
                                            .map_err(recorded_outcome_error)
                                            .map(|result| EventRunOutcome::Duplicate {
                                                result,
                                                cursor: JournalCursor::new(entry.sequence.get()),
                                            });
                                        Effect::from_result_once(result)
                                    }
                                    AppendReceipt::Decided(AppendDecision::ConflictingEventId) => {
                                        Effect::sync(|| EventRunOutcome::ConflictingEventId)
                                    }
                                    AppendReceipt::Decided(AppendDecision::CompactedEventId) => {
                                        Effect::sync(|| EventRunOutcome::CompactedEventId)
                                    }
                                    AppendReceipt::Decided(AppendDecision::StaleCursor {
                                        actual,
                                    }) => Effect::sync(move || EventRunOutcome::StaleCursor {
                                        actual,
                                    }),
                                })
                        })
                })
        })
    }
}
