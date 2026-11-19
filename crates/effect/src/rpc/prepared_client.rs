//! Unary calls which preadmit both transport and codec storage.

use super::{
    PreparedRpcDecoder, PreparedRpcExchange, PreparedRpcHeader, PreparedRpcMemory, PreparedRpcWire,
    PreparedRpcWireError, RpcCallId, RpcClientError, RpcEnvelopeKind, RpcEnvelopePolicy,
    RpcOperationKind, RpcOperationSpec,
};
use crate::encoding::FRAME_HEADER_LENGTH;
use crate::internal::runtime::prepared_graph::FutureContext;
use crate::memory::{Bytes, MemoryError};
use crate::schema::PreparedJsonValue;
use crate::{Exit, JsonWriter, ParseLimits, SchemaIssue, SchemaIssueKind};
use futures_util::future::{Either, select};
use std::future::Future;
use std::pin::pin;
use std::sync::Arc;

/// The three payload roles of one prepared unary operation. Writers and JSON
/// views already own their configured storage. A callback that retains decoded
/// bytes must retain an actual backing view, or use its own admitted storage.
pub trait PreparedRpcPayloadCodec: Send + Sync + 'static {
    type Request: Send + Sync + 'static;
    type Success: Send + 'static;
    type DomainError: Send + 'static;

    fn write_request(&self, value: &Self::Request, to: &mut JsonWriter) -> Result<(), SchemaIssue>;
    fn read_request(&self, from: PreparedJsonValue<'_>) -> Result<Self::Request, SchemaIssue>;
    fn write_success(&self, value: &Self::Success, to: &mut JsonWriter) -> Result<(), SchemaIssue>;
    fn read_success(&self, from: PreparedJsonValue<'_>) -> Result<Self::Success, SchemaIssue>;
    fn write_error(
        &self,
        value: &Self::DomainError,
        to: &mut JsonWriter,
    ) -> Result<(), SchemaIssue>;
    fn read_error(&self, from: PreparedJsonValue<'_>) -> Result<Self::DomainError, SchemaIssue>;
}

/// An operation's stable wire identity and its selected prepared payload codecs.
pub struct PreparedRpcOperation<C: PreparedRpcPayloadCodec> {
    pub(super) spec: RpcOperationSpec,
    pub(super) limits: ParseLimits,
    pub(super) codec: C,
}

impl<C: PreparedRpcPayloadCodec> PreparedRpcOperation<C> {
    /// Construct the selected unary operation at setup.
    pub fn new(spec: RpcOperationSpec, limits: ParseLimits, codec: C) -> Result<Self, MemoryError> {
        if spec.kind != RpcOperationKind::Unary {
            return Err(MemoryError::UnsupportedRunContract);
        }
        Ok(Self {
            spec,
            limits,
            codec,
        })
    }

    #[must_use]
    pub fn spec(&self) -> &RpcOperationSpec {
        &self.spec
    }

    #[must_use]
    pub const fn limits(&self) -> ParseLimits {
        self.limits
    }

    #[must_use]
    pub const fn codec(&self) -> &C {
        &self.codec
    }
}

/// A unary transport which can preadmit its native demand before encoding.
/// The reservation must own any provider capacity needed for one exchange.
/// Hold the actual exchange owner until transport work settles, including if
/// cancellation hands outstanding native work to a separate owner. A success
/// returns the same exchange with its response bytes initialized.
pub trait PreparedRpcChannel: Send + Sync {
    type Reservation: Send;
    type Error: Send + 'static;

    fn try_reserve(&self, exchange: &PreparedRpcExchange)
    -> Result<Self::Reservation, MemoryError>;

    fn exchange(
        &self,
        reservation: Self::Reservation,
        exchange: PreparedRpcExchange,
    ) -> impl Future<Output = Result<PreparedRpcExchange, Self::Error>> + Send;
}

/// Prepared admission refusal is distinct from schema, domain and provider errors.
#[derive(Debug)]
pub enum PreparedRpcClientError<E, B> {
    /// The request did not dispatch.
    Memory(MemoryError),
    /// The response arrived but could not be decoded into the selected storage.
    ResponseMemory {
        call: RpcCallId,
        error: MemoryError,
    },
    Call(RpcClientError<E, B>),
}

type ClientError<C, T> = PreparedRpcClientError<
    <C as PreparedRpcPayloadCodec>::DomainError,
    <T as PreparedRpcChannel>::Error,
>;

/// A unary client with real per-call pending, request and response ownership.
pub struct PreparedRpcClient<C: PreparedRpcPayloadCodec> {
    operation: Arc<PreparedRpcOperation<C>>,
    policy: RpcEnvelopePolicy,
    wire: PreparedRpcWire,
    memory: PreparedRpcMemory,
}

struct EncodedCall<R> {
    exchange: PreparedRpcExchange,
    decoder: PreparedRpcDecoder,
    provider: R,
}

pub(crate) fn check_frame_storage(
    capacity: Bytes,
    policy: RpcEnvelopePolicy,
) -> Result<(), MemoryError> {
    let payload = policy.frame.maximum_payload.min(policy.json.bytes);
    let requested = Bytes::new(payload.get()).checked_add(Bytes::new(FRAME_HEADER_LENGTH))?;
    if requested > capacity {
        return Err(MemoryError::ValueTooLarge {
            requested,
            capacity,
        });
    }
    Ok(())
}

impl<C: PreparedRpcPayloadCodec> PreparedRpcClient<C> {
    #[must_use]
    pub const fn new(
        operation: Arc<PreparedRpcOperation<C>>,
        policy: RpcEnvelopePolicy,
        wire: PreparedRpcWire,
        memory: PreparedRpcMemory,
    ) -> Self {
        Self {
            operation,
            policy,
            wire,
            memory,
        }
    }

    /// Admit pending control, both byte buffers, both wire workspaces, and the
    /// provider reservation before invoking the request codec or dispatching.
    pub async fn call<T: PreparedRpcChannel>(
        &self,
        channel: &T,
        request: &C::Request,
        call: &RpcCallId,
        metadata: &[(&str, &str)],
    ) -> Result<C::Success, ClientError<C, T>> {
        let encoded = self.encode_request(channel, request, call, metadata)?;
        self.dispatch(channel, call, encoded).await
    }

    fn encode_request<T: PreparedRpcChannel>(
        &self,
        channel: &T,
        request: &C::Request,
        call: &RpcCallId,
        metadata: &[(&str, &str)],
    ) -> Result<EncodedCall<T::Reservation>, ClientError<C, T>> {
        let mut exchange = self
            .memory
            .try_admit(call.clone())
            .map_err(PreparedRpcClientError::Memory)?;
        check_frame_storage(exchange.request_capacity(), self.policy)
            .map_err(PreparedRpcClientError::Memory)?;
        check_frame_storage(exchange.response_capacity(), self.policy)
            .map_err(PreparedRpcClientError::Memory)?;
        let operation = &self.operation;
        let encoder = self
            .wire
            .reserve_encode(self.policy, operation.limits)
            .map_err(PreparedRpcClientError::Memory)?;
        let decoder = self
            .wire
            .reserve_decode(self.policy, operation.limits)
            .map_err(PreparedRpcClientError::Memory)?;
        let provider = channel
            .try_reserve(&exchange)
            .map_err(PreparedRpcClientError::Memory)?;
        encoder
            .encode(
                PreparedRpcHeader {
                    operation: operation.spec.id.as_str(),
                    version: operation.spec.version,
                    call: call.as_str(),
                    kind: RpcEnvelopeKind::Request,
                    sequence: None,
                },
                metadata.iter().copied(),
                exchange.request_mut(),
                |writer| operation.codec.write_request(request, writer),
            )
            .map_err(|error| {
                client_wire(
                    error,
                    RpcClientError::RequestSchema,
                    PreparedRpcClientError::Memory,
                )
            })?;
        Ok(EncodedCall {
            exchange,
            decoder,
            provider,
        })
    }

    async fn dispatch<T: PreparedRpcChannel>(
        &self,
        channel: &T,
        call: &RpcCallId,
        encoded: EncodedCall<T::Reservation>,
    ) -> Result<C::Success, ClientError<C, T>> {
        let EncodedCall {
            exchange,
            decoder,
            provider,
        } = encoded;
        let id = exchange.id();
        let operation = &self.operation;
        let exchange = channel
            .exchange(provider, exchange)
            .await
            .map_err(|source| {
                PreparedRpcClientError::Call(RpcClientError::UncertainTransport {
                    call: call.clone(),
                    source,
                })
            })?;
        if !self.memory.matches_exchange(id, &exchange) {
            return Err(PreparedRpcClientError::Call(
                RpcClientError::CorrelationMismatch,
            ));
        }
        let mut response = decoder.decode(exchange.response()).map_err(|error| {
            client_wire(error, RpcClientError::ResponseSchema, |error| {
                response_memory(call, error)
            })
        })?;
        let header = response.header();
        if header.call != call.as_str() {
            return Err(PreparedRpcClientError::Call(
                RpcClientError::CorrelationMismatch,
            ));
        }
        if header.operation != operation.spec.id.as_str()
            || header.version != operation.spec.version
        {
            return Err(PreparedRpcClientError::Call(
                RpcClientError::OperationMismatch,
            ));
        }
        let schema_error = |error: SchemaIssue| {
            if let SchemaIssueKind::Memory(memory) = error.kind() {
                response_memory(call, *memory)
            } else {
                PreparedRpcClientError::Call(RpcClientError::ResponseSchema(error))
            }
        };
        match header.kind {
            RpcEnvelopeKind::Success => {
                response.check_payload().map_err(schema_error)?;
                operation
                    .codec
                    .read_success(response.payload())
                    .map_err(schema_error)
            }
            RpcEnvelopeKind::DomainError => {
                response.check_payload().map_err(schema_error)?;
                let error = operation
                    .codec
                    .read_error(response.payload())
                    .map_err(schema_error)?;
                Err(PreparedRpcClientError::Call(RpcClientError::Domain(error)))
            }
            kind @ (RpcEnvelopeKind::Request
            | RpcEnvelopeKind::StreamChunk
            | RpcEnvelopeKind::StreamEnd
            | RpcEnvelopeKind::Acknowledge
            | RpcEnvelopeKind::Cancel) => Err(PreparedRpcClientError::Call(
                RpcClientError::UnexpectedResponseKind(kind),
            )),
        }
    }

    /// Use the enclosing prepared native operation's existing cancellation
    /// registration. A cancelled exchange remains the provider's responsibility
    /// until its actual transport work has settled.
    pub async fn call_in<T: PreparedRpcChannel, ContextError: Send + 'static>(
        &self,
        context: &mut FutureContext<ContextError>,
        channel: &T,
        request: &C::Request,
        call: &RpcCallId,
        metadata: &[(&str, &str)],
    ) -> Exit<C::Success, ClientError<C, T>> {
        if context.is_requested() {
            return Exit::Interrupted;
        }
        let encoded = match self.encode_request(channel, request, call, metadata) {
            Ok(encoded) => encoded,
            Err(error) => return Exit::Fail(error),
        };
        if context.is_requested() {
            return Exit::Interrupted;
        }
        let outcome = {
            let cancelled = pin!(context.cancelled());
            let exchange = pin!(self.dispatch(channel, call, encoded));
            match select(cancelled, exchange).await {
                Either::Left(_) => return Exit::Interrupted,
                Either::Right((outcome, _)) => outcome,
            }
        };
        match outcome {
            Ok(_) if context.is_requested() => Exit::Interrupted,
            Ok(value) => Exit::Success(value),
            Err(error) => Exit::Fail(error),
        }
    }
}

fn client_wire<E, B>(
    error: PreparedRpcWireError,
    schema: impl FnOnce(SchemaIssue) -> RpcClientError<E, B>,
    memory: impl FnOnce(MemoryError) -> PreparedRpcClientError<E, B>,
) -> PreparedRpcClientError<E, B> {
    let error = match error {
        PreparedRpcWireError::Memory(error) => return memory(error),
        PreparedRpcWireError::Wire(error) => RpcClientError::Wire(error),
        PreparedRpcWireError::Payload(error) => schema(error),
        PreparedRpcWireError::MissingFrame => RpcClientError::MissingResponse,
        PreparedRpcWireError::ExtraFrames => RpcClientError::ExtraResponses,
    };
    PreparedRpcClientError::Call(error)
}

fn response_memory<E, B>(call: &RpcCallId, error: MemoryError) -> PreparedRpcClientError<E, B> {
    PreparedRpcClientError::ResponseMemory {
        call: call.clone(),
        error,
    }
}
