//! Typed unary and streaming RPC protocols.
//!
//! [`RpcOperation`] pairs a stable operation identity and version with separate
//! request, success, and domain-error codecs. Protocols register typed handlers;
//! framing and envelopes turn those calls into bounded encoded messages.
//! Malformed input, unknown operations, domain failures, and backend failures
//! remain distinguishable. Streaming requires acknowledgement before pulling
//! the next item. The transport and any runtime remain capabilities supplied
//! by the caller's environment.

mod client;
mod definition;
mod envelope;
mod in_process;
mod middleware;
mod prepared_client;
mod prepared_in_process;
mod prepared_memory;
mod prepared_wire;
mod protocol;
mod server;
mod stream_protocol;
mod streaming;
mod streaming_server;

pub use client::{HasRpcChannel, RpcChannel, RpcClient, RpcClientError};
pub use definition::{
    OperationId, OperationIdError, RpcHandler, RpcOperation, RpcOperationKind, RpcOperationSpec,
    RpcRegistry, RpcRegistryError, RpcVersion,
};
pub use envelope::{
    RpcCallId, RpcCallIdError, RpcEnvelope, RpcEnvelopeDecoder, RpcEnvelopeKind, RpcEnvelopePolicy,
    RpcWireError, RpcWireFailure,
};
pub use in_process::InProcessRpcChannel;
pub use middleware::{
    RpcClientMiddleware, RpcClientMiddlewareError, RpcServerMiddleware, RpcServerMiddlewareError,
    RpcStreamMiddleware, RpcStreamMiddlewareError,
};
pub use prepared_client::{
    PreparedRpcChannel, PreparedRpcClient, PreparedRpcClientError, PreparedRpcOperation,
    PreparedRpcPayloadCodec,
};
pub(crate) use prepared_client::check_frame_storage;
pub use prepared_in_process::{
    PreparedInProcessAdmission, PreparedInProcessRpcChannel, PreparedRpcHandler,
    PreparedRpcServerError,
};
pub use prepared_memory::{PreparedRpcCapacity, PreparedRpcExchange, PreparedRpcMemory};
pub use prepared_wire::{
    PreparedRpcDecoder, PreparedRpcEncoder, PreparedRpcEnvelope, PreparedRpcHeader,
    PreparedRpcWire, PreparedRpcWireError,
};
pub use protocol::{InProcessRpcProtocolChannel, RpcProtocol, RpcProtocolError};
pub use server::{RpcServerError, RpcUnaryServer};
pub use stream_protocol::{RpcStreamProtocol, RpcStreamProtocolError};
pub use streaming::{
    HasRpcStreamConnector, RpcStreamClient, RpcStreamConnector, RpcStreamError, RpcStreamItem,
    RpcStreamOperation, RpcStreamReader, RpcStreamSession,
};
pub use streaming_server::{
    RpcProduced, RpcStreamContext, RpcStreamHandler, RpcStreamProducer, RpcStreamServer,
    RpcStreamServerError,
};

/// Scoped service acquisition and explicit protocol grouping.
mod service_middleware;
pub use service_middleware::{RpcProtocolGroup, RpcServiceMiddleware, RpcServiceMiddlewareError};
