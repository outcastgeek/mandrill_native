//! Event handling and acknowledged journal publication.

use super::{
    AggregateId, AppendDecision, AppendReceipt, EventCodecError, EventDefinition, EventEnvelope,
    EventId, EventInspection, EventJournal, EventJournalId, HasEventJournal, JournalCursor,
    JournalStream,
};
use crate::{Effect, UtcMillis};
use crate::{effect, effect_map_error, effect_provide_some, effect_sync, effect_try_sync, service};
use std::sync::{Arc, Mutex};

/// Type alias for journal error in eventlog event log.
///
/// # Type parameters
/// - `R`: Environment type.
type JournalError<R> = <<R as HasEventJournal>::Journal as EventJournal>::Error;
/// Type alias for event run effect in eventlog event log.
///
/// # Type parameters
/// - `A`: Application value type.
/// - `E`: Domain-error type.
/// - `HandlerError`: Error produced by the registered handler.
/// - `R`: Environment type.
pub type EventRunEffect<A, E, HandlerError, R> =
    Effect<EventRunOutcome<A, E>, EventRunError<JournalError<R>, HandlerError>, R>;

/// A typed event request before encoding and append.
///
/// # Type parameters
/// - `P`: Payload type.
pub struct EventInvocation<P> {
    /// Journal that owns the authoritative records. Type: `EventJournalId`.
    pub journal: EventJournalId,
    /// Aggregate identity or accumulated metric data associated with this record. Type: `AggregateId`.
    pub aggregate: AggregateId,
    /// Event associated with this record. Type: `EventId`.
    pub event: EventId,
    /// The value required by the requested condition. Type: `JournalCursor`.
    pub expected: JournalCursor,
    /// The time at which the occurred occurred. Type: `UtcMillis`.
    pub occurred_at: UtcMillis,
    /// The payload carried by this request or record. Type: `Arc<P>`.
    pub payload: Arc<P>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// The rule for committing an event and interpreting duplicate identity.
pub enum EventCommitPolicy {
    /// Only successful outcomes are retained by this policy.
    SuccessOnly,
    /// Both a successful result and a domain failure were recorded.
    SuccessAndDomainFailure,
}

/// The committed event identity and typed result of an event invocation.
///
/// # Type parameters
/// - `A`: Application value type.
/// - `E`: Domain-error type.
pub enum EventRunOutcome<A, E> {
    /// The state change was committed and is now authoritative.
    Committed {
        /// The result produced by this operation. Type: `Result<A, E>`.
        result: Result<A, E>,
        /// The position from which reading or replay resumes. Type: `JournalCursor`.
        cursor: JournalCursor,
    },
    /// The  already exists or occurred twice.
    Duplicate {
        /// The result produced by this operation. Type: `Result<A, E>`.
        result: Result<A, E>,
        /// The position from which reading or replay resumes. Type: `JournalCursor`.
        cursor: JournalCursor,
    },
    /// The operation was accepted for processing.
    Accepted,
    /// The operation was rejected before acceptance. Carries `E`.
    Rejected(E),
    /// The event identity is already bound to different content.
    ConflictingEventId,
    /// The event identity refers to compacted history.
    CompactedEventId,
    /// The supplied cursor precedes the current recorded position.
    StaleCursor {
        /// The value observed when the requested condition failed. Type: `JournalCursor`.
        actual: JournalCursor,
    },
}

#[derive(Debug)]
/// A codec, journal, or application failure while invoking an event.
///
/// # Type parameters
/// - `JournalError`: Error returned by the event journal.
/// - `HandlerError`: Error produced by the registered handler.
pub enum EventRunError<JournalError, HandlerError> {
    /// The authoritative event journal rejected or failed the operation. Retains the `JournalError` detail.
    Journal(JournalError),
    /// The handler ran before the event append was confirmed. Retains the `HandlerError` detail.
    HandlerBeforeAppend(HandlerError),
    /// Encoding or decoding the typed value failed. Retains the `EventCodecError` detail.
    Codec(EventCodecError),
    /// A terminal record exists without its required encoded outcome.
    RecordedWithoutOutcome,
    /// The commit receipt failed validation.
    InvalidCommitReceipt {
        /// Identity or quantity requested by the caller. Type: `EventId`.
        requested: EventId,
        /// Event identity already present in the journal. Type: `EventId`.
        recorded: EventId,
    },
    /// The append may have committed; the journal outcome could not be confirmed.
    UncertainAppend {
        /// Event associated with this record. Type: `EventId`.
        event: EventId,
        /// The underlying failure or provider cause. Type: `JournalError`.
        source: JournalError,
    },
    /// An owned input was already consumed.
    InputConsumed,
}

/// A typed façade that encodes event invocations and appends them to a journal.
///
/// # Type parameters
/// - `P`: Payload type.
/// - `A`: Application value type.
/// - `E`: Domain-error type.
pub struct EventLog<P, A, E> {
    definition: EventDefinition<P, A, E>,
    commit: EventCommitPolicy,
}

///
/// # Parameters
/// - `actual` (`&EventEnvelope`): The actual supplied to this operation.
/// - `expected` (`&EventEnvelope`): Value or revision required for the proposed state transition.
///
/// # Returns
/// Type: `bool`.
/// Returns whether the requested condition holds.
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

///
/// # Type parameters
/// - `JournalError`: Error returned by the event journal.
/// - `HandlerError`: Error produced by the registered handler.
///
/// # Parameters
/// - `error` (`EventCodecError`): Expected or provider error preserved in the returned outcome.
///
/// # Returns
/// Type: `EventRunError<JournalError, HandlerError>`.
/// Returns `EventRunError<JournalError, HandlerError>`.
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
    ///
    /// # Returns
    /// Type: `Self`.
    fn clone(&self) -> Self {
        Self {
            definition: self.definition.clone(),
            commit: self.commit,
        }
    }
}

impl<P: Send + Sync + 'static, A: Send + 'static, E: Send + 'static> EventLog<P, A, E> {
    #[must_use]
    /// Construct a `EventLog` from its required inputs and explicit policies.
    ///
    /// # Type parameters
    /// - `A`: Success or application-value type.
    /// - `E`: Domain-error type.
    /// - `P`: Payload type.
    ///
    /// # Parameters
    /// - `definition` (`EventDefinition<P, A, E>`): The typed definition used by this operation.
    /// - `commit` (`EventCommitPolicy`): The commit supplied to this operation.
    ///
    /// # Returns
    /// Type: `Self`.
    pub fn new(definition: EventDefinition<P, A, E>, commit: EventCommitPolicy) -> Self {
        Self { definition, commit }
    }

    /// Execute the selected typed operation using its declared environment capabilities.
    ///
    /// # Type parameters
    /// - `R`: Environment type. Additional bound: `HasEventJournal + Send + Sync + 'static`.
    /// - `HandlerError`: Error produced by the registered handler. Additional bound: `Send + 'static`.
    /// - `A`: Success or application-value type.
    /// - `E`: Domain-error type.
    /// - `P`: Payload type.
    ///
    /// # Parameters
    /// - `invocation` (`EventInvocation<P>`): The invocation supplied to this operation.
    /// - `handler` (`impl Fn(Arc<P>) -> Effect<Result<A, E>, HandlerError, R> + Send + Sync + 'static`): The handler invoked for the matching request or event.
    ///
    /// # Returns
    /// Type: `EventRunEffect<A, E, HandlerError, R>`.
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
        {
            let __dsl_program = service! { env: |env: &R| env.event_journal(), };
            effect! {
                let __dsl_program = __dsl_program.clone(); let aggregate = aggregate.clone(); let definition = definition.clone(); let event = event.clone(); let handler = handler.clone(); let journal = journal.clone(); let payload = payload.clone(); let stream = stream.clone(); let store = __dsl_program.clone()?; {
                let definition = definition.clone();
                let handler = Arc::clone(&handler);
                let stream = stream.clone();
                let event = event.clone();
                let journal = journal.clone();
                let aggregate = aggregate.clone();
                let payload = Arc::clone(&payload);
                {
                    let __dsl_program = effect_provide_some! {
                    program: effect_map_error! {
                    program: store
                    .inspect(stream, event.clone()), map: EventRunError::Journal, }, project: |_: &R| (), }; effect! {
                        let __dsl_program = __dsl_program.clone(); let aggregate = aggregate.clone(); let definition = definition.clone(); let event = event.clone(); let handler = handler.clone(); let journal = journal.clone(); let payload = payload.clone(); let store = store.clone(); let existing = __dsl_program.clone()?; (move || {
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
                                        return effect_try_sync! {
                                            once; body: {
                                            let __dsl_result = Err(EventRunError::Codec(
                                            error,
                                        )); move || __dsl_result }, on_error: ::core::convert::identity, };
                                    }
                                };
                                if !same_event_intent(&existing.event, &intent) {
                                    return effect_sync! { body: || EventRunOutcome::ConflictingEventId, };
                                }
                                let result = definition
                                    .decode_outcome(&existing.event)
                                    .map_err(recorded_outcome_error)
                                    .map(|result| EventRunOutcome::Duplicate {
                                        result,
                                        cursor: JournalCursor::new(existing.sequence.get()),
                                    });
                                return effect_try_sync! { once; body: { let __dsl_result = result; move || __dsl_result }, on_error: ::core::convert::identity, };
                            }
                            EventInspection::Compacted => {
                                return effect_sync! { body: || EventRunOutcome::CompactedEventId, };
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
                        {
                            let __dsl_program = effect_map_error! { program: handler(payload_for_handler), map: EventRunError::HandlerBeforeAppend, }; effect! {
                            let __dsl_program = __dsl_program.clone(); let aggregate = aggregate.clone(); let definition_for_result = definition_for_result.clone(); let event = event.clone(); let journal = journal.clone(); let payload_for_envelope = payload_for_envelope.clone(); let store = store.clone(); let result = __dsl_program.clone()?; (move || {
                                let definition = definition_for_result.clone();
                                let result = if matches!(commit, EventCommitPolicy::SuccessOnly) {
                                    match result {
                                        Err(error) => {
                                            return effect_sync! {
                                                once; body: move || {
                                                EventRunOutcome::Rejected(error)
                                            }, };
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
                                        return effect_try_sync! {
                                            once; body: {
                                            let __dsl_result = Err(EventRunError::Codec(
                                            error,
                                        )); move || __dsl_result }, on_error: ::core::convert::identity, };
                                    }
                                };
                                let result = Arc::new(Mutex::new(Some(result)));
                                let error_event = event.clone();
                                let definition_for_receipt = definition.clone();
                                let expected_envelope = envelope.clone();
                                {
                                    let __dsl_program = effect_provide_some! {
                                    program: effect_map_error! {
                                    program: store
                                    .append(envelope, expected), map: move |source| EventRunError::UncertainAppend {
                                        event: error_event.clone(),
                                        source,
                                    }, }, project: |_: &R| (), };  effect! {
                                        let __dsl_program = __dsl_program.clone(); let definition_for_receipt = definition_for_receipt.clone(); let expected_envelope = expected_envelope.clone(); let result = result.clone(); let receipt = __dsl_program?; (move || match receipt {
                                        AppendReceipt::Accepted { event } => {
                                            if event != expected_envelope.id {
                                                effect_try_sync! {
                                                    once; body: {
                                                    let __dsl_result = Err(
                                                    EventRunError::InvalidCommitReceipt {
                                                        requested: expected_envelope.id.clone(),
                                                        recorded: event,
                                                    },
                                                ); move || __dsl_result }, on_error: ::core::convert::identity, }
                                            } else {
                                                effect_sync! { body: || EventRunOutcome::Accepted, }
                                            }
                                        }
                                        AppendReceipt::Decided(AppendDecision::Committed(entry)) => {
                                            if entry.event != expected_envelope
                                                || expected.value().checked_add(1)
                                                    != Some(entry.sequence.get())
                                            {
                                                return effect_try_sync! {
                                                    once; body: {
                                                    let __dsl_result = Err(
                                                    EventRunError::InvalidCommitReceipt {
                                                        requested: expected_envelope.id.clone(),
                                                        recorded: entry.event.id.clone(),
                                                    },
                                                ); move || __dsl_result }, on_error: ::core::convert::identity, };
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
                                            effect_try_sync! { once; body: { let __dsl_result = result; move || __dsl_result }, on_error: ::core::convert::identity, }
                                        }
                                        AppendReceipt::Decided(AppendDecision::Duplicate(entry)) => {
                                            if entry.event != expected_envelope {
                                                return effect_sync! {
                                                    body: || {
                                                    EventRunOutcome::ConflictingEventId
                                                }, };
                                            }
                                            let result = definition_for_receipt
                                                .decode_outcome(&entry.event)
                                                .map_err(recorded_outcome_error)
                                                .map(|result| EventRunOutcome::Duplicate {
                                                    result,
                                                    cursor: JournalCursor::new(entry.sequence.get()),
                                                });
                                            effect_try_sync! { once; body: { let __dsl_result = result; move || __dsl_result }, on_error: ::core::convert::identity, }
                                        }
                                        AppendReceipt::Decided(AppendDecision::ConflictingEventId) => {
                                            effect_sync! { body: || EventRunOutcome::ConflictingEventId, }
                                        }
                                        AppendReceipt::Decided(AppendDecision::CompactedEventId) => {
                                            effect_sync! { body: || EventRunOutcome::CompactedEventId, }
                                        }
                                        AppendReceipt::Decided(AppendDecision::StaleCursor {
                                            actual,
                                        }) => effect_sync! {
                                            body: move || EventRunOutcome::StaleCursor {
                                            actual,
                                        }, },
                                    })()? } }
                            })()? } }
                    })()? } }
            }? }
        }
    }
}
