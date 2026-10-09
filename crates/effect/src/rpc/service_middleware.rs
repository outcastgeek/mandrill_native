//! Service-producing middleware whose acquisition scope encloses handler or serving execution.

use crate::internal::scoped_body::ScopedBody;
use crate::{Effect, Exit, Scope};
use std::future::Future;
use std::sync::Arc;

/// Failure at the service acquisition boundary or in the connected handler.
/// Defects, interruption and cleanup retain the enclosing Effect's separate channels.
#[derive(Debug)]
pub enum RpcServiceMiddlewareError<MiddlewareError, HandlerError> {
    /// Acquisition or a service-resource finalizer failed.
    Acquire(MiddlewareError),
    /// The connected handler or complete streaming serving operation failed.
    Handler(HandlerError),
}
/// A lazy service-producing description. Its primary environment R supplies both acquisition
/// and execution capabilities; `apply` explicitly connects the service to the handler environment.
/// Payloads are moved, and the service is shared only for environment projection.
///
/// ```
/// use effect::{Effect, rpc_middleware};
/// use effect::rpc::RpcServiceMiddleware;
/// let middleware: RpcServiceMiddleware<i32, (), ()> = rpc_middleware! {
///     acquire { Effect::sync(|| 7)? },
/// };
/// let program = middleware.apply(
///     |service, _environment| *service,
///     |input: i32| Effect::<i32, (), i32>::service(move |service| input + service),
///     3,
/// );
/// # let _ = program;
/// ```
pub struct RpcServiceMiddleware<
    S: Send + Sync + 'static,
    E: Send + 'static,
    R: Send + Sync + 'static,
> {
    /// Acquisition executed under a scope which survives until the corresponding handler finishes.
    acquire: ScopedBody<S, E, R>,
}
impl<S: Send + Sync + 'static, E: Send + 'static, R: Send + Sync + 'static> Clone
    for RpcServiceMiddleware<S, E, R>
{
    /// Share the acquisition description without copying a service payload.
    fn clone(&self) -> Self {
        Self {
            acquire: self.acquire.clone(),
        }
    }
}
impl<S: Send + Sync + 'static, E: Send + 'static, R: Send + Sync + 'static>
    RpcServiceMiddleware<S, E, R>
{
    /// Reuse a typed acquisition Effect. Resources bound inside that Effect retain its own
    /// lexical lifetime; use the macro's inline acquire body for handler-owned resource binds.
    #[must_use]
    pub fn new(acquire: Effect<S, E, R>) -> Self {
        Self {
            acquire: ScopedBody::from_effect(acquire),
        }
    }
    /// Generated inline acquisition with a scope owned through the handler's complete execution.
    #[doc(hidden)]
    pub fn from_body<F>(
        factory: impl Fn(crate::__private::MacroContext<E, R>) -> F + Send + Sync + 'static,
    ) -> Self
    where
        F: Future<Output = Result<S, Exit<S, E>>> + Send + 'static,
    {
        Self {
            acquire: ScopedBody::from_body(factory),
        }
    }
    /// Acquire once, connect to a typed handler environment, then execute and protect cleanup.
    ///
    /// `handler` returns an Effect for its complete operation. For streaming RPC, pass a
    /// callback executing the existing server/protocol `serve` Effect so the acquisition
    /// surrounds the full serving lifetime rather than only producer construction.
    pub fn apply<I, A, H, L>(
        &self,
        connect: impl Fn(Arc<S>, &R) -> L + Send + Sync + 'static,
        handler: impl Fn(I) -> Effect<A, H, L> + Send + Sync + 'static,
        input: I,
    ) -> Effect<A, RpcServiceMiddlewareError<E, H>, R>
    where
        I: Send + 'static,
        A: Send + 'static,
        H: Send + 'static,
        L: Send + Sync + 'static,
    {
        let acquire = self.acquire.clone();
        let connect = Arc::new(connect);
        let handler = Arc::new(handler);
        // The input is one-shot, but the reusable description never requires its Clone.
        let input = Effect::sync_once(move || input);
        Effect::defer(move || {
            let scope = Scope::new();
            let connect = Arc::clone(&connect);
            let handler = Arc::clone(&handler);
            let input = input.clone();
            let program = acquire
                .evaluate(scope.clone())
                .map_error(RpcServiceMiddlewareError::Acquire)
                .flat_map(move |service| {
                    let service = Arc::new(service);
                    let connect = Arc::clone(&connect);
                    let handler = Arc::clone(&handler);
                    input.clone().flat_map(move |input| {
                        let service = Arc::clone(&service);
                        let connect = Arc::clone(&connect);
                        handler(input)
                            .map_error(RpcServiceMiddlewareError::Handler)
                            .provide_some(move |environment| {
                                connect(Arc::clone(&service), environment)
                            })
                    })
                });
            scope.own_mapped(program, RpcServiceMiddlewareError::Acquire)
        })
    }
}
/// Explicit grouping of existing protocols, preserving all streaming backend/session types.
/// Each registry retains its own conflict validation and handle/serve execution API.
pub struct RpcProtocolGroup<Unary, Streaming> {
    /// Unary registry with its existing typed handling operation.
    pub unary: Unary,
    /// Streaming registry with its existing typed serving operation.
    pub streaming: Streaming,
}

impl<S: Send + Sync + 'static, E: Send + 'static, R: Send + Sync + 'static>
    RpcServiceMiddleware<S, E, R>
{
    /// Wrap an existing unary handler with service acquisition and an explicit combined error
    /// codec. The operation retains request/success codecs, identity and idempotency selector;
    /// acquisition and handler failures use their distinct typed variants on the wire.
    pub fn wrap_handler<Q, A, H, L>(
        &self,
        connect: impl Fn(Arc<S>, &R) -> L + Send + Sync + 'static,
        handler: super::RpcHandler<Q, A, H, L>,
        error_codec: crate::PureCodec<RpcServiceMiddlewareError<E, H>, crate::JsonValue>,
    ) -> super::RpcHandler<Q, A, RpcServiceMiddlewareError<E, H>, R>
    where
        Q: Send + 'static,
        A: Send + 'static,
        H: Send + 'static,
        L: Send + Sync + 'static,
    {
        let operation = handler.operation().with_error_codec(error_codec);
        let handler = Arc::new(handler);
        let middleware = self.clone();
        let connect = Arc::new(connect);
        super::RpcHandler::new(operation, move |input| {
            let connect = Arc::clone(&connect);
            let handler = Arc::clone(&handler);
            middleware.apply(
                move |service, environment| connect(service, environment),
                move |input| handler.call(input),
                input,
            )
        })
    }

    /// Apply service acquisition around an existing streaming handler's complete serve Effect.
    /// The caller supplies the accepted session and protocol policy; the original server owns
    /// acknowledgement/close while the middleware owner retains its resources until serving
    /// and session cleanup finish. Streaming backend errors remain inside the typed handler variant.
    #[allow(
        clippy::type_complexity,
        reason = "Retains separate acquisition, handler, and transport provider error channels"
    )]
    pub fn serve_stream<Q, C, T, H, L, Session>(
        &self,
        connect: impl Fn(Arc<S>, &R) -> L + Send + Sync + 'static,
        handler: super::RpcStreamHandler<Q, C, T, H, L>,
        policy: super::RpcEnvelopePolicy,
        accepted: Arc<Session>,
    ) -> Effect<(), RpcServiceMiddlewareError<E, super::RpcStreamServerError<H, Session::Error>>, R>
    where
        Q: Send + 'static,
        C: Send + 'static,
        T: Send + 'static,
        H: Send + 'static,
        L: Send + Sync + 'static,
        Session: super::RpcStreamSession,
    {
        let server = Arc::new(super::RpcStreamServer::new(handler, policy));
        self.apply(connect, move |accepted| server.serve(accepted), accepted)
    }
}
