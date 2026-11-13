//! The access guard flow.
//!
//! The guard flow owns the per-request access decision: public-surface bypass,
//! principal resolution, rule evaluation, and the allowed/denied outcome with the
//! response selected for the denial.
//!
//! # Behavior
//!
//! - A managed-surface guard skips the check when the surface is not managed (the
//!   public bypass) and otherwise applies the rule.
//! - An explicitly supplied principal wins over the injected resolver, exactly as the
//!   A supplied principal returns its non-nil argument first.
//! - A missing principal produces the configured unauthenticated response; with
//!   redirect allowed that response is the localized-login redirect, otherwise it is
//!   the unauthorized renderer.
//! - An absent rule allows an authenticated principal (the default
//!   allow); a present rule decides.
//! - A denied principal produces the forbidden renderer. When a renderer is absent,
//!   the plain status-text fallback is used.
//! - No mutation happens on denial: the flow produces a response value only, and the
//!   caller performs no further work when the outcome does not proceed.
//!
//! # Ownership provides the one-shot disposition guarantee
//!
//! The outcome is a single owned value with private state: only this module constructs
//! it, its response can be taken once by consuming the outcome, and the allowed/bypassed
//! states carry no response. Replay or forgery is unrepresentable rather than tracked
//! with runtime flags or pointers.
//!
//! # Injected identity capability
//!
//! The principal is supplied by a required [`PrincipalResolver`] captured by the
//! guard, or by the explicit principal option. The owner holds no global identity
//! context, no account endpoint and no identity store; authentication remains the
//! consumer's, and an authenticated principal is never by itself treated as a
//! permission.

use std::sync::Arc;

use effect::{Effect, effect, effect_sync};
use effect_server::{BodyStatus, Reply, Request};
use http::StatusCode;

use super::principal::{AccessPrincipal, PrincipalAccessRule};
use super::responses::{AccessResponse, ManagedSurfaceAccessGuardOptions, fallback_status_reply};

/// Resolves the authenticated principal for one request.
///
/// The consumer supplies this capability from its own identity integration; the
/// resolver is a required argument of every guard, never an optional dependency, a
/// global or a runtime lookup.
pub type PrincipalResolver<E, R> =
    Arc<dyn Fn(&Request) -> Effect<Option<AccessPrincipal>, E, R> + Send + Sync + 'static>;

/// Configures one principal access check.
pub struct PrincipalAccessOptions<E: Send + 'static, R: Send + Sync + 'static> {
    /// Explicit principal, which wins over the resolver.
    pub principal: Option<AccessPrincipal>,
    /// Rule deciding access; absent means an authenticated principal is allowed.
    pub can_access: Option<PrincipalAccessRule>,
    /// Response for an unauthenticated principal.
    pub redirect_unauthenticated: Option<AccessResponse<E, R>>,
    /// Response for an unauthenticated principal when redirecting is not allowed.
    pub render_unauthorized: Option<AccessResponse<E, R>>,
    /// Response for a denied principal.
    pub render_forbidden: Option<AccessResponse<E, R>>,
}

impl<E: Send + 'static, R: Send + Sync + 'static> Default for PrincipalAccessOptions<E, R> {
    fn default() -> Self {
        Self {
            principal: None,
            can_access: None,
            redirect_unauthenticated: None,
            render_unauthorized: None,
            render_forbidden: None,
        }
    }
}

/// Configures a reusable principal access guard.
pub struct PrincipalAccessGuardOptions<E: Send + 'static, R: Send + Sync + 'static> {
    /// Rule deciding access; absent means an authenticated principal is allowed.
    pub can_access: Option<PrincipalAccessRule>,
    /// Response for an unauthenticated principal.
    pub redirect_unauthenticated: Option<AccessResponse<E, R>>,
    /// Response for an unauthenticated principal when redirecting is not allowed.
    pub render_unauthorized: Option<AccessResponse<E, R>>,
    /// Response for a denied principal.
    pub render_forbidden: Option<AccessResponse<E, R>>,
}

impl<E: Send + 'static, R: Send + Sync + 'static> Default for PrincipalAccessGuardOptions<E, R> {
    fn default() -> Self {
        Self {
            can_access: None,
            redirect_unauthenticated: None,
            render_unauthorized: None,
            render_forbidden: None,
        }
    }
}

/// The closed access outcome of one guard invocation.
///
/// The state is private: only this owner's flow constructs an outcome, and the
/// denial response can be taken exactly once by consuming it.
pub struct AccessOutcome<E: Send + 'static, R: Send + Sync + 'static> {
    state: AccessOutcomeState<E, R>,
}

/// The private outcome payload.
///
/// The denial variant owns the reply, which is larger than the two marker variants;
/// the outcome is moved once per check, so the size difference is not a cost worth an
/// extra allocation.
#[allow(clippy::large_enum_variant)]
enum AccessOutcomeState<E: Send + 'static, R: Send + Sync + 'static> {
    /// The rule admitted the principal.
    Allowed,
    /// The surface is public, so no rule ran.
    Bypassed,
    /// The principal was missing or denied, and this response is the outcome.
    Denied(Reply<E, R>),
}

impl<E, R> AccessOutcome<E, R>
where
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    /// Whether the rule admitted the principal.
    #[must_use]
    pub fn is_allowed(&self) -> bool {
        matches!(self.state, AccessOutcomeState::Allowed)
    }

    /// Whether the check was skipped because the surface is public.
    #[must_use]
    pub fn is_bypassed(&self) -> bool {
        matches!(self.state, AccessOutcomeState::Bypassed)
    }

    /// Whether the caller may continue with the protected work.
    #[must_use]
    pub fn proceeds(&self) -> bool {
        !matches!(self.state, AccessOutcomeState::Denied(_))
    }

    /// Take the denial response, when access was not granted.
    #[must_use]
    pub fn into_response(self) -> Option<Reply<E, R>> {
        match self.state {
            AccessOutcomeState::Denied(reply) => Some(reply),
            AccessOutcomeState::Allowed | AccessOutcomeState::Bypassed => None,
        }
    }

    /// An allowed outcome.
    fn allowed() -> Self {
        Self {
            state: AccessOutcomeState::Allowed,
        }
    }

    /// A bypassed outcome.
    fn bypassed() -> Self {
        Self {
            state: AccessOutcomeState::Bypassed,
        }
    }

    /// A denied outcome carrying the response.
    fn denied(reply: Reply<E, R>) -> Self {
        Self {
            state: AccessOutcomeState::Denied(reply),
        }
    }
}

/// A reusable principal access guard.
pub struct PrincipalAccessGuard<E: Send + 'static, R: Send + Sync + 'static> {
    options: PrincipalAccessGuardOptions<E, R>,
    resolver: PrincipalResolver<E, R>,
}

/// A reusable principal access requirement with a fixed redirect policy.
pub struct PrincipalAccessRequirement<E: Send + 'static, R: Send + Sync + 'static> {
    guard: PrincipalAccessGuard<E, R>,
    allow_redirect: bool,
}

/// A reusable managed-surface access guard.
pub struct ManagedSurfaceAccessGuard<E: Send + 'static, R: Send + Sync + 'static> {
    options: ManagedSurfaceAccessGuardOptions<E, R>,
    resolver: PrincipalResolver<E, R>,
}

/// The validated unauthorized status for the fallback reply.
fn unauthorized_status() -> BodyStatus {
    BodyStatus::new(StatusCode::UNAUTHORIZED).unwrap_or(BodyStatus::Ok)
}

/// The validated forbidden status for the fallback reply.
fn forbidden_status() -> BodyStatus {
    BodyStatus::new(StatusCode::FORBIDDEN).unwrap_or(BodyStatus::Ok)
}

/// Render the configured response, or the plain status-text fallback.
fn render_access_response<E, R>(
    request: &Request,
    response: &Option<AccessResponse<E, R>>,
    fallback: BodyStatus,
) -> Effect<Reply<E, R>, E, R>
where
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    match response {
        Some(render) => render(request),
        None => effect_sync! { body: move || fallback_status_reply(fallback), },
    }
}

/// Run one principal access check.
fn guard_principal_access<E, R>(
    request: &Request,
    options: PrincipalAccessOptions<E, R>,
    unauthenticated: Option<AccessResponse<E, R>>,
    resolver: &PrincipalResolver<E, R>,
) -> Effect<AccessOutcome<E, R>, E, R>
where
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    let request = request.clone();
    let explicit = options.principal.clone();
    let can_access = options.can_access.clone();
    let render_forbidden = options.render_forbidden.clone();
    let resolver = Arc::clone(resolver);
    effect! {
        // The factory closure is reusable, so every value a nested step closure
        // captures is first copied into a factory-local slot. A nested step then
        // takes the slot, never the factory's own required input.
        let request = request.clone();
        let can_access = can_access.clone();
        let resolver = resolver.clone();
        let unauthenticated = unauthenticated.clone();
        let render_forbidden = render_forbidden.clone();
        let resolver_for_lookup = resolver.clone();
        let request_for_lookup = request.clone();
        let can_access_for_rule = can_access.clone();
        let request_for_forbidden = request.clone();
        let forbidden_for_render = render_forbidden.clone();
        let request_for_unauthorized = request.clone();
        let unauthorized_for_render = unauthenticated.clone();
        let resolved = match explicit.as_ref() {
            Some(principal) => Some(principal.clone()),
            None => {
                let found = resolver_for_lookup(&request_for_lookup)?;
                found
            }
        };
        let outcome = if let Some(principal) = resolved {
            let allowed = match &can_access_for_rule {
                Some(rule) => rule(&principal),
                None => true,
            };
            if allowed {
                AccessOutcome::allowed()
            } else {
                let reply = render_access_response(
                    &request_for_forbidden,
                    &forbidden_for_render,
                    forbidden_status(),
                )?;
                AccessOutcome::denied(reply)
            }
        } else {
            let reply = render_access_response(
                &request_for_unauthorized,
                &unauthorized_for_render,
                unauthorized_status(),
            )?;
            AccessOutcome::denied(reply)
        };
        outcome
    }
}

/// Require a principal for a protected surface.
///
/// `allow_redirect` selects the localized-login redirect for an unauthenticated
/// principal; otherwise the unauthorized renderer is used.
pub fn require_principal_access<E, R>(
    request: &Request,
    allow_redirect: bool,
    options: PrincipalAccessOptions<E, R>,
    resolver: &PrincipalResolver<E, R>,
) -> Effect<AccessOutcome<E, R>, E, R>
where
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    let unauthenticated = if allow_redirect {
        options.redirect_unauthenticated.clone()
    } else {
        options.render_unauthorized.clone()
    };
    guard_principal_access(request, options, unauthenticated, resolver)
}

/// Require a principal for a managed surface.
///
/// A surface that is not managed is public and bypasses the check. A managed surface
/// always uses the configured unauthenticated (login redirect) response.
pub fn require_managed_principal_access<E, R>(
    request: &Request,
    manage: bool,
    options: PrincipalAccessOptions<E, R>,
    resolver: &PrincipalResolver<E, R>,
) -> Effect<AccessOutcome<E, R>, E, R>
where
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    if !manage {
        return effect_sync! { body: AccessOutcome::<E, R>::bypassed, };
    }
    let unauthenticated = options.redirect_unauthenticated.clone();
    guard_principal_access(request, options, unauthenticated, resolver)
}

/// Construct a reusable principal access guard.
#[must_use]
pub fn new_principal_access_guard<E: Send + 'static, R: Send + Sync + 'static>(
    options: PrincipalAccessGuardOptions<E, R>,
    resolver: PrincipalResolver<E, R>,
) -> PrincipalAccessGuard<E, R> {
    PrincipalAccessGuard { options, resolver }
}

/// Construct a reusable principal access requirement with a fixed redirect policy.
#[must_use]
pub fn new_principal_access_requirement<E: Send + 'static, R: Send + Sync + 'static>(
    options: PrincipalAccessGuardOptions<E, R>,
    allow_redirect: bool,
    resolver: PrincipalResolver<E, R>,
) -> PrincipalAccessRequirement<E, R> {
    PrincipalAccessRequirement {
        guard: new_principal_access_guard(options, resolver),
        allow_redirect,
    }
}

/// Construct a reusable managed-surface access guard.
#[must_use]
pub fn new_managed_surface_access_guard<E: Send + 'static, R: Send + Sync + 'static>(
    options: ManagedSurfaceAccessGuardOptions<E, R>,
    resolver: PrincipalResolver<E, R>,
) -> ManagedSurfaceAccessGuard<E, R> {
    ManagedSurfaceAccessGuard { options, resolver }
}

impl<E, R> PrincipalAccessGuard<E, R>
where
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    /// Require a principal for one request.
    pub fn require(
        &self,
        request: &Request,
        allow_redirect: bool,
    ) -> Effect<AccessOutcome<E, R>, E, R> {
        let options = PrincipalAccessOptions {
            principal: None,
            can_access: self.options.can_access.clone(),
            redirect_unauthenticated: self.options.redirect_unauthenticated.clone(),
            render_unauthorized: self.options.render_unauthorized.clone(),
            render_forbidden: self.options.render_forbidden.clone(),
        };
        require_principal_access(request, allow_redirect, options, &self.resolver)
    }
}

impl<E, R> PrincipalAccessRequirement<E, R>
where
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    /// Require a principal for one request under the configured redirect policy.
    pub fn require(&self, request: &Request) -> Effect<AccessOutcome<E, R>, E, R> {
        self.guard.require(request, self.allow_redirect)
    }
}

impl<E, R> ManagedSurfaceAccessGuard<E, R>
where
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    /// Require a principal for one managed surface.
    ///
    /// `manage` is the surface's management flag; `can_manage` is the resource's
    /// rule or an absent rule, which allows an authenticated principal;
    /// `render_key` selects the key-based forbidden renderer when one was configured.
    pub fn require(
        &self,
        request: &Request,
        manage: bool,
        can_manage: Option<&PrincipalAccessRule>,
        render_key: bool,
    ) -> Effect<AccessOutcome<E, R>, E, R> {
        let forbidden = if render_key {
            self.options
                .render_forbidden_key
                .clone()
                .or_else(|| self.options.render_forbidden.clone())
        } else {
            self.options.render_forbidden.clone()
        };
        let options = PrincipalAccessOptions {
            principal: None,
            // The optional `CanAccess` rule passes straight through, so an
            // absent rule keeps the authenticated default-allow branch: the surface
            // still requires a principal, but any authenticated principal proceeds.
            can_access: can_manage.map(Arc::clone),
            redirect_unauthenticated: self.options.redirect_unauthenticated.clone(),
            render_unauthorized: None,
            render_forbidden: forbidden,
        };
        require_managed_principal_access(request, manage, options, &self.resolver)
    }
}
