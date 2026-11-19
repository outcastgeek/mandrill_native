//! Prepared typed submission and observation over the existing store contract.

use super::{
    AcceptancePolicy, ClusterAddress, ClusterClientError, ClusterObserverId, ClusterPoll,
    ClusterRequestId, ClusterStoreError, MemoryClusterMessageStore, MessageAcceptance,
    PreparedAcceptedRequest, PreparedClusterStoreError, ReplyKind,
};
use crate::memory::MemoryError;
use crate::rpc::{
    PreparedRpcHeader, PreparedRpcOperation, PreparedRpcPayloadCodec, PreparedRpcWire,
    PreparedRpcWireError, RpcCallId, RpcEnvelopeKind, RpcEnvelopePolicy, check_frame_storage,
};
use crate::{SchemaIssue, SchemaIssueKind, UtcMillis};
use std::sync::Arc;

/// Typed submitted data with its existing stable call, message and optional
/// idempotency identity. The submitted value is borrowed through encoding.
pub struct PreparedClusterSubmission<'a, Q> {
    pub address: ClusterAddress,
    pub request_id: ClusterRequestId,
    pub request: &'a Q,
    pub call: &'a RpcCallId,
    pub idempotency_key: Option<Arc<[u8]>>,
    pub accepted_at: UtcMillis,
    pub policy: AcceptancePolicy,
}

#[derive(Debug)]
pub enum PreparedClusterClientError {
    Memory(MemoryError),
    Client(ClusterClientError<ClusterStoreError>),
}

/// The prepared unary RPC codec applied to actual Cluster store payloads.
pub struct PreparedClusterClient<C: PreparedRpcPayloadCodec> {
    operation: Arc<PreparedRpcOperation<C>>,
    policy: RpcEnvelopePolicy,
    wire: PreparedRpcWire,
}

impl<C: PreparedRpcPayloadCodec> PreparedClusterClient<C> {
    #[must_use]
    pub const fn new(
        operation: Arc<PreparedRpcOperation<C>>,
        policy: RpcEnvelopePolicy,
        wire: PreparedRpcWire,
    ) -> Self {
        Self {
            operation,
            policy,
            wire,
        }
    }

    /// Encode directly into one pending store payload slot, then transfer it
    /// into the atomic acceptance transition. The wire workspace and actual
    /// byte slot exist before the prepared codec is invoked.
    pub fn submit(
        &self,
        store: &MemoryClusterMessageStore,
        input: PreparedClusterSubmission<'_, C::Request>,
    ) -> Result<MessageAcceptance, PreparedClusterClientError> {
        let mut payload = store
            .reserve_prepared_payload()
            .map_err(PreparedClusterClientError::Memory)?;
        check_frame_storage(payload.capacity(), self.policy)
            .map_err(PreparedClusterClientError::Memory)?;
        let encoder = self
            .wire
            .reserve_encode(self.policy, self.operation.limits())
            .map_err(PreparedClusterClientError::Memory)?;
        let spec = self.operation.spec();
        encoder
            .encode(
                PreparedRpcHeader {
                    operation: spec.id.as_str(),
                    version: spec.version,
                    call: input.call.as_str(),
                    kind: RpcEnvelopeKind::Request,
                    sequence: None,
                },
                std::iter::empty(),
                payload.buffer_mut(),
                |writer| self.operation.codec().write_request(input.request, writer),
            )
            .map_err(|error| wire_error(error, ClusterClientError::RequestSchema))?;
        store
            .accept_prepared_now(
                PreparedAcceptedRequest {
                    address: input.address,
                    id: input.request_id,
                    idempotency_key: input.idempotency_key,
                    accepted_at: input.accepted_at,
                    policy: input.policy,
                },
                payload,
            )
            .map_err(store_error)
    }

    /// Observe and decode a unary result without copying the stored payload.
    /// Acknowledgement remains a separate explicit store operation.
    pub fn poll(
        &self,
        store: &MemoryClusterMessageStore,
        address: ClusterAddress,
        request: ClusterRequestId,
        observer: ClusterObserverId,
        call: &RpcCallId,
    ) -> Result<ClusterPoll<C::Success, C::DomainError>, PreparedClusterClientError> {
        let decoder = self
            .wire
            .reserve_decode(self.policy, self.operation.limits())
            .map_err(PreparedClusterClientError::Memory)?;
        let observed = store
            .observe_prepared_now(address, request, observer, None)
            .map_err(store_error)?;
        if !observed.chunks().is_empty() {
            return Err(PreparedClusterClientError::Client(
                ClusterClientError::UnexpectedChunks,
            ));
        }
        let Some(reply) = observed.reply.as_ref() else {
            return Ok(ClusterPoll::Pending {
                disposition: observed.disposition,
            });
        };
        let mut envelope = decoder
            .decode(reply.payload.as_ref())
            .map_err(|error| wire_error(error, ClusterClientError::ReplySchema))?;
        let header = envelope.header();
        let spec = self.operation.spec();
        if header.operation != spec.id.as_str() || header.version != spec.version {
            return Err(PreparedClusterClientError::Client(
                ClusterClientError::ReplyOperationMismatch,
            ));
        }
        if header.call != call.as_str() {
            return Err(PreparedClusterClientError::Client(
                ClusterClientError::ReplyCorrelationMismatch,
            ));
        }
        let kind = header.kind;
        let schema = |issue: SchemaIssue| {
            if let SchemaIssueKind::Memory(error) = issue.kind() {
                PreparedClusterClientError::Memory(*error)
            } else {
                PreparedClusterClientError::Client(ClusterClientError::ReplySchema(issue))
            }
        };
        envelope.check_payload().map_err(schema)?;
        let body = envelope.payload();
        match (reply.kind, kind) {
            (ReplyKind::Success, RpcEnvelopeKind::Success) => self
                .operation
                .codec()
                .read_success(body)
                .map(|value| ClusterPoll::Completed(Ok(value)))
                .map_err(schema),
            (ReplyKind::DomainError, RpcEnvelopeKind::DomainError) => self
                .operation
                .codec()
                .read_error(body)
                .map(|error| ClusterPoll::Completed(Err(error)))
                .map_err(schema),
            _ => Err(PreparedClusterClientError::Client(
                ClusterClientError::ReplyKindMismatch,
            )),
        }
    }
}

fn wire_error(
    error: PreparedRpcWireError,
    payload: impl FnOnce(SchemaIssue) -> ClusterClientError<ClusterStoreError>,
) -> PreparedClusterClientError {
    let error = match error {
        PreparedRpcWireError::Memory(error) => return PreparedClusterClientError::Memory(error),
        PreparedRpcWireError::Wire(error) => ClusterClientError::Wire(error),
        PreparedRpcWireError::Payload(issue) => payload(issue),
        PreparedRpcWireError::MissingFrame => ClusterClientError::MissingReplyFrame,
        PreparedRpcWireError::ExtraFrames => ClusterClientError::ExtraReplyFrames,
    };
    PreparedClusterClientError::Client(error)
}

fn store_error(error: PreparedClusterStoreError) -> PreparedClusterClientError {
    match error {
        PreparedClusterStoreError::Memory(error) => PreparedClusterClientError::Memory(error),
        PreparedClusterStoreError::Store(error) => {
            PreparedClusterClientError::Client(ClusterClientError::Store(error))
        }
    }
}
