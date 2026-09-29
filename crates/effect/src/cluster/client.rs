//! Typed submission and observation of Cluster requests.

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
/// A typed failure while sending, observing, or decoding an addressed request.
///
/// # Type parameters
/// - `Backend`: Backend-specific error or provider type.
pub enum ClusterClientError<Backend> {
    /// The request failed its declared schema validation. Retains the `SchemaIssue` detail.
    RequestSchema(SchemaIssue),
    /// The reply failed its declared schema validation. Retains the `SchemaIssue` detail.
    ReplySchema(SchemaIssue),
    /// RPC framing or envelope validation failed. Retains the `RpcWireError` detail.
    Wire(RpcWireError),
    /// The provider may have accepted the request before acknowledgement was
    /// lost. Reconcile by stable request ID before deciding to submit again.
    UncertainAcceptance {
        /// The request being handled. Type: `ClusterRequestId`.
        request: ClusterRequestId,
        /// The underlying failure or provider cause. Type: `Backend`.
        source: Backend,
    },
    /// The selected store rejected or failed the operation. Retains the `Backend` detail.
    Store(Backend),
    /// The required reply frame is missing.
    MissingReplyFrame,
    /// Unexpected additional reply frames were present.
    ExtraReplyFrames,
    /// The reply operation differs from the required or recorded value.
    ReplyOperationMismatch,
    /// The reply correlation differs from the required or recorded value.
    ReplyCorrelationMismatch,
    /// The reply kind differs from the required or recorded value.
    ReplyKindMismatch,
    /// The received chunks is invalid in the current protocol state.
    UnexpectedChunks,
    /// An owned input was already consumed.
    InputConsumed,
}

/// The recorded state currently visible to a client polling an addressed request.
///
/// # Type parameters
/// - `A`: Application value type.
/// - `E`: Domain-error type.
pub enum ClusterPoll<A, E> {
    /// The request is accepted but its reply has not been recorded.
    Pending {
        /// Current message-delivery state. Type: `MessageDisposition`.
        disposition: MessageDisposition,
    },
    /// The operation completed and its outcome is available. Carries `Result<A, E>`.
    Completed(Result<A, E>),
}

/// A typed client for submitting addressed messages and observing recorded replies.
/// only records work; a separate owner/runner performs execution. Observer
/// cancellation never changes the saved message's execution state.
///
/// # Type parameters
/// - `Q`: Request type.
/// - `A`: Application value type.
/// - `E`: Domain-error type.
/// - `Backend`: Backend-specific error or provider type.
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
    /// Construct a `ClusterClient` from its required inputs and explicit policies.
    ///
    /// # Type parameters
    /// - `Backend`: Backend-specific error or provider type.
    /// - `A`: Success or application-value type.
    /// - `E`: Domain-error type.
    /// - `Q`: Request type.
    ///
    /// # Parameters
    /// - `operation` (`RpcOperation<Q, A, E>`): Typed operation definition or callback selected for execution.
    /// - `policy` (`RpcEnvelopePolicy`): The caller-supplied behavior policy.
    ///
    /// # Returns
    /// Type: `Self`.
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
    ///
    /// # Type parameters
    /// - `A`: Application value type.
    /// - `E`: Domain-error type.
    /// - `Backend`: Backend-specific error or provider type.
    /// - `R`: Environment type. Additional bound: `HasClusterMessageStore + Send + Sync + 'static`. Additional bound: `:Store: DurableClusterMessageStore<Error = Backend>`.
    /// - `Q`: Request type.
    ///
    /// # Parameters
    /// - `address` (`ClusterAddress`): Stable destination address of the entity receiving this request.
    /// - `request_id` (`ClusterRequestId`): The request id supplied to this operation.
    /// - `request` (`Q`): The typed request to process.
    /// - `call` (`RpcCallId`): Identity and contents of the selected RPC call.
    /// - `accepted_at` (`UtcMillis`): The accepted at supplied to this operation.
    /// - `acceptance` (`AcceptancePolicy`): The acceptance supplied to this operation.
    ///
    /// # Returns
    /// Type: `Effect<MessageAcceptance, ClusterClientError<Backend>, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
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

    /// Transfer accepted work to the selected provider for independent execution.
    ///
    /// # Type parameters
    /// - `A`: Application value type.
    /// - `E`: Domain-error type.
    /// - `Backend`: Backend-specific error or provider type.
    /// - `R`: Environment type. Additional bound: `HasClusterMessageStore + Send + Sync + 'static`. Additional bound: `:Store: ClusterMessageStore<Error = Backend>`.
    /// - `Q`: Request type.
    ///
    /// # Parameters
    /// - `address` (`ClusterAddress`): Stable destination address of the entity receiving this request.
    /// - `request_id` (`ClusterRequestId`): The request id supplied to this operation.
    /// - `request` (`Q`): The typed request to process.
    /// - `call` (`RpcCallId`): Identity and contents of the selected RPC call.
    /// - `accepted_at` (`UtcMillis`): The accepted at supplied to this operation.
    /// - `acceptance` (`AcceptancePolicy`): The acceptance supplied to this operation.
    ///
    /// # Returns
    /// Type: `Effect<MessageAcceptance, ClusterClientError<Backend>, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
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

    /// Inspect the current state without waiting for a later change.
    ///
    /// # Type parameters
    /// - `Q`: Request type.
    /// - `Backend`: Backend-specific error or provider type.
    /// - `R`: Environment type. Additional bound: `HasClusterMessageStore + Send + Sync + 'static`. Additional bound: `:Store: ClusterMessageStore<Error = Backend>`.
    /// - `A`: Success or application-value type.
    /// - `E`: Domain-error type.
    ///
    /// # Parameters
    /// - `address` (`ClusterAddress`): Stable destination address of the entity receiving this request.
    /// - `request` (`ClusterRequestId`): The typed request to process.
    /// - `observer` (`ClusterObserverId`): Stable identity of the client observing recorded results.
    /// - `call` (`RpcCallId`): Identity and contents of the selected RPC call.
    ///
    /// # Returns
    /// Type: `Effect<ClusterPoll<A, E>, ClusterClientError<Backend>, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
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

    /// Record that the selected observer has consumed the reply.
    ///
    /// # Type parameters
    /// - `Q`: Request type.
    /// - `A`: Application value type.
    /// - `E`: Domain-error type.
    /// - `Backend`: Backend-specific error or provider type.
    /// - `R`: Environment type. Additional bound: `HasClusterMessageStore + Send + Sync + 'static`. Additional bound: `:Store: ClusterMessageStore<Error = Backend>`.
    ///
    /// # Parameters
    /// - `address` (`ClusterAddress`): Stable destination address of the entity receiving this request.
    /// - `request` (`ClusterRequestId`): The typed request to process.
    /// - `observer` (`ClusterObserverId`): Stable identity of the client observing recorded results.
    ///
    /// # Returns
    /// Type: `Effect<StoreMutation, ClusterClientError<Backend>, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
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

    /// Remove an observer without discarding authoritative work.
    ///
    /// # Type parameters
    /// - `Q`: Request type.
    /// - `A`: Application value type.
    /// - `E`: Domain-error type.
    /// - `Backend`: Backend-specific error or provider type.
    /// - `R`: Environment type. Additional bound: `HasClusterMessageStore + Send + Sync + 'static`. Additional bound: `:Store: ClusterMessageStore<Error = Backend>`.
    ///
    /// # Parameters
    /// - `address` (`ClusterAddress`): Stable destination address of the entity receiving this request.
    /// - `request` (`ClusterRequestId`): The typed request to process.
    /// - `observer` (`ClusterObserverId`): Stable identity of the client observing recorded results.
    ///
    /// # Returns
    /// Type: `Effect<bool, ClusterClientError<Backend>, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
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

///
/// # Type parameters
/// - `Q`: Request payload type. Additional bound: `Send + 'static`.
/// - `A`: Successful application-value type. Additional bound: `Send + 'static`.
/// - `E`: Expected application-error type. Additional bound: `Send + 'static`.
/// - `Backend`: Backend-specific error or provider type. Additional bound: `Send + 'static`.
///
/// # Parameters
/// - `operation` (`&RpcOperation<Q, A, E>`): Typed operation definition or callback selected for execution.
/// - `call` (`&RpcCallId`): Identity and contents of the selected RPC call.
/// - `reply` (`&SavedReply`): The reply supplied to this operation.
/// - `policy` (`RpcEnvelopePolicy`): The caller-supplied behavior policy.
///
/// # Returns
/// Type: `Result<Result<A, E>, ClusterClientError<Backend>>`.
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
