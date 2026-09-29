use super::{
    OperationId, RpcChannel, RpcEnvelope, RpcEnvelopePolicy, RpcHandler, RpcRegistry,
    RpcRegistryError, RpcServerError, RpcUnaryServer, RpcVersion, RpcWireError,
};
use crate::Effect;
use crate::encoding::{FrameDecodeFailure, FrameDecoder, FrameParts};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

type EncodedHandler<E, R> =
    Arc<dyn Fn(RpcEnvelope) -> Effect<Vec<u8>, RpcProtocolError<E>, R> + Send + Sync>;

#[derive(Debug)]
pub enum RpcProtocolError<E> {
    Wire(RpcWireError),
    Server(RpcServerError<E>),
    UnknownOperation {
        id: OperationId,
        version: RpcVersion,
    },
    MissingFrame,
    ExtraFrames,
    InputConsumed,
}

/// Several typed operations share one encoded protocol. Only the private
/// dispatch function is erased; each registration still typechecks its
/// request, success and common application-domain error.
pub struct RpcProtocol<E: Send + 'static, R: Send + Sync + 'static> {
    policy: RpcEnvelopePolicy,
    registry: RpcRegistry,
    handlers: Arc<BTreeMap<(OperationId, RpcVersion), EncodedHandler<E, R>>>,
}

impl<E: Send + 'static, R: Send + Sync + 'static> RpcProtocol<E, R> {
    pub fn from_handler<Q: Send + 'static, A: Send + 'static>(
        policy: RpcEnvelopePolicy,
        handler: RpcHandler<Q, A, E, R>,
    ) -> Result<Self, RpcRegistryError> {
        let mut protocol = Self {
            policy,
            registry: RpcRegistry::new(),
            handlers: Arc::new(BTreeMap::new()),
        };
        protocol.register(handler)?;
        Ok(protocol)
    }

    pub fn register<Q: Send + 'static, A: Send + 'static>(
        &mut self,
        handler: RpcHandler<Q, A, E, R>,
    ) -> Result<(), RpcRegistryError> {
        let spec = handler.operation().spec().clone();
        self.registry.register(&spec)?;
        let server = RpcUnaryServer::new(handler, self.policy);
        Arc::make_mut(&mut self.handlers).insert(
            (spec.id, spec.version),
            Arc::new(move |envelope| {
                server
                    .handle_envelope(envelope)
                    .map_error(RpcProtocolError::Server)
                    .flat_map(|frame| {
                        Effect::from_result_once(
                            frame.into_bytes().map_err(|error| {
                                RpcProtocolError::Wire(RpcWireError::Frame(error))
                            }),
                        )
                    })
            }),
        );
        Ok(())
    }

    pub fn handle(&self, bytes: Vec<u8>) -> Effect<Vec<u8>, RpcProtocolError<E>, R> {
        let policy = self.policy;
        let handlers = Arc::clone(&self.handlers);
        let bytes = Mutex::new(Some(bytes));
        Effect::try_sync(
            move || {
                let bytes = bytes
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .take()
                    .ok_or(RpcProtocolError::InputConsumed)?;
                let mut decoder = FrameDecoder::new(policy.frame);
                let frames =
                    decoder
                        .push(&bytes)
                        .map_err(|FrameDecodeFailure { error, .. }| {
                            RpcProtocolError::Wire(RpcWireError::Frame(error))
                        })?;
                decoder
                    .finish()
                    .map_err(|error| RpcProtocolError::Wire(RpcWireError::Frame(error)))?;
                let mut frames = frames.into_iter();
                let first = frames.next().ok_or(RpcProtocolError::MissingFrame)?;
                if frames.next().is_some() {
                    return Err(RpcProtocolError::ExtraFrames);
                }
                let envelope =
                    RpcEnvelope::decode(&first, policy).map_err(RpcProtocolError::Wire)?;
                Ok(((envelope.operation.clone(), envelope.version), envelope))
            },
            std::convert::identity,
        )
        .flat_map(move |(key, envelope)| match handlers.get(&key) {
            Some(handler) => handler(envelope),
            None => Effect::from_result_once(Err(RpcProtocolError::UnknownOperation {
                id: key.0,
                version: key.1,
            })),
        })
    }
}

pub struct InProcessRpcProtocolChannel<E: Send + 'static, R: Send + Sync + 'static> {
    protocol: Arc<RpcProtocol<E, R>>,
    environment: Arc<R>,
}

impl<E: Send + 'static, R: Send + Sync + 'static> InProcessRpcProtocolChannel<E, R> {
    #[must_use]
    pub fn new(protocol: Arc<RpcProtocol<E, R>>, environment: Arc<R>) -> Self {
        Self {
            protocol,
            environment,
        }
    }
}

impl<E: Send + 'static, R: Send + Sync + 'static> RpcChannel for InProcessRpcProtocolChannel<E, R> {
    type Error = RpcProtocolError<E>;

    fn exchange(&self, request: FrameParts) -> Effect<Vec<u8>, Self::Error> {
        let bytes = match request.into_bytes() {
            Ok(bytes) => bytes,
            Err(error) => {
                return Effect::from_result_once(Err(RpcProtocolError::Wire(RpcWireError::Frame(
                    error,
                ))));
            }
        };
        self.protocol
            .handle(bytes)
            .provide(Arc::clone(&self.environment))
    }
}
