use super::{RpcChannel, RpcServerError, RpcUnaryServer, RpcWireError};
use crate::Effect;
use crate::encoding::FrameParts;
use std::sync::Arc;

/// Uses the actual encoded client/server machinery without opening a socket.
/// The caller supplies the server's environment, which may itself hold any
/// runtime, scheduler, tracer or backing capability.
pub struct InProcessRpcChannel<Q, A, E, R>
where
    Q: Send + 'static,
    A: Send + 'static,
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    server: RpcUnaryServer<Q, A, E, R>,
    environment: Arc<R>,
}

impl<Q, A, E, R> InProcessRpcChannel<Q, A, E, R>
where
    Q: Send + 'static,
    A: Send + 'static,
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    #[must_use]
    pub fn new(server: RpcUnaryServer<Q, A, E, R>, environment: Arc<R>) -> Self {
        Self {
            server,
            environment,
        }
    }
}

impl<Q, A, E, R> RpcChannel for InProcessRpcChannel<Q, A, E, R>
where
    Q: Send + 'static,
    A: Send + 'static,
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    type Error = RpcServerError<E>;

    fn exchange(&self, request: FrameParts) -> Effect<Vec<u8>, Self::Error> {
        let bytes = match request.into_bytes() {
            Ok(bytes) => bytes,
            Err(error) => {
                return Effect::from_result_once(Err(RpcServerError::Wire(RpcWireError::Frame(
                    error,
                ))));
            }
        };
        self.server
            .handle(bytes)
            .provide(Arc::clone(&self.environment))
            .flat_map(|response| {
                Effect::from_result_once(
                    response
                        .into_bytes()
                        .map_err(|error| RpcServerError::Wire(RpcWireError::Frame(error))),
                )
            })
    }
}
