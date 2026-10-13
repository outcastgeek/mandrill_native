//! Checked registration and dispatch of unary handlers.

use super::{
    OperationId, RpcChannel, RpcEnvelope, RpcEnvelopePolicy, RpcHandler, RpcRegistry,
    RpcRegistryError, RpcServerError, RpcUnaryServer, RpcVersion, RpcWireError,
};
use crate::Effect;
use crate::encoding::{FrameDecodeFailure, FrameDecoder, FrameParts};
use crate::{effect, effect_provided, effect_try_sync};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

/// Private vtable-dispatched boundary after a typed handler is registered.
/// Dispatch erases request and success types only inside this protocol;
/// registration preserves their checked codec and handler pairing.
///
/// # Type parameters
/// - `E`: Domain-error type.
/// - `R`: Environment type.
type EncodedHandler<E, R> =
    Arc<dyn Fn(RpcEnvelope) -> Effect<Vec<u8>, RpcProtocolError<E>, R> + Send + Sync>;

#[derive(Debug)]
/// A registration, decoding, or dispatch failure in a typed unary protocol.
///
/// # Type parameters
/// - `E`: Domain-error type.
pub enum RpcProtocolError<E> {
    /// RPC framing or envelope validation failed. Retains the `RpcWireError` detail.
    Wire(RpcWireError),
    /// The server could not dispatch or encode the reply. Retains the `RpcServerError<E>` detail.
    Server(RpcServerError<E>),
    /// The operation is not recognized.
    UnknownOperation {
        /// Stable identity of this record. Type: `OperationId`.
        id: OperationId,
        /// The declared compatibility version. Type: `RpcVersion`.
        version: RpcVersion,
    },
    /// The required frame is missing.
    MissingFrame,
    /// Unexpected additional frames were present.
    ExtraFrames,
    /// An owned input was already consumed.
    InputConsumed,
}

/// Several typed operations share one encoded protocol. Only the private
/// dispatch function is erased; each registration still typechecks its
/// request, success and common application-domain error.
///
/// # Type parameters
/// - `E`: Domain-error type. Bound or type: `Send + 'static`.
/// - `R`: Environment type. Bound or type: `Send + Sync + 'static`.
pub struct RpcProtocol<E: Send + 'static, R: Send + Sync + 'static> {
    policy: RpcEnvelopePolicy,
    registry: RpcRegistry,
    handlers: Arc<BTreeMap<(OperationId, RpcVersion), EncodedHandler<E, R>>>,
}

impl<E: Send + 'static, R: Send + Sync + 'static> RpcProtocol<E, R> {
    /// Create a protocol with its first typed handler already registered.
    ///
    /// # Type parameters
    /// - `Q`: Request type. Bounds: `Send + 'static`.
    /// - `A`: Application value type. Bounds: `Send + 'static`.
    /// - `E`: Domain-error type.
    /// - `R`: Environment type.
    ///
    /// # Parameters
    /// - `policy` (`RpcEnvelopePolicy`): The caller-supplied behavior policy.
    /// - `handler` (`RpcHandler<Q, A, E, R>`): The handler invoked for the matching request or event.
    ///
    /// # Returns
    /// Type: `Result<Self, RpcRegistryError>`.
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

    /// Register the supplied definition, rejecting an identity or compatibility conflict.
    ///
    /// # Type parameters
    /// - `Q`: Request type. Bounds: `Send + 'static`.
    /// - `A`: Application value type. Bounds: `Send + 'static`.
    /// - `E`: Domain-error type.
    /// - `R`: Environment type.
    ///
    /// # Parameters
    /// - `handler` (`RpcHandler<Q, A, E, R>`): The handler invoked for the matching request or event.
    ///
    /// # Returns
    /// Type: `Result<(), RpcRegistryError>`.
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

    /// Dispatch the supplied input to its registered typed handler.
    ///
    /// # Type parameters
    /// - `E`: Domain-error type.
    /// - `R`: Environment type.
    ///
    /// # Parameters
    /// - `bytes` (`Vec<u8>`): Encoded bytes to validate, decode, write, or transfer.
    ///
    /// # Returns
    /// Type: `Effect<Vec<u8>, RpcProtocolError<E>, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    pub fn handle(&self, bytes: Vec<u8>) -> Effect<Vec<u8>, RpcProtocolError<E>, R> {
        let policy = self.policy;
        let handlers = Arc::clone(&self.handlers);
        let bytes = Mutex::new(Some(bytes));
        {
            let __dsl_program = effect_try_sync! { body: move || {
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
            }, on_error: std::convert::identity, };
            effect! { let handlers = handlers.clone(); let (key , envelope) = __dsl_program.clone()?; match handlers.get(&key) {
                Some(handler) => handler(envelope),
                None => effect_try_sync! { once; body: { let __dsl_result = Err(RpcProtocolError::UnknownOperation {
                    id: key.0,
                    version: key.1,
                }); move || __dsl_result }, on_error: ::core::convert::identity, },
            }? }
        }
    }
}

/// An RPC channel that invokes a registered protocol without a network transport.
///
/// # Type parameters
/// - `E`: Domain-error type. Bound or type: `Send + 'static`.
/// - `R`: Environment type. Bound or type: `Send + Sync + 'static`.
pub struct InProcessRpcProtocolChannel<E: Send + 'static, R: Send + Sync + 'static> {
    protocol: Arc<RpcProtocol<E, R>>,
    environment: Arc<R>,
}

impl<E: Send + 'static, R: Send + Sync + 'static> InProcessRpcProtocolChannel<E, R> {
    #[must_use]
    /// Construct a `InProcessRpcProtocolChannel` from its required inputs and explicit policies.
    ///
    /// # Type parameters
    /// - `E`: Domain-error type.
    /// - `R`: Environment type.
    ///
    /// # Parameters
    /// - `protocol` (`Arc<RpcProtocol<E, R>>`): The protocol supplied to this operation.
    /// - `environment` (`Arc<R>`): The environment supplying this operation's capabilities.
    ///
    /// # Returns
    /// Type: `Self`.
    pub fn new(protocol: Arc<RpcProtocol<E, R>>, environment: Arc<R>) -> Self {
        Self {
            protocol,
            environment,
        }
    }
}

impl<E: Send + 'static, R: Send + Sync + 'static> RpcChannel for InProcessRpcProtocolChannel<E, R> {
    /// Type alias for error in rpc protocol.
    type Error = RpcProtocolError<E>;

    ///
    /// # Parameters
    /// - `request` (`FrameParts`): Typed request submitted to a handler or transport.
    ///
    /// # Returns
    /// Type: `Effect<Vec<u8>, Self::Error>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    fn exchange(&self, request: FrameParts) -> Effect<Vec<u8>, Self::Error> {
        let bytes = match request.into_bytes() {
            Ok(bytes) => bytes,
            Err(error) => {
                return effect_try_sync! { once; body: { let __dsl_result = Err(RpcProtocolError::Wire(RpcWireError::Frame(
                    error,
                ))); move || __dsl_result }, on_error: ::core::convert::identity, };
            }
        };
        effect_provided! { program: self.protocol
        .handle(bytes), environment: Arc::clone(&self.environment), }
    }
}
