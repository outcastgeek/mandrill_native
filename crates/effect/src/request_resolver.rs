//! Scoped batching and sharing of request loads.

use crate::internal::runtime::{self, Step};
use crate::{
    Defect, Effect, Exit, HasTaskSpawner, Release, Request, RequestKey, Scope, SharedValue,
};
use futures_util::FutureExt;
use std::panic::AssertUnwindSafe;
use std::sync::{Arc, Mutex};

/// Type alias for resolve many effect in request resolver.
///
/// # Type parameters
/// - `A`: Application value type.
/// - `E`: Domain-error type.
/// - `R`: Environment type.
pub type ResolveManyEffect<A, E, R> =
    Effect<Vec<Exit<SharedValue<A>, RequestResolverError<E>>>, RequestResolverError<E>, R>;

mod internal;
use internal::ResolverState;

/// Process-local identity of one resolver/cache environment. The application
/// creates and shares this token through R; it is never a durable request key.
#[derive(Clone, Default)]
pub struct RequestScope(Arc<()>);

impl RequestScope {
    #[must_use]
    /// Construct a `RequestScope` from its required inputs and explicit policies.
    ///
    /// # Returns
    /// Type: `Self`.
    pub fn new() -> Self {
        Self(Arc::new(()))
    }

    #[must_use]
    /// Check whether two resolver requests share the same stable identity.
    ///
    /// # Parameters
    /// - `other` (`&Self`): Second value used for comparison or composition.
    ///
    /// # Returns
    /// Type: `bool`.
    /// Returns whether the requested condition holds.
    pub fn same(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

/// Only request-resolution operations require this environment projection.
pub trait HasRequestScope {
    /// # Returns
    /// Type: `&RequestScope`.
    fn request_scope(&self) -> &RequestScope;
}

/// Expected request, batch and construction failures remain distinct. Error
/// payloads are shared handles, so concurrent observers need no E: Clone.
///
/// # Type parameters
/// - `E`: Domain-error type. Bound or type: `Send + 'static`.
pub enum RequestResolverError<E: Send + 'static> {
    /// The request operation failed. Retains the `SharedValue<E>` detail.
    RequestFailed(SharedValue<E>),
    /// The batch operation failed. Retains the `SharedValue<E>` detail.
    BatchFailed(SharedValue<E>),
    /// The same request key was assigned to another group. Retains the `RequestKey` detail.
    ConflictingGroup(RequestKey),
    /// A batch resolver returned a different number of results than requests.
    CardinalityMismatch {
        /// Number of submitted requests. Type: `usize`.
        expected: usize,
        /// Number of returned results. Type: `usize`.
        actual: usize,
    },
    /// The supplied environment does not match the required or recorded value.
    WrongEnvironment,
    /// The provider has closed admission for new work.
    Closed,
    /// An owned input was already consumed.
    InputConsumed,
    /// The operation ended without recording a terminal result.
    Uncompleted,
}

impl<E: Send + 'static> Clone for RequestResolverError<E> {
    ///
    /// # Returns
    /// Type: `Self`.
    fn clone(&self) -> Self {
        match self {
            Self::RequestFailed(error) => Self::RequestFailed(error.clone()),
            Self::BatchFailed(error) => Self::BatchFailed(error.clone()),
            Self::ConflictingGroup(key) => Self::ConflictingGroup(key.clone()),
            Self::CardinalityMismatch { expected, actual } => Self::CardinalityMismatch {
                expected: *expected,
                actual: *actual,
            },
            Self::WrongEnvironment => Self::WrongEnvironment,
            Self::Closed => Self::Closed,
            Self::InputConsumed => Self::InputConsumed,
            Self::Uncompleted => Self::Uncompleted,
        }
    }
}

impl<E: Send + 'static> std::fmt::Debug for RequestResolverError<E> {
    ///
    /// # Parameters
    /// - `formatter` (`&mut std::fmt::Formatter<'_>`): Destination formatter used by the Rust formatting trait.
    ///
    /// # Returns
    /// Type: `std::fmt::Result`.
    /// Returns `std::fmt::Result`.
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::RequestFailed(_) => formatter.write_str("RequestFailed"),
            Self::BatchFailed(_) => formatter.write_str("BatchFailed"),
            Self::ConflictingGroup(_) => formatter.write_str("ConflictingGroup"),
            Self::CardinalityMismatch { expected, actual } => formatter
                .debug_struct("CardinalityMismatch")
                .field("expected", expected)
                .field("actual", actual)
                .finish(),
            Self::WrongEnvironment => formatter.write_str("WrongEnvironment"),
            Self::Closed => formatter.write_str("Closed"),
            Self::InputConsumed => formatter.write_str("InputConsumed"),
            Self::Uncompleted => formatter.write_str("Uncompleted"),
        }
    }
}

impl<E: Send + 'static> std::fmt::Display for RequestResolverError<E> {
    ///
    /// # Parameters
    /// - `formatter` (`&mut std::fmt::Formatter<'_>`): Destination formatter used by the Rust formatting trait.
    ///
    /// # Returns
    /// Type: `std::fmt::Result`.
    /// Returns `std::fmt::Result`.
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(self, formatter)
    }
}

impl<E: Send + 'static> std::error::Error for RequestResolverError<E> {}

/// Type alias for batch runner in request resolver.
///
/// # Type parameters
/// - `Q`: Request type.
/// - `A`: Application value type.
/// - `E`: Domain-error type.
/// - `R`: Environment type.
type BatchRunner<Q, A, E, R> =
    dyn Fn(crate::RequestGroup, Vec<Q>) -> Effect<Vec<Exit<A, E>>, E, R> + Send + Sync;

/// A live resolver is bound to the request scope token and Scope that
/// constructed it. Distinct R values may share that token when they represent
/// the same application environment; a different token cannot reuse loads.
/// A batch is explicit: no implicit tick, debounce, worker count or executor
/// chooses when requests leave the pending registry. Identical pending keys
/// within this instance share one load; completed entries are not retained.
///
/// # Type parameters
/// - `Q`: Request type. Bound or type: `Send + 'static`.
/// - `A`: Application value type. Bound or type: `Send + 'static`.
/// - `E`: Domain-error type. Bound or type: `Send + 'static`.
/// - `R`: Environment type. Bound or type: `HasTaskSpawner + HasRequestScope + Send + Sync + 'static`.
pub struct RequestResolver<
    Q: Send + 'static,
    A: Send + 'static,
    E: Send + 'static,
    R: HasTaskSpawner + HasRequestScope + Send + Sync + 'static,
> {
    inner: Arc<ResolverState<Q, A, E, R>>,
}

impl<
    Q: Send + 'static,
    A: Send + 'static,
    E: Send + 'static,
    R: HasTaskSpawner + HasRequestScope + Send + Sync + 'static,
> Clone for RequestResolver<Q, A, E, R>
{
    ///
    /// # Returns
    /// Type: `Self`.
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

impl<
    Q: Send + 'static,
    A: Send + 'static,
    E: Send + 'static,
    R: HasTaskSpawner + HasRequestScope + Send + Sync + 'static,
> RequestResolver<Q, A, E, R>
{
    /// Acquire one resolver for one explicit environment token and one
    /// structured lifetime.
    /// `run` receives a nonempty same-group batch and returns exactly one
    /// outcome per input, in the same order. A whole-batch E is separate from
    /// the per-request E values carried by its successful result vector.
    ///
    /// # Type parameters
    /// - `A`: Success or application-value type.
    /// - `E`: Domain-error type.
    /// - `Q`: Request type.
    /// - `R`: Environment type.
    ///
    /// # Parameters
    /// - `scope` (`Scope<RequestResolverError<E>>`): Owner that joins child work and releases resources at close.
    /// - `run` (`impl Fn(crate::RequestGroup, Vec<Q>) -> Effect<Vec<Exit<A, E>>, E, R> + Send + Sync + 'static`): Stable identity of the workflow run being inspected or changed.
    ///
    /// # Returns
    /// Type: `Effect<Self, RequestResolverError<E>, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    pub fn make(
        scope: Scope<RequestResolverError<E>>,
        run: impl Fn(crate::RequestGroup, Vec<Q>) -> Effect<Vec<Exit<A, E>>, E, R>
        + Send
        + Sync
        + 'static,
    ) -> Effect<Self, RequestResolverError<E>, R> {
        let run: Arc<BatchRunner<Q, A, E, R>> = Arc::new(run);
        Effect::new(move |env: Arc<R>, context, finish| {
            let state: Arc<ResolverState<Q, A, E, R>> = Arc::new(ResolverState::new(
                scope.clone(),
                env.request_scope().clone(),
                Arc::clone(&run),
            ));
            let finalizer_state = Arc::downgrade(&state);
            let register = scope.add_release(move || {
                let state = finalizer_state.clone();
                Release::sync(move || {
                    if let Some(state) = state.upgrade() {
                        state.close();
                    }
                })
            });
            Step::Suspended(Box::pin(async move {
                let protected = context.protected();
                let registered = runtime::evaluate_context(register, env, &protected).await;
                runtime::finish(
                    finish,
                    match registered.into_failure::<Self>() {
                        Ok(()) => Exit::Success(Self { inner: state }),
                        Err(failure) => failure,
                    },
                )
            }))
        })
    }

    /// Resolve an explicit batch. Construction/scope conflicts use the outer
    /// error; each position retains its typed request, batch or admission Exit
    /// in input order.
    ///
    /// # Type parameters
    /// - `A`: Success or application-value type.
    /// - `E`: Domain-error type.
    /// - `Q`: Request type.
    /// - `R`: Environment type.
    ///
    /// # Parameters
    /// - `requests` (`Vec<Request<Q, A, E>>`): The requests supplied to this operation.
    ///
    /// # Returns
    /// Type: `ResolveManyEffect<A, E, R>`.
    pub fn resolve_many(&self, requests: Vec<Request<Q, A, E>>) -> ResolveManyEffect<A, E, R> {
        let resolver = Arc::clone(&self.inner);
        let requests = Mutex::new(Some(requests));
        Effect::new(move |env, context, finish| {
            let requests = requests
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take();
            let Some(requests) = requests else {
                return runtime::finish(finish, Exit::Fail(RequestResolverError::InputConsumed));
            };
            let resolver = Arc::clone(&resolver);
            Step::Suspended(Box::pin(async move {
                let outcome = AssertUnwindSafe(resolver.resolve(requests, env, context))
                    .catch_unwind()
                    .await
                    .unwrap_or_else(|panic| Exit::Die(Defect::from_panic(panic)));
                runtime::finish(finish, outcome)
            }))
        })
    }

    /// Resolve one request and expose its expected failure in the Effect E
    /// channel while retaining shared, non-Clone result ownership.
    ///
    /// # Type parameters
    /// - `A`: Success or application-value type.
    /// - `E`: Domain-error type.
    /// - `Q`: Request type.
    /// - `R`: Environment type.
    ///
    /// # Parameters
    /// - `request` (`Request<Q, A, E>`): The typed request to process.
    ///
    /// # Returns
    /// Type: `Effect<SharedValue<A>, RequestResolverError<E>, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    pub fn resolve(
        &self,
        request: Request<Q, A, E>,
    ) -> Effect<SharedValue<A>, RequestResolverError<E>, R> {
        let resolver = Arc::clone(&self.inner);
        let request = Mutex::new(Some(request));
        Effect::new(move |env, context, finish| {
            let request = request
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take();
            let Some(request) = request else {
                return runtime::finish(finish, Exit::Fail(RequestResolverError::InputConsumed));
            };
            let resolver = Arc::clone(&resolver);
            Step::Suspended(Box::pin(async move {
                let work = async {
                    match resolver
                        .resolve(vec![request], env, context)
                        .await
                        .into_failure()
                    {
                        Ok(mut outcomes) => outcomes
                            .pop()
                            .unwrap_or(Exit::Fail(RequestResolverError::Uncompleted)),
                        Err(failure) => failure,
                    }
                };
                let outcome = AssertUnwindSafe(work)
                    .catch_unwind()
                    .await
                    .unwrap_or_else(|panic| Exit::Die(Defect::from_panic(panic)));
                runtime::finish(finish, outcome)
            }))
        })
    }
}
