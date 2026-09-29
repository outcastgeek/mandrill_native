//! Typed in-process RPC invocation.

use super::{RpcChannel, RpcServerError, RpcUnaryServer, RpcWireError};
use crate::Effect;
use crate::encoding::FrameParts;
use std::sync::Arc;

/// Uses the actual encoded client/server machinery without opening a socket.
/// The caller supplies the server's environment, which may itself hold any
/// runtime, scheduler, tracer or backing capability.
///
/// # Type parameters
/// - `Q`: Request type.
/// - `A`: Application value type.
/// - `E`: Domain-error type.
/// - `R`: Environment type.
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
    /// Construct a `InProcessRpcChannel` from its required inputs and explicit policies.
    ///
    /// # Type parameters
    /// - `A`: Success or application-value type.
    /// - `E`: Domain-error type.
    /// - `Q`: Request type.
    /// - `R`: Environment type.
    ///
    /// # Parameters
    /// - `server` (`RpcUnaryServer<Q, A, E, R>`): The server supplied to this operation.
    /// - `environment` (`Arc<R>`): The environment supplying this operation's capabilities.
    ///
    /// # Returns
    /// Type: `Self`.
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
    /// Type alias for error in rpc in process.
    type Error = RpcServerError<E>;

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
