//! Optional request propagation and final-body observation over Server's owners.
//!
//! Extraction and the single request span belong to `Server::around_selection`,
//! after route classification and before authentication or environment resolution.
//! Its poll context covers native authentication and the complete selected request
//! scope, including delivery and joined asynchronous cleanup. Completion
//! observation uses `ServerPolicy::on_exit`; response construction is not completion.
//! This module neither consumes a request body nor constructs an axum/Effect runtime.

use crate::{HasOtel, NativeError, Otel, OtelContext, Propagation};
use crate::{effect_sync, effect_try_sync, otel};
use effect::{Clock, Effect, Exit, PrimaryOutcomeRef, SpanOutcome};
use effect::{effect, service};
use effect_server::server;
use effect_server::{BodyOutcome, Request, RequestCompletion, Server};
use opentelemetry::propagation::{Extractor, Injector};
use std::sync::Arc;
use std::time::Duration;

/// Borrowed native HTTP carrier; no request/header payload is duplicated.
pub struct HeaderExtractor<'a> {
    /// Native structured map retains repeated headers and original validation.
    headers: &'a http::HeaderMap,
}
impl<'a> HeaderExtractor<'a> {
    /// Borrow headers for the synchronous native propagator call only.
    #[must_use]
    pub fn new(headers: &'a http::HeaderMap) -> Self {
        Self { headers }
    }
}
impl Extractor for HeaderExtractor<'_> {
    /// Malformed nontext fields are absent to the selected native propagator.
    fn get(&self, key: &str) -> Option<&str> {
        self.headers.get(key)?.to_str().ok()
    }
    /// Native protocol field names are borrowed, never case-reencoded payloads.
    fn keys(&self) -> Vec<&str> {
        self.headers.keys().map(http::HeaderName::as_str).collect()
    }
    /// Retain repeated carrier values for native composite/W3C propagation.
    fn get_all(&self, key: &str) -> Option<Vec<&str>> {
        let values = self
            .headers
            .get_all(key)
            .iter()
            .filter_map(|value| value.to_str().ok())
            .collect::<Vec<_>>();
        if values.is_empty() {
            None
        } else {
            Some(values)
        }
    }
}

/// Checked native injection carrier retaining invalid native header failures.
pub struct HeaderInjector<'a> {
    /// Fresh outbound header owner; no native body is retained here.
    headers: &'a mut http::HeaderMap,
    /// Native propagator's infallible trait is widened at `finish`.
    error: Option<NativeError>,
}
impl<'a> HeaderInjector<'a> {
    /// Borrow a mutable carrier for one injection without copying headers/body.
    #[must_use]
    pub fn new(headers: &'a mut http::HeaderMap) -> Self {
        Self {
            headers,
            error: None,
        }
    }
    /// Recover native header validation after the official infallible setter calls.
    pub fn finish(self) -> Result<(), NativeError> {
        self.error.map_or(Ok(()), Err)
    }
}
impl Injector for HeaderInjector<'_> {
    /// Replace an official propagation field only after native name/value validation.
    fn set(&mut self, key: &str, value: String) {
        if self.error.is_some() {
            return;
        }
        let name = http::HeaderName::from_bytes(key.as_bytes());
        let value = http::HeaderValue::from_str(&value);
        match (name, value) {
            (Ok(name), Ok(value)) => {
                self.headers.insert(name, value);
            }
            (Err(error), Ok(_)) => {
                self.error = Some(NativeError::Native(effect::Defect::new(error.to_string())))
            }
            (Ok(_), Err(error)) => {
                self.error = Some(NativeError::Native(effect::Defect::new(error.to_string())))
            }
            (Err(error), Err(_)) => {
                self.error = Some(NativeError::Native(effect::Defect::new(error.to_string())))
            }
        }
    }
}

/// Private parsed parent metadata; telemetry context grants no authentication authority.
#[derive(Clone)]
struct IncomingContext {
    /// Detached native propagation identity produced by the selected parser.
    context: OtelContext,
}
/// Private per-request span observation, retained beyond native body completion.
#[derive(Clone)]
struct ActiveRequest {
    /// Detached context contains no live native span handle or cleanup owner.
    context: OtelContext,
    /// Core monotonic origin used only to compute a duration, never an epoch.
    started: Option<Duration>,
    /// Identity of the selected clock service; different origins cannot be subtracted.
    /// The address is local metadata only, never dereferenced or exported.
    clock_origin: Option<usize>,
}

/// Extract/install a parent lazily before authentication and resolving request `R`.
/// Missing/malformed values follow the selected native propagator's contract;
/// actual propagator panics remain typed through the caller's error mapping.
/// `install` invokes this inside the early outer selection decorator, before the
/// span starts; manually appending it after authentication hooks loses that order.
pub fn extract_parent<E, R>(
    request: Request,
    propagation: Propagation,
    on_error: impl Fn(NativeError) -> E + Send + Sync + 'static,
) -> Effect<(), E, R>
where
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    effect_try_sync! {
    body: move || {
        let context = propagation.extract(
            &OtelContext::default(),
            &HeaderExtractor::new(request.headers()),
        )?;
        request.insert_extension(IncomingContext { context });
        Ok(())
    }, on_error: on_error, }
}

/// Borrow the extracted detached identity for a native request environment resolver.
/// This metadata never verifies a user, principal, permission or storage handle.
#[must_use]
pub fn incoming_context(request: &Request) -> Option<OtelContext> {
    request.with_extensions(|extensions| {
        extensions
            .get::<IncomingContext>()
            .map(|value| value.context.clone())
    })
}

/// Application-selected safe server span metadata, with no raw request serializer.
pub struct RequestSpan {
    /// Stable operation name, typically derived from matched route identity.
    pub name: Arc<str>,
    /// Typed attributes pass the configured OTel redactor before export.
    pub attributes: Vec<opentelemetry::KeyValue>,
}

/// Install propagation, the outer request span and joined completion observation.
///
/// `around_selection` runs with the original host `R`, after route classification
/// and matched metadata. Extraction precedes the single request span, every
/// authentication/culture hook and `RequestBinding`. Server hands that decorated
/// `NativeContext` to authentication and retains it through the selected `R`,
/// endpoint/native response, actual body delivery, cleanup and terminal observers.
/// Native short circuits use the same boundary. The selected request environment
/// must share the root's core tracer/OTel services so checked span operations refer
/// to the same native map; no global provider bridges distinct environments.
/// A metadata callback panic is diagnostic-only and the original request program
/// still runs under the extracted detached context. Extraction failures are
/// explicitly checked through `on_error`. No later hook extracts again or starts
/// another request span; Web's installer declaration order cannot move extraction
/// behind its `before_environment` authentication features.
/// No provider, exporter, listener, request body or application error is created
/// or serialized here. Observer errors retain Server's complete primary outcome.
#[must_use]
pub fn install<E, R>(
    mut server: Server<E, R>,
    propagation: Propagation,
    metadata: impl Fn(&Request) -> RequestSpan + Send + Sync + 'static,
    observe: impl Fn(RequestObservation) -> Effect<(), E, R> + Send + Sync + 'static,
    on_error: impl Fn(NativeError) -> E + Send + Sync + 'static,
) -> Server<E, R>
where
    E: Send + 'static,
    R: HasOtel + Send + Sync + 'static,
{
    let on_error = Arc::new(on_error);
    let metadata = Arc::new(metadata);
    server = server! {
           source: server,
           around_selection(move |request, complete| {
           let on_error = Arc::clone(&on_error);
           let metadata = Arc::clone(&metadata);
           effect! {
    bind;
    program: extract_parent(request.clone(), propagation.clone(), move |error| {
               on_error(error)
           }),
    body: move |()| {
               let metadata = Arc::clone(&metadata);
               let request_effect = request.clone();
               let request = request.clone();
               let complete = complete.clone();
               effect! {
    bind;
    program: service! {
        env: move |env: &R| {
                   match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                       metadata(&request_effect)
                   })) {
                       Ok(metadata) => Some(metadata),
                       Err(panic) => {
                           env.otel()
                               .diagnostics()
                               .record(effect::Defect::from_panic(panic));
                           None
                       }
                   }
               }, },
    body: move |metadata| match metadata {
                   Some(metadata) => trace_request(
                       request.clone(),
                       metadata.name,
                       metadata.attributes,
                       complete.clone(),
                   ),
                   None => match incoming_context(&request) {
                       Some(context) => Otel::envelope(context, complete.clone()),
                       None => complete.clone(),
                   },
               },
    }
           },
    }
           }),
       };
    let observe = Arc::new(observe);
    server.policy = server.policy.on_exit(move |_, request, completion| {
        let observe = Arc::clone(&observe);
        observe_completion(request, completion, move |summary| observe(summary))
    });
    server
}

/// Wrap the supplied complete request Effect on the existing span lifetime path.
///
/// The host passes its `around_selection` program here after parent extraction.
/// Its existing continuation spans native authentication, R selection and the
/// complete scoped response through observers. Wrapping only endpoint
/// `Reply` construction or an inner pipeline would end before cleanup and is not
/// this contract. `name` and typed attributes are application-selected safe
/// metadata: no URI/query/cookie/authorization/body value is serialized implicitly.
pub fn trace_request<E, R>(
    request: Request,
    name: impl Into<Arc<str>>,
    attributes: Vec<opentelemetry::KeyValue>,
    complete_request: Effect<(), E, R>,
) -> Effect<(), E, R>
where
    E: Send + 'static,
    R: HasOtel + Send + Sync + 'static,
{
    let parent = incoming_context(&request);
    let captured = effect! {
    bind;
    program: Otel::current_context(),
    body: move |context| {
           let request = request.clone();
           service! {
               env: move |env: &R| {
               let (started, clock_origin) =
                   match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                       let clock = env.clock();
                       (clock.now(), clock_origin(clock))
                   })) {
                       Ok((now, origin)) => (Some(now), Some(origin)),
                       Err(panic) => {
                           env.otel()
                               .diagnostics()
                               .record(effect::Defect::from_panic(panic));
                           (None, None)
                       }
                   };
               request.insert_extension(ActiveRequest {
                   context: context.clone(),
                   started,
                   clock_origin,
               });
           }, }
       },
    };
    let program = effect! {
    bind;
    program: captured,
    body: move |()| complete_request.clone(),
    };
    otel! {
        name: name,
        kind: opentelemetry::trace::SpanKind::Server,
        parent_option: parent,
        attributes: attributes,
        effect: program,
    }
}

/// Safe final-body/request summary; no borrowed application error escapes.
#[derive(Clone)]
pub struct RequestObservation {
    /// Detached context correlates completion logs after request cleanup settles.
    pub context: OtelContext,
    /// Duration from the same selected monotonic service; a changed clock origin
    /// yields absence and a diagnostic rather than an invented conversion.
    pub elapsed: Option<Duration>,
    /// Original success/failure/defect/interruption, classified without payloads.
    pub primary: SpanOutcome,
    /// Scope cleanup has finished and reported at least one retained failure.
    pub cleanup_failed: bool,
    /// Acknowledged native body state, distinct from response construction.
    pub body: BodyOutcome,
    /// Whether the native response headers were committed.
    pub committed: bool,
}

/// Build an observation Effect for `ServerPolicy::on_exit` after joined cleanup.
///
/// Missing active telemetry context yields no observation; it never manufactures
/// a span or replaces a business failure. A clock reversal reports a diagnostic
/// and `elapsed: None`. The selected callback decides metric/log routing and may
/// map checked native errors explicitly. Concise OTel operations remain best effort.
pub fn observe_completion<E, R>(
    request: &Request,
    completion: &RequestCompletion<E>,
    observe: impl Fn(RequestObservation) -> Effect<(), E, R> + Send + Sync + 'static,
) -> Effect<(), E, R>
where
    E: Send + 'static,
    R: HasOtel + Send + Sync + 'static,
{
    let active = request.with_extensions(|extensions| extensions.get::<ActiveRequest>().cloned());
    let primary = match completion.exit.primary() {
        PrimaryOutcomeRef::Succeeded => SpanOutcome::Succeeded,
        PrimaryOutcomeRef::Failed(_) => SpanOutcome::Failed,
        PrimaryOutcomeRef::Defected(_) => SpanOutcome::Defected,
        PrimaryOutcomeRef::WasInterrupted => SpanOutcome::Interrupted,
    };
    let cleanup_failed = matches!(completion.exit, Exit::FinalizationFailed(_));
    let body = completion.body;
    let committed = completion.committed;
    let observe = Arc::new(observe);
    effect! {
    bind;
    program: service! {
        env: move |env: &R| {
           active.as_ref().map(|active| {
               let elapsed = active.started.and_then(|started| {
                   match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                       let clock = env.clock(); (clock.now(), clock_origin(clock))
                   })) {
                       Ok((now, origin)) => {
                           if active.clock_origin != Some(origin) {
                               env.otel().diagnostics().record(effect::Defect::new("request telemetry clock service changed during environment selection"));
                               return None;
                           }
                           let elapsed = now.checked_sub(started);
                           if elapsed.is_none() { env.otel().diagnostics().record(effect::Defect::new("request telemetry clock origin moved backwards")); }
                           elapsed
                       },
                       Err(panic) => { env.otel().diagnostics().record(effect::Defect::from_panic(panic)); None },
                   }
               });
               RequestObservation { context: active.context.clone(), elapsed, primary, cleanup_failed, body, committed }
           })
       }, },
    body: move |summary| match summary {
           Some(summary) => { let context = summary.context.clone(); Otel::envelope(context, observe(summary)) },
           None => effect_sync! { body: || (), },
       },
    }
}

/// Compare an exact borrowed clock service without retaining `R` in request
/// extensions (which could form a request/environment ownership cycle).
fn clock_origin<C: Clock + ?Sized>(clock: &C) -> usize {
    std::ptr::from_ref(clock).cast::<()>() as usize
}
