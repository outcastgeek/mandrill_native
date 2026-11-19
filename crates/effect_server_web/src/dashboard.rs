//! Operator-authorized finite views and live bounded SSE using the existing sampler.
//!
//! Labels, HTML page/fragment renderers and capacity/rejection responses belong to
//! the application. Native body completion owns session reservations and subscription
//! cleanup; the long-lived stream is never collected into a finite value.

use crate::{
    admission::SessionAdmission,
    diagnostics::{DiagnosticsError, RuntimeSampler, RuntimeSnapshot},
};
use crate::{effect_future, effect_map_error, effect_sync, effect_try_sync, effect_using};
use effect::memory::{
    Buffer, BufferLease, BufferView, Bytes, MemoryError, PreparedMemory, RegionId, SlotPool,
};
use effect::{Effect, Exit, PreparedAdmissionError, PreparedProviderAdmission, effect};
use effect_server::{
    HasRequest, HtmlContent, HttpMethodSet, Reply, Request, Router, ServerError, SsePolicy,
    SseRecord,
};
use effect_server::{endpoint, registered, reply, router, server, stream};
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
    /// Web supplies HTTP 503 Service Unavailable and `Cache-Control: no-store`.
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

/// Dedicated finite HTML, SSE output and live session owners for protected operational routes.
pub struct PreparedDashboardOutput {
    pages: SlotPool<'static, Buffer<'static>>,
    frames: SlotPool<'static, Buffer<'static>>,
    sessions: Arc<PreparedProviderAdmission>,
    capacity: NonZeroUsize,
}
impl PreparedDashboardOutput {
    /// Region bytes for both buffers and live session admission, excluding allocator overhead.
    pub fn storage_bytes(
        pages: usize,
        html_bytes: Bytes,
        frames: usize,
        frame_bytes: Bytes,
    ) -> Result<Bytes, MemoryError> {
        SlotPool::<Buffer<'static>>::storage_bytes(pages)?
            .checked_add(html_bytes.checked_mul(pages)?)?
            .checked_add(SlotPool::<Buffer<'static>>::storage_bytes(frames)?)?
            .checked_add(frame_bytes.checked_mul(frames)?)?
            .checked_add(PreparedProviderAdmission::storage_bytes(0)?)
    }

    /// Provision finite views, live frames and the selected session limit.
    pub fn prepare(
        memory: &PreparedMemory<'static>,
        region: RegionId,
        pages: usize,
        html_bytes: Bytes,
        frames: usize,
        frame_bytes: Bytes,
        sessions: NonZeroUsize,
    ) -> Result<Self, MemoryError> {
        Self::storage_bytes(pages, html_bytes, frames, frame_bytes)?;
        Ok(Self {
            pages: memory.buffers(region, pages, html_bytes)?,
            frames: memory.buffers(region, frames, frame_bytes)?,
            sessions: Arc::new(PreparedProviderAdmission::prepare(
                memory, region, sessions, 0,
            )?),
            capacity: sessions,
        })
    }
}

#[derive(Clone)]
enum DashboardSessions {
    Ordinary(SessionAdmission),
    Prepared(Arc<PreparedProviderAdmission>),
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
        Self::build(
            policy,
            access,
            presentation,
            sampler,
            inject,
            native_error,
            None,
        )
    }

    /// Build the protected routes with independent finite view and frame backing.
    pub fn new_prepared<S: Send + 'static>(
        policy: DashboardPolicy,
        access: OperatorAccess<E, R>,
        presentation: DashboardPresentation<E, R>,
        sampler: RuntimeSampler<S>,
        inject: impl Fn(DiagnosticsError<S>) -> E + Send + Sync + 'static,
        native_error: impl Fn(ServerError) -> E + Send + Sync + 'static,
        output: Arc<PreparedDashboardOutput>,
    ) -> Result<Self, ServerError> {
        Self::build(
            policy,
            access,
            presentation,
            sampler,
            inject,
            native_error,
            Some(output),
        )
    }

    fn build<S: Send + 'static>(
        policy: DashboardPolicy,
        access: OperatorAccess<E, R>,
        presentation: DashboardPresentation<E, R>,
        sampler: RuntimeSampler<S>,
        inject: impl Fn(DiagnosticsError<S>) -> E + Send + Sync + 'static,
        native_error: impl Fn(ServerError) -> E + Send + Sync + 'static,
        output: Option<Arc<PreparedDashboardOutput>>,
    ) -> Result<Self, ServerError> {
        let admission = match &output {
            Some(storage) if storage.capacity != policy.sessions => {
                return Err(ServerError::Invalid("dashboard session capacity".into()));
            }
            Some(storage) => DashboardSessions::Prepared(storage.sessions.clone()),
            None => DashboardSessions::Ordinary(SessionAdmission::new(policy.sessions)),
        };
        let inject = Arc::new(inject);
        let native_error: Arc<dyn Fn(ServerError) -> E + Send + Sync> = Arc::new(native_error);
        let mut routes = router! {}?;
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
            let page_output = output.clone();
            let reply = effect! {
                let access = access.clone();
                let sampler = sampler.clone();
                let inject = inject.clone();
                let render = render.clone();
                let path = path_for_view.clone();
                let page_error = page_error.clone();
                let page_output = page_output.clone();
                let reserve_error = page_error.clone();
                let reserved = effect_try_sync! {
                    body: move || page_output.as_ref()
                        .map(|storage| storage.pages.try_checkout())
                        .transpose(),
                    on_error: move |error| reserve_error(ServerError::Memory(error)),
                }?;
                let request = server! { request; }?;
                (access.authorize)(request)?;
                let status = effect_map_error! { program: sampler.status(), map: move |error| inject(error), }?;
                let html = render(DashboardPage {
                    snapshot: status.sample.clone(),
                    degraded: status.failure.is_some(),
                    path,
                    interval: sampler.interval,
                })?;
                effect::effect_from_result! { once; result: finite_html(html, bound, &page_error, reserved), }?
            };
            routes = router! {
                source: routes,
                methods(methods.clone(), path.as_ref(), registered! {
                    endpoint(endpoint! { effect(reply) }),
                }),
            }?;
        }
        let live_policy = policy.clone();
        let live_access = access.clone();
        let live_sampler = sampler.clone();
        let live_inject = inject.clone();
        let live_errors = native_error.clone();
        let live_presentation = presentation.clone();
        let live_output = output;
        let live = endpoint! {
            body: move |context| {
            let policy = live_policy.clone();
            let sampler = live_sampler.clone();
            let presentation = live_presentation.clone();
            let inject = live_inject.clone();
            let errors = live_errors.clone();
            let admission = admission.clone();
            let access = live_access.clone();
            let output = live_output.clone();
            async move {
                let request = effect! { bind_in_context; context: context, program: server! { request; }, }
                    .await?;
                effect! { bind_in_context; context: context, program: (access.authorize)(request.clone()), }.await?;
                let cursor = effect_server::sse_cursor(&request, policy.sse).map_err({
                    let errors = errors.clone();
                    move |error| errors(error)
                });
                effect! { bind_in_context; context: context, program: effect::effect_from_result! { once; result: cursor, }, }.await?;
                let admitted = match admission {
                    DashboardSessions::Ordinary(admission) => {
                        let reservation = effect! { bind_in_context; context: context, program: admission.reservation(), }.await?;
                        reservation.with(|result| result.as_ref().map(|_| ()).map_err(|error| *error))
                            .map_err(Exit::Die)?
                    }
                    DashboardSessions::Prepared(admission) => {
                        let reservation = effect! {
                            bind_in_context;
                            context: context,
                            program: effect_using! {
                            binding;
                            acquire: effect_sync! { body: move || admission.admit() },
                        },
                        }.await?;
                        reservation.with(|result| match result {
                            Ok(_) => Ok(Ok(())),
                            Err(PreparedAdmissionError::Admission(denial)) => Ok(Err(*denial)),
                            Err(PreparedAdmissionError::Memory(error)) => Err(*error),
                        }).map_err(Exit::Die)?
                            .map_err(|error| Exit::Fail(errors(ServerError::Memory(error))))?
                    }
                };
                if let Err(denial) = admitted {
                    let reserved = output.as_ref().map(|storage| storage.pages.try_checkout()).transpose()
                        .map_err(|error| Exit::Fail(errors(ServerError::Memory(error))))?;
                    let html = effect! { bind_in_context; context: context, program: (presentation.capacity)(denial), }.await?;
                    let reply =
                        finite_html(html, policy.html_bytes, &errors, reserved).map_err(Exit::Fail)?;
                    let status =
                        effect_server::BodyStatus::new(http::StatusCode::SERVICE_UNAVAILABLE)
                            .map_err(|error| Exit::Fail(errors(error)))?;
                    return Ok(reply! { status(status); source(reply); });
                }
                let source = sampler.changes_with({
                    let inject = inject.clone();
                    move |error| inject(error)
                });
                let fragment = presentation.fragment.clone();
                let native_error = errors.clone();
                let view_policy = policy.clone();
                let interval = sampler.interval;
                let program = stream! {
                    source: source,
                    write: move |status, writer| {
                        let errors = native_error.clone();
                        let policy = view_policy.clone();
                        let fragment = fragment.clone();
                        let output = output.clone();
                        effect_future! {
                            exit_with_context; body: move |env: Arc<R>, context| {
                                let errors = errors.clone();
                                let policy = policy.clone();
                                let fragment = fragment.clone();
                                let output = output.clone();
                                let writer = writer.clone();
                                let status = status.clone();
                                async move {
                                    let reserved = match output.as_ref().map(|slots| slots.frames.try_checkout()).transpose() {
                                        Ok(reserved) => reserved,
                                        Err(error) => return Exit::Fail(errors(ServerError::Memory(error))),
                                    };
                                    let html = match effect::effect! {
                                        run_in_context;
                                        context: context,
                                        program: fragment(DashboardPage {
                                        snapshot: status.sample.clone(),
                                        degraded: status.failure.is_some(),
                                        path: policy.page.clone(),
                                        interval,
                                    }),
                                        environment: env.clone(),
                                    }.await.into_failure() {
                                        Ok(html) => html,
                                        Err(failure) => return failure,
                                    };
                                    let reply: Reply<E,R> = reply! { html(html) };
                                    let finite = match reply.into_finite(policy.html_bytes) {
                                        Ok(finite) => finite,
                                        Err(error) => return Exit::Fail(errors(error)),
                                    };
                                    let html = match std::str::from_utf8(&finite.bytes) {
                                        Ok(html) => html,
                                        Err(_) => return Exit::Fail(errors(ServerError::Invalid("diagnostic HTML UTF-8".into()))),
                                    };
                                    if let Some(buffer) = reserved {
                                        let view = match prepared_fragment(buffer, html, policy.sse) {
                                            Ok(view) => view,
                                            Err(error) => return Exit::Fail(errors(error)),
                                        };
                                        drop(finite);
                                        effect::effect! {
                                            run_in_context;
                                            context: context,
                                            program: effect_server::stream! { write_lease; writer: writer, view: view, },
                                            environment: env,
                                        }.await
                                    } else {
                                        let mut data = String::new();
                                        for (index, line) in html.lines().enumerate() {
                                            if index != 0 {
                                                data.push('\n');
                                            }
                                            data.push_str("elements ");
                                            data.push_str(line);
                                        }
                                        drop(finite);
                                        effect::effect! {
                                            run_in_context;
                                            context: context,
                                            program: effect_server::stream! {
                                                sse;
                                                writer: writer,
                                                record: SseRecord {
                                            event: Some("datastar-patch-elements".into()),
                                            id: None,
                                            retry: None,
                                            data,
                                        },
                                                policy: policy.sse,
                                            },
                                            environment: env,
                                        }.await
                                    }
                                }
                            },
                        }
                    },

                };
                Ok(reply! {
                    header(
                        http::header::CACHE_CONTROL,
                        http::HeaderValue::from_static("no-store"),
                    );
                    header(
                        http::header::VARY,
                        http::HeaderValue::from_static("Authorization, Cookie"),
                    );
                    sse(program);
                })
            }
            },
        };
        routes = router! {
            source: routes,
            methods(methods, policy.live.as_ref(), registered! {
                endpoint(live),
            }),
        }?;
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
    reserved: Option<BufferLease<'static>>,
) -> Result<Reply<E, R>, E> {
    let reply: Reply<E, R> = reply! { html(html) };
    let finite = reply.into_finite(bound).map_err(|error| errors(error))?;
    let reply = if let Some(mut output) = reserved {
        output.clear();
        output
            .extend_from_slice(&finite.bytes)
            .map_err(|error| errors(ServerError::Memory(error)))?;
        reply! { lease(BufferView::new(output.share()), http::HeaderValue::from_static("text/html; charset=utf-8")) }
    } else {
        reply! { finite(finite) }
    };
    Ok(reply! {
        header(
            http::header::CACHE_CONTROL,
            http::HeaderValue::from_static("no-store"),
        );
        header(
            http::header::VARY,
            http::HeaderValue::from_static("Authorization, Cookie"),
        );
        source(reply);
    })
}

fn prepared_fragment(
    mut buffer: BufferLease<'static>,
    html: &str,
    policy: SsePolicy,
) -> Result<BufferView<'static>, ServerError> {
    buffer.clear();
    let mut push = |bytes: &[u8]| {
        let end = buffer
            .len()
            .checked_add(bytes.len())
            .ok_or(ServerError::Memory(MemoryError::ArithmeticOverflow))?;
        if end > policy.frame_bytes.get() {
            return Err(ServerError::Limit("SSE frame".into()));
        }
        buffer.extend_from_slice(bytes).map_err(ServerError::Memory)
    };
    push(b"event: datastar-patch-elements\n")?;
    if html.is_empty() {
        push(b"data: \n")?;
    } else {
        for html_line in html.lines() {
            for (index, line) in html_line.split('\r').enumerate() {
                push(b"data: ")?;
                if index == 0 {
                    push(b"elements ")?;
                }
                push(line.as_bytes())?;
                push(b"\n")?;
            }
        }
    }
    push(b"\n")?;
    Ok(BufferView::new(buffer.share()))
}
