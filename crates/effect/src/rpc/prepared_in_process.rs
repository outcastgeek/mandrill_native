//! An in-process channel using the same prepared wire as external transports.

use super::prepared_client::check_frame_storage;
use super::{
    PreparedRpcChannel, PreparedRpcDecoder, PreparedRpcEncoder, PreparedRpcEnvelope,
    PreparedRpcExchange, PreparedRpcHeader, PreparedRpcOperation, PreparedRpcPayloadCodec,
    PreparedRpcWire, PreparedRpcWireError, RpcEnvelopeKind, RpcEnvelopePolicy, RpcServerError,
};
use crate::SchemaIssue;
use crate::memory::MemoryError;
use std::future::Future;
use std::sync::Arc;

/// A native prepared handler. The borrowed envelope retains decoded request
/// metadata for the duration of the call. The handler's domain error is
/// encoded into the same response wire as a success.
pub trait PreparedRpcHandler<Q, A, E>: Send + Sync {
    fn handle(
        &self,
        request: Q,
        envelope: &PreparedRpcEnvelope,
    ) -> impl Future<Output = Result<A, E>> + Send;
}

/// Local wire, schema and capacity failures visible to the channel caller.
#[derive(Debug)]
pub enum PreparedRpcServerError {
    Memory(MemoryError),
    Server(RpcServerError<!>),
    CorrelationMismatch,
}

/// Both server codec workspaces, held before the client request codec runs.
pub struct PreparedInProcessAdmission {
    decoder: PreparedRpcDecoder,
    encoder: PreparedRpcEncoder,
}

/// A prepared local channel that exchanges framed bytes and runs one native
/// handler. No ordinary transport or ordinary JSON codec is invoked.
pub struct PreparedInProcessRpcChannel<C: PreparedRpcPayloadCodec, H> {
    operation: Arc<PreparedRpcOperation<C>>,
    handler: H,
    policy: RpcEnvelopePolicy,
    wire: PreparedRpcWire,
}

impl<C, H> PreparedInProcessRpcChannel<C, H>
where
    C: PreparedRpcPayloadCodec,
    H: PreparedRpcHandler<C::Request, C::Success, C::DomainError>,
{
    #[must_use]
    pub const fn new(
        operation: Arc<PreparedRpcOperation<C>>,
        handler: H,
        policy: RpcEnvelopePolicy,
        wire: PreparedRpcWire,
    ) -> Self {
        Self {
            operation,
            handler,
            policy,
            wire,
        }
    }
}

impl<C, H> PreparedRpcChannel for PreparedInProcessRpcChannel<C, H>
where
    C: PreparedRpcPayloadCodec,
    H: PreparedRpcHandler<C::Request, C::Success, C::DomainError>,
{
    type Reservation = PreparedInProcessAdmission;
    type Error = PreparedRpcServerError;

    fn try_reserve(
        &self,
        exchange: &PreparedRpcExchange,
    ) -> Result<Self::Reservation, MemoryError> {
        check_frame_storage(exchange.response_capacity(), self.policy)?;
        Ok(PreparedInProcessAdmission {
            decoder: self
                .wire
                .reserve_decode(self.policy, self.operation.limits)?,
            encoder: self
                .wire
                .reserve_encode(self.policy, self.operation.limits)?,
        })
    }

    async fn exchange(
        &self,
        reservation: Self::Reservation,
        mut exchange: PreparedRpcExchange,
    ) -> Result<PreparedRpcExchange, Self::Error> {
        let operation = &self.operation;
        let mut envelope = reservation
            .decoder
            .decode(exchange.request())
            .map_err(|error| server_wire(error, RpcServerError::RequestSchema))?;
        let header = envelope.header();
        if header.operation != operation.spec.id.as_str()
            || header.version != operation.spec.version
        {
            return Err(PreparedRpcServerError::Server(
                RpcServerError::OperationMismatch,
            ));
        }
        if header.kind != RpcEnvelopeKind::Request {
            return Err(PreparedRpcServerError::Server(
                RpcServerError::UnexpectedRequestKind(header.kind),
            ));
        }
        if header.call != exchange.call().as_str() {
            return Err(PreparedRpcServerError::CorrelationMismatch);
        }
        let schema_error =
            |error| PreparedRpcServerError::Server(RpcServerError::RequestSchema(error));
        envelope.check_payload().map_err(schema_error)?;
        let request = operation
            .codec
            .read_request(envelope.payload())
            .map_err(schema_error)?;
        let outcome = self.handler.handle(request, &envelope).await;
        let kind = if outcome.is_ok() {
            RpcEnvelopeKind::Success
        } else {
            RpcEnvelopeKind::DomainError
        };
        reservation
            .encoder
            .encode(
                PreparedRpcHeader {
                    operation: operation.spec.id.as_str(),
                    version: operation.spec.version,
                    call: envelope.header().call,
                    kind,
                    sequence: None,
                },
                envelope.metadata(),
                exchange.response_mut(),
                |writer| match &outcome {
                    Ok(value) => operation.codec.write_success(value, writer),
                    Err(error) => operation.codec.write_error(error, writer),
                },
            )
            .map_err(|error| server_wire(error, RpcServerError::ResponseSchema))?;
        Ok(exchange)
    }
}

fn server_wire(
    error: PreparedRpcWireError,
    schema: impl FnOnce(SchemaIssue) -> RpcServerError<!>,
) -> PreparedRpcServerError {
    let error = match error {
        PreparedRpcWireError::Memory(error) => return PreparedRpcServerError::Memory(error),
        PreparedRpcWireError::Wire(error) => RpcServerError::Wire(error),
        PreparedRpcWireError::Payload(error) => schema(error),
        PreparedRpcWireError::MissingFrame => RpcServerError::MissingRequest,
        PreparedRpcWireError::ExtraFrames => RpcServerError::ExtraRequests,
    };
    PreparedRpcServerError::Server(error)
}
