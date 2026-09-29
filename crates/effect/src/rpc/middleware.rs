use super::{
    HasRpcChannel, RpcCallId, RpcChannel, RpcClient, RpcClientError, RpcHandler, RpcProduced,
    RpcStreamHandler, RpcStreamProducer,
};
use crate::{Effect, JsonValue, PureCodec};
use std::collections::BTreeMap;
use std::sync::Arc;

#[derive(Debug)]
pub enum RpcClientMiddlewareError<DomainError, Backend, MiddlewareError> {
    Before(MiddlewareError),
    Client(RpcClientError<DomainError, Backend>),
    After(MiddlewareError),
}

/// Request/response transformations retain all client failure channels. The
/// R parameter statically carries middleware and transport requirements;
/// no hidden executor or global service map is consulted.
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

    pub(crate) fn apply_after(&self, response: A) -> Effect<B, MiddlewareError, R> {
        (self.after)(response)
    }
}

#[derive(Debug)]
pub enum RpcServerMiddlewareError<DomainError, MiddlewareError> {
    Before(MiddlewareError),
    Domain(DomainError),
    After(MiddlewareError),
}

pub struct RpcServerMiddleware;

impl RpcServerMiddleware {
    /// The caller supplies the codec for the combined error sum. The wrapped
    /// operation keeps the original request constructor and idempotency key.
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
pub enum RpcStreamMiddlewareError<DomainError, MiddlewareError> {
    Before(MiddlewareError),
    Domain(DomainError),
    Chunk(MiddlewareError),
    Terminal(MiddlewareError),
}

pub struct RpcStreamMiddleware;

impl RpcStreamMiddleware {
    /// Each stream pull remains typed and lazy. Middleware runs once before
    /// handler creation and separately for every emitted chunk or terminal.
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
