//! Service-aware client and handler middleware.

use super::{
    HasRpcChannel, RpcCallId, RpcChannel, RpcClient, RpcClientError, RpcHandler, RpcProduced,
    RpcStreamHandler, RpcStreamProducer,
};
use crate::{Effect, JsonValue, PureCodec};
use std::collections::BTreeMap;
use std::sync::Arc;

#[derive(Debug)]
/// A client transform failure distinguished from transport and domain errors.
///
/// # Type parameters
/// - `DomainError`: Expected domain-error type.
/// - `Backend`: Backend-specific error or provider type.
/// - `MiddlewareError`: Error produced by the middleware layer.
pub enum RpcClientMiddlewareError<DomainError, Backend, MiddlewareError> {
    /// The requested state lies before the retained range. Retains the `MiddlewareError` detail.
    Before(MiddlewareError),
    /// The client could not complete or decode the call. Retains the `RpcClientError<DomainError, Backend>` detail.
    Client(RpcClientError<DomainError, Backend>),
    /// The requested state lies after the current range. Retains the `MiddlewareError` detail.
    After(MiddlewareError),
}

/// Request/response transformations retain all client failure channels. The
/// R parameter statically carries middleware and transport requirements;
/// no hidden executor or global service map is consulted.
///
/// # Type parameters
/// - `Q`: Request type.
/// - `A`: Application value type.
/// - `B`: Transformed value type.
/// - `DomainError`: Expected domain-error type.
/// - `Backend`: Backend-specific error or provider type.
/// - `MiddlewareError`: Error produced by the middleware layer.
/// - `R`: Environment type.
pub struct RpcClientMiddleware<Q, A, B, DomainError, Backend, MiddlewareError, R>
where
    Q: Send + 'static,
    A: Send + 'static,
    B: Send + 'static,
    DomainError: Send + 'static,
    Backend: Send + 'static,
    MiddlewareError: Send + 'static,
    R: Send + Sync + 'static,
{
    client: RpcClient<Q, A, DomainError, Backend>,
    before: Arc<dyn Fn(Q) -> Effect<Q, MiddlewareError, R> + Send + Sync>,
    after: Arc<dyn Fn(A) -> Effect<B, MiddlewareError, R> + Send + Sync>,
}

impl<Q, A, B, DomainError, Backend, MiddlewareError, R> Clone
    for RpcClientMiddleware<Q, A, B, DomainError, Backend, MiddlewareError, R>
where
    Q: Send + 'static,
    A: Send + 'static,
    B: Send + 'static,
    DomainError: Send + 'static,
    Backend: Send + 'static,
    MiddlewareError: Send + 'static,
    R: Send + Sync + 'static,
{
    ///
    /// # Returns
    /// Type: `Self`.
    fn clone(&self) -> Self {
        Self {
            client: self.client.clone(),
            before: Arc::clone(&self.before),
            after: Arc::clone(&self.after),
        }
    }
}

impl<Q, A, B, DomainError, Backend, MiddlewareError, R>
    RpcClientMiddleware<Q, A, B, DomainError, Backend, MiddlewareError, R>
where
    Q: Send + 'static,
    A: Send + 'static,
    B: Send + 'static,
    DomainError: Send + 'static,
    Backend: Send + 'static,
    MiddlewareError: Send + 'static,
    R: HasRpcChannel + Send + Sync + 'static,
    R::Channel: RpcChannel<Error = Backend>,
{
    #[must_use]
    /// Construct a `RpcClientMiddleware` from its required inputs and explicit policies.
    ///
    /// # Type parameters
    /// - `DomainError`: Expected domain-error type.
    /// - `Backend`: Backend-specific error or provider type.
    /// - `MiddlewareError`: Error produced by the middleware layer.
    /// - `A`: Success or application-value type.
    /// - `B`: Transformed output type.
    /// - `Q`: Request type.
    /// - `R`: Environment type.
    ///
    /// # Parameters
    /// - `client` (`RpcClient<Q, A, DomainError, Backend>`): Client capability used to submit or observe the remote call.
    /// - `before` (`impl Fn(Q) -> Effect<Q, MiddlewareError, R> + Send + Sync + 'static`): The before supplied to this operation.
    /// - `after` (`impl Fn(A) -> Effect<B, MiddlewareError, R> + Send + Sync + 'static`): Cursor or instant after which results are selected.
    ///
    /// # Returns
    /// Type: `Self`.
    pub fn new(
        client: RpcClient<Q, A, DomainError, Backend>,
        before: impl Fn(Q) -> Effect<Q, MiddlewareError, R> + Send + Sync + 'static,
        after: impl Fn(A) -> Effect<B, MiddlewareError, R> + Send + Sync + 'static,
    ) -> Self {
        Self {
            client,
            before: Arc::new(before),
            after: Arc::new(after),
        }
    }

    /// Invoke the typed operation through its selected channel or handler.
    ///
    /// # Type parameters
    /// - `A`: Application value type.
    /// - `DomainError`: Expected domain-error type.
    /// - `Backend`: Backend-specific error or provider type.
    /// - `MiddlewareError`: Error produced by the middleware layer.
    /// - `B`: Transformed output type.
    /// - `Q`: Request type.
    /// - `R`: Environment type.
    ///
    /// # Parameters
    /// - `request` (`Q`): The typed request to process.
    /// - `call` (`RpcCallId`): Identity and contents of the selected RPC call.
    /// - `metadata` (`BTreeMap<Arc<str>, Arc<str>>`): Version, identity, or protection data associated with the payload.
    ///
    /// # Returns
    /// Type: `Effect<B, RpcClientMiddlewareError<DomainError, Backend, MiddlewareError>, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    pub fn call(
        &self,
        request: Q,
        call: RpcCallId,
        metadata: BTreeMap<Arc<str>, Arc<str>>,
    ) -> Effect<B, RpcClientMiddlewareError<DomainError, Backend, MiddlewareError>, R> {
        let client = self.clone();
        self.call_raw(request, call, metadata)
            .flat_map(move |response| {
                client
                    .apply_after(response)
                    .map_error(RpcClientMiddlewareError::After)
            })
    }

    /// Return only after the remote call has produced a successful typed
    /// response. A mutation bridge must publish local invalidation at this
    /// boundary, even if the subsequent response transform fails.
    ///
    /// # Type parameters
    /// - `B`: Transformed value type.
    /// - `DomainError`: Expected domain-error type.
    /// - `Backend`: Backend-specific error or provider type.
    /// - `MiddlewareError`: Error produced by the middleware layer.
    /// - `A`: Success or application-value type.
    /// - `Q`: Request type.
    /// - `R`: Environment type.
    ///
    /// # Parameters
    /// - `request` (`Q`): The typed request to process.
    /// - `call` (`RpcCallId`): Identity and contents of the selected RPC call.
    /// - `metadata` (`BTreeMap<Arc<str>, Arc<str>>`): Version, identity, or protection data associated with the payload.
    ///
    /// # Returns
    /// Type: `Effect<A, RpcClientMiddlewareError<DomainError, Backend, MiddlewareError>, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    pub(crate) fn call_raw(
        &self,
        request: Q,
        call: RpcCallId,
        metadata: BTreeMap<Arc<str>, Arc<str>>,
    ) -> Effect<A, RpcClientMiddlewareError<DomainError, Backend, MiddlewareError>, R> {
        let client = self.client.clone();
        (self.before)(request)
            .map_error(RpcClientMiddlewareError::Before)
            .flat_map(move |request| {
                client
                    .call(request, call.clone(), metadata.clone())
                    .map_error(RpcClientMiddlewareError::Client)
            })
    }

    /// # Type parameters
    /// - `Q`: Request type.
    /// - `DomainError`: Expected domain-error type.
    /// - `Backend`: Backend-specific error or provider type.
    /// - `MiddlewareError`: Error produced by the middleware layer.
    /// - `A`: Success or application-value type.
    /// - `B`: Transformed output type.
    /// - `R`: Environment type.
    ///
    /// # Parameters
    /// - `response` (`A`): The response supplied to this operation.
    ///
    /// # Returns
    /// Type: `Effect<B, MiddlewareError, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    pub(crate) fn apply_after(&self, response: A) -> Effect<B, MiddlewareError, R> {
        (self.after)(response)
    }
}

#[derive(Debug)]
/// A server middleware failure before or after typed handler execution.
///
/// # Type parameters
/// - `DomainError`: Expected domain-error type.
/// - `MiddlewareError`: Error produced by the middleware layer.
pub enum RpcServerMiddlewareError<DomainError, MiddlewareError> {
    /// The requested state lies before the retained range. Retains the `MiddlewareError` detail.
    Before(MiddlewareError),
    /// The application’s expected domain operation failed. Retains the `DomainError` detail.
    Domain(DomainError),
    /// The requested state lies after the current range. Retains the `MiddlewareError` detail.
    After(MiddlewareError),
}

/// A composable wrapper around typed RPC server dispatch.
pub struct RpcServerMiddleware;

impl RpcServerMiddleware {
    /// The caller supplies the codec for the combined error sum. The wrapped
    /// operation keeps the original request constructor and idempotency key.
    ///
    /// # Type parameters
    /// - `Q`: Request type. Additional bound: `Send + 'static`.
    /// - `A`: Application value type. Additional bound: `Send + 'static`.
    /// - `DomainError`: Expected domain-error type. Additional bound: `Send + 'static`.
    /// - `MiddlewareError`: Error produced by the middleware layer. Additional bound: `Send + 'static`.
    /// - `R`: Environment type. Additional bound: `Send + Sync + 'static`.
    ///
    /// # Parameters
    /// - `handler` (`RpcHandler<Q, A, DomainError, R>`): The handler invoked for the matching request or event.
    /// - `error_codec` (`PureCodec<RpcServerMiddlewareError<DomainError, MiddlewareError>, JsonValue>`): The error codec supplied to this operation.
    /// - `before` (`impl Fn(Q) -> Effect<Q, MiddlewareError, R> + Send + Sync + 'static`): The before supplied to this operation.
    /// - `after` (`impl Fn(A) -> Effect<A, MiddlewareError, R> + Send + Sync + 'static`): Cursor or instant after which results are selected.
    ///
    /// # Returns
    /// Type: `RpcHandler<Q, A, RpcServerMiddlewareError<DomainError, MiddlewareError>, R>`.
    pub fn wrap<Q, A, DomainError, MiddlewareError, R>(
        handler: RpcHandler<Q, A, DomainError, R>,
        error_codec: PureCodec<RpcServerMiddlewareError<DomainError, MiddlewareError>, JsonValue>,
        before: impl Fn(Q) -> Effect<Q, MiddlewareError, R> + Send + Sync + 'static,
        after: impl Fn(A) -> Effect<A, MiddlewareError, R> + Send + Sync + 'static,
    ) -> RpcHandler<Q, A, RpcServerMiddlewareError<DomainError, MiddlewareError>, R>
    where
        Q: Send + 'static,
        A: Send + 'static,
        DomainError: Send + 'static,
        MiddlewareError: Send + 'static,
        R: Send + Sync + 'static,
    {
        let operation = handler.operation().with_error_codec(error_codec);
        let handler = Arc::new(handler);
        let before = Arc::new(before);
        let after = Arc::new(after);
        RpcHandler::new(operation, move |request| {
            let handler = Arc::clone(&handler);
            let after = Arc::clone(&after);
            before(request)
                .map_error(RpcServerMiddlewareError::Before)
                .flat_map(move |request| {
                    handler
                        .call(request)
                        .map_error(RpcServerMiddlewareError::Domain)
                })
                .flat_map(move |response| {
                    after(response).map_error(RpcServerMiddlewareError::After)
                })
        })
    }
}

#[derive(Debug)]
/// A middleware failure while establishing or consuming a streaming call.
///
/// # Type parameters
/// - `DomainError`: Expected domain-error type.
/// - `MiddlewareError`: Error produced by the middleware layer.
pub enum RpcStreamMiddlewareError<DomainError, MiddlewareError> {
    /// The requested state lies before the retained range. Retains the `MiddlewareError` detail.
    Before(MiddlewareError),
    /// The application’s expected domain operation failed. Retains the `DomainError` detail.
    Domain(DomainError),
    /// Producing or consuming a stream chunk failed. Retains the `MiddlewareError` detail.
    Chunk(MiddlewareError),
    /// The stream terminated before the expected result was available. Retains the `MiddlewareError` detail.
    Terminal(MiddlewareError),
}

/// A composable wrapper around typed RPC streaming dispatch.
pub struct RpcStreamMiddleware;

impl RpcStreamMiddleware {
    /// Each stream pull remains typed and lazy. Middleware runs once before
    /// handler creation and separately for every emitted chunk or terminal.
    ///
    /// # Type parameters
    /// - `Q`: Request type. Additional bound: `Send + 'static`.
    /// - `C`: Stream chunk or item type. Additional bound: `Send + 'static`.
    /// - `T`: Value or item type. Additional bound: `Send + 'static`.
    /// - `DomainError`: Expected domain-error type. Additional bound: `Send + 'static`.
    /// - `MiddlewareError`: Error produced by the middleware layer. Additional bound: `Send + 'static`.
    /// - `R`: Environment type. Additional bound: `Send + Sync + 'static`.
    ///
    /// # Parameters
    /// - `handler` (`RpcStreamHandler<Q, C, T, DomainError, R>`): The handler invoked for the matching request or event.
    /// - `error_codec` (`PureCodec<RpcStreamMiddlewareError<DomainError, MiddlewareError>, JsonValue>`): The error codec supplied to this operation.
    /// - `before` (`impl Fn(Q) -> Effect<Q, MiddlewareError, R> + Send + Sync + 'static`): The before supplied to this operation.
    /// - `chunk` (`impl Fn(C) -> Effect<C, MiddlewareError, R> + Send + Sync + 'static`): The chunk supplied to this operation.
    /// - `terminal` (`impl Fn(T) -> Effect<T, MiddlewareError, R> + Send + Sync + 'static`): The terminal supplied to this operation.
    ///
    /// # Returns
    /// Type: `RpcStreamHandler<Q, C, T, RpcStreamMiddlewareError<DomainError, MiddlewareError>, R>`.
    pub fn wrap<Q, C, T, DomainError, MiddlewareError, R>(
        handler: RpcStreamHandler<Q, C, T, DomainError, R>,
        error_codec: PureCodec<RpcStreamMiddlewareError<DomainError, MiddlewareError>, JsonValue>,
        before: impl Fn(Q) -> Effect<Q, MiddlewareError, R> + Send + Sync + 'static,
        chunk: impl Fn(C) -> Effect<C, MiddlewareError, R> + Send + Sync + 'static,
        terminal: impl Fn(T) -> Effect<T, MiddlewareError, R> + Send + Sync + 'static,
    ) -> RpcStreamHandler<Q, C, T, RpcStreamMiddlewareError<DomainError, MiddlewareError>, R>
    where
        Q: Send + 'static,
        C: Send + 'static,
        T: Send + 'static,
        DomainError: Send + 'static,
        MiddlewareError: Send + 'static,
        R: Send + Sync + 'static,
    {
        let operation = handler.operation().with_error_codec(error_codec);
        let handler = Arc::new(handler);
        let before = Arc::new(before);
        let chunk = Arc::new(chunk);
        let terminal = Arc::new(terminal);
        RpcStreamHandler::new(operation, move |request, context| {
            let handler = Arc::clone(&handler);
            let chunk = Arc::clone(&chunk);
            let terminal = Arc::clone(&terminal);
            before(request)
                .map_error(RpcStreamMiddlewareError::Before)
                .flat_map(move |request| {
                    handler
                        .call(request, context.clone())
                        .map_error(RpcStreamMiddlewareError::Domain)
                })
                .map(move |producer| {
                    let chunk = Arc::clone(&chunk);
                    let terminal = Arc::clone(&terminal);
                    RpcStreamProducer::new(move || {
                        let chunk = Arc::clone(&chunk);
                        let terminal = Arc::clone(&terminal);
                        producer
                            .pull()
                            .map_error(RpcStreamMiddlewareError::Domain)
                            .flat_map(move |produced| match produced {
                                RpcProduced::Chunk(value) => chunk(value)
                                    .map(RpcProduced::Chunk)
                                    .map_error(RpcStreamMiddlewareError::Chunk),
                                RpcProduced::Terminal(value) => terminal(value)
                                    .map(RpcProduced::Terminal)
                                    .map_error(RpcStreamMiddlewareError::Terminal),
                            })
                    })
                })
        })
    }
}
