//! The management access policy.
//!
//! The policy owns the configured management roles, the per-resource/media/authoring
//! rules and the three guard requirements the rest of the CMS consumes.
//!
//! # Behavior
//!
//! - Blank configured keys fall back to the owner's default unauthorized/forbidden
//!   message keys.
//! - Resource rules are built per declared kind; an empty kind is skipped and a kind
//!   with no configured roles receives the empty role rule, which admits nobody.
//! - Media and authoring rules are built from their configured role lists.
//! - The media manager guard redirects an unauthenticated principal to the request
//!   login path, renders the localized unauthorized message for an unauthenticated
//!   principal when no redirect is possible, and renders the localized forbidden
//!   message for a denied one.
//! - The authoring translation requirement always redirects an unauthenticated
//!   principal to the language login path and renders only the forbidden message for
//!   a denied principal. No unauthorized renderer is supplied there and
//!   this port keeps that absent branch.
//! - The managed-surface guard shares the language login redirect and the forbidden
//!   responses, including the key-based one.
//!
//! # Adaptation recorded
//!
//! The queries owner's first-non-blank helper supplies the key
//! fallbacks, and this port keeps that dependency on the queries owner's public
//! function rather than duplicating the rule.

use std::collections::BTreeMap;
use std::sync::Arc;

use effect_server::BodyStatus;
use http::StatusCode;

use crate::definitions::ResourceKind;
use crate::localization::TranslateKeyFunc;
use crate::queries::first_non_blank;

use super::flow::{
    ManagedSurfaceAccessGuard, PrincipalAccessGuard, PrincipalAccessGuardOptions,
    PrincipalAccessRequirement, PrincipalResolver, new_managed_surface_access_guard,
    new_principal_access_guard, new_principal_access_requirement,
};
use super::principal::{
    DEFAULT_FORBIDDEN_MESSAGE_KEY, DEFAULT_UNAUTHORIZED_MESSAGE_KEY, PrincipalAccessRule,
    principal_has_any_role,
};
use super::responses::{
    AccessKeyRenderer, AccessMessageRenderer, AccessPathBuilder, ManagedSurfaceAccessGuardOptions,
    new_redirect_access_response, new_status_key_access_response,
    new_translated_status_access_response,
};
use super::work::PreparedAccessWork;

/// The configured management roles.
#[derive(Clone, Debug, Default)]
pub struct ManagementAccessRoles {
    /// Roles allowed to manage each resource kind.
    pub resources: BTreeMap<ResourceKind, Vec<String>>,
    /// Roles allowed to manage media.
    pub media: Vec<String>,
    /// Roles allowed to use authoring AI.
    pub authoring: Vec<String>,
}

/// Configures the management access policy.
pub struct ManagementAccessPolicyOptions<E: Send + 'static, R: Send + Sync + 'static> {
    /// Configured roles.
    pub roles: ManagementAccessRoles,
    /// Language login path builder.
    pub language_login_path: Option<AccessPathBuilder>,
    /// Request login path builder.
    pub request_login_path: Option<AccessPathBuilder>,
    /// Translation port used for the localized messages.
    ///
    /// The context carrier is the native request type: the port receives the
    /// request's own cancellation/extensions context.
    pub translate_key: Option<TranslateKeyFunc<effect_server::Request>>,
    /// Localized message renderer.
    pub render_error: Option<AccessMessageRenderer<E, R>>,
    /// Raw-key message renderer.
    pub render_error_key: Option<AccessKeyRenderer<E, R>>,
    /// Configured unauthorized key.
    pub unauthorized_key: String,
    /// Configured forbidden key.
    pub forbidden_key: String,
}

impl<E: Send + 'static, R: Send + Sync + 'static> Default for ManagementAccessPolicyOptions<E, R> {
    fn default() -> Self {
        Self {
            roles: ManagementAccessRoles::default(),
            language_login_path: None,
            request_login_path: None,
            translate_key: None,
            render_error: None,
            render_error_key: None,
            unauthorized_key: String::new(),
            forbidden_key: String::new(),
        }
    }
}

/// The management access policy.
pub struct ManagementAccessPolicy<E: Send + 'static, R: Send + Sync + 'static> {
    resource_rules: BTreeMap<ResourceKind, PrincipalAccessRule>,
    default_resource_rule: PrincipalAccessRule,
    can_manage_media: PrincipalAccessRule,
    can_use_authoring_ai: PrincipalAccessRule,
    require_media_manager: PrincipalAccessGuard<E, R>,
    require_authoring_translation_access: PrincipalAccessRequirement<E, R>,
    require_managed_surface_access: ManagedSurfaceAccessGuard<E, R>,
}

/// The validated redirect status for the fallback reply.
fn redirect_status() -> BodyStatus {
    BodyStatus::new(StatusCode::FOUND).unwrap_or(BodyStatus::Ok)
}

/// The validated unauthorized status.
fn unauthorized_status() -> BodyStatus {
    BodyStatus::new(StatusCode::UNAUTHORIZED).unwrap_or(BodyStatus::Ok)
}

/// The validated forbidden status.
fn forbidden_status() -> BodyStatus {
    BodyStatus::new(StatusCode::FORBIDDEN).unwrap_or(BodyStatus::Ok)
}

/// Build the per-kind role rules, skipping an empty kind.
fn resource_access_rules(
    resources: &BTreeMap<ResourceKind, Vec<String>>,
) -> BTreeMap<ResourceKind, PrincipalAccessRule> {
    let mut rules: BTreeMap<ResourceKind, PrincipalAccessRule> = BTreeMap::new();
    for (kind, roles) in resources {
        if kind.as_str().is_empty() {
            continue;
        }
        rules.insert(kind.clone(), principal_has_any_role(roles));
    }
    rules
}

/// Construct the management access policy.
#[must_use]
pub fn new_management_access_policy<E, R>(
    options: ManagementAccessPolicyOptions<E, R>,
    resolver: PrincipalResolver<E, R>,
) -> ManagementAccessPolicy<E, R>
where
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    let ManagementAccessPolicyOptions {
        roles,
        language_login_path,
        request_login_path,
        translate_key,
        render_error,
        render_error_key,
        unauthorized_key,
        forbidden_key,
    } = options;
    let unauthorized_key = first_non_blank(&[
        unauthorized_key,
        DEFAULT_UNAUTHORIZED_MESSAGE_KEY.to_owned(),
    ]);
    let forbidden_key = first_non_blank(&[forbidden_key, DEFAULT_FORBIDDEN_MESSAGE_KEY.to_owned()]);

    let redirect_to_language_login =
        new_redirect_access_response(language_login_path, redirect_status());
    let redirect_to_request_login =
        new_redirect_access_response(request_login_path, redirect_status());
    let render_unauthorized_message = new_translated_status_access_response(
        unauthorized_status(),
        unauthorized_key,
        translate_key.clone(),
        render_error.clone(),
    );
    let render_forbidden_message = new_translated_status_access_response(
        forbidden_status(),
        forbidden_key.clone(),
        translate_key,
        render_error,
    );
    let render_forbidden_key =
        new_status_key_access_response(forbidden_status(), forbidden_key, render_error_key);

    let resource_rules = resource_access_rules(&roles.resources);
    let can_manage_media = principal_has_any_role(&roles.media);
    let can_use_authoring_ai = principal_has_any_role(&roles.authoring);

    let require_media_manager = new_principal_access_guard(
        PrincipalAccessGuardOptions {
            can_access: Some(can_manage_media.clone()),
            redirect_unauthenticated: Some(redirect_to_request_login),
            render_unauthorized: Some(render_unauthorized_message),
            render_forbidden: Some(render_forbidden_message.clone()),
        },
        resolver.clone(),
    );
    let require_authoring_translation_access = new_principal_access_requirement(
        PrincipalAccessGuardOptions {
            can_access: Some(can_use_authoring_ai.clone()),
            redirect_unauthenticated: Some(redirect_to_language_login.clone()),
            render_unauthorized: None,
            render_forbidden: Some(render_forbidden_message.clone()),
        },
        true,
        resolver.clone(),
    );
    let require_managed_surface_access = new_managed_surface_access_guard(
        ManagedSurfaceAccessGuardOptions {
            redirect_unauthenticated: Some(redirect_to_language_login),
            render_forbidden: Some(render_forbidden_message),
            render_forbidden_key: Some(render_forbidden_key),
        },
        resolver,
    );

    ManagementAccessPolicy {
        resource_rules,
        default_resource_rule: principal_has_any_role(&[]),
        can_manage_media,
        can_use_authoring_ai,
        require_media_manager,
        require_authoring_translation_access,
        require_managed_surface_access,
    }
}

impl<E: Send + 'static, R: Send + Sync + 'static> ManagementAccessPolicy<E, R> {
    /// Select shared prepared admission for this policy's existing guards.
    #[must_use]
    pub fn with_prepared_work(mut self, work: Arc<PreparedAccessWork<E>>) -> Self {
        self.require_media_manager.prepared = Some(work.clone());
        self.require_authoring_translation_access.guard.prepared = Some(work.clone());
        self.require_managed_surface_access.prepared = Some(work);
        self
    }

    /// The rule for one resource kind.
    ///
    /// A kind with no configured roles receives the empty role rule, which admits
    /// nobody.
    #[must_use]
    pub fn can_manage_resource(&self, kind: &ResourceKind) -> PrincipalAccessRule {
        match self.resource_rules.get(kind) {
            Some(rule) => rule.clone(),
            None => self.default_resource_rule.clone(),
        }
    }

    /// The rule for media management.
    #[must_use]
    pub fn can_manage_media(&self) -> PrincipalAccessRule {
        self.can_manage_media.clone()
    }

    /// The rule for authoring AI use.
    #[must_use]
    pub fn can_use_authoring_ai(&self) -> PrincipalAccessRule {
        self.can_use_authoring_ai.clone()
    }

    /// The media manager guard.
    #[must_use]
    pub fn require_media_manager(&self) -> &PrincipalAccessGuard<E, R> {
        &self.require_media_manager
    }

    /// The authoring translation requirement.
    #[must_use]
    pub fn require_authoring_translation_access(&self) -> &PrincipalAccessRequirement<E, R> {
        &self.require_authoring_translation_access
    }

    /// The managed-surface guard.
    #[must_use]
    pub fn require_managed_surface_access(&self) -> &ManagedSurfaceAccessGuard<E, R> {
        &self.require_managed_surface_access
    }
}
