//! Typed entity registration and dispatch.

use super::{ClusterAddress, ClusterEntityType, ReplyKind, SavedReply};
use crate::encoding::{FrameDecodeFailure, FrameDecoder};
use crate::rpc::{
    OperationId, RpcEnvelope, RpcEnvelopeKind, RpcEnvelopePolicy, RpcHandler, RpcVersion,
    RpcWireError,
};
use crate::{Effect, SchemaIssue};
use std::collections::BTreeMap;
use std::marker::PhantomData;
use std::sync::{Arc, Mutex};

#[derive(Debug)]
/// A framing, address, codec, or handler failure in typed entity dispatch.
pub enum EntityDispatchError {
    /// RPC framing or envelope validation failed. Retains the `RpcWireError` detail.
    Wire(RpcWireError),
    /// The request failed its declared schema validation. Retains the `SchemaIssue` detail.
    RequestSchema(SchemaIssue),
    /// The reply failed its declared schema validation. Retains the `SchemaIssue` detail.
    ReplySchema(SchemaIssue),
    /// The operation is not recognized.
    UnknownOperation {
        /// Stable type identity of the addressed entity. Type: `ClusterEntityType`.
        entity_type: ClusterEntityType,
        /// Registered operation associated with this value. Type: `OperationId`.
        operation: OperationId,
        /// The declared compatibility version. Type: `RpcVersion`.
        version: RpcVersion,
    },
    /// The supplied operation does not match the required or recorded value.
    WrongOperation,
    /// The address differs from the required or recorded value.
    AddressMismatch,
    /// The received kind is invalid in the current protocol state. Retains the `RpcEnvelopeKind` detail.
    UnexpectedKind(RpcEnvelopeKind),
    /// An expected-error value appeared where the protocol required another outcome.
    UnexpectedExpectedError,
    /// The required frame is missing.
    MissingFrame,
    /// Unexpected additional frames were present.
    ExtraFrames,
    /// An owned input was already consumed.
    InputConsumed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// A conflict or invalid definition detected while registering an entity handler.
pub enum EntityRegistrationError {
    /// The  already exists or occurred twice.
    Duplicate {
        /// Stable type identity of the addressed entity. Type: `ClusterEntityType`.
        entity_type: ClusterEntityType,
        /// Registered operation associated with this value. Type: `OperationId`.
        operation: OperationId,
        /// The declared compatibility version. Type: `RpcVersion`.
        version: RpcVersion,
    },
}

/// Executes accepted entity messages through a typed application capability.
pub trait ClusterExecutor<R: Send + Sync + 'static>: Send + Sync + 'static {
    /// Provider-specific failure type returned by this capability.
    type Error: Send + 'static;
    /// # Type parameters
    /// - `R`: Environment type.
    ///
    /// # Parameters
    /// - `address` (`ClusterAddress`): Stable destination address of the entity receiving this request.
    /// - `encoded_request` (`Arc<[u8]>`): The encoded request supplied to this operation.
    ///
    /// # Returns
    /// Type: `Effect<SavedReply, Self::Error, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    fn execute(
        &self,
        address: ClusterAddress,
        encoded_request: Arc<[u8]>,
    ) -> Effect<SavedReply, Self::Error, R>;
}

/// Selects the Cluster executor in an application environment.
pub trait HasClusterExecutor: Sized + Send + Sync + 'static {
    /// Executor selected by this environment.
    type Executor: ClusterExecutor<Self>;
    /// # Returns
    /// Type: `Arc<Self::Executor>`.
    fn cluster_executor(&self) -> Arc<Self::Executor>;
}

/// Private vtable-dispatched handler after typed entity registration. Stored
/// messages retain encoded data and identity, not the live handler closure.
///
/// # Type parameters
/// - `R`: Environment type.
type Dispatch<R> = Arc<
    dyn Fn(ClusterAddress, RpcEnvelope) -> Effect<SavedReply, EntityDispatchError, R> + Send + Sync,
>;

/// A registry that decodes addressed requests, validates their target, and invokes typed handlers.
/// request, success or domain-error value is downcast at runtime.
///
/// # Type parameters
/// - `R`: Environment type. Bound or type: `Send + Sync + 'static`.
pub struct EntityDispatcher<R: Send + Sync + 'static> {
    policy: RpcEnvelopePolicy,
    handlers: BTreeMap<(ClusterEntityType, OperationId, RpcVersion), Dispatch<R>>,
    _env: PhantomData<fn() -> R>,
}

impl<R: Send + Sync + 'static> Clone for EntityDispatcher<R> {
    ///
    /// # Returns
    /// Type: `Self`.
    fn clone(&self) -> Self {
        Self {
            policy: self.policy,
            handlers: self.handlers.clone(),
            _env: PhantomData,
        }
    }
}

impl<R: Send + Sync + 'static> EntityDispatcher<R> {
    ///
    /// # Parameters
    /// - `policy` (`RpcEnvelopePolicy`): The caller-supplied behavior policy.
    ///
    /// # Returns
    /// Type: `Self`.
    fn empty(policy: RpcEnvelopePolicy) -> Self {
        Self {
            policy,
            handlers: BTreeMap::new(),
            _env: PhantomData,
        }
    }

    /// Create a protocol with its first typed handler already registered.
    ///
    /// # Type parameters
    /// - `Q`: Request type. Additional bound: `Send + 'static`.
    /// - `A`: Application value type. Additional bound: `Send + 'static`.
    /// - `E`: Domain-error type. Additional bound: `Send + 'static`.
    /// - `R`: Environment type.
    ///
    /// # Parameters
    /// - `policy` (`RpcEnvelopePolicy`): The caller-supplied behavior policy.
    /// - `entity_type` (`ClusterEntityType`): Stable type identity used for addressed dispatch.
    /// - `handler` (`RpcHandler<Q, A, E, R>`): The handler invoked for the matching request or event.
    ///
    /// # Returns
    /// Type: `Result<Self, EntityRegistrationError>`.
    pub fn from_handler<Q, A, E>(
        policy: RpcEnvelopePolicy,
        entity_type: ClusterEntityType,
        handler: RpcHandler<Q, A, E, R>,
    ) -> Result<Self, EntityRegistrationError>
    where
        Q: Send + 'static,
        A: Send + 'static,
        E: Send + 'static,
    {
        Self::from_addressed_handler(policy, entity_type, handler, |_, _| true)
    }

    /// Register a first handler that validates the request against its addressed entity.
    ///
    /// # Type parameters
    /// - `Q`: Request type. Additional bound: `Send + 'static`.
    /// - `A`: Application value type. Additional bound: `Send + 'static`.
    /// - `E`: Domain-error type. Additional bound: `Send + 'static`.
    /// - `R`: Environment type.
    ///
    /// # Parameters
    /// - `policy` (`RpcEnvelopePolicy`): The caller-supplied behavior policy.
    /// - `entity_type` (`ClusterEntityType`): Stable type identity used for addressed dispatch.
    /// - `handler` (`RpcHandler<Q, A, E, R>`): The handler invoked for the matching request or event.
    /// - `accepts` (`impl Fn(&ClusterAddress, &Q) -> bool + Send + Sync + 'static`): The accepts supplied to this operation.
    ///
    /// # Returns
    /// Type: `Result<Self, EntityRegistrationError>`.
    pub fn from_addressed_handler<Q, A, E>(
        policy: RpcEnvelopePolicy,
        entity_type: ClusterEntityType,
        handler: RpcHandler<Q, A, E, R>,
        accepts: impl Fn(&ClusterAddress, &Q) -> bool + Send + Sync + 'static,
    ) -> Result<Self, EntityRegistrationError>
    where
        Q: Send + 'static,
        A: Send + 'static,
        E: Send + 'static,
    {
        let mut dispatcher = Self::empty(policy);
        dispatcher.register_addressed(entity_type, handler, accepts)?;
        Ok(dispatcher)
    }

    /// Register the supplied definition, rejecting an identity or compatibility conflict.
    ///
    /// # Type parameters
    /// - `Q`: Request type. Additional bound: `Send + 'static`.
    /// - `A`: Application value type. Additional bound: `Send + 'static`.
    /// - `E`: Domain-error type. Additional bound: `Send + 'static`.
    /// - `R`: Environment type.
    ///
    /// # Parameters
    /// - `entity_type` (`ClusterEntityType`): Stable type identity used for addressed dispatch.
    /// - `handler` (`RpcHandler<Q, A, E, R>`): The handler invoked for the matching request or event.
    ///
    /// # Returns
    /// Type: `Result<(), EntityRegistrationError>`.
    pub fn register<Q, A, E>(
        &mut self,
        entity_type: ClusterEntityType,
        handler: RpcHandler<Q, A, E, R>,
    ) -> Result<(), EntityRegistrationError>
    where
        Q: Send + 'static,
        A: Send + 'static,
        E: Send + 'static,
    {
        self.register_addressed(entity_type, handler, |_, _| true)
    }

    /// Validate the decoded request against its actual Cluster address
    /// before invoking the handler. Protocol adapters use this to prevent a
    /// saved request from being replayed against a different logical entity.
    ///
    /// # Type parameters
    /// - `Q`: Request type. Additional bound: `Send + 'static`.
    /// - `A`: Application value type. Additional bound: `Send + 'static`.
    /// - `E`: Domain-error type. Additional bound: `Send + 'static`.
    /// - `R`: Environment type.
    ///
    /// # Parameters
    /// - `entity_type` (`ClusterEntityType`): Stable type identity used for addressed dispatch.
    /// - `handler` (`RpcHandler<Q, A, E, R>`): The handler invoked for the matching request or event.
    /// - `accepts` (`impl Fn(&ClusterAddress, &Q) -> bool + Send + Sync + 'static`): The accepts supplied to this operation.
    ///
    /// # Returns
    /// Type: `Result<(), EntityRegistrationError>`.
    pub fn register_addressed<Q, A, E>(
        &mut self,
        entity_type: ClusterEntityType,
        handler: RpcHandler<Q, A, E, R>,
        accepts: impl Fn(&ClusterAddress, &Q) -> bool + Send + Sync + 'static,
    ) -> Result<(), EntityRegistrationError>
    where
        Q: Send + 'static,
        A: Send + 'static,
        E: Send + 'static,
    {
        let spec = handler.operation().spec();
        let key = (entity_type, spec.id.clone(), spec.version);
        if self.handlers.contains_key(&key) {
            return Err(EntityRegistrationError::Duplicate {
                entity_type: key.0,
                operation: key.1,
                version: key.2,
            });
        }
        let policy = self.policy;
        self.handlers.insert(
            key,
            Arc::new(move |address, envelope| {
                let operation = handler.operation().clone();
                if envelope.operation != operation.spec().id
                    || envelope.version != operation.spec().version
                {
                    return Effect::from_result_once(Err(EntityDispatchError::WrongOperation));
                }
                if envelope.kind != RpcEnvelopeKind::Request {
                    return Effect::from_result_once(Err(EntityDispatchError::UnexpectedKind(
                        envelope.kind,
                    )));
                }
                if let Err(issue) = crate::schema::internal::parser::check_limits(
                    &envelope.payload,
                    &operation.limits(),
                ) {
                    return Effect::from_result_once(Err(EntityDispatchError::RequestSchema(
                        issue,
                    )));
                }
                let request = match operation.request_codec().decode_ref(&envelope.payload) {
                    Ok(request) => request,
                    Err(issue) => {
                        return Effect::from_result_once(Err(EntityDispatchError::RequestSchema(
                            issue,
                        )));
                    }
                };
                if !accepts(&address, &request) {
                    return Effect::from_result_once(Err(EntityDispatchError::AddressMismatch));
                }
                let envelope = Mutex::new(Some(envelope));
                handler
                    .call(request)
                    .either()
                    .map_error(|_| EntityDispatchError::UnexpectedExpectedError)
                    .flat_map(move |result| {
                        let encoded = match &result {
                            Ok(value) => operation.success_codec().encode_ref(value),
                            Err(error) => operation.error_codec().encode_ref(error),
                        }
                        .map_err(EntityDispatchError::ReplySchema);
                        let payload = match encoded {
                            Ok(payload) => payload,
                            Err(error) => return Effect::from_result_once(Err(error)),
                        };
                        if let Err(issue) = crate::schema::internal::parser::check_limits(
                            &payload,
                            &operation.limits(),
                        ) {
                            return Effect::from_result_once(Err(
                                EntityDispatchError::ReplySchema(issue),
                            ));
                        }
                        let envelope = envelope
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner)
                            .take();
                        let Some(mut envelope) = envelope else {
                            return Effect::from_result_once(Err(
                                EntityDispatchError::InputConsumed,
                            ));
                        };
                        let kind = if result.is_ok() {
                            envelope.kind = RpcEnvelopeKind::Success;
                            ReplyKind::Success
                        } else {
                            envelope.kind = RpcEnvelopeKind::DomainError;
                            ReplyKind::DomainError
                        };
                        envelope.sequence = None;
                        envelope.payload = payload;
                        let encoded = envelope
                            .encode(policy)
                            .map_err(EntityDispatchError::Wire)
                            .and_then(|frame| {
                                frame.into_bytes().map_err(|error| {
                                    EntityDispatchError::Wire(RpcWireError::Frame(error))
                                })
                            })
                            .map(|bytes| SavedReply {
                                kind,
                                payload: Arc::from(bytes),
                            });
                        Effect::from_result_once(encoded)
                    })
            }),
        );
        Ok(())
    }

    ///
    /// # Type parameters
    /// - `R`: Environment type.
    ///
    /// # Parameters
    /// - `address` (`ClusterAddress`): Stable destination address of the entity receiving this request.
    /// - `encoded_request` (`Arc<[u8]>`): The encoded request supplied to this operation.
    ///
    /// # Returns
    /// Type: `Effect<SavedReply, EntityDispatchError, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    fn dispatch(
        &self,
        address: ClusterAddress,
        encoded_request: Arc<[u8]>,
    ) -> Effect<SavedReply, EntityDispatchError, R> {
        let dispatcher = self.clone();
        Effect::defer(move || {
            dispatcher.dispatch_now(address.clone(), Arc::clone(&encoded_request))
        })
    }

    ///
    /// # Type parameters
    /// - `R`: Environment type.
    ///
    /// # Parameters
    /// - `address` (`ClusterAddress`): Stable destination address of the entity receiving this request.
    /// - `encoded_request` (`Arc<[u8]>`): The encoded request supplied to this operation.
    ///
    /// # Returns
    /// Type: `Effect<SavedReply, EntityDispatchError, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    fn dispatch_now(
        &self,
        address: ClusterAddress,
        encoded_request: Arc<[u8]>,
    ) -> Effect<SavedReply, EntityDispatchError, R> {
        let mut decoder = FrameDecoder::new(self.policy.frame);
        let frames = match decoder.push(&encoded_request) {
            Ok(frames) => frames,
            Err(FrameDecodeFailure { error, .. }) => {
                return Effect::from_result_once(Err(EntityDispatchError::Wire(
                    RpcWireError::Frame(error),
                )));
            }
        };
        if let Err(error) = decoder.finish() {
            return Effect::from_result_once(Err(EntityDispatchError::Wire(RpcWireError::Frame(
                error,
            ))));
        }
        let mut frames = frames.into_iter();
        let Some(first) = frames.next() else {
            return Effect::from_result_once(Err(EntityDispatchError::MissingFrame));
        };
        if frames.next().is_some() {
            return Effect::from_result_once(Err(EntityDispatchError::ExtraFrames));
        }
        let envelope = match RpcEnvelope::decode(&first, self.policy) {
            Ok(envelope) => envelope,
            Err(error) => return Effect::from_result_once(Err(EntityDispatchError::Wire(error))),
        };
        let key = (
            address.entity_type.clone(),
            envelope.operation.clone(),
            envelope.version,
        );
        match self.handlers.get(&key) {
            Some(handler) => handler(address, envelope),
            None => Effect::from_result_once(Err(EntityDispatchError::UnknownOperation {
                entity_type: key.0,
                operation: key.1,
                version: key.2,
            })),
        }
    }
}

impl<R: Send + Sync + 'static> ClusterExecutor<R> for EntityDispatcher<R> {
    /// Type alias for error in cluster entity.
    type Error = EntityDispatchError;
    ///
    /// # Type parameters
    /// - `R`: Environment type.
    ///
    /// # Parameters
    /// - `address` (`ClusterAddress`): Stable destination address of the entity receiving this request.
    /// - `encoded_request` (`Arc<[u8]>`): The encoded request supplied to this operation.
    ///
    /// # Returns
    /// Type: `Effect<SavedReply, Self::Error, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    fn execute(
        &self,
        address: ClusterAddress,
        encoded_request: Arc<[u8]>,
    ) -> Effect<SavedReply, Self::Error, R> {
        self.dispatch(address, encoded_request)
    }
}
