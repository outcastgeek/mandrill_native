use super::{InvalidationError, InvalidationKey, InvalidationRegistry, ReactiveQuery};
use crate::rpc::{
    HasRpcChannel, RpcCallId, RpcChannel, RpcClient, RpcClientError, RpcClientMiddleware,
    RpcClientMiddlewareError,
};
use crate::{Effect, Stream};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

/// The observable error is the complete typed RPC client error. Domain,
/// request/response schema, protocol, admission and transport failures do not
/// collapse into a single missing-value state.
pub struct RpcQueryAtom<Q, A, E, Backend, R>
where
    Q: Send + 'static,
    A: Send + 'static,
    E: Send + 'static,
    Backend: Send + 'static,
    R: HasRpcChannel + Send + Sync + 'static,
    R::Channel: RpcChannel<Error = Backend>,
{
    query: ReactiveQuery<A, RpcClientError<E, Backend>, R>,
    _request: std::marker::PhantomData<fn() -> Q>,
}

impl<Q, A, E, Backend, R> RpcQueryAtom<Q, A, E, Backend, R>
where
    Q: Send + 'static,
    A: Send + 'static,
    E: Send + 'static,
    Backend: Send + 'static,
    R: HasRpcChannel + Send + Sync + 'static,
    R::Channel: RpcChannel<Error = Backend>,
{
    #[must_use]
    pub fn new(
        registry: InvalidationRegistry,
        dependencies: impl IntoIterator<Item = InvalidationKey>,
        client: RpcClient<Q, A, E, Backend>,
        request: impl Fn() -> Q + Send + Sync + 'static,
        call: impl Fn() -> RpcCallId + Send + Sync + 'static,
        metadata: impl Fn() -> BTreeMap<Arc<str>, Arc<str>> + Send + Sync + 'static,
    ) -> Self {
        let request = Arc::new(request);
        let call = Arc::new(call);
        let metadata = Arc::new(metadata);
        let query = ReactiveQuery::new(registry, dependencies, move || {
            client.call(request(), call(), metadata())
        });
        Self {
            query,
            _request: std::marker::PhantomData,
        }
    }

    #[must_use]
    pub fn stream(
        &self,
    ) -> Stream<super::AsyncResult<A, RpcClientError<E, Backend>>, super::ReactiveQueryError, R>
    {
        self.query.stream()
    }

    #[must_use]
    pub fn query(&self) -> &ReactiveQuery<A, RpcClientError<E, Backend>, R> {
        &self.query
    }
}

/// Middleware contributes its own typed failure channel to the observable
/// query. The reactive layer never converts that failure to a missing value.
pub struct RpcMiddlewareQueryAtom<A, DomainError, Backend, MiddlewareError, R>
where
    A: Send + 'static,
    DomainError: Send + 'static,
    Backend: Send + 'static,
    MiddlewareError: Send + 'static,
    R: Send + Sync + 'static,
{
    query: ReactiveQuery<A, RpcClientMiddlewareError<DomainError, Backend, MiddlewareError>, R>,
}

impl<A, DomainError, Backend, MiddlewareError, R>
    RpcMiddlewareQueryAtom<A, DomainError, Backend, MiddlewareError, R>
where
    A: Send + 'static,
    DomainError: Send + 'static,
    Backend: Send + 'static,
    MiddlewareError: Send + 'static,
    R: Send + Sync + 'static,
{
    #[must_use]
    pub fn from_call(
        registry: InvalidationRegistry,
        dependencies: impl IntoIterator<Item = InvalidationKey>,
        call: impl Fn() -> Effect<A, RpcClientMiddlewareError<DomainError, Backend, MiddlewareError>, R>
        + Send
        + Sync
        + 'static,
    ) -> Self {
        Self {
            query: ReactiveQuery::new(registry, dependencies, call),
        }
    }

    #[must_use]
    pub fn stream(
        &self,
    ) -> Stream<
        super::AsyncResult<A, RpcClientMiddlewareError<DomainError, Backend, MiddlewareError>>,
        super::ReactiveQueryError,
        R,
    > {
        self.query.stream()
    }

    #[must_use]
    pub fn query(
        &self,
    ) -> &ReactiveQuery<A, RpcClientMiddlewareError<DomainError, Backend, MiddlewareError>, R> {
        &self.query
    }
}

impl<A, DomainError, Backend, MiddlewareError, R>
    RpcMiddlewareQueryAtom<A, DomainError, Backend, MiddlewareError, R>
where
    A: Send + 'static,
    DomainError: Send + 'static,
    Backend: Send + 'static,
    MiddlewareError: Send + 'static,
    R: HasRpcChannel + Send + Sync + 'static,
    R::Channel: RpcChannel<Error = Backend>,
{
    #[must_use]
    pub fn new<Q: Send + 'static, Raw: Send + 'static>(
        registry: InvalidationRegistry,
        dependencies: impl IntoIterator<Item = InvalidationKey>,
        client: RpcClientMiddleware<Q, Raw, A, DomainError, Backend, MiddlewareError, R>,
        request: impl Fn() -> Q + Send + Sync + 'static,
        call: impl Fn() -> RpcCallId + Send + Sync + 'static,
        metadata: impl Fn() -> BTreeMap<Arc<str>, Arc<str>> + Send + Sync + 'static,
    ) -> Self {
        let request = Arc::new(request);
        let call = Arc::new(call);
        let metadata = Arc::new(metadata);
        Self::from_call(registry, dependencies, move || {
            client.call(request(), call(), metadata())
        })
    }
}

#[derive(Debug)]
pub enum RpcMutationError<E, Backend> {
    Client(RpcClientError<E, Backend>),
    /// The server replied successfully, but local invalidation failed. The
    /// authoritative remote mutation must not be retried blindly.
    CommittedWithoutInvalidation(InvalidationError),
    InputConsumed,
}

/// A mutation invalidates dependent local queries only after a successful
/// typed reply. Local invalidation is not a distributed durability record.
pub struct RpcMutationAtom<Q, A, E, Backend>
where
    Q: Send + 'static,
    A: Send + 'static,
    E: Send + 'static,
    Backend: Send + 'static,
{
    client: RpcClient<Q, A, E, Backend>,
    registry: InvalidationRegistry,
    affected: Arc<[InvalidationKey]>,
}

impl<Q, A, E, Backend> RpcMutationAtom<Q, A, E, Backend>
where
    Q: Send + 'static,
    A: Send + 'static,
    E: Send + 'static,
    Backend: Send + 'static,
{
    #[must_use]
    pub fn new(
        client: RpcClient<Q, A, E, Backend>,
        registry: InvalidationRegistry,
        affected: impl IntoIterator<Item = InvalidationKey>,
    ) -> Self {
        let affected = affected
            .into_iter()
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        Self {
            client,
            registry,
            affected: Arc::from(affected),
        }
    }

    pub fn execute<R>(
        &self,
        request: Q,
        call: RpcCallId,
        metadata: BTreeMap<Arc<str>, Arc<str>>,
    ) -> Effect<A, RpcMutationError<E, Backend>, R>
    where
        R: HasRpcChannel + Send + Sync + 'static,
        R::Channel: RpcChannel<Error = Backend>,
    {
        let registry = self.registry.clone();
        let affected = Arc::clone(&self.affected);
        self.client
            .call(request, call, metadata)
            .map_error(RpcMutationError::Client)
            .flat_map(move |value| {
                let value = Mutex::new(Some(value));
                registry
                    .invalidate(affected.iter().cloned().collect())
                    .map_error(RpcMutationError::CommittedWithoutInvalidation)
                    .flat_map(move |_| {
                        let value = value
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner)
                            .take();
                        Effect::from_result_once(value.ok_or(RpcMutationError::InputConsumed))
                    })
            })
    }
}

#[derive(Debug)]
pub enum RpcMiddlewareMutationError<DomainError, Backend, MiddlewareError> {
    Client(RpcClientMiddlewareError<DomainError, Backend, MiddlewareError>),
    /// The remote mutation was acknowledged and local invalidation completed,
    /// but the response transform failed. Retrying the mutation is unsafe
    /// unless its own idempotency contract permits it.
    CommittedWithMiddlewareFailure(MiddlewareError),
    CommittedWithoutInvalidation(InvalidationError),
    InputConsumed,
}

pub struct RpcMiddlewareMutationAtom<Q, Raw, A, DomainError, Backend, MiddlewareError, R>
where
    Q: Send + 'static,
    Raw: Send + 'static,
    A: Send + 'static,
    DomainError: Send + 'static,
    Backend: Send + 'static,
    MiddlewareError: Send + 'static,
    R: HasRpcChannel + Send + Sync + 'static,
    R::Channel: RpcChannel<Error = Backend>,
{
    client: RpcClientMiddleware<Q, Raw, A, DomainError, Backend, MiddlewareError, R>,
    registry: InvalidationRegistry,
    affected: Arc<[InvalidationKey]>,
}

impl<Q, Raw, A, DomainError, Backend, MiddlewareError, R>
    RpcMiddlewareMutationAtom<Q, Raw, A, DomainError, Backend, MiddlewareError, R>
where
    Q: Send + 'static,
    Raw: Send + 'static,
    A: Send + 'static,
    DomainError: Send + 'static,
    Backend: Send + 'static,
    MiddlewareError: Send + 'static,
    R: HasRpcChannel + Send + Sync + 'static,
    R::Channel: RpcChannel<Error = Backend>,
{
    #[must_use]
    pub fn new(
        client: RpcClientMiddleware<Q, Raw, A, DomainError, Backend, MiddlewareError, R>,
        registry: InvalidationRegistry,
        affected: impl IntoIterator<Item = InvalidationKey>,
    ) -> Self {
        let affected = affected
            .into_iter()
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        Self {
            client,
            registry,
            affected: Arc::from(affected),
        }
    }

    pub fn execute(
        &self,
        request: Q,
        call: RpcCallId,
        metadata: BTreeMap<Arc<str>, Arc<str>>,
    ) -> Effect<A, RpcMiddlewareMutationError<DomainError, Backend, MiddlewareError>, R> {
        let registry = self.registry.clone();
        let affected = Arc::clone(&self.affected);
        let client = self.client.clone();
        self.client
            .call_raw(request, call, metadata)
            .map_error(RpcMiddlewareMutationError::Client)
            .flat_map(move |response| {
                let response = Mutex::new(Some(response));
                let client = client.clone();
                registry
                    .invalidate(affected.iter().cloned().collect())
                    .map_error(RpcMiddlewareMutationError::CommittedWithoutInvalidation)
                    .flat_map(move |_| {
                        let response = response
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner)
                            .take();
                        match response {
                            Some(response) => client.apply_after(response).map_error(
                                RpcMiddlewareMutationError::CommittedWithMiddlewareFailure,
                            ),
                            None => Effect::from_result_once(Err(
                                RpcMiddlewareMutationError::InputConsumed,
                            )),
                        }
                    })
            })
    }
}
