//! Typed association of event metadata, aggregate selection and Effect execution.

use super::{
    AggregateId, EventCommitPolicy, EventDefinition, EventId, EventInvocation, EventJournalId,
    EventLog, HasEventJournal, JournalCursor,
};
use crate::effect;
use crate::{Effect, UtcMillis};
use std::sync::Arc;

/// Typed definition view used by grouping and projection registration without hiding handlers.
pub trait EventDefinitionView {
    /// Event payload accepted by the definition's pure codec.
    type Payload: Send + 'static;
    /// Successful domain outcome decoded by the same definition.
    type Success: Send + 'static;
    /// Expected domain failure, separate from handler/infrastructure failure.
    type Error: Send + 'static;
    /// Borrow the authoritative codec/identity definition without copying a payload.
    fn definition(&self) -> &EventDefinition<Self::Payload, Self::Success, Self::Error>;
}
impl<P: Send + 'static, A: Send + 'static, E: Send + 'static> EventDefinitionView
    for EventDefinition<P, A, E>
{
    /// Original typed payload.
    type Payload = P;
    /// Original typed success.
    type Success = A;
    /// Original domain error.
    type Error = E;
    /// The low-level descriptor is its own definition view.
    fn definition(&self) -> &EventDefinition<P, A, E> {
        self
    }
}
/// Single aggregate-selection authority retained by an aggregate-bound event definition.
type Aggregate<P> = dyn Fn(&P) -> AggregateId + Send + Sync;
/// An event descriptor paired with its typed aggregate selector. It remains inert and retains
/// the underlying definition for codec/grouping operations and handled-event composition.
pub struct AggregateEvent<P, A, E> {
    /// Authoritative codecs, identity and binary format choices.
    definition: EventDefinition<P, A, E>,
    /// Aggregate derived from the payload for each explicit invocation.
    aggregate: Arc<Aggregate<P>>,
}
impl<P: Send + 'static, A: Send + 'static, E: Send + 'static> AggregateEvent<P, A, E> {
    /// Bind an application-provided selector without generating aggregate identities.
    pub fn new(
        definition: EventDefinition<P, A, E>,
        aggregate: impl Fn(&P) -> AggregateId + Send + Sync + 'static,
    ) -> Self {
        Self {
            definition,
            aggregate: Arc::new(aggregate),
        }
    }
    /// Borrow the underlying typed identity and codec definition.
    #[must_use]
    pub fn definition(&self) -> &EventDefinition<P, A, E> {
        &self.definition
    }
}
impl<P: Send + 'static, A: Send + 'static, E: Send + 'static> EventDefinitionView
    for AggregateEvent<P, A, E>
{
    /// Typed event payload.
    type Payload = P;
    /// Typed successful domain outcome.
    type Success = A;
    /// Expected domain failure.
    type Error = E;
    /// Retain the same definition used by aggregate-bound execution.
    fn definition(&self) -> &EventDefinition<P, A, E> {
        &self.definition
    }
}
/// Existing invocation metadata exposed to a contextual handler. Identities, times and versions
/// are supplied by the caller; the descriptor invents none of them.
#[derive(Clone)]
pub struct EventHandlerContext {
    /// Journal owning the authoritative event record.
    pub journal: EventJournalId,
    /// Aggregate selected by the registered payload selector.
    pub aggregate: AggregateId,
    /// Application-provided idempotency identity.
    pub event: EventId,
    /// Explicit expected stream version.
    pub expected: JournalCursor,
    /// Explicit occurrence instant.
    pub occurred_at: UtcMillis,
}
/// Typed handler keeps domain Result separate from Effect/infrastructure failures.
type Handler<P, A, D, H, R> =
    dyn Fn(EventHandlerContext, Arc<P>) -> Effect<Result<A, D>, H, R> + Send + Sync;
/// Executable event description joining the codec definition, one aggregate selector, commit
/// policy and typed Effect handler. It delegates to EventLog rather than installing a journal.
pub struct HandledEvent<
    P: Send + Sync + 'static,
    A: Send + 'static,
    D: Send + 'static,
    H: Send + 'static,
    R: Send + Sync + 'static,
> {
    /// Single definition and aggregate-selection authority.
    event: AggregateEvent<P, A, D>,
    /// Existing explicit commit/idempotency rule.
    commit: EventCommitPolicy,
    /// Application behavior executed only at the EventLog boundary.
    handler: Arc<Handler<P, A, D, H, R>>,
}
impl<
    P: Send + Sync + 'static,
    A: Send + 'static,
    D: Send + 'static,
    H: Send + 'static,
    R: Send + Sync + 'static,
> HandledEvent<P, A, D, H, R>
{
    /// Pair a low-level definition with an explicit aggregate selector and payload handler.
    pub fn new(
        definition: EventDefinition<P, A, D>,
        aggregate: impl Fn(&P) -> AggregateId + Send + Sync + 'static,
        commit: EventCommitPolicy,
        handler: impl Fn(Arc<P>) -> Effect<Result<A, D>, H, R> + Send + Sync + 'static,
    ) -> Self {
        Self::with_context(
            AggregateEvent::new(definition, aggregate),
            commit,
            move |_, payload| handler(payload),
        )
    }
    /// Reuse an aggregate-bound definition, preserving its one selector authority.
    pub fn from_bound(
        event: AggregateEvent<P, A, D>,
        commit: EventCommitPolicy,
        handler: impl Fn(Arc<P>) -> Effect<Result<A, D>, H, R> + Send + Sync + 'static,
    ) -> Self {
        Self::with_context(event, commit, move |_, payload| handler(payload))
    }
    /// Associate a contextual handler with existing invocation metadata and the same bound definition.
    pub fn with_context(
        event: AggregateEvent<P, A, D>,
        commit: EventCommitPolicy,
        handler: impl Fn(EventHandlerContext, Arc<P>) -> Effect<Result<A, D>, H, R>
        + Send
        + Sync
        + 'static,
    ) -> Self {
        Self {
            event,
            commit,
            handler: Arc::new(handler),
        }
    }
    /// Borrow the descriptor for typed grouping/decoding without erasing access to this handler.
    #[must_use]
    pub fn definition(&self) -> &EventDefinition<P, A, D> {
        self.event.definition()
    }
    /// Construct the EventLog execution Effect from explicit metadata and the registered selector.
    /// Expected domain outcomes remain EventRunOutcome; handler/journal errors remain EventRunError.
    pub fn execute(
        &self,
        journal: EventJournalId,
        event: EventId,
        expected: JournalCursor,
        occurred_at: UtcMillis,
        payload: Arc<P>,
    ) -> super::event_log::EventRunEffect<A, D, H, R>
    where
        R: HasEventJournal,
    {
        let selector = Arc::clone(&self.event.aggregate);
        let definition = self.event.definition.clone();
        let commit = self.commit;
        let handler = Arc::clone(&self.handler);
        {
            let __dsl_factory = move || {
                let aggregate = selector(&payload);
                let context = EventHandlerContext {
                    journal: journal.clone(),
                    aggregate: aggregate.clone(),
                    event: event.clone(),
                    expected,
                    occurred_at,
                };
                let handler = Arc::clone(&handler);
                EventLog::new(definition.clone(), commit).execute(
                    EventInvocation {
                        journal: journal.clone(),
                        aggregate,
                        event: event.clone(),
                        expected,
                        occurred_at,
                        payload: Arc::clone(&payload),
                    },
                    move |payload| handler(context.clone(), payload),
                )
            };
            effect! { __dsl_factory()? }
        }
    }
}
impl<
    P: Send + Sync + 'static,
    A: Send + 'static,
    D: Send + 'static,
    H: Send + 'static,
    R: Send + Sync + 'static,
> EventDefinitionView for HandledEvent<P, A, D, H, R>
{
    /// Typed payload retained by the handler and definition.
    type Payload = P;
    /// Expected successful domain outcome.
    type Success = A;
    /// Expected domain failure, independent of handler failures.
    type Error = D;
    /// Use the same definition for grouping, execution and typed projections.
    fn definition(&self) -> &EventDefinition<P, A, D> {
        self.event.definition()
    }
}

impl<D: EventDefinitionView + ?Sized> EventDefinitionView for &D {
    /// Preserve a borrowed view's payload type.
    type Payload = D::Payload;
    /// Preserve its successful domain outcome type.
    type Success = D::Success;
    /// Preserve its expected domain failure type.
    type Error = D::Error;
    /// Borrowed views reuse the same codec/identity authority without payload copies.
    fn definition(&self) -> &EventDefinition<Self::Payload, Self::Success, Self::Error> {
        (*self).definition()
    }
}
