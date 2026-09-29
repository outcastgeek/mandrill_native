//! Transport-neutral unary handler execution.

use super::{RpcCallId, RpcEnvelope, RpcEnvelopeKind, RpcEnvelopePolicy, RpcHandler, RpcWireError};
use crate::encoding::{FrameDecodeFailure, FrameDecoder, FrameParts};
use crate::{Effect, SchemaIssue};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

/// Type alias for decoded request in rpc server.
///
/// # Type parameters
/// - `Q`: Request type.
type DecodedRequest<Q> = (Q, RpcCallId, BTreeMap<Arc<str>, Arc<str>>);

#[derive(Debug)]
/// A framing, decode, handler, or reply-encoding failure on the RPC server.
///
/// # Type parameters
/// - `E`: Domain-error type.
pub enum RpcServerError<E> {
    /// RPC framing or envelope validation failed. Retains the `RpcWireError` detail.
    Wire(RpcWireError),
    /// The operation differs from the required or recorded value.
    OperationMismatch,
    /// The received request kind is invalid in the current protocol state. Retains the `RpcEnvelopeKind` detail.
    UnexpectedRequestKind(RpcEnvelopeKind),
    /// The request failed its declared schema validation. Retains the `SchemaIssue` detail.
    RequestSchema(SchemaIssue),
    /// The response failed its declared schema validation. Retains the `SchemaIssue` detail.
    ResponseSchema(SchemaIssue),
    /// The registered typed handler failed. Retains the `E` detail.
    Handler(E),
    /// The required request is missing.
    MissingRequest,
    /// Unexpected additional requests were present.
    ExtraRequests,
    /// An owned input was already consumed.
    InputConsumed,
}

/// One typed unary handler executed in the caller's R. Framing, schema and
/// domain failures remain distinct; transport accept loops belong to an
/// application-supplied service, not to this server description.
///
/// # Type parameters
/// - `Q`: Request type.
/// - `A`: Application value type.
/// - `E`: Domain-error type.
/// - `R`: Environment type.
pub struct RpcUnaryServer<Q, A, E, R>
where
    Q: Send + 'static,
    A: Send + 'static,
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    handler: RpcHandler<Q, A, E, R>,
    policy: RpcEnvelopePolicy,
}

impl<Q, A, E, R> Clone for RpcUnaryServer<Q, A, E, R>
where
    Q: Send + 'static,
    A: Send + 'static,
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    ///
    /// # Returns
    /// Type: `Self`.
    fn clone(&self) -> Self {
        Self {
            handler: self.handler.clone(),
            policy: self.policy,
        }
    }
}

impl<Q, A, E, R> RpcUnaryServer<Q, A, E, R>
where
    Q: Send + 'static,
    A: Send + 'static,
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    #[must_use]
    /// Construct a `RpcUnaryServer` from its required inputs and explicit policies.
    ///
    /// # Type parameters
    /// - `A`: Success or application-value type.
    /// - `E`: Domain-error type.
    /// - `Q`: Request type.
    /// - `R`: Environment type.
    ///
    /// # Parameters
    /// - `handler` (`RpcHandler<Q, A, E, R>`): The handler invoked for the matching request or event.
    /// - `policy` (`RpcEnvelopePolicy`): The caller-supplied behavior policy.
    ///
    /// # Returns
    /// Type: `Self`.
    pub fn new(handler: RpcHandler<Q, A, E, R>, policy: RpcEnvelopePolicy) -> Self {
        Self { handler, policy }
    }

    /// Dispatch the supplied input to its registered typed handler.
    ///
    /// # Type parameters
    /// - `Q`: Request type.
    /// - `A`: Application value type.
    /// - `E`: Domain-error type.
    /// - `R`: Environment type.
    ///
    /// # Parameters
    /// - `bytes` (`Vec<u8>`): Encoded bytes to validate, decode, write, or transfer.
    ///
    /// # Returns
    /// Type: `Effect<FrameParts, RpcServerError<E>, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    pub fn handle(&self, bytes: Vec<u8>) -> Effect<FrameParts, RpcServerError<E>, R> {
        let handler = self.handler.clone();
        let policy = self.policy;
        let pending = Mutex::new(Some(bytes));
        let server = self.clone();
        Effect::try_sync(
            move || {
                let bytes = pending
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .take()
                    .ok_or(RpcServerError::InputConsumed)?;
                decode_request(&handler, &bytes, policy)
            },
            std::convert::identity,
        )
        .flat_map(move |decoded| server.execute(decoded))
    }

    /// Use an envelope already validated by an encoded protocol dispatcher.
    /// The operation, request kind and typed payload are still checked here.
    ///
    /// # Type parameters
    /// - `Q`: Request type.
    /// - `A`: Application value type.
    /// - `E`: Domain-error type.
    /// - `R`: Environment type.
    ///
    /// # Parameters
    /// - `envelope` (`RpcEnvelope`): Versioned encoded message crossing the transport boundary.
    ///
    /// # Returns
    /// Type: `Effect<FrameParts, RpcServerError<E>, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    pub(crate) fn handle_envelope(
        &self,
        envelope: RpcEnvelope,
    ) -> Effect<FrameParts, RpcServerError<E>, R> {
        let handler = self.handler.clone();
        let server = self.clone();
        let pending = Mutex::new(Some(envelope));
        Effect::try_sync(
            move || {
                let envelope = pending
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .take()
                    .ok_or(RpcServerError::InputConsumed)?;
                decode_envelope(&handler, envelope)
            },
            std::convert::identity,
        )
        .flat_map(move |decoded| server.execute(decoded))
    }

    ///
    /// # Type parameters
    /// - `E`: Domain-error type.
    /// - `Q`: Request type.
    /// - `R`: Environment type.
    ///
    /// # Returns
    /// Type: `Effect<FrameParts, RpcServerError<E>, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    fn execute(
        &self,
        (request, call, metadata): DecodedRequest<Q>,
    ) -> Effect<FrameParts, RpcServerError<E>, R> {
        let handler = self.handler.clone();
        let operation = handler.operation().clone();
        let policy = self.policy;
        let metadata = Mutex::new(Some(metadata));
        handler
            .call(request)
            .either()
            .map_error(RpcServerError::Handler)
            .flat_map(move |outcome| {
                let encoded = match &outcome {
                    Ok(value) => operation.success_codec().encode_ref(value),
                    Err(error) => operation.error_codec().encode_ref(error),
                }
                .map_err(RpcServerError::ResponseSchema);
                let payload = match encoded {
                    Ok(payload) => payload,
                    Err(error) => return Effect::from_result_once(Err(error)),
                };
                if let Err(error) =
                    crate::schema::internal::parser::check_limits(&payload, &operation.limits())
                {
                    return Effect::from_result_once(Err(RpcServerError::ResponseSchema(error)));
                }
                let metadata = metadata
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .take();
                let Some(metadata) = metadata else {
                    return Effect::from_result_once(Err(RpcServerError::InputConsumed));
                };
                let kind = if outcome.is_ok() {
                    RpcEnvelopeKind::Success
                } else {
                    RpcEnvelopeKind::DomainError
                };
                let envelope = RpcEnvelope {
                    operation: operation.spec().id.clone(),
                    version: operation.spec().version,
                    call: call.clone(),
                    kind,
                    sequence: None,
                    payload,
                    metadata,
                };
                Effect::from_result_once(envelope.encode(policy).map_err(RpcServerError::Wire))
            })
    }
}

///
/// # Type parameters
/// - `Q`: Request payload type. Additional bound: `Send + 'static`.
/// - `A`: Successful application-value type. Additional bound: `Send + 'static`.
/// - `E`: Expected application-error type. Additional bound: `Send + 'static`.
/// - `R`: Environment type supplying required capabilities. Additional bound: `Send + Sync + 'static`.
///
/// # Parameters
/// - `handler` (`&RpcHandler<Q, A, E, R>`): The handler invoked for the matching request or event.
/// - `bytes` (`&[u8]`): Encoded bytes to validate, decode, write, or transfer.
/// - `policy` (`RpcEnvelopePolicy`): The caller-supplied behavior policy.
///
/// # Returns
/// Type: `Result<DecodedRequest<Q>, RpcServerError<E>>`.
fn decode_request<Q, A, E, R>(
    handler: &RpcHandler<Q, A, E, R>,
    bytes: &[u8],
    policy: RpcEnvelopePolicy,
) -> Result<DecodedRequest<Q>, RpcServerError<E>>
where
    Q: Send + 'static,
    A: Send + 'static,
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    let mut decoder = FrameDecoder::new(policy.frame);
    let frames = decoder
        .push(bytes)
        .map_err(|FrameDecodeFailure { error, .. }| {
            RpcServerError::Wire(RpcWireError::Frame(error))
        })?;
    decoder
        .finish()
        .map_err(|error| RpcServerError::Wire(RpcWireError::Frame(error)))?;
    let mut frames = frames.into_iter();
    let Some(first) = frames.next() else {
        return Err(RpcServerError::MissingRequest);
    };
    if frames.next().is_some() {
        return Err(RpcServerError::ExtraRequests);
    }
    let envelope = RpcEnvelope::decode(&first, policy).map_err(RpcServerError::Wire)?;
    decode_envelope(handler, envelope)
}

///
/// # Type parameters
/// - `Q`: Request payload type. Additional bound: `Send + 'static`.
/// - `A`: Successful application-value type. Additional bound: `Send + 'static`.
/// - `E`: Expected application-error type. Additional bound: `Send + 'static`.
/// - `R`: Environment type supplying required capabilities. Additional bound: `Send + Sync + 'static`.
///
/// # Parameters
/// - `handler` (`&RpcHandler<Q, A, E, R>`): The handler invoked for the matching request or event.
/// - `envelope` (`RpcEnvelope`): Versioned encoded message crossing the transport boundary.
///
/// # Returns
/// Type: `Result<DecodedRequest<Q>, RpcServerError<E>>`.
fn decode_envelope<Q, A, E, R>(
    handler: &RpcHandler<Q, A, E, R>,
    envelope: RpcEnvelope,
) -> Result<DecodedRequest<Q>, RpcServerError<E>>
where
    Q: Send + 'static,
    A: Send + 'static,
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    if envelope.operation != handler.operation().spec().id
        || envelope.version != handler.operation().spec().version
    {
        return Err(RpcServerError::OperationMismatch);
    }
    if envelope.kind != RpcEnvelopeKind::Request {
        return Err(RpcServerError::UnexpectedRequestKind(envelope.kind));
    }
    crate::schema::internal::parser::check_limits(&envelope.payload, &handler.operation().limits())
        .map_err(RpcServerError::RequestSchema)?;
    let request = handler
        .operation()
        .request_codec()
        .decode_ref(&envelope.payload)
        .map_err(RpcServerError::RequestSchema)?;
    Ok((request, envelope.call, envelope.metadata))
}
