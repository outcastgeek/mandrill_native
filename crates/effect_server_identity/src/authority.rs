//! Privately constructed Identity authority bound to storage, freshness and versions.
//!
//! Persisted snapshots and arbitrary HTTP extensions are not authentication proof.
//! Trusted native verification precedes grant issuance; each mutation/emitted view
//! reloads current security state. Native manager workflows still span independent
//! acknowledged store transactions rather than one implicit business transaction.

use crate::{IdentityError, capabilities::NativeBackend};
use identity_rs::identity::{
    CompletedAuthentication,
    models::{ConcurrencyStamp, IdentityUser, RoleId, SecurityStamp, UserId},
};
use std::{future::Future, sync::Arc};
use time::{Duration, OffsetDateTime};

/// Explicit native UTC function selection; arbitrary environment captures do not
/// fit the upstream fn-pointer clock contract and are not bridged through globals.
#[derive(Clone, Copy)]
pub struct IdentityClock {
    /// The selected UTC callable retained by all compatible native components.
    utc_now: fn() -> OffsetDateTime,
}
impl IdentityClock {
    /// Select the compatible native UTC callable without reading the clock.
    pub const fn new(utc_now: fn() -> OffsetDateTime) -> Self {
        Self { utc_now }
    }
    /// Borrow the native-compatible callable for configured token/session services.
    #[must_use]
    pub const fn callable(self) -> fn() -> OffsetDateTime {
        self.utc_now
    }
    /// Read native UTC lazily at an actual operation boundary.
    #[must_use]
    pub fn now(self) -> OffsetDateTime {
        (self.utc_now)()
    }
}
/// Caller-selected finite grant lifetimes; no operational duration defaults exist.
#[derive(Clone, Copy)]
pub struct GrantPolicy {
    /// Maximum age of ordinary verified self authority.
    pub self_lifetime: Duration,
    /// Maximum age of fresh completed authentication for sensitive changes.
    pub recent_lifetime: Duration,
}
impl GrantPolicy {
    /// Reject nonpositive lifetimes and recent authority broader than ordinary authority.
    pub fn validate(self) -> Result<Self, IdentityError> {
        if self.self_lifetime <= Duration::ZERO
            || self.recent_lifetime <= Duration::ZERO
            || self.recent_lifetime > self.self_lifetime
        {
            return Err(IdentityError::Configuration("grant lifetimes"));
        }
        Ok(self)
    }
}
/// Private definition generation; identical pools do not make different definitions interchangeable.
pub(crate) struct AuthorityIdentity;
/// Shared immutable verified authority binding; sensitive native version accessors
/// require owned stamp conversion, not cloning account/request/credential payloads.
struct GrantBinding {
    /// Exact issuing definition generation.
    authority: Arc<AuthorityIdentity>,
    /// Stable native verified account identity.
    user: UserId,
    /// Native credential-invalidating security version.
    security: SecurityStamp,
    /// Native metadata revision at issuance, checked again before protected work.
    concurrency: ConcurrencyStamp,
    /// Explicit UTC issuance instant; a backwards clock does not extend authority.
    issued: OffsetDateTime,
    /// Caller-policy expiry bound.
    expires: OffsetDateTime,
}
/// Verified ordinary self authority. Clone retains the same binding and expiry;
/// it does not renew authentication or establish one-use protocol consumption.
#[derive(Clone)]
pub struct SelfGrant(Arc<GrantBinding>);
impl SelfGrant {
    /// Stable actor ID for application-owned permission/resource selection.
    #[must_use]
    pub fn user_id(&self) -> UserId {
        self.0.user
    }
    /// Original expiry; copying this service evidence cannot extend it.
    #[must_use]
    pub fn expires_at(&self) -> OffsetDateTime {
        self.0.expires
    }
}
/// Fresh completion required by sensitive self workflows; construction is private
/// to actual native sign-in completion and refresh cannot mint it.
pub struct RecentSelfGrant {
    /// Original verified self binding retained without renewing its proof.
    actor: SelfGrant,
    /// Additional tighter freshness bound selected by the application.
    expires: OffsetDateTime,
}
/// Server-origin/CSRF proof for one concrete native request. It remains paired
/// with current self authority and is rechecked with the selected native browser service.
#[derive(Clone)]
pub struct ProtectedRequestGrant {
    /// Verified current actor, distinct from browser context proof.
    actor: SelfGrant,
    /// Exact method/resource of the request authorized by native browser policy.
    method: http::Method,
    /// URI binding avoids reusing a proof at another application mutation path.
    uri: http::Uri,
}
/// Owned native pending primary evidence for one factor completion, never self authority.
/// No Clone/Serialize or public constructor can mint another pending completion.
pub struct PendingLoginGrant {
    /// Exact issuing definition generation.
    authority: Arc<AuthorityIdentity>,
    /// Native verified primary evidence; plaintext request claims never enter this slot.
    session: Arc<identity_rs::identity::SignInSession>,
    /// Expiry from explicit application grant policy.
    expires: OffsetDateTime,
    /// Original external evidence retained for token acknowledgment after factor completion.
    pub(crate) external: Option<VerifiedExternalGrant>,
}
impl PendingLoginGrant {
    /// Borrow the original native primary evidence without copying its account payload.
    pub(crate) fn native(
        &self,
    ) -> Result<&identity_rs::identity::PendingAuthentication, IdentityError> {
        self.session.pending().ok_or(IdentityError::Unauthorized)
    }
}
/// Owned enrollment authorization bound to sensitive self authority and one native
/// stored ceremony; native challenge consumption still establishes actual one-use.
pub struct EnrollmentGrant {
    /// Privately minted recent self authority.
    recent: RecentSelfGrant,
    /// Actual native ceremony identity, not a caller-supplied proof object.
    pub(crate) options: Arc<identity_rs::identity::passkeys::PublicKeyCredentialCreationOptions>,
}
/// Native trusted external verification receipt. It is primary evidence, not an MFA bypass.
pub struct VerifiedExternalGrant {
    /// Exact selected definition generation.
    authority: Arc<AuthorityIdentity>,
    /// Privately minted upstream receipt from its selected external verifier.
    pub(crate) evidence: identity_rs::identity::external::VerifiedExternalIdentity,
    /// Explicit native UTC deadline for consuming this proof in a workflow.
    expires: OffsetDateTime,
}
/// Trusted application decision at an exact permission-policy revision.
pub struct PermissionDecision {
    /// Explicit decision; absence/errors must deny rather than become an empty grant.
    pub allowed: bool,
    /// Application-owned permission-policy revision, revalidated at every protected action.
    pub revision: uuid::Uuid,
    /// Explicit application permission expiry, bounded again by actor expiry.
    pub expires_at: OffsetDateTime,
    /// Trusted application's exact native target/scope, never a request-supplied
    /// replacement ID paired with an independently authorized resource string.
    pub target: PermissionTarget,
    /// Exact native operation approved by the application, separate from its action vocabulary.
    pub operation: PermissionOperation,
}
/// Typed application-owned target for exact permission/administrative scope binding.
#[derive(Clone, PartialEq, Eq)]
pub enum PermissionTarget {
    /// One stable native account identity.
    Account(UserId),
    /// One stable native role identity.
    Role(RoleId),
    /// Explicit account query/creation scope, subject to per-record visibility.
    Accounts,
    /// Explicit role query/creation scope, subject to per-record visibility.
    Roles,
    /// Application-owned non-native resource identity; it cannot substitute for a native target.
    Application(Arc<str>),
}
/// Native operation binding prevents a valid read grant authorizing a different mutation.
/// Application policy still selects its own action/resource strings and revision.
#[derive(Clone, PartialEq, Eq)]
pub enum PermissionOperation {
    /// Emit finite account views under per-record visibility checks.
    AccountsRead,
    /// Execute an explicitly selected administrative account mutation.
    AccountMutation,
    /// Add a current account to the application-authorized role resource.
    MembershipAdd,
    /// Remove current membership in the application-authorized role resource.
    MembershipRemove,
    /// Emit finite role views under per-record visibility checks.
    RolesRead,
    /// Create a native role using selected validation and normalization.
    RoleCreate,
    /// Rename the exact native role while preserving stable identity.
    RoleRename,
    /// Delete the exact native role and its native membership/claim effects.
    RoleDelete,
    /// Add one claim to the exact native role.
    RoleClaimAdd,
    /// Remove one claim from the exact native role.
    RoleClaimRemove,
    /// Application-owned callback operation, requiring its exact selected name.
    Application(Arc<str>),
}
/// Private actor/action/resource/revision-bound application permission authority.
pub struct PermissionGrant {
    /// Original verified actor whose current security state is reloaded.
    actor: SelfGrant,
    /// Exact selected action, never a wildcard invented by the extension.
    action: Arc<str>,
    /// Exact application-owned resource descriptor.
    resource: Arc<str>,
    /// Selected permission-policy revision requiring fresh approval on use.
    revision: uuid::Uuid,
    /// Tighter actor/permission expiry bound.
    expires: OffsetDateTime,
    /// Exact selected native target/scope acknowledged by trusted application policy.
    target: PermissionTarget,
    /// Immutable native operation acknowledged by the application policy.
    operation: PermissionOperation,
}
impl PermissionGrant {
    /// Inspect the original immutable target binding without manufacturing a grant.
    #[must_use]
    pub fn target(&self) -> &PermissionTarget {
        &self.target
    }
    /// Inspect the exact operation without broadening private authority.
    #[must_use]
    pub fn operation(&self) -> &PermissionOperation {
        &self.operation
    }
}
/// Query scope retaining its approved permission, revalidated for each emitted view.
pub struct ScopeGrant {
    /// Exact actor/query action/resource/revision binding.
    permission: PermissionGrant,
}
impl ScopeGrant {
    /// Inspect the original selected query collection without broadening permission.
    #[must_use]
    pub fn target(&self) -> &PermissionTarget {
        self.permission.target()
    }
    /// Original operation selection used by query workflows.
    #[must_use]
    pub fn operation(&self) -> &PermissionOperation {
        self.permission.operation()
    }
}
/// Role-target authority retains both approved actor permission and current native role revision.
pub struct RoleGrant {
    /// Approved application actor/action/resource binding.
    permission: PermissionGrant,
    /// Stable role identity surviving rename.
    role: RoleId,
    /// Native role concurrency revision checked before administrative mutation.
    concurrency: ConcurrencyStamp,
}
/// Trusted grant issuer retaining one real SQLx backend, clock and explicit policy.
#[derive(Clone)]
pub struct IdentityAuthority<B: NativeBackend> {
    /// Exact selected native account/role persistence, never memory fallback.
    backend: B,
    /// Selected compatible UTC clock used by all grant checks.
    clock: IdentityClock,
    /// Validated explicit freshness policy.
    policy: GrantPolicy,
    /// Private owner generation retained by all grants issued by this definition.
    pub(crate) identity: Arc<AuthorityIdentity>,
}
impl<B: NativeBackend> IdentityAuthority<B> {
    /// Construct inert checked authority around the selected actual SQLx backend.
    pub fn new(
        backend: B,
        clock: IdentityClock,
        policy: GrantPolicy,
    ) -> Result<Self, IdentityError> {
        if backend
            .utc_clock()
            .is_some_and(|selected| !std::ptr::fn_addr_eq(selected, clock.callable()))
        {
            return Err(IdentityError::Configuration(
                "identity store/component UTC clock mismatch",
            ));
        }
        Ok(Self {
            backend,
            clock,
            policy: policy.validate()?,
            identity: Arc::new(AuthorityIdentity),
        })
    }
    /// Borrow the real backend for native interoperation and selected component factories.
    pub fn backend(&self) -> &B {
        &self.backend
    }
    /// Borrow the selected UTC capability without creating a default clock.
    pub fn clock(&self) -> IdentityClock {
        self.clock
    }
    /// Retain the same private grant owner while binding the whole invocation's
    /// actual native store diagnostic scope into selected HTTP/provider factories.
    pub(crate) fn with_backend(&self, backend: B) -> Self {
        Self {
            backend,
            clock: self.clock,
            policy: self.policy,
            identity: self.identity.clone(),
        }
    }
    /// Private issuance after a trusted native transport/provider invocation has
    /// verified this exact current aggregate; arbitrary public snapshots cannot call it.
    pub(crate) async fn verify_user_grant(
        &self,
        user: &IdentityUser,
    ) -> Result<SelfGrant, IdentityError> {
        let grant = self.bind(user)?;
        self.revalidate(&grant).await?;
        Ok(grant)
    }
    /// Retain original diagnostics for a current persisted account reload.
    async fn load_user(&self, id: &UserId) -> Result<Option<IdentityUser>, IdentityError> {
        let backend = self.backend.scoped();
        let result = backend
            .find_user_by_id(id)
            .await
            .map_err(IdentityError::Store);
        backend.capture_outcome(result)
    }
    /// Retain original diagnostics for a current persisted role reload.
    async fn load_role(
        &self,
        id: &RoleId,
    ) -> Result<Option<identity_rs::identity::models::IdentityRole>, IdentityError> {
        let backend = self.backend.scoped();
        let result = backend
            .find_role_by_id(id)
            .await
            .map_err(IdentityError::Store);
        backend.capture_outcome(result)
    }
    /// Mint an immutable binding only after a trusted native verifier has established identity.
    fn bind(&self, user: &IdentityUser) -> Result<SelfGrant, IdentityError> {
        let issued = self.clock.now();
        let expires = issued
            .checked_add(self.policy.self_lifetime)
            .ok_or(IdentityError::Configuration("grant expiry overflow"))?;
        Ok(SelfGrant(Arc::new(GrantBinding {
            authority: self.identity.clone(),
            user: user.id(),
            security: user.security_stamp().clone(),
            concurrency: user.concurrency_stamp().clone(),
            issued,
            expires,
        })))
    }
    /// Revalidate a verifier-produced native completion against currently persisted versions.
    pub async fn from_completed(
        &self,
        completed: &CompletedAuthentication,
    ) -> Result<SelfGrant, IdentityError> {
        let current = self
            .load_user(&completed.user().id())
            .await?
            .ok_or(IdentityError::Unauthorized)?;
        if current.security_stamp() != completed.user().security_stamp()
            || current.concurrency_stamp() != completed.user().concurrency_stamp()
        {
            return Err(IdentityError::StaleGrant);
        }
        self.bind(&current)
    }
    /// Native access-token validation reloads the account and invokes the selected
    /// provider/purpose before issuing self authority; arbitrary extensions are ignored.
    pub async fn bearer<P: identity_rs::identity::tokens::UserTokenProvider<IdentityUser>>(
        &self,
        provider: &P,
        token: &str,
    ) -> Result<Option<SelfGrant>, IdentityError> {
        let backend = self.backend.scoped();
        let result =
            identity_rs::identity::tokens::validate_access_token(&backend, provider, token)
                .await
                .map_err(IdentityError::Bearer);
        let user = backend.capture_outcome(result)?;
        let Some(user) = user else { return Ok(None) };
        let grant = self.bind(&user)?;
        self.revalidate(&grant).await?;
        Ok(Some(grant))
    }
    /// Recheck owner, expiry, clock direction, existence and both native versions
    /// immediately before mutation or each emitted authorized view.
    pub async fn revalidate(&self, grant: &SelfGrant) -> Result<IdentityUser, IdentityError> {
        if !Arc::ptr_eq(&self.identity, &grant.0.authority) {
            return Err(IdentityError::WrongConfiguration);
        }
        let now = self.clock.now();
        if now < grant.0.issued || now >= grant.0.expires {
            return Err(IdentityError::StaleGrant);
        }
        let current = self
            .load_user(&grant.0.user)
            .await?
            .ok_or(IdentityError::Unauthorized)?;
        if current.security_stamp() != &grant.0.security
            || current.concurrency_stamp() != &grant.0.concurrency
        {
            return Err(IdentityError::StaleGrant);
        }
        Ok(current)
    }
    /// Mint fresh sensitive authority only from this workflow's actual native completion.
    pub(crate) async fn recent_completion(
        &self,
        completed: &CompletedAuthentication,
    ) -> Result<RecentSelfGrant, IdentityError> {
        if completed.assurance() == identity_rs::identity::AuthenticationAssurance::Refresh {
            return Err(IdentityError::Unauthorized);
        }
        let actor = self.from_completed(completed).await?;
        let expires = actor
            .0
            .issued
            .checked_add(self.policy.recent_lifetime)
            .ok_or(IdentityError::Configuration("recent grant expiry overflow"))?;
        Ok(RecentSelfGrant { actor, expires })
    }
    /// Revalidate the tighter recent proof before a sensitive self operation.
    pub async fn revalidate_recent(
        &self,
        grant: &RecentSelfGrant,
    ) -> Result<IdentityUser, IdentityError> {
        if self.clock.now() >= grant.expires {
            return Err(IdentityError::StaleGrant);
        }
        self.revalidate(&grant.actor).await
    }
    /// Invoke native origin/session-bound CSRF validation after current self verification.
    pub async fn protect_request<
        Browser: identity_rs::identity::axum::browser::BrowserRequestSecurity,
    >(
        &self,
        actor: &SelfGrant,
        request: &effect_server::Request,
        browser: &Browser,
    ) -> Result<ProtectedRequestGrant, IdentityError> {
        self.revalidate(actor).await?;
        browser
            .authorize(request.headers())
            .await
            .map_err(IdentityError::Policy)?;
        Ok(ProtectedRequestGrant {
            actor: actor.clone(),
            method: request.method().clone(),
            uri: request.uri().clone(),
        })
    }
    /// Reject request-proof reuse at another target and revalidate actual native browser state.
    pub async fn revalidate_protected<
        Browser: identity_rs::identity::axum::browser::BrowserRequestSecurity,
    >(
        &self,
        grant: &ProtectedRequestGrant,
        request: &effect_server::Request,
        browser: &Browser,
    ) -> Result<IdentityUser, IdentityError> {
        if grant.method != *request.method() || grant.uri != *request.uri() {
            return Err(IdentityError::Unauthorized);
        }
        let user = self.revalidate(&grant.actor).await?;
        browser
            .authorize(request.headers())
            .await
            .map_err(IdentityError::Policy)?;
        Ok(user)
    }
    /// Invoke the selected application decision before privately minting exact permission authority.
    pub async fn permission<F, Fut>(
        &self,
        actor: &SelfGrant,
        action: Arc<str>,
        resource: Arc<str>,
        decide: F,
    ) -> Result<PermissionGrant, IdentityError>
    where
        F: FnOnce(UserId, Arc<str>, Arc<str>) -> Fut,
        Fut: Future<Output = Result<PermissionDecision, IdentityError>>,
    {
        self.revalidate(actor).await?;
        let decision = decide(actor.user_id(), action.clone(), resource.clone()).await?;
        let now = self.clock.now();
        if !decision.allowed || decision.expires_at <= now {
            return Err(IdentityError::Unauthorized);
        }
        Ok(PermissionGrant {
            actor: actor.clone(),
            action,
            resource,
            revision: decision.revision,
            expires: decision.expires_at.min(actor.0.expires),
            target: decision.target,
            operation: decision.operation,
        })
    }
    /// Revalidate actor/action/resource/current decision revision at the actual protected action.
    pub async fn revalidate_permission<F, Fut>(
        &self,
        grant: &PermissionGrant,
        action: &str,
        resource: &str,
        decide: F,
    ) -> Result<IdentityUser, IdentityError>
    where
        F: FnOnce(UserId, Arc<str>, Arc<str>) -> Fut,
        Fut: Future<Output = Result<PermissionDecision, IdentityError>>,
    {
        if action != grant.action.as_ref()
            || resource != grant.resource.as_ref()
            || self.clock.now() >= grant.expires
        {
            return Err(IdentityError::StaleGrant);
        }
        let user = self.revalidate(&grant.actor).await?;
        let decision = decide(user.id(), grant.action.clone(), grant.resource.clone()).await?;
        if !decision.allowed
            || decision.revision != grant.revision
            || decision.target != grant.target
            || decision.operation != grant.operation
            || decision.expires_at <= self.clock.now()
        {
            return Err(IdentityError::StaleGrant);
        }
        Ok(user)
    }
    /// Retain an approved query scope; each emitted record must revalidate its policy.
    pub fn scope(&self, permission: PermissionGrant) -> Result<ScopeGrant, IdentityError> {
        if !Arc::ptr_eq(&self.identity, &permission.actor.0.authority) {
            return Err(IdentityError::WrongConfiguration);
        }
        Ok(ScopeGrant { permission })
    }
    /// Revalidate one emitted view without keeping a SQL transaction open for its client.
    pub async fn revalidate_scope<F, Fut>(
        &self,
        scope: &ScopeGrant,
        decide: F,
    ) -> Result<IdentityUser, IdentityError>
    where
        F: FnOnce(UserId, Arc<str>, Arc<str>) -> Fut,
        Fut: Future<Output = Result<PermissionDecision, IdentityError>>,
    {
        self.revalidate_permission(
            &scope.permission,
            &scope.permission.action,
            &scope.permission.resource,
            decide,
        )
        .await
    }
    /// Bind approved role authority to the current stable ID/concurrency revision.
    pub async fn role(
        &self,
        permission: PermissionGrant,
        id: RoleId,
    ) -> Result<RoleGrant, IdentityError> {
        if !Arc::ptr_eq(&self.identity, &permission.actor.0.authority) {
            return Err(IdentityError::WrongConfiguration);
        }
        if permission.target() != &PermissionTarget::Role(id) {
            return Err(IdentityError::Unauthorized);
        }
        self.revalidate(&permission.actor).await?;
        let role = self
            .load_role(&id)
            .await?
            .ok_or(IdentityError::Unauthorized)?;
        Ok(RoleGrant {
            permission,
            role: id,
            concurrency: role.concurrency_stamp().clone(),
        })
    }
    /// Revalidate selected permission and native role revision immediately before administration.
    pub async fn revalidate_role<F, Fut>(
        &self,
        grant: &RoleGrant,
        operation: &PermissionOperation,
        decide: F,
    ) -> Result<identity_rs::identity::models::IdentityRole, IdentityError>
    where
        F: FnOnce(UserId, Arc<str>, Arc<str>) -> Fut,
        Fut: Future<Output = Result<PermissionDecision, IdentityError>>,
    {
        if grant.permission.operation() != operation {
            return Err(IdentityError::Unauthorized);
        }
        self.revalidate_permission(
            &grant.permission,
            &grant.permission.action,
            &grant.permission.resource,
            decide,
        )
        .await?;
        let role = self
            .load_role(&grant.role)
            .await?
            .ok_or(IdentityError::Unauthorized)?;
        if role.concurrency_stamp() != &grant.concurrency {
            return Err(IdentityError::StaleGrant);
        }
        Ok(role)
    }
    /// Mint owned pending authority only from native verified primary evidence.
    pub(crate) fn pending(
        &self,
        session: Arc<identity_rs::identity::SignInSession>,
    ) -> Result<PendingLoginGrant, IdentityError> {
        Ok(PendingLoginGrant {
            authority: self.identity.clone(),
            session,
            expires: self
                .clock
                .now()
                .checked_add(self.policy.recent_lifetime)
                .ok_or(IdentityError::Configuration("pending expiry overflow"))?,
            external: None,
        })
    }
    /// Revalidate a pending owner/version before the native factor service consumes it.
    pub(crate) async fn validate_pending(
        &self,
        pending: &PendingLoginGrant,
    ) -> Result<(), IdentityError> {
        if !Arc::ptr_eq(&self.identity, &pending.authority) {
            return Err(IdentityError::WrongConfiguration);
        }
        if self.clock.now() >= pending.expires {
            return Err(IdentityError::StaleGrant);
        }
        let user = self
            .load_user(&pending.native()?.user().id())
            .await?
            .ok_or(IdentityError::Unauthorized)?;
        if user.security_stamp() != pending.native()?.user().security_stamp()
            || user.concurrency_stamp() != pending.native()?.user().concurrency_stamp()
        {
            return Err(IdentityError::StaleGrant);
        }
        Ok(())
    }
    /// Bind a real native stored registration ceremony to privately minted recent authority.
    pub(crate) fn enrollment(
        &self,
        recent: RecentSelfGrant,
        options: Arc<identity_rs::identity::passkeys::PublicKeyCredentialCreationOptions>,
    ) -> EnrollmentGrant {
        EnrollmentGrant { recent, options }
    }
    /// Revalidate enrollment owner/freshness before native ceremony consumption and attestation validation.
    pub(crate) async fn validate_enrollment(
        &self,
        enrollment: &EnrollmentGrant,
    ) -> Result<IdentityUser, IdentityError> {
        self.revalidate_recent(&enrollment.recent).await
    }
    /// Invoke the selected native external verifier before minting primary external authority.
    pub async fn external<
        E: Sync + ?Sized,
        V: identity_rs::identity::external::ExternalIdentityVerifier<E>,
    >(
        &self,
        evidence: &E,
        verifier: &V,
    ) -> Result<Option<VerifiedExternalGrant>, IdentityError> {
        let proof =
            identity_rs::identity::external::VerifiedExternalIdentity::verify(evidence, verifier)
                .await
                .map_err(IdentityError::Native)?;
        let Some(evidence) = proof else {
            return Ok(None);
        };
        Ok(Some(VerifiedExternalGrant {
            authority: self.identity.clone(),
            evidence,
            expires: self
                .clock
                .now()
                .checked_add(self.policy.recent_lifetime)
                .ok_or(IdentityError::Configuration("external expiry overflow"))?,
        }))
    }
    /// Recheck external proof ownership/freshness before native admission and factor handling.
    pub(crate) fn validate_external(
        &self,
        grant: &VerifiedExternalGrant,
    ) -> Result<(), IdentityError> {
        if !Arc::ptr_eq(&self.identity, &grant.authority) {
            return Err(IdentityError::WrongConfiguration);
        }
        if self.clock.now() >= grant.expires {
            return Err(IdentityError::StaleGrant);
        }
        Ok(())
    }
}
