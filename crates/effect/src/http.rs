use crate::internal::runtime::{self, Step};
use crate::internal::workflow::{self, bind, success};
use crate::{
    AdmissionCapacity, Cancellation, Defect, Effect, Exit, ProviderAdmission, Release,
    ResourceValue, Scope,
};
use futures_util::FutureExt;
use futures_util::future::{Either, select};
use std::collections::BTreeMap;
use std::future::Future;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::pin::Pin;
use std::sync::{Arc, Mutex};

pub type TransportFuture<T> = Pin<Box<dyn Future<Output = Result<T, Defect>> + Send>>;
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HttpMethod {
    Get,
    Head,
    Post,
    Put,
    Delete,
    Patch,
    Options,
    Extension(Arc<str>),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HttpStatus(pub u16);
pub type HttpHeaders = BTreeMap<Arc<str>, Arc<str>>;
#[derive(Clone)]
pub struct RequestOptions {
    pub method: HttpMethod,
    pub headers: Arc<HttpHeaders>,
    pub body: Option<Arc<[u8]>>,
}
#[derive(Clone)]
pub struct HttpRequest {
    pub url: Arc<str>,
    pub options: RequestOptions,
}
#[derive(Debug)]
pub enum FetchFailure {
    Transport(Defect),
    Http(HttpStatus),
    Decode(Defect),
}
/// Methods return owned futures; implementations retain their reactor/client
/// and join admitted reads before close completes. No global HTTP client exists.
pub trait HttpBody: Send + Sync + 'static {
    fn read(&self, cancellation: Cancellation) -> TransportFuture<Vec<u8>>;
    fn abort(&self) -> Result<(), Defect>;
    fn close(&self) -> TransportFuture<()>;
}
pub trait HttpTransport: Send + Sync + 'static {
    fn send(
        &self,
        request: HttpRequest,
        cancellation: Cancellation,
    ) -> TransportFuture<HttpResponse>;
}
pub trait HasHttpTransport {
    type Transport: HttpTransport + ?Sized;
    fn http_transport(&self) -> &Self::Transport;
}
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
    pub const fn status(&self) -> HttpStatus {
        self.status
    }
    #[must_use]
    pub fn headers(&self) -> &HttpHeaders {
        &self.headers
    }
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
    let owned = scope.acquire(acquire, move |response| {
        Release::from_exit_future(move |_| response.close())
    });
    owned.flat_map(move |response| match response.with(HttpResponse::status) {
        Ok(status) if accept_status(status) => Effect::sync(move || response.clone()),
        Ok(status) => {
            let on_error = Arc::clone(&on_error);
            Effect::fail_with(move || on_error(FetchFailure::Http(status)))
        }
        Err(defect) => Effect::die(defect),
    })
}
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
    Scope::scoped(move |scope| {
        let accept_status = Arc::clone(&accept_status);
        let on_error = Arc::clone(&on_error);
        let request_error = Arc::clone(&on_error);
        let acquire = request(
            scope,
            request_value.clone(),
            move |status| accept_status(status),
            move |error| request_error(error),
        );
        workflow::reusable(move || {
            let on_error = Arc::clone(&on_error);
            bind(acquire.clone(), move |response| {
                bind(text(response, move |error| on_error(error)), success)
            })
        })
    })
}
