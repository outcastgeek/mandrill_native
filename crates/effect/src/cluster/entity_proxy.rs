//! Derivation of entity-addressed RPC operations.

use super::{
    AcceptancePolicy, ClusterAddress, ClusterClient, ClusterClientError, ClusterMessageStore,
    ClusterObserverId, ClusterPoll, ClusterRequestId, HasClusterMessageStore, MessageAcceptance,
    StoreMutation,
};
use crate::rpc::RpcCallId;
use crate::{Effect, UtcMillis};
use std::sync::Arc;

/// An address-bound typed client. The application supplies stable request and
/// observer identities, the acceptance policy and clock value for each call;
/// wrapping a client never changes its saved-message authority.
///
/// # Type parameters
/// - `Q`: Request type.
/// - `A`: Application value type.
/// - `E`: Domain-error type.
/// - `Backend`: Backend-specific error or provider type.
pub struct EntityProxy<Q, A, E, Backend>
where
    Q: Send + 'static,
    A: Send + 'static,
    E: Send + 'static,
    Backend: Send + 'static,
{
    address: ClusterAddress,
    client: Arc<ClusterClient<Q, A, E, Backend>>,
}

impl<Q, A, E, Backend> EntityProxy<Q, A, E, Backend>
where
    Q: Send + 'static,
    A: Send + 'static,
    E: Send + 'static,
    Backend: Send + 'static,
{
    #[must_use]
    /// Construct a `EntityProxy` from its required inputs and explicit policies.
    ///
    /// # Type parameters
    /// - `Backend`: Backend-specific error or provider type.
    /// - `A`: Success or application-value type.
    /// - `E`: Domain-error type.
    /// - `Q`: Request type.
    ///
    /// # Parameters
    /// - `address` (`ClusterAddress`): Stable destination address of the entity receiving this request.
    /// - `client` (`Arc<ClusterClient<Q, A, E, Backend>>`): Client capability used to submit or observe the remote call.
    ///
    /// # Returns
    /// Type: `Self`.
    pub fn new(address: ClusterAddress, client: Arc<ClusterClient<Q, A, E, Backend>>) -> Self {
        Self { address, client }
    }

    #[must_use]
    /// Return the stable entity address targeted by this proxy.
    ///
    /// # Type parameters
    /// - `Q`: Request type.
    /// - `A`: Application value type.
    /// - `E`: Domain-error type.
    /// - `Backend`: Backend-specific error or provider type.
    ///
    /// # Returns
    /// Type: `&ClusterAddress`.
    pub fn address(&self) -> &ClusterAddress {
        &self.address
    }

    /// Transfer accepted work to the selected provider for independent execution.
    ///
    /// # Type parameters
    /// - `A`: Application value type.
    /// - `E`: Domain-error type.
    /// - `Backend`: Backend-specific error or provider type.
    /// - `R`: Environment type. Additional bound: `HasClusterMessageStore + Send + Sync + 'static`. Additional bound: `:Store: ClusterMessageStore<Error = Backend>`.
    /// - `Q`: Request type.
    ///
    /// # Parameters
    /// - `request_id` (`ClusterRequestId`): The request id supplied to this operation.
    /// - `request` (`Q`): The typed request to process.
    /// - `call` (`RpcCallId`): Identity and contents of the selected RPC call.
    /// - `accepted_at` (`UtcMillis`): The accepted at supplied to this operation.
    /// - `acceptance` (`AcceptancePolicy`): The acceptance supplied to this operation.
    ///
    /// # Returns
    /// Type: `Effect<MessageAcceptance, ClusterClientError<Backend>, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    pub fn submit<R>(
        &self,
        request_id: ClusterRequestId,
        request: Q,
        call: RpcCallId,
        accepted_at: UtcMillis,
        acceptance: AcceptancePolicy,
    ) -> Effect<MessageAcceptance, ClusterClientError<Backend>, R>
    where
        R: HasClusterMessageStore + Send + Sync + 'static,
        R::Store: ClusterMessageStore<Error = Backend>,
    {
        self.client.submit(
            self.address.clone(),
            request_id,
            request,
            call,
            accepted_at,
            acceptance,
        )
    }

    /// Inspect the current state without waiting for a later change.
    ///
    /// # Type parameters
    /// - `Q`: Request type.
    /// - `Backend`: Backend-specific error or provider type.
    /// - `R`: Environment type. Additional bound: `HasClusterMessageStore + Send + Sync + 'static`. Additional bound: `:Store: ClusterMessageStore<Error = Backend>`.
    /// - `A`: Success or application-value type.
    /// - `E`: Domain-error type.
    ///
    /// # Parameters
    /// - `request` (`ClusterRequestId`): The typed request to process.
    /// - `observer` (`ClusterObserverId`): Stable identity of the client observing recorded results.
    /// - `call` (`RpcCallId`): Identity and contents of the selected RPC call.
    ///
    /// # Returns
    /// Type: `Effect<ClusterPoll<A, E>, ClusterClientError<Backend>, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    pub fn poll<R>(
        &self,
        request: ClusterRequestId,
        observer: ClusterObserverId,
        call: RpcCallId,
    ) -> Effect<ClusterPoll<A, E>, ClusterClientError<Backend>, R>
    where
        R: HasClusterMessageStore + Send + Sync + 'static,
        R::Store: ClusterMessageStore<Error = Backend>,
    {
        self.client
            .poll(self.address.clone(), request, observer, call)
    }

    /// Record that the selected observer has consumed the reply.
    ///
    /// # Type parameters
    /// - `Q`: Request type.
    /// - `A`: Application value type.
    /// - `E`: Domain-error type.
    /// - `Backend`: Backend-specific error or provider type.
    /// - `R`: Environment type. Additional bound: `HasClusterMessageStore + Send + Sync + 'static`. Additional bound: `:Store: ClusterMessageStore<Error = Backend>`.
    ///
    /// # Parameters
    /// - `request` (`ClusterRequestId`): The typed request to process.
    /// - `observer` (`ClusterObserverId`): Stable identity of the client observing recorded results.
    ///
    /// # Returns
    /// Type: `Effect<StoreMutation, ClusterClientError<Backend>, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    pub fn acknowledge_reply<R>(
        &self,
        request: ClusterRequestId,
        observer: ClusterObserverId,
    ) -> Effect<StoreMutation, ClusterClientError<Backend>, R>
    where
        R: HasClusterMessageStore + Send + Sync + 'static,
        R::Store: ClusterMessageStore<Error = Backend>,
    {
        self.client
            .acknowledge_reply(self.address.clone(), request, observer)
    }

    /// Remove an observer without discarding authoritative work.
    ///
    /// # Type parameters
    /// - `Q`: Request type.
    /// - `A`: Application value type.
    /// - `E`: Domain-error type.
    /// - `Backend`: Backend-specific error or provider type.
    /// - `R`: Environment type. Additional bound: `HasClusterMessageStore + Send + Sync + 'static`. Additional bound: `:Store: ClusterMessageStore<Error = Backend>`.
    ///
    /// # Parameters
    /// - `request` (`ClusterRequestId`): The typed request to process.
    /// - `observer` (`ClusterObserverId`): Stable identity of the client observing recorded results.
    ///
    /// # Returns
    /// Type: `Effect<bool, ClusterClientError<Backend>, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    pub fn detach_observer<R>(
        &self,
        request: ClusterRequestId,
        observer: ClusterObserverId,
    ) -> Effect<bool, ClusterClientError<Backend>, R>
    where
        R: HasClusterMessageStore + Send + Sync + 'static,
        R::Store: ClusterMessageStore<Error = Backend>,
    {
        self.client
            .detach_observer(self.address.clone(), request, observer)
    }
}
