use super::{
    RpcCallId, RpcEnvelope, RpcEnvelopeKind, RpcEnvelopePolicy, RpcOperation, RpcWireError,
};
use crate::encoding::{FrameDecodeFailure, FrameDecoder, FrameParts};
use crate::{Effect, SchemaIssue, Semaphore, SemaphoreError};
use std::collections::BTreeMap;
use std::marker::PhantomData;
use std::num::NonZeroUsize;
use std::sync::{Arc, Mutex};

/// A unary transport capability. An implementation may use a socket, a local
/// loopback session or another runtime, but must complete one exchange with
/// the full encoded response frame or return its own typed transport error.
/// It must make cancellation of an in-flight exchange observable according to
/// its provider contract; the core never starts an ambient runtime.
pub trait RpcChannel: Send + Sync + 'static {
    type Error: Send + 'static;
    fn exchange(&self, request: FrameParts) -> Effect<Vec<u8>, Self::Error>;
}

pub trait HasRpcChannel {
    type Channel: RpcChannel;
    fn rpc_channel(&self) -> Arc<Self::Channel>;
}

impl<T: RpcChannel> HasRpcChannel for Arc<T> {
    type Channel = T;

    fn rpc_channel(&self) -> Arc<Self::Channel> {
        Arc::clone(self)
    }
}

#[derive(Debug)]
pub enum RpcClientError<E, Backend> {
    Domain(E),
    RequestSchema(SchemaIssue),
    ResponseSchema(SchemaIssue),
    Wire(RpcWireError),
    /// The request may have reached the server and committed. Reconcile by
    /// the stable call ID before retrying a mutation.
    UncertainTransport {
        call: RpcCallId,
        source: Backend,
    },
    Admission(SemaphoreError),
    MissingResponse,
    ExtraResponses,
    CorrelationMismatch,
    OperationMismatch,
    UnexpectedResponseKind(RpcEnvelopeKind),
    InputConsumed,
}

/// Admission is scoped to this client instance. The caller chooses its exact
/// capacity; shared cross-process admission requires a stronger capability.
/// The transport remains entirely in R.
pub struct RpcClient<Q, A, E, Backend>
where
    Q: Send + 'static,
    A: Send + 'static,
    E: Send + 'static,
    Backend: Send + 'static,
{
    operation: RpcOperation<Q, A, E>,
    policy: RpcEnvelopePolicy,
    admission: Semaphore,
    _backend: PhantomData<fn() -> Backend>,
}

impl<Q, A, E, Backend> Clone for RpcClient<Q, A, E, Backend>
where
    Q: Send + 'static,
    A: Send + 'static,
    E: Send + 'static,
    Backend: Send + 'static,
{
    fn clone(&self) -> Self {
        Self {
            operation: self.operation.clone(),
            policy: self.policy,
            admission: self.admission.clone(),
            _backend: PhantomData,
        }
    }
}

impl<Q, A, E, Backend> RpcClient<Q, A, E, Backend>
where
    Q: Send + 'static,
    A: Send + 'static,
    E: Send + 'static,
    Backend: Send + 'static,
{
    #[must_use]
    pub fn new(
        operation: RpcOperation<Q, A, E>,
        policy: RpcEnvelopePolicy,
        maximum_in_flight: NonZeroUsize,
    ) -> Self {
        Self {
            operation,
            policy,
            admission: Semaphore::full(maximum_in_flight),
            _backend: PhantomData,
        }
    }

    pub fn call<R>(
        &self,
        request: Q,
        call: RpcCallId,
        metadata: BTreeMap<Arc<str>, Arc<str>>,
    ) -> Effect<A, RpcClientError<E, Backend>, R>
    where
        R: HasRpcChannel + Send + Sync + 'static,
        R::Channel: RpcChannel<Error = Backend>,
    {
        let operation = self.operation.clone();
        let response_operation = self.operation.clone();
        let policy = self.policy;
        let admission = self.admission.clone();
        let request = Mutex::new(Some((request, metadata)));
        let request_call = call.clone();
        Effect::try_sync(
            move || {
                let (request, metadata) = request
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .take()
                    .ok_or(RpcClientError::InputConsumed)?;
                let payload = operation
                    .request_codec()
                    .encode_ref(&request)
                    .map_err(RpcClientError::RequestSchema)?;
                crate::schema::internal::parser::check_limits(&payload, &operation.limits())
                    .map_err(RpcClientError::RequestSchema)?;
                let envelope = RpcEnvelope {
                    operation: operation.spec().id.clone(),
                    version: operation.spec().version,
                    call: request_call.clone(),
                    kind: RpcEnvelopeKind::Request,
                    sequence: None,
                    payload,
                    metadata,
                };
                envelope.encode(policy).map_err(RpcClientError::Wire)
            },
            std::convert::identity,
        )
        .flat_map(move |frame| {
            let frame = Mutex::new(Some(frame));
            let admission = admission.clone();
            let operation = response_operation.clone();
            let call = call.clone();
            admission
                .acquire(NonZeroUsize::MIN)
                .map_error(RpcClientError::Admission)
                .flat_map(move |permit| {
                    let frame = frame
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .take();
                    let response = match frame {
                        Some(frame) => {
                            let operation = operation.clone();
                            let call = call.clone();
                            let frame = Mutex::new(Some(frame));
                            Effect::service(|env: &R| env.rpc_channel()).flat_map(move |channel| {
                                let operation = operation.clone();
                                let call = call.clone();
                                let frame = frame
                                    .lock()
                                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                                    .take();
                                match frame {
                                    Some(frame) => channel
                                        .exchange(frame)
                                        .map_error({
                                            let uncertain_call = call.clone();
                                            move |source| RpcClientError::UncertainTransport {
                                                call: uncertain_call.clone(),
                                                source,
                                            }
                                        })
                                        .provide_some(|_: &R| ())
                                        .flat_map(move |bytes| {
                                            Effect::from_result_once(decode_response(
                                                &operation, &call, &bytes, policy,
                                            ))
                                        }),
                                    None => {
                                        Effect::from_result_once(Err(RpcClientError::InputConsumed))
                                    }
                                }
                            })
                        }
                        None => Effect::from_result_once(Err(RpcClientError::InputConsumed)),
                    };
                    response.ensuring(Effect::sync_once(move || drop(permit)))
                })
        })
    }
}

fn decode_response<Q, A, E, Backend>(
    operation: &RpcOperation<Q, A, E>,
    call: &RpcCallId,
    bytes: &[u8],
    policy: RpcEnvelopePolicy,
) -> Result<A, RpcClientError<E, Backend>>
where
    Q: Send + 'static,
    A: Send + 'static,
    E: Send + 'static,
    Backend: Send + 'static,
{
    let mut decoder = FrameDecoder::new(policy.frame);
    let frames = decoder
        .push(bytes)
        .map_err(|FrameDecodeFailure { error, .. }| {
            RpcClientError::Wire(RpcWireError::Frame(error))
        })?;
    decoder
        .finish()
        .map_err(|error| RpcClientError::Wire(RpcWireError::Frame(error)))?;
    let mut frames = frames.into_iter();
    let Some(first) = frames.next() else {
        return Err(RpcClientError::MissingResponse);
    };
    if frames.next().is_some() {
        return Err(RpcClientError::ExtraResponses);
    }
    let response = RpcEnvelope::decode(&first, policy).map_err(RpcClientError::Wire)?;
    if response.call != *call {
        return Err(RpcClientError::CorrelationMismatch);
    }
    if response.operation != operation.spec().id || response.version != operation.spec().version {
        return Err(RpcClientError::OperationMismatch);
    }
    match response.kind {
        RpcEnvelopeKind::Success => {
            crate::schema::internal::parser::check_limits(&response.payload, &operation.limits())
                .map_err(RpcClientError::ResponseSchema)?;
            operation
                .success_codec()
                .decode_ref(&response.payload)
                .map_err(RpcClientError::ResponseSchema)
        }
        RpcEnvelopeKind::DomainError => {
            crate::schema::internal::parser::check_limits(&response.payload, &operation.limits())
                .map_err(RpcClientError::ResponseSchema)?;
            let error = operation
                .error_codec()
                .decode_ref(&response.payload)
                .map_err(RpcClientError::ResponseSchema)?;
            Err(RpcClientError::Domain(error))
        }
        kind @ (RpcEnvelopeKind::Request
        | RpcEnvelopeKind::StreamChunk
        | RpcEnvelopeKind::StreamEnd
        | RpcEnvelopeKind::Acknowledge
        | RpcEnvelopeKind::Cancel) => Err(RpcClientError::UnexpectedResponseKind(kind)),
    }
}
