//! Transport-neutral HTTP requests, responses, and body ownership.

use crate::internal::runtime::{self, Step};
use crate::{
    AdmissionCapacity, Cancellation, Defect, Effect, Exit, ProviderAdmission, Release,
    ResourceValue, Scope,
};
use crate::{effect, effect_die, effect_scope, effect_scope_acquire, effect_sync, effect_try_sync};
use futures_util::FutureExt;
use futures_util::future::{Either, select};
use std::collections::BTreeMap;
use std::future::Future;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::pin::Pin;
use std::sync::{Arc, Mutex};

/// Owned transport future. Polling uses the boxed future's Rust vtable; the
/// future and its provider remain process-local rather than part of the wire format.
///
/// # Type parameters
/// - `T`: Value or item type.
pub type TransportFuture<T> = Pin<Box<dyn Future<Output = Result<T, Defect>> + Send>>;
#[derive(Clone, Debug, PartialEq, Eq)]
/// The HTTP method selected for a transport request.
pub enum HttpMethod {
    /// Read the selected stored value.
    Get,
    /// Use the current head position of the stream.
    Head,
    /// Run this middleware stage after the handler response.
    Post,
    /// Store or replace the selected value.
    Put,
    /// Delete the selected stored value.
    Delete,
    /// Apply a partial state update.
    Patch,
    /// The encoded value is an optional value.
    Options,
    /// The request uses an extension operation. Contains `Arc<str>`.
    Extension(Arc<str>),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// A validated status code returned by an HTTP transport.
pub struct HttpStatus(pub u16);
/// Type alias for http headers in http.
pub type HttpHeaders = BTreeMap<Arc<str>, Arc<str>>;
#[derive(Clone)]
/// Caller-supplied timeout, size, and response handling bounds for an HTTP request.
pub struct RequestOptions {
    /// HTTP method used by this request. Type: `HttpMethod`.
    pub method: HttpMethod,
    /// HTTP headers sent or received with this message. Type: `Arc<HttpHeaders>`.
    pub headers: Arc<HttpHeaders>,
    /// Application body invoked by this definition. Type: `Option<Arc<[u8]>>`.
    pub body: Option<Arc<[u8]>>,
}
#[derive(Clone)]
/// The method, target, headers, and body submitted to an HTTP transport.
pub struct HttpRequest {
    /// Target URL supplied to the HTTP transport. Type: `Arc<str>`.
    pub url: Arc<str>,
    /// Timeout and resource policies for this request. Type: `RequestOptions`.
    pub options: RequestOptions,
}
#[derive(Debug)]
/// A typed failure from status validation, body decoding, or transport execution.
pub enum FetchFailure {
    /// The selected transport provider failed. Retains the `Defect` detail.
    Transport(Defect),
    /// The HTTP transport or response handling failed. Retains the `HttpStatus` detail.
    Http(HttpStatus),
    /// Decoding the stored or transported value failed. Retains the `Defect` detail.
    Decode(Defect),
}
/// Methods return owned futures; implementations retain their reactor/client
/// and join admitted reads before close completes. No global HTTP client exists.
/// An `Arc<dyn HttpBody>` dispatches these methods through a Rust vtable;
/// callers rely on this trait contract, not on a concrete vtable layout.
pub trait HttpBody: Send + Sync + 'static {
    /// # Parameters
    /// - `cancellation` (`Cancellation`): The cancellation token for this operation.
    ///
    /// # Returns
    /// Type: `TransportFuture<Vec<u8>>`.
    fn read(&self, cancellation: Cancellation) -> TransportFuture<Vec<u8>>;
    /// # Returns
    /// Type: `Result<(), Defect>`.
    fn abort(&self) -> Result<(), Defect>;
    /// # Returns
    /// Type: `TransportFuture<()>`.
    fn close(&self) -> TransportFuture<()>;
}
/// Sends requests through a provider-owned HTTP transport.
/// An environment may expose a concrete implementor or `dyn HttpTransport`;
/// only the latter uses trait-object vtable dispatch.
pub trait HttpTransport: Send + Sync + 'static {
    /// # Parameters
    /// - `request` (`HttpRequest`): The typed request to process.
    /// - `cancellation` (`Cancellation`): The cancellation token for this operation.
    ///
    /// # Returns
    /// Type: `TransportFuture<HttpResponse>`.
    fn send(
        &self,
        request: HttpRequest,
        cancellation: Cancellation,
    ) -> TransportFuture<HttpResponse>;
}
/// Selects the HTTP transport in an application environment.
pub trait HasHttpTransport {
    /// Transport selected by this environment; an unsized trait object is permitted.
    type Transport: HttpTransport + ?Sized;
    /// # Returns
    /// Type: `&Self::Transport`.
    fn http_transport(&self) -> &Self::Transport;
}
/// A response whose body remains owned until it is consumed or closed.
pub struct HttpResponse {
    status: HttpStatus,
    headers: Arc<HttpHeaders>,
    body: Arc<dyn HttpBody>,
    reads: ProviderAdmission,
    lifetime: Cancellation,
    failures: Arc<Mutex<Vec<Defect>>>,
}
impl HttpResponse {
    #[must_use]
    /// Construct a `HttpResponse` from its required inputs and explicit policies.
    ///
    /// # Parameters
    /// - `status` (`HttpStatus`): The status supplied to this operation.
    /// - `headers` (`HttpHeaders`): The headers supplied to this operation.
    /// - `body` (`Arc<dyn HttpBody>`): Application callback or effect executed inside the owning scope.
    ///
    /// # Returns
    /// Type: `Self`.
    pub fn new(status: HttpStatus, headers: HttpHeaders, body: Arc<dyn HttpBody>) -> Self {
        Self {
            status,
            headers: Arc::new(headers),
            body,
            reads: ProviderAdmission::new(AdmissionCapacity::Unbounded),
            lifetime: Cancellation::new(),
            failures: Arc::new(Mutex::new(Vec::new())),
        }
    }
    #[must_use]
    /// # Returns
    /// Type: `HttpStatus`.
    pub const fn status(&self) -> HttpStatus {
        self.status
    }
    #[must_use]
    /// Borrow the request or response headers without copying their values.
    ///
    /// # Returns
    /// Type: `&HttpHeaders`.
    pub fn headers(&self) -> &HttpHeaders {
        &self.headers
    }
    ///
    /// # Parameters
    /// - `cancellation` (`Cancellation`): The cancellation token for this operation.
    ///
    /// # Returns
    /// Type: `TransportFuture<Vec<u8>>`.
    /// Returns `TransportFuture<Vec<u8>>`.
    fn read(&self, cancellation: Cancellation) -> TransportFuture<Vec<u8>> {
        let permit = self.reads.admit();
        let body = Arc::clone(&self.body);
        let lifetime = self.lifetime.clone();
        let failures = Arc::clone(&self.failures);
        Box::pin(async move {
            let permit = permit.map_err(Defect::from)?;
            let read = body.read(lifetime.clone());
            let result = match select(read, Box::pin(cancellation.cancelled())).await {
                Either::Left((result, _)) => result,
                Either::Right(((), remaining)) => {
                    let _ = lifetime.request();
                    let aborted = catch_unwind(AssertUnwindSafe(|| body.abort()))
                        .unwrap_or_else(|panic| Err(Defect::from_panic(panic)));
                    if let Err(defect) = aborted {
                        failures
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner)
                            .push(defect);
                    }
                    remaining.await
                }
            };
            drop(permit);
            result
        })
    }
    ///
    /// # Type parameters
    /// - `E`: Bounds: `Send + 'static`.
    ///
    /// # Returns
    /// Type: `Pin<Box<dyn Future<Output = Exit<(), E>> + Send>>`.
    /// Returns `Pin<Box<dyn Future<Output = Exit<(), E>> + Send>>`.
    fn close<E: Send + 'static>(self) -> Pin<Box<dyn Future<Output = Exit<(), E>> + Send>> {
        Box::pin(async move {
            let draining = self.reads.stop();
            let _ = self.lifetime.request();
            let aborted = catch_unwind(AssertUnwindSafe(|| self.body.abort()))
                .unwrap_or_else(|panic| Err(Defect::from_panic(panic)));
            draining.wait().await;
            let closed = AssertUnwindSafe(async { self.body.close().await })
                .catch_unwind()
                .await
                .unwrap_or_else(|panic| Err(Defect::from_panic(panic)));
            let mut cleanup = Exit::Success(());
            let failures = std::mem::take(
                &mut *self
                    .failures
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner),
            );
            for defect in failures
                .into_iter()
                .chain(aborted.err())
                .chain(closed.err())
            {
                cleanup = crate::internal::outcome::append_cleanup(cleanup, Exit::Die(defect));
            }
            cleanup
        })
    }
}
impl<E: Send + 'static, R: HasHttpTransport + Send + Sync + 'static> Effect<HttpResponse, E, R> {
    /// Construct a typed HTTP request with caller-supplied method, target, and body.
    ///
    /// # Type parameters
    /// - `R`: Environment type. Bound: `HasHttpTransport + Send + Sync + 'static`.
    /// - `E`: Domain-error type.
    ///
    /// # Parameters
    /// - `request` (`HttpRequest`): The typed request to process.
    /// - `on_error` (`impl Fn(FetchFailure) -> E + Send + Sync + 'static`): Callback that maps a source error into the declared error type.
    ///
    /// # Returns
    /// Type: `Self`.
    pub fn http_request(
        request: HttpRequest,
        on_error: impl Fn(FetchFailure) -> E + Send + Sync + 'static,
    ) -> Self {
        let on_error = Arc::new(on_error);
        Self::new(move |env, context, finish| {
            let future = match catch_unwind(AssertUnwindSafe(|| {
                env.http_transport()
                    .send(request.clone(), context.cancellation.clone())
            })) {
                Ok(future) => future,
                Err(panic) => return runtime::finish(finish, Exit::Die(Defect::from_panic(panic))),
            };
            let on_error = Arc::clone(&on_error);
            Step::Suspended(Box::pin(async move {
                let outcome = match AssertUnwindSafe(future).catch_unwind().await {
                    Ok(Ok(response)) => Exit::Success(response),
                    Ok(Err(defect)) => match catch_unwind(AssertUnwindSafe(|| {
                        on_error(FetchFailure::Transport(defect))
                    })) {
                        Ok(error) => Exit::Fail(error),
                        Err(panic) => Exit::Die(Defect::from_panic(panic)),
                    },
                    Err(panic) => Exit::Die(Defect::from_panic(panic)),
                };
                drop(env);
                // Acquisition publishes late success to ResourceBinding first.
                runtime::finish(finish, outcome)
            }))
        })
    }
}
/// Execute a request through the HTTP transport selected by the environment.
///
/// # Type parameters
/// - `E`: Domain-error type. Additional bound: `Send + 'static`.
/// - `R`: Environment type. Additional bound: `HasHttpTransport + Send + Sync + 'static`.
///
/// # Parameters
/// - `scope` (`Scope<E>`): Owner that joins child work and releases resources at close.
/// - `request` (`HttpRequest`): The typed request to process.
/// - `accept_status` (`impl Fn(HttpStatus) -> bool + Send + Sync + 'static`): The accept status supplied to this operation.
/// - `on_error` (`impl Fn(FetchFailure) -> E + Send + Sync + 'static`): Callback that maps a source error into the declared error type.
///
/// # Returns
/// Type: `Effect<ResourceValue<HttpResponse>, E, R>`.
/// Returns a lazy `Effect` with the declared success, error, and environment types.
pub fn request<E, R>(
    scope: Scope<E>,
    request: HttpRequest,
    accept_status: impl Fn(HttpStatus) -> bool + Send + Sync + 'static,
    on_error: impl Fn(FetchFailure) -> E + Send + Sync + 'static,
) -> Effect<ResourceValue<HttpResponse>, E, R>
where
    E: Send + 'static,
    R: HasHttpTransport + Send + Sync + 'static,
{
    let on_error = Arc::new(on_error);
    let transport_error = Arc::clone(&on_error);
    let acquire = Effect::http_request(request, move |error| transport_error(error));
    let owned = effect_scope_acquire! {
        scope: scope,
        acquire: acquire,
        release: move |response| Release::from_exit_future(move |_| response.close()),
    };
    effect! {
    bind;
    program: owned,
    body: move |response| match response.with(HttpResponse::status) {
           Ok(status) if accept_status(status) => effect_sync! { body: move || response.clone(), },
           Ok(status) => {
               let on_error = Arc::clone(&on_error);
               effect_try_sync! { body: { let __dsl_error = move || on_error(FetchFailure::Http(status)); move || Err(__dsl_error()) }, on_error: ::core::convert::identity, }
           }
           Err(defect) => effect_die! { defect: defect, },
       },
    }
}
/// Decode a response body as UTF-8 after honoring status and size policies.
///
/// # Type parameters
/// - `E`: Domain-error type. Bounds: `Send + 'static`.
/// - `R`: Environment type. Bounds: `Send + Sync + 'static`.
///
/// # Parameters
/// - `response` (`ResourceValue<HttpResponse>`): The response supplied to this operation.
/// - `on_error` (`impl Fn(FetchFailure) -> E + Send + Sync + 'static`): Callback that maps a source error into the declared error type.
///
/// # Returns
/// Type: `Effect<String, E, R>`.
/// Returns a lazy `Effect` with the declared success, error, and environment types.
pub fn text<E: Send + 'static, R: Send + Sync + 'static>(
    response: ResourceValue<HttpResponse>,
    on_error: impl Fn(FetchFailure) -> E + Send + Sync + 'static,
) -> Effect<String, E, R> {
    let on_error = Arc::new(on_error);
    Effect::new(move |env, context, finish| {
        let future = match response.with(|response| response.read(context.cancellation.clone())) {
            Ok(future) => future,
            Err(defect) => return runtime::finish(finish, Exit::Die(defect)),
        };
        let on_error = Arc::clone(&on_error);
        Step::Suspended(Box::pin(async move {
            let work = async {
                let result = AssertUnwindSafe(future).catch_unwind().await;
                match result {
                    Ok(Ok(bytes)) => match String::from_utf8(bytes) {
                        Ok(text) => Exit::Success(text),
                        Err(error) => {
                            Exit::Fail(on_error(FetchFailure::Decode(Defect::from_utf8(error))))
                        }
                    },
                    Ok(Err(defect)) => Exit::Fail(on_error(FetchFailure::Decode(defect))),
                    Err(panic) => Exit::Die(Defect::from_panic(panic)),
                }
            };
            let outcome = AssertUnwindSafe(work)
                .catch_unwind()
                .await
                .unwrap_or_else(|panic| Exit::Die(Defect::from_panic(panic)));
            drop(env);
            runtime::finish(finish, runtime::publish(outcome, &context))
        }))
    })
}
/// Fetch a response and decode its body as bounded UTF-8 text.
///
/// # Type parameters
/// - `E`: Domain-error type. Additional bound: `Send + 'static`.
/// - `R`: Environment type. Additional bound: `HasHttpTransport + Send + Sync + 'static`.
///
/// # Parameters
/// - `request_value` (`HttpRequest`): The request value supplied to this operation.
/// - `accept_status` (`impl Fn(HttpStatus) -> bool + Send + Sync + 'static`): The accept status supplied to this operation.
/// - `on_error` (`impl Fn(FetchFailure) -> E + Send + Sync + 'static`): Callback that maps a source error into the declared error type.
///
/// # Returns
/// Type: `Effect<String, E, R>`.
/// Returns a lazy `Effect` with the declared success, error, and environment types.
pub fn fetch_text<E, R>(
    request_value: HttpRequest,
    accept_status: impl Fn(HttpStatus) -> bool + Send + Sync + 'static,
    on_error: impl Fn(FetchFailure) -> E + Send + Sync + 'static,
) -> Effect<String, E, R>
where
    E: Send + 'static,
    R: HasHttpTransport + Send + Sync + 'static,
{
    let accept_status = Arc::new(accept_status);
    let on_error = Arc::new(on_error);
    effect_scope! {
        body: move |scope| {
        let accept_status = Arc::clone(&accept_status);
        let on_error = Arc::clone(&on_error);
        let request_error = Arc::clone(&on_error);
        let acquire = request(
            scope,
            request_value.clone(),
            move |status| accept_status(status),
            move |error| request_error(error),
        );
        effect! {
            let on_error = Arc::clone(&on_error);
            let response = acquire.clone()?;
            text(response, move |error| on_error(error))?
        }
    }, }
}
