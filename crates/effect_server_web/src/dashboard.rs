//! Operator-authorized finite views and live bounded SSE using the existing sampler.
//!
//! Labels, HTML page/fragment renderers and capacity/rejection responses belong to
//! the application. Native body completion owns session reservations and subscription
//! cleanup; the long-lived stream is never collected into a finite value.

use crate::{
    admission::SessionAdmission,
    diagnostics::{DiagnosticsError, RuntimeSampler, RuntimeSnapshot},
};
use effect::{Effect, Exit};
use effect_server::{
    BodyProgram, Endpoint, HasRequest, HtmlContent, HttpMethodSet, Registered, Registration, Reply,
    ReplyDraft, Request, Router, ServerError, SsePolicy, SseRecord,
};
use std::num::NonZeroUsize;
use std::sync::Arc;
use std::time::Duration;

/// Fixed reference live-fragment protocol target. This is a renderer/transport
/// identity from the existing dashboard contract, not an application quota/default.
pub const FRAGMENT_TARGET: &str = "runtime-snapshot";
/// Prepared operator access; the caller supplies the native policy as a typed
/// Effect. This handle carries no credentials and is not granted by a header value.
#[must_use]
pub struct OperatorAccess<E: Send + 'static, R: Send + Sync + 'static> {
    /// Selected authorization action, retained privately and checked on each route.
    authorize: Arc<dyn Fn(Request) -> Effect<(), E, R> + Send + Sync>,
}
impl<E: Send + 'static, R: Send + Sync + 'static> Clone for OperatorAccess<E, R> {
    /// Share selected authority callback without acquiring new credentials.
    fn clone(&self) -> Self {
        Self {
            authorize: self.authorize.clone(),
        }
    }
}
impl<E: Send + 'static, R: Send + Sync + 'static> OperatorAccess<E, R> {
    /// Adapt an explicit native/application operator policy; no default allow rule.
    pub fn native(authorize: impl Fn(Request) -> Effect<(), E, R> + Send + Sync + 'static) -> Self {
        Self {
            authorize: Arc::new(authorize),
        }
    }
}
/// Finite safe diagnostic view model; typed sampler errors are excluded.
#[derive(Clone)]
#[must_use]
pub struct DashboardPage {
    /// Last acknowledged native measurement/timestamp retained on source failure.
    pub snapshot: Arc<RuntimeSnapshot>,
    /// Refresh degraded because the last selected source returned an expected failure.
    pub degraded: bool,
    /// Application-selected diagnostic route identity.
    pub path: Arc<str>,
    /// Explicit elapsed refresh period, also used by the native sampler.
    pub interval: Duration,
}
/// Application-selected finite page/fragment/compatibility renderers and capacity
/// copy. Web never supplies fixed English labels, localization fallbacks or a product.
#[must_use]
pub struct DashboardPresentation<E: Send + 'static, R: Send + Sync + 'static> {
    /// Initial finite operator page.
    pub page: Arc<dyn Fn(DashboardPage) -> Effect<HtmlContent, E, R> + Send + Sync>,
    /// Finite live fragment retaining FRAGMENT_TARGET in application markup.
    pub fragment: Arc<dyn Fn(DashboardPage) -> Effect<HtmlContent, E, R> + Send + Sync>,
    /// Finite compatibility view selected by application policy.
    pub compatibility: Arc<dyn Fn(DashboardPage) -> Effect<HtmlContent, E, R> + Send + Sync>,
    /// Finite application Effect rendering a capacity rejection before commitment.
    /// Web supplies the named HTTP 429/no-store protocol metadata.
    pub capacity: Arc<dyn Fn(effect::AdmissionError) -> Effect<HtmlContent, E, R> + Send + Sync>,
}
impl<E: Send + 'static, R: Send + Sync + 'static> Clone for DashboardPresentation<E, R> {
    /// Share renderer descriptions only, never model/response payloads.
    fn clone(&self) -> Self {
        Self {
            page: self.page.clone(),
            fragment: self.fragment.clone(),
            compatibility: self.compatibility.clone(),
            capacity: self.capacity.clone(),
        }
    }
}
/// Explicit paths and native frame/finite-body bounds; there are no hidden listener
/// paths, operator concurrency quotas or SSE replay budgets.
#[derive(Clone)]
#[must_use]
pub struct DashboardPolicy {
    /// Initial operator view path.
    pub page: Arc<str>,
    /// Live SSE path with authorization on each connection/reconnection.
    pub live: Arc<str>,
    /// Compatibility finite view path.
    pub compatibility: Arc<str>,
    /// Maximum encoded finite HTML page/fragment bytes.
    pub html_bytes: NonZeroUsize,
    /// Native bounded SSE frame/cursor policy.
    pub sse: SsePolicy,
    /// Positive maximum concurrent live operator sessions.
    pub sessions: NonZeroUsize,
}
/// Prepared protected route set, distinct from public availability aliases.
#[must_use]
pub struct ProtectedDashboard<E: Send + 'static, R: Send + Sync + 'static> {
    /// Real existing Server registration descriptions.
    routes: Router<E, R>,
}
impl<E: Send + 'static, R: Send + Sync + 'static> Clone for ProtectedDashboard<E, R> {
    /// Reuse inert registrations without duplicating request/body owners.
    fn clone(&self) -> Self {
        Self {
            routes: self.routes.clone(),
        }
    }
}
impl<E: Send + 'static, R: HasRequest + Send + Sync + 'static> ProtectedDashboard<E, R> {
    /// Build protected initial/live/compatibility routes. The sampler remains host
    /// owned; every live session acquires a permit in the actual native body scope.
    pub fn new<S: Send + 'static>(
        policy: DashboardPolicy,
        access: OperatorAccess<E, R>,
        presentation: DashboardPresentation<E, R>,
        sampler: RuntimeSampler<S>,
        inject: impl Fn(DiagnosticsError<S>) -> E + Send + Sync + 'static,
        native_error: impl Fn(ServerError) -> E + Send + Sync + 'static,
    ) -> Result<Self, ServerError> {
        let inject = Arc::new(inject);
        let native_error: Arc<dyn Fn(ServerError) -> E + Send + Sync> = Arc::new(native_error);
        let admission = SessionAdmission::new(policy.sessions);
        let mut routes = Router::new();
        let methods = HttpMethodSet::new(vec![http::Method::GET])?;
        for (path, render) in [
            (policy.page.clone(), presentation.page.clone()),
            (
                policy.compatibility.clone(),
                presentation.compatibility.clone(),
            ),
        ] {
            let access = access.clone();
            let sampler = sampler.clone();
            let inject = inject.clone();
            let render = render.clone();
            let path_for_view = path.clone();
            let bound = policy.html_bytes;
            let page_error = native_error.clone();
            let reply = Effect::service(|env: &R| env.request().clone()).flat_map(move |request| {
                let sampler = sampler.clone();
                let inject = inject.clone();
                let render = render.clone();
                let path = path_for_view.clone();
                let page_error = page_error.clone();
                (access.authorize)(request).flat_map(move |()| {
                    let inject = inject.clone();
                    let render = render.clone();
                    let path = path.clone();
                    let page_error = page_error.clone();
                    sampler
                        .status()
                        .map_error(move |error| inject(error))
                        .flat_map({
                            let interval = sampler.interval;
                            move |status| {
                                render(DashboardPage {
                                    snapshot: status.sample.clone(),
                                    degraded: status.failure.is_some(),
                                    path: path.clone(),
                                    interval,
                                })
                                .flat_map({
                                    let errors = page_error.clone();
                                    move |html| {
                                        Effect::from_result_once(finite_html(html, bound, &errors))
                                    }
                                })
                            }
                        })
                })
            });
            routes = routes.registered(
                methods.clone(),
                &path,
                Registered::new(Endpoint::from_effect(reply), Registration::default()),
            )?;
        }
        let live_policy = policy.clone();
        let live_access = access.clone();
        let live_sampler = sampler.clone();
        let live_inject = inject.clone();
        let live_errors = native_error.clone();
        let live_presentation = presentation.clone();
        let live = Endpoint::from_body(move |context| {
            let policy = live_policy.clone();
            let sampler = live_sampler.clone();
            let presentation = live_presentation.clone();
            let inject = live_inject.clone();
            let errors = live_errors.clone();
            let admission = admission.clone();
            let access = live_access.clone();
            async move {
                let request = context
                    .bind(Effect::service(|env: &R| env.request().clone()))
                    .await?;
                context.bind((access.authorize)(request.clone())).await?;
                let cursor = effect_server::sse_cursor(&request, policy.sse).map_err({
                    let errors = errors.clone();
                    move |error| errors(error)
                });
                context.bind(Effect::from_result_once(cursor)).await?;
                let reservation = context.bind(admission.reservation()).await?;
                let admitted = reservation
                    .with(|result| result.as_ref().map(|_| ()).map_err(|error| *error))
                    .map_err(Exit::Die)?;
                if let Err(denial) = admitted {
                    let html = context.bind((presentation.capacity)(denial)).await?;
                    let reply =
                        finite_html(html, policy.html_bytes, &errors).map_err(Exit::Fail)?;
                    let status =
                        effect_server::BodyStatus::new(http::StatusCode::TOO_MANY_REQUESTS)
                            .map_err(|error| Exit::Fail(errors(error)))?;
                    return Ok(ReplyDraft::new().status(status).finish(reply));
                }
                let source = sampler.changes_with({
                    let inject = inject.clone();
                    move |error| inject(error)
                });
                let fragment = presentation.fragment.clone();
                let native_error = errors.clone();
                let view_policy = policy.clone();
                let interval = sampler.interval;
                let program = BodyProgram::source(source, move |status, writer| {
                    let errors = native_error.clone();
                    let policy = view_policy.clone();
                    let writer = writer.clone();
                    fragment(DashboardPage {
                        snapshot: status.sample.clone(),
                        degraded: status.failure.is_some(),
                        path: policy.page.clone(),
                        interval,
                    })
                    .flat_map(move |html| {
                        let finite = Reply::<E, R>::html(html)
                            .into_finite(policy.html_bytes)
                            .map_err({
                                let errors = errors.clone();
                                move |error| errors(error)
                            });
                        let writer = writer.clone();
                        let errors = errors.clone();
                        Effect::from_result_once(finite).flat_map(move |finite| {
                            let data = std::str::from_utf8(&finite.bytes)
                                .map(|html| {
                                    html.lines()
                                        .map(|line| format!("elements {line}"))
                                        .collect::<Vec<_>>()
                                        .join("\n")
                                })
                                .map_err(|_| {
                                    errors(ServerError::Invalid("diagnostic HTML UTF-8".into()))
                                });
                            let writer = writer.clone();
                            Effect::from_result_once(data).flat_map(move |data| {
                                // Datastar's patch-elements protocol repeats the elements line
                                // prefix within native bounded SSE data framing.
                                writer.sse(
                                    SseRecord {
                                        event: Some("datastar-patch-elements".into()),
                                        id: None,
                                        retry: None,
                                        data,
                                    },
                                    policy.sse,
                                )
                            })
                        })
                    })
                });
                Ok(ReplyDraft::new()
                    .header(
                        http::header::CACHE_CONTROL,
                        http::HeaderValue::from_static("no-store"),
                    )
                    .header(
                        http::header::VARY,
                        http::HeaderValue::from_static("Authorization, Cookie"),
                    )
                    .finish(Reply::sse(program)))
            }
        });
        routes = routes.registered(
            methods,
            &policy.live,
            Registered::new(live, Registration::default()),
        )?;
        Ok(Self { routes })
    }
    /// Consume protected registrations into the existing server/router assembly.
    #[must_use]
    pub fn routes(self) -> Router<E, R> {
        self.routes
    }
}

/// Bound every finite application view before it can enter native delivery.
fn finite_html<E: Send + 'static, R: Send + Sync + 'static>(
    html: HtmlContent,
    bound: NonZeroUsize,
    errors: &Arc<dyn Fn(ServerError) -> E + Send + Sync>,
) -> Result<Reply<E, R>, E> {
    let finite = Reply::<E, R>::html(html)
        .into_finite(bound)
        .map_err(|error| errors(error))?;
    Ok(ReplyDraft::new()
        .header(
            http::header::CACHE_CONTROL,
            http::HeaderValue::from_static("no-store"),
        )
        .header(
            http::header::VARY,
            http::HeaderValue::from_static("Authorization, Cookie"),
        )
        .finish(Reply::from_finite(finite)))
}
