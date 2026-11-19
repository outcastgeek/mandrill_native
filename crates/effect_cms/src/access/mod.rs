//! Access: principal/role rules, management policy and guard outcomes.
//!
//! Access owns the principal each guard reads, the role-matching rule, the management
//! access policy, the supplied response renderers, the redirect/unauthorized/forbidden
//! response contracts, the public-surface bypass and the allowed/denied outcome of a
//! protected request.
//!
//! # API
//!
//! - [`new_management_access_policy`] builds the configured rules and the three guard
//!   requirements the rest of the CMS consumes; [`principal_has_any_role`] builds a
//!   role rule.
//! - [`require_principal_access`]/[`require_managed_principal_access`] and the
//!   reusable [`PrincipalAccessGuard`], [`PrincipalAccessRequirement`] and
//!   [`ManagedSurfaceAccessGuard`] run one check and return an [`AccessOutcome`].
//! - [`new_redirect_access_response`], [`new_translated_status_access_response`] and
//!   [`new_status_key_access_response`] build the response contracts.
//!
//! # Identity and authorization boundary
//!
//! The authenticated principal is supplied by the consumer through the required
//! [`PrincipalResolver`] capability (or as an explicit option value). This owner
//! creates no account endpoint, identity store,
//! session, token or permission record, and it never treats authentication as a
//! permission: a present principal is checked against the configured rule, absent
//! rules allow an authenticated principal only where the default allow
//! applies, and a managed surface's rule is supplied per request (the
//! nilable `CanAccess`), so the nil rule keeps that authenticated default-allow
//! branch.
//!
//! # Effects and transport
//!
//! Guard operations return lazy [`Effect`] values; the principal resolver and every
//! response renderer are effectful supplied callbacks under the caller's error family
//! and environment. A denial produces a response description and never mutates
//! anything. HTML authoring stays with the consumer's renderer and presentation
//! owner; built-in fallbacks are plain status text and the Maud redirect anchor.
//!
//! # Private implementation
//!
//! `principal`, `responses`, `flow` and `policy` are private implementation modules of
//! this owner: they hold the declarations and the mechanics, while this module is the
//! owner's deliberate public surface.

use effect::Effect;
use effect_server::Request;

mod flow;
mod policy;
mod principal;
mod responses;
mod work;

pub use self::flow::{
    AccessOutcome, ManagedSurfaceAccessGuard, PrincipalAccessGuard, PrincipalAccessGuardOptions,
    PrincipalAccessOptions, PrincipalAccessRequirement, PrincipalResolver,
    new_managed_surface_access_guard, new_principal_access_guard, new_principal_access_requirement,
    require_managed_principal_access, require_principal_access,
};
pub use self::policy::{
    ManagementAccessPolicy, ManagementAccessPolicyOptions, ManagementAccessRoles,
    new_management_access_policy,
};
pub use self::principal::{
    AccessPrincipal, DEFAULT_FORBIDDEN_MESSAGE_KEY, DEFAULT_UNAUTHORIZED_MESSAGE_KEY,
    PrincipalAccessRule, principal_has_any_role,
};
pub use self::responses::{
    AccessKeyRenderer, AccessMessageRenderer, AccessPathBuilder, AccessResponse,
    ManagedSurfaceAccessGuardOptions, new_redirect_access_response, new_status_key_access_response,
    new_translated_status_access_response,
};
pub(crate) use self::responses::{hex_escape_non_ascii, resolved_redirect_target};
pub use self::work::{AccessWorkLimits, PreparedAccessWork};

/// Compile-time witness that the guard flow keeps the owner's effect and outcome
/// shapes for any caller error family and environment.
///
/// The guards stay lazy [`Effect`] values and the denial response is taken from the
/// owned outcome.
const _: fn() = || {
    fn witness<E, R>()
    where
        E: Send + 'static,
        R: Send + Sync + 'static,
    {
        #[allow(clippy::type_complexity)]
        let _: fn(
            &Request,
            bool,
            PrincipalAccessOptions<E, R>,
            &PrincipalResolver<E, R>,
        ) -> Effect<AccessOutcome<E, R>, E, R> = require_principal_access::<E, R>;
        #[allow(clippy::type_complexity)]
        let _: fn(
            &Request,
            bool,
            PrincipalAccessOptions<E, R>,
            &PrincipalResolver<E, R>,
        ) -> Effect<AccessOutcome<E, R>, E, R> = require_managed_principal_access::<E, R>;
        #[allow(clippy::type_complexity)]
        let _: fn(AccessOutcome<E, R>) -> Option<effect_server::Reply<E, R>> =
            AccessOutcome::into_response;
    }
    witness::<std::fmt::Error, ()>();
};
