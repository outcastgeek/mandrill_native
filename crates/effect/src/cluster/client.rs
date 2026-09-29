use super::{
    AcceptancePolicy, AcceptedRequest, ClusterAddress, ClusterMessageStore, ClusterObserverId,
    ClusterRequestId, DurableClusterMessageStore, HasClusterMessageStore, MessageAcceptance,
    MessageDisposition, ReplyKind, SavedReply, StoreMutation,
};
use crate::encoding::{FrameDecodeFailure, FrameDecoder};
use crate::rpc::{
    RpcCallId, RpcEnvelope, RpcEnvelopeKind, RpcEnvelopePolicy, RpcOperation, RpcWireError,
};
use crate::{Effect, SchemaIssue, UtcMillis};
use std::marker::PhantomData;
use std::sync::{Arc, Mutex};

#[derive(Debug)]
pub enum ClusterClientError<Backend> {
    RequestSchema(SchemaIssue),
    ReplySchema(SchemaIssue),
    Wire(RpcWireError),
    /// The provider may have accepted the request before acknowledgement was
    /// lost. Reconcile by stable request ID before deciding to submit again.
    UncertainAcceptance {
        request: ClusterRequestId,
        source: Backend,
    },
    Store(Backend),
    MissingReplyFrame,
    ExtraReplyFrames,
    ReplyOperationMismatch,
    ReplyCorrelationMismatch,
    ReplyKindMismatch,
    UnexpectedChunks,
    InputConsumed,
}

pub enum ClusterPoll<A, E> {
    Pending { disposition: MessageDisposition },
    Completed(Result<A, E>),
}

/// Typed unary Cluster client backed by atomic message acceptance. A submit
/// only records work; a separate owner/runner performs execution. Observer
/// cancellation never changes the saved message's execution state.
pub struct ClusterClient<Q, A, E, Backend>
where
    Q: Send + 'static,
    A: Send + 'static,
    E: Send + 'static,
    Backend: Send + 'static,
{
    operation: RpcOperation<Q, A, E>,
    policy: RpcEnvelopePolicy,
    _backend: PhantomData<fn() -> Backend>,
}

impl<Q, A, E, Backend> ClusterClient<Q, A, E, Backend>
where
    Q: Send + 'static,
    A: Send + 'static,
    E: Send + 'static,
    Backend: Send + 'static,
{
    #[must_use]
    pub fn new(operation: RpcOperation<Q, A, E>, policy: RpcEnvelopePolicy) -> Self {
        Self {
            operation,
            policy,
            _backend: PhantomData,
        }
    }

    /// Admission that requires acknowledged requests to survive the store's
    /// documented restart boundary. The in-memory store cannot satisfy this
    /// bound, while ordinary `submit` remains available for volatile use.
    pub fn submit_durable<R>(
        &self,
        address: ClusterAddress,
        request_id: ClusterRequestId,
        request: Q,
        call: RpcCallId,
        accepted_at: UtcMillis,
        acceptance: AcceptancePolicy,
    ) -> Effect<MessageAcceptance, ClusterClientError<Backend>, R>
    where
        R: HasClusterMessageStore + Send + Sync + 'static,
        R::Store: DurableClusterMessageStore<Error = Backend>,
    {
        self.submit(address, request_id, request, call, accepted_at, acceptance)
    }

    pub fn submit<R>(
        &self,
        address: ClusterAddress,
        request_id: ClusterRequestId,
        request: Q,
        call: RpcCallId,
        accepted_at: UtcMillis,
        acceptance: AcceptancePolicy,
    ) -> Effect<MessageAcceptance, ClusterClientError<Backend>, R>
    where
        R: HasClusterMessageStore + Send + Sync + 'static,
        R::Store: ClusterMessageStore<Error = Backend>,
    {
        let operation = self.operation.clone();
        let policy = self.policy;
        let request = Mutex::new(Some(request));
        Effect::try_sync(
            move || {
                let request = request
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .take()
                    .ok_or(ClusterClientError::InputConsumed)?;
                let idempotency_key = operation
                    .idempotency_key(&request)
                    .map_err(ClusterClientError::RequestSchema)?;
                let payload = operation
                    .request_codec()
                    .encode_ref(&request)
                    .map_err(ClusterClientError::RequestSchema)?;
                crate::schema::internal::parser::check_limits(&payload, &operation.limits())
                    .map_err(ClusterClientError::RequestSchema)?;
                let bytes = RpcEnvelope {
                    operation: operation.spec().id.clone(),
                    version: operation.spec().version,
                    call: call.clone(),
                    kind: RpcEnvelopeKind::Request,
                    sequence: None,
                    payload,
                    metadata: Default::default(),
                }
                .encode(policy)
                .map_err(ClusterClientError::Wire)?
                .into_bytes()
                .map_err(|error| ClusterClientError::Wire(RpcWireError::Frame(error)))?;
                Ok((Arc::<[u8]>::from(bytes), idempotency_key))
            },
            std::convert::identity,
        )
        .flat_map(move |(payload, idempotency_key)| {
            let request = AcceptedRequest {
                address: address.clone(),
                id: request_id.clone(),
                payload,
                idempotency_key,
                accepted_at,
                policy: acceptance,
            };
            let request = Mutex::new(Some(request));
            let error_request = request_id.clone();
            Effect::service(|env: &R| env.cluster_message_store()).flat_map(move |store| {
                let request = request
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .take();
                match request {
                    Some(request) => store
                        .accept(request)
                        .map_error({
                            let error_request = error_request.clone();
                            move |source| ClusterClientError::UncertainAcceptance {
                                request: error_request.clone(),
                                source,
                            }
                        })
                        .provide_some(|_: &R| ()),
                    None => Effect::from_result_once(Err(ClusterClientError::InputConsumed)),
                }
            })
        })
    }

    pub fn poll<R>(
        &self,
        address: ClusterAddress,
        request: ClusterRequestId,
        observer: ClusterObserverId,
        call: RpcCallId,
    ) -> Effect<ClusterPoll<A, E>, ClusterClientError<Backend>, R>
    where
        R: HasClusterMessageStore + Send + Sync + 'static,
        R::Store: ClusterMessageStore<Error = Backend>,
    {
        let operation = self.operation.clone();
        let policy = self.policy;
        Effect::service(|env: &R| env.cluster_message_store()).flat_map(move |store| {
            store
                .observe(address.clone(), request.clone(), observer.clone(), None)
                .map_error(ClusterClientError::Store)
                .provide_some(|_: &R| ())
                .flat_map({
                    let operation = operation.clone();
                    let call = call.clone();
                    move |observed| {
                        let result = if !observed.chunks.is_empty() {
                            Err(ClusterClientError::UnexpectedChunks)
                        } else {
                            match observed.reply {
                                Some(reply) => decode_reply(&operation, &call, &reply, policy)
                                    .map(ClusterPoll::Completed),
                                None => Ok(ClusterPoll::Pending {
                                    disposition: observed.disposition,
                                }),
                            }
                        };
                        Effect::from_result_once(result)
                    }
                })
        })
    }

    pub fn acknowledge_reply<R>(
        &self,
        address: ClusterAddress,
        request: ClusterRequestId,
        observer: ClusterObserverId,
    ) -> Effect<StoreMutation, ClusterClientError<Backend>, R>
    where
        R: HasClusterMessageStore + Send + Sync + 'static,
        R::Store: ClusterMessageStore<Error = Backend>,
    {
        Effect::service(|env: &R| env.cluster_message_store()).flat_map(move |store| {
            store
                .acknowledge_reply(address.clone(), request.clone(), observer.clone())
                .map_error(ClusterClientError::Store)
                .provide_some(|_: &R| ())
        })
    }

    pub fn detach_observer<R>(
        &self,
        address: ClusterAddress,
        request: ClusterRequestId,
        observer: ClusterObserverId,
    ) -> Effect<bool, ClusterClientError<Backend>, R>
    where
        R: HasClusterMessageStore + Send + Sync + 'static,
        R::Store: ClusterMessageStore<Error = Backend>,
    {
        Effect::service(|env: &R| env.cluster_message_store()).flat_map(move |store| {
            store
                .detach_observer(address.clone(), request.clone(), observer.clone())
                .map_error(ClusterClientError::Store)
                .provide_some(|_: &R| ())
        })
    }
}

fn decode_reply<Q, A, E, Backend>(
    operation: &RpcOperation<Q, A, E>,
    call: &RpcCallId,
    reply: &SavedReply,
    policy: RpcEnvelopePolicy,
) -> Result<Result<A, E>, ClusterClientError<Backend>>
where
    Q: Send + 'static,
    A: Send + 'static,
    E: Send + 'static,
    Backend: Send + 'static,
{
    let mut decoder = FrameDecoder::new(policy.frame);
    let frames = decoder
        .push(&reply.payload)
        .map_err(|FrameDecodeFailure { error, .. }| {
            ClusterClientError::Wire(RpcWireError::Frame(error))
        })?;
    decoder
        .finish()
        .map_err(|error| ClusterClientError::Wire(RpcWireError::Frame(error)))?;
    let mut frames = frames.into_iter();
    let Some(first) = frames.next() else {
        return Err(ClusterClientError::MissingReplyFrame);
    };
    if frames.next().is_some() {
        return Err(ClusterClientError::ExtraReplyFrames);
    }
    let envelope = RpcEnvelope::decode(&first, policy).map_err(ClusterClientError::Wire)?;
    if envelope.operation != operation.spec().id || envelope.version != operation.spec().version {
        return Err(ClusterClientError::ReplyOperationMismatch);
    }
    if envelope.call != *call {
        return Err(ClusterClientError::ReplyCorrelationMismatch);
    }
    crate::schema::internal::parser::check_limits(&envelope.payload, &operation.limits())
        .map_err(ClusterClientError::ReplySchema)?;
    match (reply.kind, envelope.kind) {
        (ReplyKind::Success, RpcEnvelopeKind::Success) => operation
            .success_codec()
            .decode_ref(&envelope.payload)
            .map(Ok)
            .map_err(ClusterClientError::ReplySchema),
        (ReplyKind::DomainError, RpcEnvelopeKind::DomainError) => operation
            .error_codec()
            .decode_ref(&envelope.payload)
            .map(Err)
            .map_err(ClusterClientError::ReplySchema),
        _ => Err(ClusterClientError::ReplyKindMismatch),
    }
}
