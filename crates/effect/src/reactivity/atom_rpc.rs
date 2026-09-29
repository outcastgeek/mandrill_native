//! RPC query and mutation atoms with typed failures.

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
///
/// # Type parameters
/// - `Q`: Request type.
/// - `A`: Application value type.
/// - `E`: Domain-error type.
/// - `Backend`: Backend-specific error or provider type.
/// - `R`: Environment type.
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
    /// Construct a `RpcQueryAtom` from its required inputs and explicit policies.
    ///
    /// # Type parameters
    /// - `Backend`: Backend-specific error or provider type.
    /// - `R`: Environment type.
    /// - `A`: Success or application-value type.
    /// - `E`: Domain-error type.
    /// - `Q`: Request type.
    ///
    /// # Parameters
    /// - `registry` (`InvalidationRegistry`): Registry that owns definitions and rejects identity conflicts.
    /// - `dependencies` (`impl IntoIterator<Item = InvalidationKey>`): The dependencies supplied to this operation.
    /// - `client` (`RpcClient<Q, A, E, Backend>`): Client capability used to submit or observe the remote call.
    /// - `request` (`impl Fn() -> Q + Send + Sync + 'static`): The typed request to process.
    /// - `call` (`impl Fn() -> RpcCallId + Send + Sync + 'static`): Identity and contents of the selected RPC call.
    /// - `metadata` (`impl Fn() -> BTreeMap<Arc<str>, Arc<str>> + Send + Sync + 'static`): Version, identity, or protection data associated with the payload.
    ///
    /// # Returns
    /// Type: `Self`.
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
    /// Return the stream associated with this value.
    ///
    /// # Type parameters
    /// - `Q`: Request type.
    /// - `Backend`: Backend-specific error or provider type.
    /// - `A`: Success or application-value type.
    /// - `E`: Domain-error type.
    /// - `R`: Environment type.
    ///
    /// # Returns
    /// Type: `Stream<super::AsyncResult<A, RpcClientError<E, Backend>>, super::ReactiveQueryError, R>`.
    /// Returns a scoped stream with the declared item, error, and environment types.
    pub fn stream(
        &self,
    ) -> Stream<super::AsyncResult<A, RpcClientError<E, Backend>>, super::ReactiveQueryError, R>
    {
        self.query.stream()
    }

    #[must_use]
    /// Evaluate the selected query through its declared dependencies.
    ///
    /// # Type parameters
    /// - `Q`: Request type.
    /// - `Backend`: Backend-specific error or provider type.
    /// - `A`: Success or application-value type.
    /// - `E`: Domain-error type.
    /// - `R`: Environment type.
    ///
    /// # Returns
    /// Type: `&ReactiveQuery<A, RpcClientError<E, Backend>, R>`.
    pub fn query(&self) -> &ReactiveQuery<A, RpcClientError<E, Backend>, R> {
        &self.query
    }
}

/// Middleware contributes its own typed failure channel to the observable
/// query. The reactive layer never converts that failure to a missing value.
///
/// # Type parameters
/// - `A`: Application value type.
/// - `DomainError`: Expected domain-error type.
/// - `Backend`: Backend-specific error or provider type.
/// - `MiddlewareError`: Error produced by the middleware layer.
/// - `R`: Environment type.
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
    /// Build a mutation atom from a typed RPC call and invalidation policy.
    ///
    /// # Type parameters
    /// - `DomainError`: Expected domain-error type.
    /// - `Backend`: Backend-specific error or provider type.
    /// - `MiddlewareError`: Error produced by the middleware layer.
    /// - `A`: Success or application-value type.
    /// - `R`: Environment type.
    ///
    /// # Parameters
    /// - `registry` (`InvalidationRegistry`): Registry that owns definitions and rejects identity conflicts.
    /// - `dependencies` (`impl IntoIterator<Item = InvalidationKey>`): The dependencies supplied to this operation.
    /// - `call` (`impl Fn() -> Effect<A, RpcClientMiddlewareError<DomainError, Backend, MiddlewareError>, R> + Send + Sync + 'static`): Identity and contents of the selected RPC call.
    ///
    /// # Returns
    /// Type: `Self`.
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
    /// Return the stream associated with this value.
    ///
    /// # Type parameters
    /// - `DomainError`: Expected domain-error type.
    /// - `Backend`: Backend-specific error or provider type.
    /// - `MiddlewareError`: Error produced by the middleware layer.
    /// - `A`: Success or application-value type.
    /// - `R`: Environment type.
    ///
    /// # Returns
    /// Type: `Stream<super::AsyncResult<A, RpcClientMiddlewareError<DomainError, Backend, MiddlewareError>>, super::ReactiveQueryError, R>`.
    /// Returns a scoped stream with the declared item, error, and environment types.
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
    /// Evaluate the selected query through its declared dependencies.
    ///
    /// # Type parameters
    /// - `DomainError`: Expected domain-error type.
    /// - `Backend`: Backend-specific error or provider type.
    /// - `MiddlewareError`: Error produced by the middleware layer.
    /// - `A`: Success or application-value type.
    /// - `R`: Environment type.
    ///
    /// # Returns
    /// Type: `&ReactiveQuery<A, RpcClientMiddlewareError<DomainError, Backend, MiddlewareError>, R>`.
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
    /// Construct a `RpcMiddlewareQueryAtom` from its required inputs and explicit policies.
    ///
    /// # Type parameters
    /// - `DomainError`: Expected domain-error type.
    /// - `Backend`: Backend-specific error or provider type.
    /// - `MiddlewareError`: Error produced by the middleware layer.
    /// - `Q`: Request type. Bounds: `Send + 'static`.
    /// - `Raw`: Bounds: `Send + 'static`.
    /// - `A`: Success or application-value type.
    /// - `R`: Environment type.
    ///
    /// # Parameters
    /// - `registry` (`InvalidationRegistry`): Registry that owns definitions and rejects identity conflicts.
    /// - `dependencies` (`impl IntoIterator<Item = InvalidationKey>`): The dependencies supplied to this operation.
    /// - `client` (`RpcClientMiddleware<Q, Raw, A, DomainError, Backend, MiddlewareError, R>`): Client capability used to submit or observe the remote call.
    /// - `request` (`impl Fn() -> Q + Send + Sync + 'static`): The typed request to process.
    /// - `call` (`impl Fn() -> RpcCallId + Send + Sync + 'static`): Identity and contents of the selected RPC call.
    /// - `metadata` (`impl Fn() -> BTreeMap<Arc<str>, Arc<str>> + Send + Sync + 'static`): Version, identity, or protection data associated with the payload.
    ///
    /// # Returns
    /// Type: `Self`.
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
/// A remote mutation failure that distinguishes uncommitted and committed outcomes.
///
/// # Type parameters
/// - `E`: Domain-error type.
/// - `Backend`: Backend-specific error or provider type.
pub enum RpcMutationError<E, Backend> {
    /// The client could not complete or decode the call. Retains the `RpcClientError<E, Backend>` detail.
    Client(RpcClientError<E, Backend>),
    /// The server replied successfully, but local invalidation failed. The
    /// authoritative remote mutation must not be retried blindly.
    CommittedWithoutInvalidation(InvalidationError),
    /// An owned input was already consumed.
    InputConsumed,
}

/// A mutation invalidates dependent local queries only after a successful
/// typed reply. Local invalidation is not a distributed durability record.
///
/// # Type parameters
/// - `Q`: Request type.
/// - `A`: Application value type.
/// - `E`: Domain-error type.
/// - `Backend`: Backend-specific error or provider type.
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
    /// Construct a `RpcMutationAtom` from its required inputs and explicit policies.
    ///
    /// # Type parameters
    /// - `Backend`: Backend-specific error or provider type.
    /// - `A`: Success or application-value type.
    /// - `E`: Domain-error type.
    /// - `Q`: Request type.
    ///
    /// # Parameters
    /// - `client` (`RpcClient<Q, A, E, Backend>`): Client capability used to submit or observe the remote call.
    /// - `registry` (`InvalidationRegistry`): Registry that owns definitions and rejects identity conflicts.
    /// - `affected` (`impl IntoIterator<Item = InvalidationKey>`): The affected supplied to this operation.
    ///
    /// # Returns
    /// Type: `Self`.
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

    /// Execute the selected typed operation using its declared environment capabilities.
    ///
    /// # Type parameters
    /// - `Backend`: Backend-specific error or provider type.
    /// - `R`: Environment type. Additional bound: `HasRpcChannel + Send + Sync + 'static`. Additional bound: `:Channel: RpcChannel<Error = Backend>`.
    /// - `A`: Success or application-value type.
    /// - `E`: Domain-error type.
    /// - `Q`: Request type.
    ///
    /// # Parameters
    /// - `request` (`Q`): The typed request to process.
    /// - `call` (`RpcCallId`): Identity and contents of the selected RPC call.
    /// - `metadata` (`BTreeMap<Arc<str>, Arc<str>>`): Version, identity, or protection data associated with the payload.
    ///
    /// # Returns
    /// Type: `Effect<A, RpcMutationError<E, Backend>, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
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
/// A remote mutation result that preserves middleware failure after a committed reply.
///
/// # Type parameters
/// - `DomainError`: Expected domain-error type.
/// - `Backend`: Backend-specific error or provider type.
/// - `MiddlewareError`: Error produced by the middleware layer.
pub enum RpcMiddlewareMutationError<DomainError, Backend, MiddlewareError> {
    /// The client could not complete or decode the call. Retains the `RpcClientMiddlewareError<DomainError, Backend, MiddlewareError>` detail.
    Client(RpcClientMiddlewareError<DomainError, Backend, MiddlewareError>),
    /// The remote mutation was acknowledged and local invalidation completed,
    /// but the response transform failed. Retrying the mutation is unsafe
    /// unless its own idempotency contract permits it.
    CommittedWithMiddlewareFailure(MiddlewareError),
    /// The write committed, but publishing its invalidation failed. Retains the `InvalidationError` detail.
    CommittedWithoutInvalidation(InvalidationError),
    /// An owned input was already consumed.
    InputConsumed,
}

/// A reactive atom whose remote writes pass through RPC middleware before invalidation.
///
/// # Type parameters
/// - `Q`: Request type.
/// - `Raw`: Encoded representation type.
/// - `A`: Application value type.
/// - `DomainError`: Expected domain-error type.
/// - `Backend`: Backend-specific error or provider type.
/// - `MiddlewareError`: Error produced by the middleware layer.
/// - `R`: Environment type.
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
    /// Construct a `RpcMiddlewareMutationAtom` from its required inputs and explicit policies.
    ///
    /// # Type parameters
    /// - `Raw`: Encoded representation type.
    /// - `DomainError`: Expected domain-error type.
    /// - `Backend`: Backend-specific error or provider type.
    /// - `MiddlewareError`: Error produced by the middleware layer.
    /// - `A`: Success or application-value type.
    /// - `Q`: Request type.
    /// - `R`: Environment type.
    ///
    /// # Parameters
    /// - `client` (`RpcClientMiddleware<Q, Raw, A, DomainError, Backend, MiddlewareError, R>`): Client capability used to submit or observe the remote call.
    /// - `registry` (`InvalidationRegistry`): Registry that owns definitions and rejects identity conflicts.
    /// - `affected` (`impl IntoIterator<Item = InvalidationKey>`): The affected supplied to this operation.
    ///
    /// # Returns
    /// Type: `Self`.
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

    /// Execute the selected typed operation using its declared environment capabilities.
    ///
    /// # Type parameters
    /// - `Raw`: Encoded representation type.
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
    /// Type: `Effect<A, RpcMiddlewareMutationError<DomainError, Backend, MiddlewareError>, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
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
