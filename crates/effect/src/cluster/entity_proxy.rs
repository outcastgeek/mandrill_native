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
    pub fn new(address: ClusterAddress, client: Arc<ClusterClient<Q, A, E, Backend>>) -> Self {
        Self { address, client }
    }

    #[must_use]
    pub fn address(&self) -> &ClusterAddress {
        &self.address
    }

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
