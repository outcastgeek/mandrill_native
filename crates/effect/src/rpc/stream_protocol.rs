//! Checked registration of typed streaming handlers.

use super::{
    OperationId, RpcEnvelope, RpcEnvelopeKind, RpcEnvelopePolicy, RpcRegistry, RpcRegistryError,
    RpcStreamHandler, RpcStreamServer, RpcStreamServerError, RpcStreamSession, RpcVersion,
    RpcWireError,
};
use crate::Effect;
use crate::encoding::{FrameDecodeFailure, FrameDecoder};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

/// Type alias for dispatch in rpc stream protocol.
///
/// # Type parameters
/// - `E`: Domain-error type.
/// - `Backend`: Backend-specific error or provider type.
/// - `R`: Environment type.
/// - `Session`: Active stream-session provider type.
type Dispatch<E, Backend, R, Session> = Arc<
    dyn Fn(Arc<Session>, Vec<u8>) -> Effect<(), RpcStreamProtocolError<E, Backend>, R>
        + Send
        + Sync,
>;
/// Type alias for stream handlers in rpc stream protocol.
///
/// # Type parameters
/// - `E`: Domain-error type.
/// - `Backend`: Backend-specific error or provider type.
/// - `R`: Environment type.
/// - `Session`: Active stream-session provider type.
type StreamHandlers<E, Backend, R, Session> =
    BTreeMap<(OperationId, RpcVersion), Dispatch<E, Backend, R, Session>>;

#[derive(Debug)]
/// A registration or dispatch failure in a typed streaming protocol.
///
/// # Type parameters
/// - `E`: Domain-error type.
/// - `Backend`: Backend-specific error or provider type.
pub enum RpcStreamProtocolError<E, Backend> {
    /// RPC framing or envelope validation failed. Retains the `RpcWireError` detail.
    Wire(RpcWireError),
    /// The selected transport provider failed. Retains the `Backend` detail.
    Transport(Backend),
    /// The server could not dispatch or encode the reply. Retains the `RpcStreamServerError<E, Backend>` detail.
    Server(RpcStreamServerError<E, Backend>),
    /// The remote connection ended before settlement.
    Disconnected,
    /// The required frame is missing.
    MissingFrame,
    /// Unexpected additional frames were present.
    ExtraFrames,
    /// The received kind is invalid in the current protocol state. Retains the `RpcEnvelopeKind` detail.
    UnexpectedKind(RpcEnvelopeKind),
    /// The operation is not recognized.
    UnknownOperation {
        /// Stable identity of this record. Type: `OperationId`.
        id: OperationId,
        /// The declared compatibility version. Type: `RpcVersion`.
        version: RpcVersion,
    },
    /// An owned input was already consumed.
    InputConsumed,
    /// The stream was closed before its required final acknowledgement.
    CloseAfter {
        /// Original outcome before cleanup was attempted. Type: `Box<Self>`.
        primary: Box<Self>,
        /// Backend failure encountered while closing the stream. Type: `Backend`.
        close: Backend,
    },
}

/// An encoded, multi-operation streaming protocol over a provider-owned
/// session. Typed registration is checked before private wire dispatch.
///
/// # Type parameters
/// - `E`: Domain-error type.
/// - `Backend`: Backend-specific error or provider type.
/// - `R`: Environment type.
/// - `Session`: Active stream-session provider type.
pub struct RpcStreamProtocol<E, Backend, R, Session>
where
    E: Send + 'static,
    Backend: Send + 'static,
    R: Send + Sync + 'static,
    Session: RpcStreamSession<Error = Backend>,
{
    policy: RpcEnvelopePolicy,
    registry: RpcRegistry,
    handlers: Arc<StreamHandlers<E, Backend, R, Session>>,
}

impl<E, Backend, R, Session> Clone for RpcStreamProtocol<E, Backend, R, Session>
where
    E: Send + 'static,
    Backend: Send + 'static,
    R: Send + Sync + 'static,
    Session: RpcStreamSession<Error = Backend>,
{
    ///
    /// # Returns
    /// Type: `Self`.
    fn clone(&self) -> Self {
        Self {
            policy: self.policy,
            registry: self.registry.clone(),
            handlers: Arc::clone(&self.handlers),
        }
    }
}

impl<E, Backend, R, Session> RpcStreamProtocol<E, Backend, R, Session>
where
    E: Send + 'static,
    Backend: Send + 'static,
    R: Send + Sync + 'static,
    Session: RpcStreamSession<Error = Backend>,
{
    /// Create a protocol with its first typed handler already registered.
    ///
    /// # Type parameters
    /// - `Backend`: Backend-specific error or provider type.
    /// - `Session`: Active stream-session provider type.
    /// - `Q`: Request type. Additional bound: `Send + 'static`.
    /// - `C`: Stream chunk or item type. Additional bound: `Send + 'static`.
    /// - `T`: Value or item type. Additional bound: `Send + 'static`.
    /// - `E`: Domain-error type.
    /// - `R`: Environment type.
    ///
    /// # Parameters
    /// - `policy` (`RpcEnvelopePolicy`): The caller-supplied behavior policy.
    /// - `handler` (`RpcStreamHandler<Q, C, T, E, R>`): The handler invoked for the matching request or event.
    ///
    /// # Returns
    /// Type: `Result<Self, RpcRegistryError>`.
    pub fn from_handler<Q, C, T>(
        policy: RpcEnvelopePolicy,
        handler: RpcStreamHandler<Q, C, T, E, R>,
    ) -> Result<Self, RpcRegistryError>
    where
        Q: Send + 'static,
        C: Send + 'static,
        T: Send + 'static,
    {
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
    /// - `Backend`: Backend-specific error or provider type.
    /// - `Session`: Active stream-session provider type.
    /// - `Q`: Request type. Additional bound: `Send + 'static`.
    /// - `C`: Stream chunk or item type. Additional bound: `Send + 'static`.
    /// - `T`: Value or item type. Additional bound: `Send + 'static`.
    /// - `E`: Domain-error type.
    /// - `R`: Environment type.
    ///
    /// # Parameters
    /// - `handler` (`RpcStreamHandler<Q, C, T, E, R>`): The handler invoked for the matching request or event.
    ///
    /// # Returns
    /// Type: `Result<(), RpcRegistryError>`.
    pub fn register<Q, C, T>(
        &mut self,
        handler: RpcStreamHandler<Q, C, T, E, R>,
    ) -> Result<(), RpcRegistryError>
    where
        Q: Send + 'static,
        C: Send + 'static,
        T: Send + 'static,
    {
        let spec = handler.operation().spec().clone();
        self.registry.register(&spec)?;
        let server = RpcStreamServer::new(handler, self.policy);
        Arc::make_mut(&mut self.handlers).insert(
            (spec.id, spec.version),
            Arc::new(move |session, first| {
                server
                    .serve_prefetched(session, first)
                    .map_error(RpcStreamProtocolError::Server)
            }),
        );
        Ok(())
    }

    /// Dispatch an encoded stream request to its registered typed handler.
    ///
    /// # Type parameters
    /// - `Backend`: Backend-specific error or provider type.
    /// - `Session`: Active stream-session provider type.
    /// - `E`: Domain-error type.
    /// - `R`: Environment type.
    ///
    /// # Parameters
    /// - `session` (`Arc<Session>`): Scoped connection or stream session retained until settlement.
    ///
    /// # Returns
    /// Type: `Effect<(), RpcStreamProtocolError<E, Backend>, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    pub fn serve(
        &self,
        session: Arc<Session>,
    ) -> Effect<(), RpcStreamProtocolError<E, Backend>, R> {
        let protocol = self.clone();
        let failed = Arc::clone(&session);
        session
            .receive_frame()
            .map_error(RpcStreamProtocolError::Transport)
            .provide_some(|_: &R| ())
            .catch_all(move |error| close_after(Arc::clone(&failed), error))
            .flat_map(move |first| {
                let protocol = protocol.clone();
                let session = Arc::clone(&session);
                match first {
                    Some(first) => protocol.dispatch_first(session, first),
                    None => close_after(session, RpcStreamProtocolError::Disconnected),
                }
            })
    }

    /// Dispatch a frame already read by an external session acceptor.
    ///
    /// # Type parameters
    /// - `Backend`: Backend-specific error or provider type.
    /// - `Session`: Active stream-session provider type.
    /// - `E`: Domain-error type.
    /// - `R`: Environment type.
    ///
    /// # Parameters
    /// - `session` (`Arc<Session>`): Scoped connection or stream session retained until settlement.
    /// - `first` (`Vec<u8>`): First value or branch in the ordered composition.
    ///
    /// # Returns
    /// Type: `Effect<(), RpcStreamProtocolError<E, Backend>, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    pub fn dispatch_first(
        &self,
        session: Arc<Session>,
        first: Vec<u8>,
    ) -> Effect<(), RpcStreamProtocolError<E, Backend>, R> {
        let key = match decode_header(&first, self.policy) {
            Ok(key) => key,
            Err(error) => return close_after(session, error),
        };
        match self.handlers.get(&key) {
            Some(handler) => handler(session, first),
            None => close_after(
                session,
                RpcStreamProtocolError::UnknownOperation {
                    id: key.0,
                    version: key.1,
                },
            ),
        }
    }
}

///
/// # Type parameters
/// - `E`: Expected application-error type.
/// - `Backend`: Backend-specific error or provider type.
///
/// # Parameters
/// - `first` (`&[u8]`): First value or branch in the ordered composition.
/// - `policy` (`RpcEnvelopePolicy`): The caller-supplied behavior policy.
///
/// # Returns
/// Type: `Result<(OperationId, RpcVersion), RpcStreamProtocolError<E, Backend>>`.
fn decode_header<E, Backend>(
    first: &[u8],
    policy: RpcEnvelopePolicy,
) -> Result<(OperationId, RpcVersion), RpcStreamProtocolError<E, Backend>> {
    let mut decoder = FrameDecoder::new(policy.frame);
    let frames = decoder
        .push(first)
        .map_err(|FrameDecodeFailure { error, .. }| {
            RpcStreamProtocolError::Wire(RpcWireError::Frame(error))
        })?;
    decoder
        .finish()
        .map_err(|error| RpcStreamProtocolError::Wire(RpcWireError::Frame(error)))?;
    let mut frames = frames.into_iter();
    let frame = frames.next().ok_or(RpcStreamProtocolError::MissingFrame)?;
    if frames.next().is_some() {
        return Err(RpcStreamProtocolError::ExtraFrames);
    }
    let envelope = RpcEnvelope::decode(&frame, policy).map_err(RpcStreamProtocolError::Wire)?;
    if envelope.kind != RpcEnvelopeKind::Request {
        return Err(RpcStreamProtocolError::UnexpectedKind(envelope.kind));
    }
    Ok((envelope.operation, envelope.version))
}

///
/// # Type parameters
/// - `T`: Stored or transferred value type. Additional bound: `Send + 'static`.
/// - `E`: Expected application-error type. Additional bound: `Send + 'static`.
/// - `Backend`: Backend-specific error or provider type. Additional bound: `Send + 'static`.
/// - `R`: Environment type supplying required capabilities. Additional bound: `Send + Sync + 'static`.
/// - `Session`: Active stream-session provider type. Additional bound: `RpcStreamSession<Error = Backend>`.
///
/// # Parameters
/// - `session` (`Arc<Session>`): Scoped connection or stream session retained until settlement.
/// - `error` (`RpcStreamProtocolError<E, Backend>`): Expected or provider error preserved in the returned outcome.
///
/// # Returns
/// Type: `Effect<T, RpcStreamProtocolError<E, Backend>, R>`.
/// Returns a lazy `Effect` with the declared success, error, and environment types.
fn close_after<T, E, Backend, R, Session>(
    session: Arc<Session>,
    error: RpcStreamProtocolError<E, Backend>,
) -> Effect<T, RpcStreamProtocolError<E, Backend>, R>
where
    T: Send + 'static,
    E: Send + 'static,
    Backend: Send + 'static,
    R: Send + Sync + 'static,
    Session: RpcStreamSession<Error = Backend>,
{
    let error = Arc::new(Mutex::new(Some(error)));
    let on_close = Arc::clone(&error);
    session
        .close()
        .map_error(move |close| {
            let primary = on_close
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take();
            match primary {
                Some(primary) => RpcStreamProtocolError::CloseAfter {
                    primary: Box::new(primary),
                    close,
                },
                None => RpcStreamProtocolError::InputConsumed,
            }
        })
        .provide_some(|_: &R| ())
        .flat_map(move |_| {
            let error = error
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take()
                .unwrap_or(RpcStreamProtocolError::InputConsumed);
            Effect::from_result_once(Err(error))
        })
}
