use super::{
    RpcCallId, RpcEnvelope, RpcEnvelopeKind, RpcEnvelopePolicy, RpcStreamOperation,
    RpcStreamSession, RpcWireError,
};
use crate::encoding::{FrameDecodeFailure, FrameDecoder};
use crate::{Effect, JsonValue, Release, SchemaIssue, Scope};
use std::collections::BTreeMap;
use std::num::NonZeroU64;
use std::sync::Arc;
use std::sync::Mutex;

type StreamPull<C, T, E, R> = dyn Fn() -> Effect<RpcProduced<C, T>, E, R> + Send + Sync;
type StreamInvoke<Q, C, T, E, R> =
    dyn Fn(Q, RpcStreamContext) -> Effect<RpcStreamProducer<C, T, E, R>, E, R> + Send + Sync;

pub enum RpcProduced<C, T> {
    Chunk(C),
    Terminal(T),
}

/// Pull is application-defined and may use any services in R. The server
/// invokes it again only after the preceding chunk is acknowledged.
pub struct RpcStreamProducer<C, T, E, R>
where
    C: Send + 'static,
    T: Send + 'static,
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    pull: Arc<StreamPull<C, T, E, R>>,
}

impl<C, T, E, R> Clone for RpcStreamProducer<C, T, E, R>
where
    C: Send + 'static,
    T: Send + 'static,
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    fn clone(&self) -> Self {
        Self {
            pull: Arc::clone(&self.pull),
        }
    }
}

impl<C, T, E, R> RpcStreamProducer<C, T, E, R>
where
    C: Send + 'static,
    T: Send + 'static,
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    #[must_use]
    pub fn new(pull: impl Fn() -> Effect<RpcProduced<C, T>, E, R> + Send + Sync + 'static) -> Self {
        Self {
            pull: Arc::new(pull),
        }
    }

    pub fn pull(&self) -> Effect<RpcProduced<C, T>, E, R> {
        (self.pull)()
    }
}

#[derive(Clone)]
pub struct RpcStreamContext {
    pub call: RpcCallId,
    pub metadata: BTreeMap<Arc<str>, Arc<str>>,
}

pub struct RpcStreamHandler<Q, C, T, E, R>
where
    Q: Send + 'static,
    C: Send + 'static,
    T: Send + 'static,
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    operation: RpcStreamOperation<Q, C, T, E>,
    invoke: Arc<StreamInvoke<Q, C, T, E, R>>,
}

impl<Q, C, T, E, R> Clone for RpcStreamHandler<Q, C, T, E, R>
where
    Q: Send + 'static,
    C: Send + 'static,
    T: Send + 'static,
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    fn clone(&self) -> Self {
        Self {
            operation: self.operation.clone(),
            invoke: Arc::clone(&self.invoke),
        }
    }
}

impl<Q, C, T, E, R> RpcStreamHandler<Q, C, T, E, R>
where
    Q: Send + 'static,
    C: Send + 'static,
    T: Send + 'static,
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    #[must_use]
    pub fn new(
        operation: RpcStreamOperation<Q, C, T, E>,
        invoke: impl Fn(Q, RpcStreamContext) -> Effect<RpcStreamProducer<C, T, E, R>, E, R>
        + Send
        + Sync
        + 'static,
    ) -> Self {
        Self {
            operation,
            invoke: Arc::new(invoke),
        }
    }

    #[must_use]
    pub fn operation(&self) -> &RpcStreamOperation<Q, C, T, E> {
        &self.operation
    }

    pub fn call(
        &self,
        request: Q,
        context: RpcStreamContext,
    ) -> Effect<RpcStreamProducer<C, T, E, R>, E, R> {
        (self.invoke)(request, context)
    }
}

#[derive(Debug)]
pub enum RpcStreamServerError<E, Backend> {
    Handler(E),
    RequestSchema(SchemaIssue),
    ChunkSchema(SchemaIssue),
    TerminalSchema(SchemaIssue),
    ErrorSchema(SchemaIssue),
    Wire(RpcWireError),
    Transport(Backend),
    Disconnected,
    OperationMismatch,
    UnexpectedRequestKind(RpcEnvelopeKind),
    UnexpectedAcknowledgementKind(RpcEnvelopeKind),
    CorrelationMismatch,
    MissingSequence,
    InvalidControlPayload,
    SequenceMismatch {
        expected: NonZeroU64,
        actual: NonZeroU64,
    },
    SequenceExhausted,
    Cancelled,
    InputConsumed,
}

pub struct RpcStreamServer<Q, C, T, E, R>
where
    Q: Send + 'static,
    C: Send + 'static,
    T: Send + 'static,
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    handler: RpcStreamHandler<Q, C, T, E, R>,
    policy: RpcEnvelopePolicy,
}

impl<Q, C, T, E, R> Clone for RpcStreamServer<Q, C, T, E, R>
where
    Q: Send + 'static,
    C: Send + 'static,
    T: Send + 'static,
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    fn clone(&self) -> Self {
        Self {
            handler: self.handler.clone(),
            policy: self.policy,
        }
    }
}

impl<Q, C, T, E, R> RpcStreamServer<Q, C, T, E, R>
where
    Q: Send + 'static,
    C: Send + 'static,
    T: Send + 'static,
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    #[must_use]
    pub fn new(handler: RpcStreamHandler<Q, C, T, E, R>, policy: RpcEnvelopePolicy) -> Self {
        Self { handler, policy }
    }

    /// Own one accepted session until terminal completion, cancellation or
    /// failure. The scope awaits the provider's close effect on every exit.
    pub fn serve<Session>(
        &self,
        accepted: Arc<Session>,
    ) -> Effect<(), RpcStreamServerError<E, Session::Error>, R>
    where
        Session: RpcStreamSession,
    {
        self.serve_with_first(accepted, None)
    }

    /// The protocol dispatcher has already read this frame to select the
    /// operation. It must be decoded again against the selected typed codec.
    pub fn serve_prefetched<Session>(
        &self,
        accepted: Arc<Session>,
        first: Vec<u8>,
    ) -> Effect<(), RpcStreamServerError<E, Session::Error>, R>
    where
        Session: RpcStreamSession,
    {
        self.serve_with_first(accepted, Some(first))
    }

    fn serve_with_first<Session>(
        &self,
        accepted: Arc<Session>,
        first: Option<Vec<u8>>,
    ) -> Effect<(), RpcStreamServerError<E, Session::Error>, R>
    where
        Session: RpcStreamSession,
    {
        let server = self.clone();
        let first = first.map(|bytes| Arc::new(Mutex::new(Some(bytes))));
        Scope::scoped(move |scope| {
            let accepted = Arc::clone(&accepted);
            let server = server.clone();
            let first = first.clone();
            scope
                .acquire(
                    Effect::sync({
                        let accepted = Arc::clone(&accepted);
                        move || Arc::clone(&accepted)
                    }),
                    |session: Arc<Session>| {
                        Release::from_effect(
                            session
                                .close()
                                .map_error(RpcStreamServerError::Transport)
                                .provide_some(|_: &R| ()),
                        )
                    },
                )
                .flat_map(move |resource| {
                    let server = server.clone();
                    let first = first.clone();
                    resource.access(Arc::clone).flat_map(move |session| {
                        let server = server.clone();
                        let receive = match &first {
                            Some(first) => match first
                                .lock()
                                .unwrap_or_else(std::sync::PoisonError::into_inner)
                                .take()
                            {
                                Some(bytes) => Effect::sync_once(move || Some(bytes)),
                                None => Effect::from_result_once(Err(
                                    RpcStreamServerError::InputConsumed,
                                )),
                            },
                            None => session
                                .receive_frame()
                                .map_error(RpcStreamServerError::Transport)
                                .provide_some(|_: &R| ()),
                        };
                        receive.flat_map(move |bytes| {
                            let Some(bytes) = bytes else {
                                return Effect::from_result_once(Err(
                                    RpcStreamServerError::Disconnected,
                                ));
                            };
                            let request =
                                decode_request(server.handler.operation(), &bytes, server.policy);
                            match request {
                                Ok((request, context)) => {
                                    let call = context.call.clone();
                                    let operation = server.handler.operation().clone();
                                    let session = Arc::clone(&session);
                                    let policy = server.policy;
                                    server
                                        .handler
                                        .call(request, context)
                                        .either()
                                        .map_error(RpcStreamServerError::Handler)
                                        .flat_map(move |result| match result {
                                            Ok(producer) => serve_pulls(
                                                Arc::clone(&session),
                                                operation.clone(),
                                                producer,
                                                call.clone(),
                                                Some(NonZeroU64::MIN),
                                                policy,
                                            ),
                                            Err(error) => send_error(
                                                Arc::clone(&session),
                                                operation.clone(),
                                                call.clone(),
                                                error,
                                                policy,
                                            ),
                                        })
                                }
                                Err(error) => Effect::from_result_once(Err(error)),
                            }
                        })
                    })
                })
        })
    }
}

fn decode_frame<E, B>(
    bytes: &[u8],
    policy: RpcEnvelopePolicy,
) -> Result<RpcEnvelope, RpcStreamServerError<E, B>> {
    let mut decoder = FrameDecoder::new(policy.frame);
    let frames = decoder
        .push(bytes)
        .map_err(|FrameDecodeFailure { error, .. }| {
            RpcStreamServerError::Wire(RpcWireError::Frame(error))
        })?;
    decoder
        .finish()
        .map_err(|error| RpcStreamServerError::Wire(RpcWireError::Frame(error)))?;
    let mut frames = frames.into_iter();
    let Some(first) = frames.next() else {
        return Err(RpcStreamServerError::Wire(RpcWireError::InvalidEnvelope));
    };
    if frames.next().is_some() {
        return Err(RpcStreamServerError::Wire(RpcWireError::InvalidEnvelope));
    }
    RpcEnvelope::decode(&first, policy).map_err(RpcStreamServerError::Wire)
}

fn decode_request<Q, C, T, E, B>(
    operation: &RpcStreamOperation<Q, C, T, E>,
    bytes: &[u8],
    policy: RpcEnvelopePolicy,
) -> Result<(Q, RpcStreamContext), RpcStreamServerError<E, B>>
where
    Q: Send + 'static,
    C: Send + 'static,
    T: Send + 'static,
    E: Send + 'static,
{
    let envelope = decode_frame(bytes, policy)?;
    if envelope.operation != operation.spec().id || envelope.version != operation.spec().version {
        return Err(RpcStreamServerError::OperationMismatch);
    }
    if envelope.kind != RpcEnvelopeKind::Request {
        return Err(RpcStreamServerError::UnexpectedRequestKind(envelope.kind));
    }
    crate::schema::internal::parser::check_limits(&envelope.payload, &operation.limits())
        .map_err(RpcStreamServerError::RequestSchema)?;
    let request = operation
        .request_codec()
        .decode_ref(&envelope.payload)
        .map_err(RpcStreamServerError::RequestSchema)?;
    Ok((
        request,
        RpcStreamContext {
            call: envelope.call,
            metadata: envelope.metadata,
        },
    ))
}

fn send_error<Q, C, T, E, R, Session>(
    session: Arc<Session>,
    operation: RpcStreamOperation<Q, C, T, E>,
    call: RpcCallId,
    error: E,
    policy: RpcEnvelopePolicy,
) -> Effect<(), RpcStreamServerError<E, Session::Error>, R>
where
    Q: Send + 'static,
    C: Send + 'static,
    T: Send + 'static,
    E: Send + 'static,
    R: Send + Sync + 'static,
    Session: RpcStreamSession,
{
    let payload = match operation.error_codec().encode_ref(&error) {
        Ok(payload) => payload,
        Err(issue) => {
            return Effect::from_result_once(Err(RpcStreamServerError::ErrorSchema(issue)));
        }
    };
    if let Err(issue) = crate::schema::internal::parser::check_limits(&payload, &operation.limits())
    {
        return Effect::from_result_once(Err(RpcStreamServerError::ErrorSchema(issue)));
    }
    send_envelope(
        session,
        RpcEnvelope {
            operation: operation.spec().id.clone(),
            version: operation.spec().version,
            call,
            kind: RpcEnvelopeKind::DomainError,
            sequence: None,
            payload,
            metadata: BTreeMap::new(),
        },
        policy,
    )
}

fn send_envelope<E, R, Session>(
    session: Arc<Session>,
    envelope: RpcEnvelope,
    policy: RpcEnvelopePolicy,
) -> Effect<(), RpcStreamServerError<E, Session::Error>, R>
where
    E: Send + 'static,
    R: Send + Sync + 'static,
    Session: RpcStreamSession,
{
    let frame = match envelope.encode(policy) {
        Ok(frame) => frame,
        Err(error) => return Effect::from_result_once(Err(RpcStreamServerError::Wire(error))),
    };
    session
        .send(frame)
        .map_error(RpcStreamServerError::Transport)
        .provide_some(|_: &R| ())
}

fn serve_pulls<Q, C, T, E, R, Session>(
    session: Arc<Session>,
    operation: RpcStreamOperation<Q, C, T, E>,
    producer: RpcStreamProducer<C, T, E, R>,
    call: RpcCallId,
    sequence: Option<NonZeroU64>,
    policy: RpcEnvelopePolicy,
) -> Effect<(), RpcStreamServerError<E, Session::Error>, R>
where
    Q: Send + 'static,
    C: Send + 'static,
    T: Send + 'static,
    E: Send + 'static,
    R: Send + Sync + 'static,
    Session: RpcStreamSession,
{
    Effect::defer(move || {
        let session = Arc::clone(&session);
        let operation = operation.clone();
        let producer = producer.clone();
        let call = call.clone();
        producer
            .pull()
            .either()
            .map_error(RpcStreamServerError::Handler)
            .flat_map(move |result| match result {
                Err(error) => send_error(
                    Arc::clone(&session),
                    operation.clone(),
                    call.clone(),
                    error,
                    policy,
                ),
                Ok(RpcProduced::Terminal(value)) => {
                    let payload = match operation.terminal_codec().encode_ref(&value) {
                        Ok(payload) => payload,
                        Err(issue) => {
                            return Effect::from_result_once(Err(
                                RpcStreamServerError::TerminalSchema(issue),
                            ));
                        }
                    };
                    if let Err(issue) =
                        crate::schema::internal::parser::check_limits(&payload, &operation.limits())
                    {
                        return Effect::from_result_once(Err(
                            RpcStreamServerError::TerminalSchema(issue),
                        ));
                    }
                    send_envelope(
                        Arc::clone(&session),
                        RpcEnvelope {
                            operation: operation.spec().id.clone(),
                            version: operation.spec().version,
                            call: call.clone(),
                            kind: RpcEnvelopeKind::StreamEnd,
                            sequence: None,
                            payload,
                            metadata: BTreeMap::new(),
                        },
                        policy,
                    )
                }
                Ok(RpcProduced::Chunk(value)) => {
                    let Some(sequence) = sequence else {
                        return Effect::from_result_once(Err(
                            RpcStreamServerError::SequenceExhausted,
                        ));
                    };
                    let payload = match operation.chunk_codec().encode_ref(&value) {
                        Ok(payload) => payload,
                        Err(issue) => {
                            return Effect::from_result_once(Err(
                                RpcStreamServerError::ChunkSchema(issue),
                            ));
                        }
                    };
                    if let Err(issue) =
                        crate::schema::internal::parser::check_limits(&payload, &operation.limits())
                    {
                        return Effect::from_result_once(Err(RpcStreamServerError::ChunkSchema(
                            issue,
                        )));
                    }
                    let frame = RpcEnvelope {
                        operation: operation.spec().id.clone(),
                        version: operation.spec().version,
                        call: call.clone(),
                        kind: RpcEnvelopeKind::StreamChunk,
                        sequence: Some(sequence),
                        payload,
                        metadata: BTreeMap::new(),
                    };
                    let session_for_ack = Arc::clone(&session);
                    let operation_for_ack = operation.clone();
                    let producer_for_ack = producer.clone();
                    let call_for_ack = call.clone();
                    send_envelope(Arc::clone(&session), frame, policy).flat_map(move |_| {
                        let session = Arc::clone(&session_for_ack);
                        let operation = operation_for_ack.clone();
                        let producer = producer_for_ack.clone();
                        let call = call_for_ack.clone();
                        session
                            .receive_frame()
                            .map_error(RpcStreamServerError::Transport)
                            .provide_some(|_: &R| ())
                            .flat_map(move |bytes| {
                                let Some(bytes) = bytes else {
                                    return Effect::from_result_once(Err(
                                        RpcStreamServerError::Disconnected,
                                    ));
                                };
                                match decode_ack(&operation, &call, sequence, &bytes, policy) {
                                    Ok(()) => {
                                        let next =
                                            sequence.get().checked_add(1).and_then(NonZeroU64::new);
                                        serve_pulls(
                                            Arc::clone(&session),
                                            operation.clone(),
                                            producer.clone(),
                                            call.clone(),
                                            next,
                                            policy,
                                        )
                                    }
                                    Err(error) => Effect::from_result_once(Err(error)),
                                }
                            })
                    })
                }
            })
    })
}

fn decode_ack<Q, C, T, E, B>(
    operation: &RpcStreamOperation<Q, C, T, E>,
    call: &RpcCallId,
    expected: NonZeroU64,
    bytes: &[u8],
    policy: RpcEnvelopePolicy,
) -> Result<(), RpcStreamServerError<E, B>>
where
    Q: Send + 'static,
    C: Send + 'static,
    T: Send + 'static,
    E: Send + 'static,
{
    let envelope = decode_frame(bytes, policy)?;
    if envelope.operation != operation.spec().id || envelope.version != operation.spec().version {
        return Err(RpcStreamServerError::OperationMismatch);
    }
    if envelope.call != *call {
        return Err(RpcStreamServerError::CorrelationMismatch);
    }
    match envelope.kind {
        RpcEnvelopeKind::Cancel if envelope.payload == JsonValue::Null => {
            Err(RpcStreamServerError::Cancelled)
        }
        RpcEnvelopeKind::Cancel => Err(RpcStreamServerError::InvalidControlPayload),
        RpcEnvelopeKind::Acknowledge => {
            if envelope.payload != JsonValue::Null {
                return Err(RpcStreamServerError::InvalidControlPayload);
            }
            let actual = envelope
                .sequence
                .ok_or(RpcStreamServerError::MissingSequence)?;
            if actual != expected {
                return Err(RpcStreamServerError::SequenceMismatch { expected, actual });
            }
            Ok(())
        }
        kind @ (RpcEnvelopeKind::Request
        | RpcEnvelopeKind::Success
        | RpcEnvelopeKind::DomainError
        | RpcEnvelopeKind::StreamChunk
        | RpcEnvelopeKind::StreamEnd) => {
            Err(RpcStreamServerError::UnexpectedAcknowledgementKind(kind))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::encoding::{FrameParts, FramePolicy};
    use crate::rpc::{OperationId, RpcVersion};
    use crate::{Exit, ParseLimits, json_i64};
    use futures_executor::block_on;
    use std::num::{NonZeroU32, NonZeroUsize};
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct BoundarySession {
        incoming: Arc<Mutex<Option<Vec<u8>>>>,
        sent: Arc<Mutex<Vec<Vec<u8>>>>,
    }

    impl RpcStreamSession for BoundarySession {
        type Error = ();

        fn send(&self, frame: FrameParts) -> Effect<(), Self::Error> {
            let sent = Arc::clone(&self.sent);
            Effect::try_sync_once(
                move || {
                    sent.lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .push(frame.into_bytes().map_err(|_| ())?);
                    Ok(())
                },
                std::convert::identity,
            )
        }

        fn receive_frame(&self) -> Effect<Option<Vec<u8>>, Self::Error> {
            let incoming = Arc::clone(&self.incoming);
            Effect::sync(move || {
                incoming
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .take()
            })
        }

        fn close(&self) -> Effect<(), Self::Error> {
            Effect::sync(|| ())
        }
    }

    #[test]
    fn final_sequence_can_be_acknowledged_before_terminal_but_not_another_chunk() {
        let limit = NonZeroUsize::MAX;
        let policy = RpcEnvelopePolicy {
            frame: FramePolicy {
                maximum_payload: limit,
            },
            json: ParseLimits {
                bytes: limit,
                depth: limit,
                nodes: limit,
                collection: limit,
                issues: limit,
            },
        };
        let operation_id = OperationId::new("final-sequence").expect("operation identity");
        let version = RpcVersion::new(NonZeroU32::MIN);
        let call = RpcCallId::new("final-sequence-call").expect("call identity");
        let operation = RpcStreamOperation::new(
            operation_id.clone(),
            version,
            json_i64(),
            json_i64(),
            json_i64(),
            json_i64(),
            policy.json,
        );
        let ack = RpcEnvelope {
            operation: operation_id.clone(),
            version,
            call: call.clone(),
            kind: RpcEnvelopeKind::Acknowledge,
            sequence: Some(NonZeroU64::MAX),
            payload: JsonValue::Null,
            metadata: BTreeMap::new(),
        }
        .encode(policy)
        .expect("acknowledgement envelope")
        .into_bytes()
        .expect("acknowledgement frame");
        let session = || {
            Arc::new(BoundarySession {
                incoming: Arc::new(Mutex::new(Some(ack.clone()))),
                sent: Arc::new(Mutex::new(Vec::new())),
            })
        };

        let pulls = Arc::new(AtomicUsize::new(0));
        let producer = RpcStreamProducer::new({
            let pulls = Arc::clone(&pulls);
            move || {
                let pulls = Arc::clone(&pulls);
                Effect::<RpcProduced<i64, i64>, i64>::sync(move || {
                    if pulls.fetch_add(1, Ordering::SeqCst) == 0 {
                        RpcProduced::Chunk(42)
                    } else {
                        RpcProduced::Terminal(43)
                    }
                })
            }
        });
        let terminal_session = session();
        let completed = block_on(
            serve_pulls(
                Arc::clone(&terminal_session),
                operation.clone(),
                producer,
                call.clone(),
                Some(NonZeroU64::MAX),
                policy,
            )
            .run(Arc::new(())),
        );
        assert!(matches!(completed, Exit::Success(())));
        assert_eq!(pulls.load(Ordering::SeqCst), 2);
        let sent = terminal_session
            .sent
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        assert_eq!(sent.len(), 2);
        let first = decode_frame::<i64, ()>(&sent[0], policy).expect("final chunk");
        let last = decode_frame::<i64, ()>(&sent[1], policy).expect("terminal result");
        assert_eq!(first.kind, RpcEnvelopeKind::StreamChunk);
        assert_eq!(first.sequence, Some(NonZeroU64::MAX));
        assert_eq!(last.kind, RpcEnvelopeKind::StreamEnd);
        assert_eq!(last.sequence, None);
        drop(sent);

        let extra_chunk = RpcStreamProducer::new(|| {
            Effect::<RpcProduced<i64, i64>, i64>::sync(|| RpcProduced::Chunk(42))
        });
        let exhausted_session = session();
        let exhausted = block_on(
            serve_pulls(
                Arc::clone(&exhausted_session),
                operation,
                extra_chunk,
                call,
                Some(NonZeroU64::MAX),
                policy,
            )
            .run(Arc::new(())),
        );
        assert!(matches!(
            exhausted,
            Exit::Fail(RpcStreamServerError::SequenceExhausted)
        ));
        assert_eq!(
            exhausted_session
                .sent
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .len(),
            1
        );
    }
}
