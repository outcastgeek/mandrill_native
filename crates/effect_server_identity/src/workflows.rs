//! Named lazy workflows over selected native manager interfaces.
//!
//! One workflow may acknowledge several native store transactions. Credential
//! algorithms, purpose validation and ceremony consumption remain native services;
//! application callbacks supply permission, delivery and public rendering policy.
//! Inputs move once, and no credential-bearing native aggregate is an HTTP view.

use crate::services::{
    AcceptedDocument, AccountSelection, AccountSnapshot, AccountSnapshotMetadata, AccountSnapshots,
    ConsentSelection, ConsentStore, DeliverySelection, DurableRequests, JobSelection,
    NotificationSelection, RequestDomain, SessionSelection, StoredSessions, VersionedDocument,
};
use crate::{
    IdentityError,
    authority::{
        EnrollmentGrant, PendingLoginGrant, PermissionDecision, PermissionGrant,
        PermissionOperation, PermissionTarget, RecentSelfGrant, RoleGrant, ScopeGrant, SelfGrant,
        VerifiedExternalGrant,
    },
    capabilities::StoreSelection,
    components::{BearerIssuance, BrowserComponents, PrincipalComponents, SessionIssuance},
    definition::{
        IdentityComponents, IdentityDefinition, LoginInput, LoginOutcome, RegistrationInput,
        complete_login,
    },
    policies::{
        AuthenticatorSelection, BrowserSelection, EmailSelection, ExternalSelection,
        PasskeySelection, PrincipalSelection, TicketSelection, TokenSelection,
    },
};
use crate::{effect_map, identity};
use effect::Effect;
use identity_rs::identity::{
    IdentityResult,
    axum::browser::BrowserRequestSecurity,
    manager::*,
    models::{
        AuthenticatorKey, EmailAddress, IdentityClaim, IdentityRole, IdentityUser, LoginProvider,
        PasskeyCredentialId, PasskeyName, PhoneNumber, ProviderKey, RecoveryCode, RoleId, UserId,
        UserName,
    },
    passkeys::{
        CollectedClientData, PasskeyAssertionResponse, PasskeyPolicy, PasskeyRegistrationResponse,
        PublicKeyCredentialCreationOptions, PublicKeyCredentialRequestOptions,
    },
    ticket::CookieSessionService,
    tokens::{TokenProviderName, UserTokenProvider, UserTokenProviderRegistry},
};
use std::{
    future::Future,
    num::{NonZeroU16, NonZeroUsize},
    sync::Arc,
};

/// Associated configured native account service, without requiring a built-in manager.
type Users<N> = <N as SignInService>::UserManager;

/// Exact administrative operation and application permission action/resource.
/// These caller-owned values are revalidated against the supplied grant before use.
pub struct AccountAction {
    /// Typed operation that the privately minted grant must authorize.
    pub operation: PermissionOperation,
    /// Application action supplied unchanged to its permission decision callback.
    pub action: Arc<str>,
    /// Application resource supplied unchanged to its permission decision callback.
    pub resource: Arc<str>,
}

/// Deliberately small public account representation with no credential/security bytes.
#[derive(Debug, serde::Serialize)]
pub struct AccountView {
    /// Stable ID for application routing and ownership checks.
    pub id: UserId,
    /// Optional public display name selected from current persisted state.
    pub user_name: Option<String>,
    /// Contact visible only to the authorized self or administrator invoking the view.
    pub email: Option<String>,
    /// Native acknowledgment of email confirmation.
    pub email_confirmed: bool,
    /// Optional authorized account contact.
    pub phone_number: Option<String>,
    /// Native acknowledgment of phone confirmation.
    pub phone_number_confirmed: bool,
    /// Public factor configuration, never its secret or recovery hashes.
    pub two_factor_enabled: bool,
}
impl AccountView {
    /// Convert only explicitly listed public fields; no security persistence codec is used.
    #[must_use]
    pub fn from_user(user: &IdentityUser) -> Self {
        Self {
            id: user.id(),
            user_name: user.user_name().map(|value| value.as_str().to_owned()),
            email: user.email().map(|value| value.as_str().to_owned()),
            email_confirmed: user.email_confirmed(),
            phone_number: user.phone_number().map(|value| value.as_str().to_owned()),
            phone_number_confirmed: user.phone_number_confirmed(),
            two_factor_enabled: user.two_factor_enabled(),
        }
    }
}
/// Bounded authorized enumeration result, detached from its completed database read.
#[derive(Debug, serde::Serialize)]
pub struct AccountViewPage {
    /// Only records whose application-owned per-record policy permitted emission.
    pub accounts: Vec<AccountView>,
    /// Stable exclusive native continuation, including filtered records.
    pub next: Option<UserId>,
}
/// Public role view excludes concurrency/security persistence representation.
#[derive(Debug, serde::Serialize)]
pub struct RoleView {
    /// Stable native role identity.
    pub id: RoleId,
    /// Authorized display label only.
    pub name: Option<String>,
}
impl RoleView {
    /// Convert only selected public native role fields.
    #[must_use]
    pub fn from_role(role: &IdentityRole) -> Self {
        Self {
            id: role.id(),
            name: role.name().map(|name| name.as_str().to_owned()),
        }
    }
}
/// Finite role page with detached authorized views.
#[derive(Debug, serde::Serialize)]
pub struct RoleViewPage {
    /// Current authorized role views.
    pub roles: Vec<RoleView>,
    /// Exclusive stable native continuation across filtered records.
    pub next: Option<RoleId>,
}
/// Secret password change input, owned by one execution.
pub struct PasswordChange {
    /// Private recent verification, minted by a real native completed sign-in.
    pub grant: RecentSelfGrant,
    /// Existing secret verified by the selected native password manager.
    pub current_password: String,
    /// New secret validated/hashed by the retained native policy.
    pub new_password: String,
}
/// Purpose-bound password recovery completion; caller ID is lookup, never proof.
pub struct PasswordRecovery {
    /// Typed account lookup identity.
    pub user_id: UserId,
    /// Native reset-purpose token verified before any credential mutation.
    pub token: String,
    /// New selected-policy password.
    pub new_password: String,
}
/// One owned pending primary proof and authenticator answer.
pub struct AuthenticatorCompletion {
    /// Cannot be constructed from an arbitrary user/ticket by HTTP callers.
    pub pending: PendingLoginGrant,
    /// Submitted code; native selected provider performs actual verification.
    pub code: String,
    /// Explicit session persistence preference.
    pub persistent: bool,
}
/// One owned pending primary proof and native one-use recovery answer.
pub struct RecoveryCodeCompletion {
    /// Real primary proof from the native sign-in service.
    pub pending: PendingLoginGrant,
    /// Validated native recovery code; storage consumes its hash atomically.
    pub code: RecoveryCode,
    /// Explicit session persistence preference.
    pub persistent: bool,
}
/// One selected named factor answer bound to real native primary evidence.
pub struct RegistryFactorCompletion {
    /// Privately minted primary proof, with current owner/version and freshness checks.
    pub pending: PendingLoginGrant,
    /// Typed selected registry key; unknown or ineligible providers fail natively.
    pub provider: TokenProviderName,
    /// Original submitted answer, moved once without Debug/Clone.
    pub code: String,
    /// Explicit selected session persistence preference.
    pub persistent: bool,
}
/// Enrollment response tied to the original stored ceremony and recent verification.
pub struct PasskeyEnrollment {
    /// Private recent/ceremony binding returned by begin_passkey_enrollment.
    pub grant: EnrollmentGrant,
    /// Original browser bytes moved into one native attestation operation.
    pub response: PasskeyRegistrationResponse,
    /// Optional caller-selected display label, not verification policy.
    pub name: Option<PasskeyName>,
}
/// Native acknowledgment expressed in the normal typed error channel.
fn acknowledged(result: IdentityResult) -> Result<(), IdentityError> {
    if result.succeeded() {
        Ok(())
    } else {
        Err(IdentityError::Native(result))
    }
}
/// Reload through the selected native account service, including selected personal
/// data protection. Authority's raw security metadata is not a replacement for a
/// decrypted manager aggregate and must never overwrite encrypted contact fields.
async fn current_user<U: UserLifecycleManager>(
    manager: &U,
    verified: IdentityUser,
) -> Result<IdentityUser, IdentityError> {
    let current = manager
        .find_by_id(&verified.id())
        .await
        .map_err(IdentityError::Native)?
        .ok_or(IdentityError::Unauthorized)?;
    if current.security_stamp() != verified.security_stamp()
        || current.concurrency_stamp() != verified.concurrency_stamp()
    {
        return Err(IdentityError::StaleGrant);
    }
    Ok(current)
}

/// Native principal adapter borrowing the whole invocation's selected manager.
struct ScopedPrincipal<'a, U, P> {
    /// Invocation-owned current native account source.
    users: &'a U,
    /// Retained selected native claim mapping and factory.
    policy: &'a P,
}
impl<U: Send + Sync, P: PrincipalComponents<U>>
    identity_rs::identity::principal::UserClaimsPrincipalFactory<IdentityUser>
    for ScopedPrincipal<'_, U, P>
{
    /// Delegate current native principal construction to the retained configured source.
    async fn create(
        &self,
        user: &IdentityUser,
    ) -> Result<identity_rs::identity::principal::ClaimsPrincipal, IdentityResult> {
        self.policy.create(self.users, user).await
    }
}
/// Forces current native stamp validation rather than accepting a periodic skip.
struct CurrentTicketValidator<'a, U, P> {
    /// Current persisted user/security-stamp source for this invocation.
    users: &'a U,
    /// Original selected claim mapping and native principal factory.
    policy: &'a P,
    /// Selected native UTC callable used for strict ticket/stamp validation.
    clock: crate::authority::IdentityClock,
}
impl<
    U: identity_rs::identity::principal::security_stamp::SecurityStampUserAccessor,
    P: PrincipalComponents<U>,
> identity_rs::identity::ticket::AuthenticationTicketValidator
    for CurrentTicketValidator<'_, U, P>
{
    /// Require explicit valid ticket UTC bounds and current native persisted security-stamp verification.
    async fn validate_ticket(
        &self,
        ticket: identity_rs::identity::ticket::AuthenticationTicket,
    ) -> Result<
        identity_rs::identity::ticket::TicketValidationOutcome,
        identity_rs::identity::ticket::TicketError,
    > {
        use identity_rs::identity::principal::security_stamp::{
            SecurityStampValidationContext, SecurityStampValidationOutcome, SecurityStampValidator,
            SecurityStampValidatorOptions,
        };
        use identity_rs::identity::ticket::{TicketError, TicketValidationOutcome};
        let now = self.clock.now();
        if ticket
            .properties()
            .issued_utc()
            .is_none_or(|issued| issued > now)
            || ticket
                .properties()
                .expires_utc()
                .is_none_or(|expires| expires <= now)
        {
            return Ok(TicketValidationOutcome::Rejected);
        }
        let validator = SecurityStampValidator::new(
            self.users,
            ScopedPrincipal {
                users: self.users,
                policy: self.policy,
            },
        )
        .with_options(SecurityStampValidatorOptions {
            validation_interval: std::time::Duration::ZERO,
            utc_now: self.clock.callable(),
        })
        .with_claims_options(self.policy.claims_options().clone());
        // Omitting issued_utc explicitly forces the native persisted-user lookup.
        match validator
            .validate(SecurityStampValidationContext::new(ticket.principal()))
            .await
            .map_err(|_| TicketError::ValidationFailed)?
        {
            SecurityStampValidationOutcome::Renewed(principal) => Ok(
                TicketValidationOutcome::Accepted(ticket.with_principal(principal)),
            ),
            SecurityStampValidationOutcome::Rejected => Ok(TicketValidationOutcome::Rejected),
            SecurityStampValidationOutcome::Unchanged => Err(TicketError::ValidationFailed),
        }
    }
}
/// Trusted native cookie verification result; only accepted current state carries authority.
pub enum CookieAuthentication {
    /// Selected cookie absent.
    Missing,
    /// Native format/current-state verification rejected it and requires removal.
    Rejected { removal: http::HeaderValue },
    /// Trusted native accepted ticket and privately issued current self grant.
    Authenticated {
        ticket: Arc<identity_rs::identity::ticket::AuthenticationTicket>,
        actor: SelfGrant,
        renewal: Option<http::HeaderValue>,
    },
}
/// Native secret credentials plus explicit selected transport lifetime. Debug is
/// unavailable and serialization is intended only for an authorized bearer response.
pub struct IssuedBearer {
    /// Original native access/refresh credentials, moved once without secret cloning.
    tokens: identity_rs::identity::tokens::BearerTokenSet,
    /// Explicit selected transport advertisement aligned with native provider expiry.
    lifetime: std::num::NonZeroU64,
}
impl serde::Serialize for IssuedBearer {
    /// Serialize only the explicitly selected bearer response fields, borrowing native credential bytes.
    fn serialize<T: serde::Serializer>(&self, serializer: T) -> Result<T::Ok, T::Error> {
        use serde::ser::SerializeStruct;
        let mut value = serializer.serialize_struct("IssuedBearer", 4)?;
        value.serialize_field("token_type", self.tokens.token_type())?;
        value.serialize_field("access_token", self.tokens.access_token())?;
        value.serialize_field("expires_in", &self.lifetime.get())?;
        value.serialize_field("refresh_token", self.tokens.refresh_token())?;
        value.end()
    }
}
/// Construct selected native principal/properties only after current verification.
async fn session_ticket<U: Send + Sync, P: PrincipalComponents<U>, T: SessionIssuance>(
    users: &U,
    user: &IdentityUser,
    original: &identity_rs::identity::ticket::AuthenticationTicket,
    principal: &P,
    settings: &T,
    clock: crate::authority::IdentityClock,
) -> Result<identity_rs::identity::ticket::AuthenticationTicket, IdentityError> {
    let claims = principal
        .create(users, user)
        .await
        .map_err(IdentityError::Native)?;
    let issued = clock.now();
    let expires = issued
        .checked_add(settings.lifetime())
        .ok_or(IdentityError::Configuration("session expiry overflow"))?;
    Ok(
        identity_rs::identity::ticket::AuthenticationTicket::with_scheme(
            claims,
            original.authentication_scheme(),
        )
        .with_properties(
            identity_rs::identity::ticket::AuthenticationProperties::new()
                .with_issued_utc(issued)
                .with_expires_utc(expires)
                .with_is_persistent(original.properties().is_persistent())
                .with_allow_refresh(settings.allow_refresh()),
        ),
    )
}

impl<S: StoreSelection, C, P, M> IdentityDefinition<S, C, P, M>
where
    C: IdentityComponents<S::Backend, P>,
    P: Send + Sync + 'static,
    M: Send + Sync + 'static,
{
    /// Register through selected native validation, asynchronous password work and
    /// actual configured store. Registration itself never mints authentication.
    pub fn register<E, R>(&self, input: RegistrationInput) -> Effect<AccountView, E, R>
    where
        E: Send + 'static,
        R: Send + Sync + 'static,
        M: Fn(IdentityError) -> E,
        C::Native: SignInService,
        Users<C::Native>: UserPasswordManager,
    {
        identity! {
        identity: self, invoke_once(input), native: |native, input, _, _| async move {
                    let user = native
                        .user_manager()
                        .create_with_password(input.user, &input.password)
                        .await
                        .map_err(IdentityError::Native)?;
                    Ok(AccountView::from_user(&user))
                },
        }
    }
    /// Resolve selected normalization, verify password, enforce admission/lockout
    /// and preserve rejected/pending/completed native outcomes without bypassing MFA.
    pub fn login<E, R>(&self, input: LoginInput) -> Effect<LoginOutcome, E, R>
    where
        E: Send + 'static,
        R: Send + Sync + 'static,
        M: Fn(IdentityError) -> E,
        C::Native: PasswordSignInManager,
    {
        identity! {
        identity: self, invoke_once(input), native: |native, input, authority, _| async move {
                    let session = native
                        .password_sign_in_session_by_name(
                            &input.user_name,
                            &input.password,
                            input.persistent,
                            input.lockout_on_failure,
                        )
                        .await
                        .map_err(IdentityError::Native)?;
                    complete_login(&authority, session).await
                },
        }
    }
    /// Reload current version-bound self state immediately before emitting its safe view.
    pub fn account<E, R>(&self, grant: SelfGrant) -> Effect<AccountView, E, R>
    where
        E: Send + 'static,
        R: Send + Sync + 'static,
        M: Fn(IdentityError) -> E,
        C::Native: SignInService,
        Users<C::Native>: UserLifecycleManager,
    {
        identity! {
        identity: self, invoke_once(grant), native: |native, grant, authority, _| async move {
                    let user =
                        current_user(native.user_manager(), authority.revalidate(&grant).await?).await?;
                    Ok(AccountView::from_user(&user))
                },
        }
    }
    /// Change a password after recent verification, with native old-secret validation
    /// and expected-version mutation. The returned acknowledgment invalidates old grants.
    pub fn change_password<E, R>(&self, input: PasswordChange) -> Effect<AccountView, E, R>
    where
        E: Send + 'static,
        R: Send + Sync + 'static,
        M: Fn(IdentityError) -> E,
        C::Native: SignInService,
        Users<C::Native>: UserPasswordManager,
    {
        identity! {
        identity: self, invoke_once(input), native: |native, input, authority, _| async move {
                    let user = current_user(
                        native.user_manager(),
                        authority.revalidate_recent(&input.grant).await?,
                    )
                    .await?;
                    let user = native
                        .user_manager()
                        .change_password(user, &input.current_password, &input.new_password)
                        .await
                        .map_err(IdentityError::Native)?;
                    Ok(AccountView::from_user(&user))
                },
        }
    }
    /// Remove the password only with recent self proof; native expected-version
    /// security invalidation remains authoritative.
    pub fn remove_password<E, R>(&self, grant: RecentSelfGrant) -> Effect<AccountView, E, R>
    where
        E: Send + 'static,
        R: Send + Sync + 'static,
        M: Fn(IdentityError) -> E,
        C::Native: SignInService,
        Users<C::Native>: UserPasswordManager,
    {
        identity! {
        identity: self, invoke_once(grant), native: |native, grant, authority, _| async move {
                    let user = current_user(
                        native.user_manager(),
                        authority.revalidate_recent(&grant).await?,
                    )
                    .await?;
                    let user = native
                        .user_manager()
                        .remove_password(user)
                        .await
                        .map_err(IdentityError::Native)?;
                    Ok(AccountView::from_user(&user))
                },
        }
    }
    /// Generate reset-purpose evidence through the selected D18 token provider.
    /// Trusted delivery adapters consume this secret result; HTTP renderers must
    /// apply their anti-enumeration policy and never expose it in a public response.
    pub fn password_recovery_token<E, R>(&self, user_id: UserId) -> Effect<Option<String>, E, R>
    where
        E: Send + 'static,
        R: Send + Sync + 'static,
        M: Fn(IdentityError) -> E,
        C::Native: SignInService,
        Users<C::Native>: UserTokenManager,
        P: TokenSelection,
        P::Provider: UserTokenProvider<IdentityUser>,
    {
        identity! {
        identity: self, invoke_once(user_id), native: |native, id, _, policies| async move {
                    let provider = policies.token_provider()?;
                    let Some(user) = native
                        .user_manager()
                        .find_by_id(&id)
                        .await
                        .map_err(IdentityError::Native)?
                    else {
                        return Ok(None);
                    };
                    native
                        .user_manager()
                        .generate_password_reset_token(&user, provider)
                        .await
                        .map(Some)
                        .map_err(IdentityError::Native)
                },
        }
    }
    /// Verify the selected native reset purpose before changing persisted credentials.
    pub fn recover_password<E, R>(&self, input: PasswordRecovery) -> Effect<AccountView, E, R>
    where
        E: Send + 'static,
        R: Send + Sync + 'static,
        M: Fn(IdentityError) -> E,
        C::Native: SignInService,
        Users<C::Native>: UserTokenManager,
        P: TokenSelection,
        P::Provider: UserTokenProvider<IdentityUser>,
    {
        identity! {
        identity: self, invoke_once(input), native: |native, input, _, policies| async move {
                    let provider = policies.token_provider()?;
                    let user = native
                        .user_manager()
                        .find_by_id(&input.user_id)
                        .await
                        .map_err(IdentityError::Native)?
                        .ok_or(IdentityError::Unauthorized)?;
                    let user = native
                        .user_manager()
                        .reset_password(user, provider, &input.token, &input.new_password)
                        .await
                        .map_err(IdentityError::Native)?;
                    Ok(AccountView::from_user(&user))
                },
        }
    }
    /// Complete MFA with current owner/version-bound primary evidence and the
    /// authenticator provider retained in the selected native composition.
    pub fn complete_authenticator<E, R>(
        &self,
        input: AuthenticatorCompletion,
    ) -> Effect<LoginOutcome, E, R>
    where
        E: Send + 'static,
        R: Send + Sync + 'static,
        M: Fn(IdentityError) -> E,
        C::Native: TwoFactorAuthenticatorSignInManager,
    {
        identity! {
        identity: self, invoke_once(input), native: |native, input, authority, _| async move {
                    authority.validate_pending(&input.pending).await?;
                    if input.pending.external.is_some() {
                        return Err(IdentityError::Configuration(
                            "external factor completion requires its token acknowledgment workflow",
                        ));
                    }
                    let session = native
                        .two_factor_authenticator_sign_in_session(
                            input.pending.native()?,
                            &input.code,
                            input.persistent,
                        )
                        .await
                        .map_err(IdentityError::Native)?;
                    complete_login(&authority, session).await
                },
        }
    }
    /// Redeem one native recovery hash before completing authentication. No cloned
    /// pending receipt can establish database one-use semantics.
    pub fn complete_recovery_code<E, R>(
        &self,
        input: RecoveryCodeCompletion,
    ) -> Effect<LoginOutcome, E, R>
    where
        E: Send + 'static,
        R: Send + Sync + 'static,
        M: Fn(IdentityError) -> E,
        C::Native: TwoFactorRecoveryCodeSignInManager,
    {
        identity! {
        identity: self, invoke_once(input), native: |native, input, authority, _| async move {
                    authority.validate_pending(&input.pending).await?;
                    if input.pending.external.is_some() {
                        return Err(IdentityError::Configuration(
                            "external factor completion requires its token acknowledgment workflow",
                        ));
                    }
                    let session = native
                        .two_factor_recovery_code_sign_in_session(
                            input.pending.native()?,
                            input.code.as_str(),
                            input.persistent,
                        )
                        .await
                        .map_err(IdentityError::Native)?;
                    complete_login(&authority, session).await
                },
        }
    }
    /// Complete an external primary proof with the selected native authenticator;
    /// acknowledge the original verified provider tokens only after actual completion.
    pub fn complete_external_authenticator<E, R>(
        &self,
        input: AuthenticatorCompletion,
    ) -> Effect<LoginOutcome, E, R>
    where
        E: Send + 'static,
        R: Send + Sync + 'static,
        M: Fn(IdentityError) -> E,
        C::Native: TwoFactorAuthenticatorSignInManager + ExternalLoginSignInManager,
    {
        identity! {
        identity: self, invoke_once(input), native: |native, mut input, authority, _| async move {
                    authority.validate_pending(&input.pending).await?;
                    let external = input
                        .pending
                        .external
                        .take()
                        .ok_or(IdentityError::Unauthorized)?;
                    authority.validate_external(&external)?;
                    let session = native
                        .two_factor_authenticator_sign_in_session(
                            input.pending.native()?,
                            &input.code,
                            input.persistent,
                        )
                        .await
                        .map_err(IdentityError::Native)?;
                    if session.completed().is_some() {
                        acknowledged(
                            native
                                .update_external_authentication_tokens(external.evidence.info())
                                .await,
                        )?
                    }
                    complete_login(&authority, session).await
                },
        }
    }
    /// Consume a native one-use recovery code for external primary evidence and
    /// acknowledge its original provider tokens only on completed authentication.
    pub fn complete_external_recovery_code<E, R>(
        &self,
        input: RecoveryCodeCompletion,
    ) -> Effect<LoginOutcome, E, R>
    where
        E: Send + 'static,
        R: Send + Sync + 'static,
        M: Fn(IdentityError) -> E,
        C::Native: TwoFactorRecoveryCodeSignInManager + ExternalLoginSignInManager,
    {
        identity! {
        identity: self, invoke_once(input), native: |native, mut input, authority, _| async move {
                    authority.validate_pending(&input.pending).await?;
                    let external = input
                        .pending
                        .external
                        .take()
                        .ok_or(IdentityError::Unauthorized)?;
                    authority.validate_external(&external)?;
                    let session = native
                        .two_factor_recovery_code_sign_in_session(
                            input.pending.native()?,
                            input.code.as_str(),
                            input.persistent,
                        )
                        .await
                        .map_err(IdentityError::Native)?;
                    if session.completed().is_some() {
                        acknowledged(
                            native
                                .update_external_authentication_tokens(external.evidence.info())
                                .await,
                        )?
                    }
                    complete_login(&authority, session).await
                },
        }
    }
    /// Verify the exact named provider from the retained native registry, then
    /// reapply current native admission/factor policy before returning completion.
    pub fn complete_registry_factor<E, R>(
        &self,
        input: RegistryFactorCompletion,
    ) -> Effect<LoginOutcome, E, R>
    where
        E: Send + 'static,
        R: Send + Sync + 'static,
        M: Fn(IdentityError) -> E,
        C::Native: TwoFactorTokenSignInManager,
        P: TokenSelection,
        P::Provider: UserTokenProviderRegistry<IdentityUser>,
    {
        identity! {
        identity: self, invoke_once(input), native: |native, input, authority, policies| async move {
                    authority.validate_pending(&input.pending).await?;
                    if input.pending.external.is_some() {
                        return Err(IdentityError::Configuration(
                            "external factor completion requires its token acknowledgment workflow",
                        ));
                    }
                    let session = native
                        .two_factor_sign_in_session_from_registry(
                            input.pending.native()?,
                            policies.token_provider()?,
                            &input.provider,
                            &input.code,
                            input.persistent,
                        )
                        .await
                        .map_err(IdentityError::Native)?;
                    complete_login(&authority, session).await
                },
        }
    }
    /// Complete externally verified primary evidence with the selected named native
    /// factor, acknowledging original provider tokens after actual completion.
    pub fn complete_external_registry_factor<E, R>(
        &self,
        input: RegistryFactorCompletion,
    ) -> Effect<LoginOutcome, E, R>
    where
        E: Send + 'static,
        R: Send + Sync + 'static,
        M: Fn(IdentityError) -> E,
        C::Native: TwoFactorTokenSignInManager + ExternalLoginSignInManager,
        P: TokenSelection,
        P::Provider: UserTokenProviderRegistry<IdentityUser>,
    {
        identity! {
        identity: self, invoke_once(input), native: |native, mut input, authority, policies| async move {
                    authority.validate_pending(&input.pending).await?;
                    let external = input
                        .pending
                        .external
                        .take()
                        .ok_or(IdentityError::Unauthorized)?;
                    authority.validate_external(&external)?;
                    let session = native
                        .two_factor_sign_in_session_from_registry(
                            input.pending.native()?,
                            policies.token_provider()?,
                            &input.provider,
                            &input.code,
                            input.persistent,
                        )
                        .await
                        .map_err(IdentityError::Native)?;
                    if session.completed().is_some() {
                        acknowledged(
                            native
                                .update_external_authentication_tokens(external.evidence.info())
                                .await,
                        )?
                    }
                    complete_login(&authority, session).await
                },
        }
    }
    /// Enumerate currently eligible native registry providers under real primary
    /// evidence; this does not issue session or recent-self authentication.
    pub fn pending_factor_providers<E, R>(
        &self,
        pending: PendingLoginGrant,
    ) -> Effect<(PendingLoginGrant, Vec<TokenProviderName>), E, R>
    where
        E: Send + 'static,
        R: Send + Sync + 'static,
        M: Fn(IdentityError) -> E,
        C::Native: SignInService,
        Users<C::Native>: UserTwoFactorTokenManager,
        P: TokenSelection,
        P::Provider: UserTokenProviderRegistry<IdentityUser>,
    {
        identity! {
        identity: self, invoke_once(pending), native: |native, pending, authority, policies| async move {
                    authority.validate_pending(&pending).await?;
                    let providers = native
                        .user_manager()
                        .get_valid_two_factor_providers_from_registry(
                            pending.native()?.user(),
                            policies.token_provider()?,
                        )
                        .await
                        .map_err(IdentityError::Native)?;
                    authority.validate_pending(&pending).await?;
                    Ok((pending, providers))
                },
        }
    }
    /// Generate only an eligible selected native factor token after real primary
    /// verification. Return the owned receipt with the secret for a trusted delivery
    /// callback; public rendering policy must not expose a factor intended for delivery.
    pub fn pending_factor_token<E, R>(
        &self,
        pending: PendingLoginGrant,
        provider: TokenProviderName,
    ) -> Effect<(PendingLoginGrant, String), E, R>
    where
        E: Send + 'static,
        R: Send + Sync + 'static,
        M: Fn(IdentityError) -> E,
        C::Native: SignInService,
        Users<C::Native>: UserTwoFactorTokenManager,
        P: TokenSelection,
        P::Provider: UserTokenProviderRegistry<IdentityUser>,
    {
        identity! {
        identity: self, invoke_once((pending, provider)), native: |native, (pending, provider), authority, policies| async move {
                        authority.validate_pending(&pending).await?;
                        let token = native
                            .user_manager()
                            .generate_two_factor_token_from_registry(
                                pending.native()?.user(),
                                policies.token_provider()?,
                                &provider,
                            )
                            .await
                            .map_err(IdentityError::Native)?;
                        authority.validate_pending(&pending).await?;
                        Ok((pending, token))
                    },
        }
    }
    /// Generate/install a selected native authenticator key after recent verification.
    /// The secret result is intended for an authorized enrollment renderer only.
    pub fn reset_authenticator<E, R>(
        &self,
        grant: RecentSelfGrant,
    ) -> Effect<AuthenticatorKey, E, R>
    where
        E: Send + 'static,
        R: Send + Sync + 'static,
        M: Fn(IdentityError) -> E,
        C::Native: SignInService,
        Users<C::Native>: UserAuthenticatorKeyManager,
    {
        identity! {
        identity: self, invoke_once(grant), native: |native, grant, authority, _| async move {
                    let user = current_user(
                        native.user_manager(),
                        authority.revalidate_recent(&grant).await?,
                    )
                    .await?;
                    let id = user.id();
                    acknowledged(native.user_manager().reset_authenticator_key(user).await)?;
                    let current = native
                        .user_manager()
                        .find_by_id(&id)
                        .await
                        .map_err(IdentityError::Native)?
                        .ok_or(IdentityError::Unauthorized)?;
                    let key = native
                        .user_manager()
                        .generate_new_authenticator_key()
                        .map_err(IdentityError::Native)?;
                    acknowledged(
                        native
                            .user_manager()
                            .set_authenticator_key(&current, key)
                            .await,
                    )?;
                    native
                        .user_manager()
                        .get_authenticator_key(&current)
                        .await
                        .map_err(IdentityError::Native)?
                        .ok_or(IdentityError::Configuration(
                            "native reset did not install an authenticator key",
                        ))
                },
        }
    }
    /// Disable MFA after recent self proof. Enabling requires a verified factor
    /// completion workflow rather than trusting an HTTP boolean.
    pub fn disable_two_factor<E, R>(&self, grant: RecentSelfGrant) -> Effect<(), E, R>
    where
        E: Send + 'static,
        R: Send + Sync + 'static,
        M: Fn(IdentityError) -> E,
        C::Native: SignInService,
        Users<C::Native>: UserTwoFactorManager,
    {
        identity! {
        identity: self, invoke_once(grant), native: |native, grant, authority, _| async move {
                    let user = current_user(
                        native.user_manager(),
                        authority.revalidate_recent(&grant).await?,
                    )
                    .await?;
                    acknowledged(
                        native
                            .user_manager()
                            .set_two_factor_enabled(&user, false)
                            .await,
                    )
                },
        }
    }
    /// Verify possession of the current persisted key through the selected native
    /// factor provider before enabling MFA. A caller boolean cannot enable it.
    pub fn enable_authenticator<E, R>(
        &self,
        grant: RecentSelfGrant,
        code: String,
    ) -> Effect<(), E, R>
    where
        E: Send + 'static,
        R: Send + Sync + 'static,
        M: Fn(IdentityError) -> E,
        C::Native: SignInService,
        Users<C::Native>: UserAuthenticatorKeyManager + UserTwoFactorManager,
        P: AuthenticatorSelection,
        P::Provider:
            for<'a> UserTokenProvider<identity_rs::identity::tokens::AuthenticatorTokenContext<'a>>,
    {
        identity! {
        identity: self, invoke_once((grant, code)), native: |native, (grant, code), authority, policies| async move {
                        let user = current_user(
                            native.user_manager(),
                            authority.revalidate_recent(&grant).await?,
                        )
                        .await?;
                        let key = native
                            .user_manager()
                            .get_authenticator_key(&user)
                            .await
                            .map_err(IdentityError::Native)?
                            .ok_or(IdentityError::Unauthorized)?;
                        let context =
                            identity_rs::identity::tokens::AuthenticatorTokenContext::new(&user, &key);
                        if !policies
                            .authenticator_provider()?
                            .validate(
                                identity_rs::identity::tokens::Purpose::TWO_FACTOR,
                                &code,
                                &context,
                            )
                            .await
                            .map_err(IdentityError::Token)?
                        {
                            return Err(IdentityError::Unauthorized);
                        }
                        acknowledged(
                            native
                                .user_manager()
                                .set_two_factor_enabled(&user, true)
                                .await,
                        )
                    },
        }
    }
    /// Replace recovery hashes through the native bounded generator/store and return
    /// plaintext only after its acknowledgment to an authorized enrollment renderer.
    pub fn regenerate_recovery_codes<E, R>(
        &self,
        grant: RecentSelfGrant,
        count: NonZeroUsize,
    ) -> Effect<Vec<RecoveryCode>, E, R>
    where
        E: Send + 'static,
        R: Send + Sync + 'static,
        M: Fn(IdentityError) -> E,
        C::Native: SignInService,
        Users<C::Native>: UserTwoFactorRecoveryCodeManager,
    {
        identity! {
        identity: self, invoke_once((grant, count)), native: |native, (grant, count), authority, _| async move {
                        let user = current_user(
                            native.user_manager(),
                            authority.revalidate_recent(&grant).await?,
                        )
                        .await?;
                        native
                            .user_manager()
                            .generate_new_two_factor_recovery_codes(&user, count.get())
                            .await
                            .map_err(IdentityError::Native)
                    },
        }
    }
    /// Select trusted RP/origin/UV settings, then store a native expiring registration
    /// challenge bound to recent self proof. The original options are Arc-retained.
    pub fn begin_passkey_enrollment<E, R>(
        &self,
        grant: RecentSelfGrant,
        origin: Option<String>,
    ) -> Effect<(EnrollmentGrant, Arc<PublicKeyCredentialCreationOptions>), E, R>
    where
        E: Send + 'static,
        R: Send + Sync + 'static,
        M: Fn(IdentityError) -> E,
        C::Native: SignInService,
        Users<C::Native>: UserPasskeyChallengeManager,
        P: PasskeySelection,
        P::Policy: PasskeyPolicy,
    {
        identity! {
        identity: self, invoke_once((grant, origin)), native: |native, (grant, origin), authority, policies| async move {
                        let user = current_user(
                            native.user_manager(),
                            authority.revalidate_recent(&grant).await?,
                        )
                        .await?;
                        let request = policies
                            .passkey_policy()?
                            .resolve(origin.as_deref())
                            .map_err(|_| IdentityError::Policy(http::StatusCode::FORBIDDEN))?;
                        request
                            .validate()
                            .map_err(|_| IdentityError::Configuration("selected passkey settings"))?;
                        let options = Arc::new(
                            native
                                .user_manager()
                                .generate_passkey_registration_options(&user, &request)
                                .await
                                .map_err(IdentityError::Native)?,
                        );
                        let grant = authority.enrollment(grant, options.clone());
                        Ok((grant, options))
                    },
        }
    }
    /// Revalidate recent owner/ceremony binding, then let native policy validation
    /// consume the one-use challenge and verify attestation before persistence.
    pub fn enroll_passkey<E, R>(&self, input: PasskeyEnrollment) -> Effect<(), E, R>
    where
        E: Send + 'static,
        R: Send + Sync + 'static,
        M: Fn(IdentityError) -> E,
        C::Native: SignInService,
        Users<C::Native>: UserPasskeyChallengeManager,
        P: PasskeySelection,
        P::Policy: PasskeyPolicy,
    {
        identity! {
        identity: self, invoke_once(input), native: |native, input, authority, policies| async move {
                    let user = current_user(
                        native.user_manager(),
                        authority.validate_enrollment(&input.grant).await?,
                    )
                    .await?;
                    let client = CollectedClientData::parse(input.response.client_data_json())
                        .map_err(|_| IdentityError::Unauthorized)?;
                    if client.challenge() != input.grant.options.challenge().as_bytes() {
                        return Err(IdentityError::Unauthorized);
                    }
                    acknowledged(
                        native
                            .user_manager()
                            .add_passkey_from_registration_challenge(
                                &user,
                                &input.response,
                                policies.passkey_policy()?,
                                input.name,
                            )
                            .await,
                    )
                },
        }
    }
    /// Begin a native discoverable assertion ceremony using retained trusted policy.
    pub fn passkey_login_options<E, R>(
        &self,
        origin: Option<String>,
    ) -> Effect<PublicKeyCredentialRequestOptions, E, R>
    where
        E: Send + 'static,
        R: Send + Sync + 'static,
        M: Fn(IdentityError) -> E,
        C::Native: SignInService,
        Users<C::Native>: UserPasskeyChallengeManager,
        P: PasskeySelection,
        P::Policy: PasskeyPolicy,
    {
        identity! {
        identity: self, invoke_once(origin), native: |native, origin, _, policies| async move {
                    let request = policies
                        .passkey_policy()?
                        .resolve(origin.as_deref())
                        .map_err(|_| IdentityError::Policy(http::StatusCode::FORBIDDEN))?;
                    request
                        .validate()
                        .map_err(|_| IdentityError::Configuration("selected passkey settings"))?;
                    native
                        .user_manager()
                        .generate_passkey_assertion_options(None, &request)
                        .await
                        .map_err(IdentityError::Native)
                },
        }
    }
    /// Complete native one-use assertion/counter handling and admission before grants.
    pub fn passkey_login<E, R>(
        &self,
        response: PasskeyAssertionResponse,
    ) -> Effect<LoginOutcome, E, R>
    where
        E: Send + 'static,
        R: Send + Sync + 'static,
        M: Fn(IdentityError) -> E,
        C::Native: PasskeySignInManager,
        P: PasskeySelection,
        P::Policy: PasskeyPolicy,
    {
        identity! {
        identity: self, invoke_once(response), native: |native, response, authority, policies| async move {
                        let session = native
                            .passkey_sign_in(&response, policies.passkey_policy()?)
                            .await
                            .map_err(IdentityError::Native)?;
                        complete_login(&authority, session).await
                    },
        }
    }
    /// Remove an owned credential only after recent verification; native atomic
    /// ownership/security invalidation governs the acknowledged result.
    pub fn remove_passkey<E, R>(
        &self,
        grant: RecentSelfGrant,
        id: PasskeyCredentialId,
    ) -> Effect<(), E, R>
    where
        E: Send + 'static,
        R: Send + Sync + 'static,
        M: Fn(IdentityError) -> E,
        C::Native: SignInService,
        Users<C::Native>: UserPasskeyManager,
    {
        identity! {
        identity: self, invoke_once((grant, id)), native: |native, (grant, id), authority, _| async move {
                        let user = current_user(
                            native.user_manager(),
                            authority.revalidate_recent(&grant).await?,
                        )
                        .await?;
                        acknowledged(native.user_manager().remove_passkey(&user, &id).await)
                    },
        }
    }
    /// Verify application-owned external protocol evidence using the retained D18
    /// verifier. Unknown/invalid evidence yields no private external receipt.
    pub fn verify_external<I, E, R>(
        &self,
        evidence: I,
    ) -> Effect<Option<VerifiedExternalGrant>, E, R>
    where
        I: Send + Sync + 'static,
        E: Send + 'static,
        R: Send + Sync + 'static,
        M: Fn(IdentityError) -> E,
        P: ExternalSelection,
        P::Verifier: identity_rs::identity::external::ExternalIdentityVerifier<I>,
    {
        identity! {
        identity: self, invoke_once(evidence), native: |_, evidence, authority, policies| async move {
                    authority
                        .external(&evidence, policies.external_verifier()?)
                        .await
                },
        }
    }
    /// Apply current native external admission/MFA policy to trusted evidence.
    /// External token persistence happens only on an actual completed outcome.
    pub fn external_login<E, R>(
        &self,
        grant: VerifiedExternalGrant,
        persistent: bool,
    ) -> Effect<LoginOutcome, E, R>
    where
        E: Send + 'static,
        R: Send + Sync + 'static,
        M: Fn(IdentityError) -> E,
        C::Native: ExternalLoginSignInManager,
    {
        identity! {
        identity: self, invoke_once((grant, persistent)), native: |native, (grant, persistent), authority, _| async move {
                        authority.validate_external(&grant)?;
                        let session = native
                            .external_login_sign_in_session(&grant.evidence, persistent)
                            .await
                            .map_err(IdentityError::Native)?;
                        if session.completed().is_some() {
                            acknowledged(
                                native
                                    .update_external_authentication_tokens(grant.evidence.info())
                                    .await,
                            )?
                        }
                        let mut outcome = complete_login(&authority, session).await?;
                        if let LoginOutcome::Pending(pending) = &mut outcome {
                            pending.external = Some(grant)
                        }
                        Ok(outcome)
                    },
        }
    }
    /// Link a provider association only after recent self and trusted external
    /// verification; the native store enforces unique provider/key ownership.
    pub fn link_external<E, R>(
        &self,
        recent: RecentSelfGrant,
        external: VerifiedExternalGrant,
    ) -> Effect<(), E, R>
    where
        E: Send + 'static,
        R: Send + Sync + 'static,
        M: Fn(IdentityError) -> E,
        C::Native: SignInService,
        Users<C::Native>: UserLoginManager,
    {
        identity! {
        identity: self, invoke_once((recent, external)), native: |native, (recent, external), authority, _| async move {
                        let user = current_user(
                            native.user_manager(),
                            authority.revalidate_recent(&recent).await?,
                        )
                        .await?;
                        authority.validate_external(&external)?;
                        acknowledged(
                            native
                                .user_manager()
                                .add_login(&user, external.evidence.info().user_login_info())
                                .await,
                        )
                    },
        }
    }
    /// Remove an owned association with recent proof and native security invalidation.
    pub fn unlink_external<E, R>(
        &self,
        grant: RecentSelfGrant,
        provider: LoginProvider,
        key: ProviderKey,
    ) -> Effect<(), E, R>
    where
        E: Send + 'static,
        R: Send + Sync + 'static,
        M: Fn(IdentityError) -> E,
        C::Native: SignInService,
        Users<C::Native>: UserLoginManager,
    {
        identity! {
        identity: self, invoke_once((grant, provider, key)), native: |native, (grant, provider, key), authority, _| async move {
                        let user = current_user(
                            native.user_manager(),
                            authority.revalidate_recent(&grant).await?,
                        )
                        .await?;
                        acknowledged(
                            native
                                .user_manager()
                                .remove_login(&user, &provider, &key)
                                .await,
                        )
                    },
        }
    }
    /// Generate native email-confirmation evidence for current authorized self state.
    pub fn email_confirmation_token<E, R>(&self, grant: SelfGrant) -> Effect<String, E, R>
    where
        E: Send + 'static,
        R: Send + Sync + 'static,
        M: Fn(IdentityError) -> E,
        C::Native: SignInService,
        Users<C::Native>: UserEmailManager,
        P: EmailSelection,
        P::Provider: UserTokenProvider<IdentityUser>,
    {
        identity! {
        identity: self, invoke_once(grant), native: |native, grant, authority, policies| async move {
                    let user =
                        current_user(native.user_manager(), authority.revalidate(&grant).await?).await?;
                    native
                        .user_manager()
                        .generate_email_confirmation_token(&user, policies.email_provider()?)
                        .await
                        .map_err(IdentityError::Native)
                },
        }
    }
    /// Confirm the native purpose-bound token; lookup identity alone grants nothing.
    pub fn confirm_email<E, R>(&self, id: UserId, token: String) -> Effect<(), E, R>
    where
        E: Send + 'static,
        R: Send + Sync + 'static,
        M: Fn(IdentityError) -> E,
        C::Native: SignInService,
        Users<C::Native>: UserEmailManager,
        P: EmailSelection,
        P::Provider: UserTokenProvider<IdentityUser>,
    {
        identity! {
        identity: self, invoke_once((id, token)), native: |native, (id, token), _, policies| async move {
                    let user = native
                        .user_manager()
                        .find_by_id(&id)
                        .await
                        .map_err(IdentityError::Native)?
                        .ok_or(IdentityError::Unauthorized)?;
                    native
                        .user_manager()
                        .confirm_email(user, policies.email_provider()?, &token)
                        .await
                        .map(|_| ())
                        .map_err(IdentityError::Native)
                },
        }
    }
    /// Change an unconfirmed contact through native normalization/uniqueness after
    /// recent verification; callers cannot claim confirmation by supplying a flag.
    pub fn set_email<E, R>(
        &self,
        grant: RecentSelfGrant,
        email: Option<EmailAddress>,
    ) -> Effect<AccountView, E, R>
    where
        E: Send + 'static,
        R: Send + Sync + 'static,
        M: Fn(IdentityError) -> E,
        C::Native: SignInService,
        Users<C::Native>: UserEmailManager,
    {
        identity! {
        identity: self, invoke_once((grant, email)), native: |native, (grant, email), authority, _| async move {
                        let user = current_user(
                            native.user_manager(),
                            authority.revalidate_recent(&grant).await?,
                        )
                        .await?;
                        let user = native
                            .user_manager()
                            .set_email(user, email.as_ref().map(EmailAddress::as_str))
                            .await
                            .map_err(IdentityError::Native)?;
                        Ok(AccountView::from_user(&user))
                    },
        }
    }
    /// Change an unconfirmed phone contact using the selected native manager.
    pub fn set_phone_number<E, R>(
        &self,
        grant: RecentSelfGrant,
        phone: Option<PhoneNumber>,
    ) -> Effect<AccountView, E, R>
    where
        E: Send + 'static,
        R: Send + Sync + 'static,
        M: Fn(IdentityError) -> E,
        C::Native: SignInService,
        Users<C::Native>: UserPhoneNumberManager,
    {
        identity! {
        identity: self, invoke_once((grant, phone)), native: |native, (grant, phone), authority, _| async move {
                        let user = current_user(
                            native.user_manager(),
                            authority.revalidate_recent(&grant).await?,
                        )
                        .await?;
                        let user = native
                            .user_manager()
                            .set_phone_number(user, phone.as_ref().map(PhoneNumber::as_str))
                            .await
                            .map_err(IdentityError::Native)?;
                        Ok(AccountView::from_user(&user))
                    },
        }
    }
    /// Read one finite native page, then revalidate scope and per-account application
    /// policy for each emitted view. No database transaction spans client rendering.
    pub fn accounts_page<E, R, F, Fut, D, DFut>(
        &self,
        scope: ScopeGrant,
        after: Option<UserId>,
        limit: NonZeroU16,
        decide: D,
        allow: F,
    ) -> Effect<AccountViewPage, E, R>
    where
        E: Send + 'static,
        R: Send + Sync + 'static,
        M: Fn(IdentityError) -> E,
        C::Native: SignInService,
        Users<C::Native>: UserQueryableManager,
        D: Fn(UserId, Arc<str>, Arc<str>) -> DFut + Send + Sync + 'static,
        DFut: Future<Output = Result<PermissionDecision, IdentityError>> + Send + 'static,
        F: Fn(UserId, UserId) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<bool, IdentityError>> + Send + 'static,
    {
        let decide = Arc::new(decide);
        let allow = Arc::new(allow);
        self.invoke_once(
            (scope, after, limit),
            move |native, (scope, after, limit), authority, _| {
                let decide = decide.clone();
                let allow = allow.clone();
                async move {
                    if scope.target() != &PermissionTarget::Accounts
                        || scope.operation() != &PermissionOperation::AccountsRead
                    {
                        return Err(IdentityError::Unauthorized);
                    }
                    authority
                        .revalidate_scope(&scope, |actor, action, resource| {
                            decide(actor, action, resource)
                        })
                        .await?;
                    let page = native
                        .user_manager()
                        .users_page(after, limit)
                        .await
                        .map_err(IdentityError::Native)?;
                    let next = page.next();
                    let mut accounts = Vec::new();
                    for user in page.into_users() {
                        let actor = authority
                            .revalidate_scope(&scope, |actor, action, resource| {
                                decide(actor, action, resource)
                            })
                            .await?;
                        if allow(actor.id(), user.id()).await? {
                            accounts.push(AccountView::from_user(&user))
                        }
                    }
                    Ok(AccountViewPage { accounts, next })
                }
            },
        )
    }
    /// Revalidate exact action/resource/revision permission before a selected
    /// administrative account callback. The callback receives current target state;
    /// native mutation interfaces retain their actual acknowledgment boundaries.
    pub fn administer_account<I, A, E, R, D, DFut, F, Fut>(
        &self,
        grant: PermissionGrant,
        action: AccountAction,
        target: UserId,
        input: I,
        decide: D,
        operation: F,
    ) -> Effect<A, E, R>
    where
        I: Send + 'static,
        A: Send + 'static,
        E: Send + 'static,
        R: Send + Sync + 'static,
        M: Fn(IdentityError) -> E,
        C::Native: SignInService,
        Users<C::Native>: UserLifecycleManager,
        D: Fn(UserId, Arc<str>, Arc<str>) -> DFut + Send + Sync + 'static,
        DFut: Future<Output = Result<PermissionDecision, IdentityError>> + Send + 'static,
        F: Fn(C::Native, IdentityUser, I) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<A, IdentityError>> + Send + 'static,
    {
        let AccountAction {
            operation: operation_kind,
            action,
            resource,
        } = action;
        let decide = Arc::new(decide);
        let operation = Arc::new(operation);
        self.invoke_once(
            (grant, operation_kind, action, resource, target, input),
            move |native,
                  (grant, operation_kind, action, resource, target, input),
                  authority,
                  _| {
                let decide = decide.clone();
                let operation = operation.clone();
                async move {
                    if grant.target() != &PermissionTarget::Account(target)
                        || grant.operation() != &operation_kind
                    {
                        return Err(IdentityError::Unauthorized);
                    }
                    authority
                        .revalidate_permission(
                            &grant,
                            &action,
                            &resource,
                            |actor, action, resource| decide(actor, action, resource),
                        )
                        .await?;
                    let user = native
                        .user_manager()
                        .find_by_id(&target)
                        .await
                        .map_err(IdentityError::Native)?
                        .ok_or(IdentityError::Unauthorized)?;
                    operation(native, user, input).await
                }
            },
        )
    }
    /// Revalidate approved role permission and current native role revision before
    /// a selected role-manager callback; role services are actual configured components.
    pub fn administer_role<I, A, E, R, D, DFut, F, Fut>(
        &self,
        grant: RoleGrant,
        operation_kind: PermissionOperation,
        input: I,
        decide: D,
        operation: F,
    ) -> Effect<A, E, R>
    where
        I: Send + 'static,
        A: Send + 'static,
        E: Send + 'static,
        R: Send + Sync + 'static,
        M: Fn(IdentityError) -> E,
        D: Fn(UserId, Arc<str>, Arc<str>) -> DFut + Send + Sync + 'static,
        DFut: Future<Output = Result<PermissionDecision, IdentityError>> + Send + 'static,
        F: Fn(C::Native, IdentityRole, I) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<A, IdentityError>> + Send + 'static,
    {
        let decide = Arc::new(decide);
        let operation = Arc::new(operation);
        self.invoke_once(
            (grant, operation_kind, input),
            move |native, (grant, operation_kind, input), authority, _| {
                let decide = decide.clone();
                let operation = operation.clone();
                async move {
                    let role = authority
                        .revalidate_role(&grant, &operation_kind, |actor, action, resource| {
                            decide(actor, action, resource)
                        })
                        .await?;
                    operation(native, role, input).await
                }
            },
        )
    }
    /// Create a role through selected native validation/normalization only after
    /// exact application action/resource/revision permission revalidation.
    pub fn create_role<E, R, D, DFut>(
        &self,
        grant: PermissionGrant,
        action: Arc<str>,
        resource: Arc<str>,
        name: UserName,
        decide: D,
    ) -> Effect<RoleView, E, R>
    where
        E: Send + 'static,
        R: Send + Sync + 'static,
        M: Fn(IdentityError) -> E,
        C::Native: RoleLifecycleManager,
        D: Fn(UserId, Arc<str>, Arc<str>) -> DFut + Send + Sync + 'static,
        DFut: Future<Output = Result<PermissionDecision, IdentityError>> + Send + 'static,
    {
        let decide = Arc::new(decide);
        self.invoke_once(
            (grant, action, resource, name),
            move |native, (grant, action, resource, name), authority, _| {
                let decide = decide.clone();
                async move {
                    if grant.target() != &PermissionTarget::Roles
                        || grant.operation() != &PermissionOperation::RoleCreate
                    {
                        return Err(IdentityError::Unauthorized);
                    }
                    authority
                        .revalidate_permission(
                            &grant,
                            &action,
                            &resource,
                            |actor, action, resource| decide(actor, action, resource),
                        )
                        .await?;
                    let role = native.create(name).await.map_err(IdentityError::Native)?;
                    Ok(RoleView::from_role(&role))
                }
            },
        )
    }
    /// Rename the current revision-bound role and preserve its stable ID.
    pub fn rename_role<E, R, D, DFut>(
        &self,
        grant: RoleGrant,
        name: UserName,
        decide: D,
    ) -> Effect<RoleView, E, R>
    where
        E: Send + 'static,
        R: Send + Sync + 'static,
        M: Fn(IdentityError) -> E,
        C::Native: RoleLifecycleManager,
        D: Fn(UserId, Arc<str>, Arc<str>) -> DFut + Send + Sync + 'static,
        DFut: Future<Output = Result<PermissionDecision, IdentityError>> + Send + 'static,
    {
        self.administer_role(
            grant,
            PermissionOperation::RoleRename,
            name,
            decide,
            |native, role, name| async move {
                // Native RoleLifecycleManager::update recomputes equality through its
                // selected normalizer; this owned accessor conversion retains a valid
                // temporary aggregate without manufacturing a new role identity.
                let normalized =
                    role.normalized_name()
                        .cloned()
                        .ok_or(IdentityError::Configuration(
                            "stored named role has no normalized key",
                        ))?;
                let role = native
                    .update(role.with_name_and_normalized_name(name, normalized))
                    .await
                    .map_err(IdentityError::Native)?;
                Ok(RoleView::from_role(&role))
            },
        )
    }
    /// Delete only the freshly authorized stable role; native deletion owns its
    /// actual membership/claims acknowledgment boundary.
    pub fn delete_role<E, R, D, DFut>(&self, grant: RoleGrant, decide: D) -> Effect<(), E, R>
    where
        E: Send + 'static,
        R: Send + Sync + 'static,
        M: Fn(IdentityError) -> E,
        C::Native: RoleLifecycleManager,
        D: Fn(UserId, Arc<str>, Arc<str>) -> DFut + Send + Sync + 'static,
        DFut: Future<Output = Result<PermissionDecision, IdentityError>> + Send + 'static,
    {
        self.administer_role(
            grant,
            PermissionOperation::RoleDelete,
            (),
            decide,
            |native, role, ()| async move { acknowledged(native.delete(&role).await) },
        )
    }
    /// Persist one native role claim only after current role/permission checks.
    pub fn add_role_claim<E, R, D, DFut>(
        &self,
        grant: RoleGrant,
        claim: IdentityClaim,
        decide: D,
    ) -> Effect<(), E, R>
    where
        E: Send + 'static,
        R: Send + Sync + 'static,
        M: Fn(IdentityError) -> E,
        C::Native: RoleClaimManager,
        D: Fn(UserId, Arc<str>, Arc<str>) -> DFut + Send + Sync + 'static,
        DFut: Future<Output = Result<PermissionDecision, IdentityError>> + Send + 'static,
    {
        self.administer_role(
            grant,
            PermissionOperation::RoleClaimAdd,
            claim,
            decide,
            |native, role, claim| async move { acknowledged(native.add_claim(&role, claim).await) },
        )
    }
    /// Remove exactly the requested native claim from the current authorized role.
    pub fn remove_role_claim<E, R, D, DFut>(
        &self,
        grant: RoleGrant,
        claim: IdentityClaim,
        decide: D,
    ) -> Effect<(), E, R>
    where
        E: Send + 'static,
        R: Send + Sync + 'static,
        M: Fn(IdentityError) -> E,
        C::Native: RoleClaimManager,
        D: Fn(UserId, Arc<str>, Arc<str>) -> DFut + Send + Sync + 'static,
        DFut: Future<Output = Result<PermissionDecision, IdentityError>> + Send + 'static,
    {
        self.administer_role(grant,PermissionOperation::RoleClaimRemove,claim,decide,|native,role,claim|async move{acknowledged(native.remove_claim(&role,&claim).await)})
    }
    /// Enumerate one finite native role page and revalidate application scope and
    /// per-role visibility before each safe view. No client lifetime owns SQL.
    pub fn roles_page<E, R, D, DFut, F, Fut>(
        &self,
        scope: ScopeGrant,
        after: Option<RoleId>,
        limit: NonZeroU16,
        decide: D,
        allow: F,
    ) -> Effect<RoleViewPage, E, R>
    where
        E: Send + 'static,
        R: Send + Sync + 'static,
        M: Fn(IdentityError) -> E,
        C::Native: RoleQueryableManager,
        D: Fn(UserId, Arc<str>, Arc<str>) -> DFut + Send + Sync + 'static,
        DFut: Future<Output = Result<PermissionDecision, IdentityError>> + Send + 'static,
        F: Fn(UserId, RoleId) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<bool, IdentityError>> + Send + 'static,
    {
        let decide = Arc::new(decide);
        let allow = Arc::new(allow);
        self.invoke_once(
            (scope, after, limit),
            move |native, (scope, after, limit), authority, _| {
                let decide = decide.clone();
                let allow = allow.clone();
                async move {
                    if scope.target() != &PermissionTarget::Roles
                        || scope.operation() != &PermissionOperation::RolesRead
                    {
                        return Err(IdentityError::Unauthorized);
                    }
                    authority
                        .revalidate_scope(&scope, |actor, action, resource| {
                            decide(actor, action, resource)
                        })
                        .await?;
                    let page = native
                        .roles_page(after, limit)
                        .await
                        .map_err(IdentityError::Native)?;
                    let next = page.next();
                    let mut roles = Vec::new();
                    for role in page.into_roles() {
                        let actor = authority
                            .revalidate_scope(&scope, |actor, action, resource| {
                                decide(actor, action, resource)
                            })
                            .await?;
                        if allow(actor.id(), role.id()).await? {
                            roles.push(RoleView::from_role(&role))
                        }
                    }
                    Ok(RoleViewPage { roles, next })
                }
            },
        )
    }
    /// Apply membership to a current target only after exact administrative
    /// permission checks. Native role lookup resolves stable ownership/normalization.
    pub fn add_account_to_role<E, R, D, DFut>(
        &self,
        grant: PermissionGrant,
        action: Arc<str>,
        resource: Arc<str>,
        target: UserId,
        name: String,
        decide: D,
    ) -> Effect<(), E, R>
    where
        E: Send + 'static,
        R: Send + Sync + 'static,
        M: Fn(IdentityError) -> E,
        C::Native: SignInService,
        Users<C::Native>: UserRoleManager,
        D: Fn(UserId, Arc<str>, Arc<str>) -> DFut + Send + Sync + 'static,
        DFut: Future<Output = Result<PermissionDecision, IdentityError>> + Send + 'static,
    {
        self.administer_account(
            grant,
            AccountAction {
                operation: PermissionOperation::MembershipAdd,
                action,
                resource,
            },
            target,
            name,
            decide,
            |native, user, name| async move {
                acknowledged(native.user_manager().add_to_role(&user, &name).await)
            },
        )
    }
    /// Remove current target membership under exact administrative permission.
    pub fn remove_account_from_role<E, R, D, DFut>(
        &self,
        grant: PermissionGrant,
        action: Arc<str>,
        resource: Arc<str>,
        target: UserId,
        name: String,
        decide: D,
    ) -> Effect<(), E, R>
    where
        E: Send + 'static,
        R: Send + Sync + 'static,
        M: Fn(IdentityError) -> E,
        C::Native: SignInService,
        Users<C::Native>: UserRoleManager,
        D: Fn(UserId, Arc<str>, Arc<str>) -> DFut + Send + Sync + 'static,
        DFut: Future<Output = Result<PermissionDecision, IdentityError>> + Send + 'static,
    {
        self.administer_account(
            grant,
            AccountAction {
                operation: PermissionOperation::MembershipRemove,
                action,
                resource,
            },
            target,
            name,
            decide,
            |native, user, name| async move {
                acknowledged(native.user_manager().remove_from_role(&user, &name).await)
            },
        )
    }
    /// Issue the configured native cookie only from a current completed result.
    /// The original native session/ticket is retained and no arbitrary ticket is accepted.
    pub fn sign_in_cookie<E, R>(&self, outcome: LoginOutcome) -> Effect<http::HeaderValue, E, R>
    where
        E: Send + 'static,
        R: Send + Sync + 'static,
        M: Fn(IdentityError) -> E,
        C::Native: SignInService,
        Users<C::Native>: UserLifecycleManager,
        P: TicketSelection + PrincipalSelection,
        P::Service: CookieSessionService + SessionIssuance,
        P::Principal: PrincipalComponents<Users<C::Native>>,
    {
        identity! {
        identity: self, invoke_once(outcome), native: |native, outcome, authority, policies| async move {
                    let LoginOutcome::Completed { session, actor, .. } = outcome else {
                        return Err(IdentityError::Unauthorized);
                    };
                    let user =
                        current_user(native.user_manager(), authority.revalidate(&actor).await?).await?;
                    let service = policies.ticket_service()?;
                    let original = session.ticket().ok_or(IdentityError::Unauthorized)?;
                    let ticket = session_ticket(
                        native.user_manager(),
                        &user,
                        original,
                        policies.principal_policy()?,
                        service,
                        authority.clock(),
                    )
                    .await?;
                    authority.revalidate(&actor).await?;
                    service
                        .sign_in_cookie(&ticket)
                        .await
                        .map_err(IdentityError::Ticket)
                },
        }
    }
    /// Authenticate native-cookie mode using native format, explicit expiry,
    /// unconditional native current-stamp validation and selected claim mapping.
    /// Arbitrary AuthenticatedIdentity request extensions are never consulted.
    pub fn authenticate_cookie<E, R>(
        &self,
        headers: http::HeaderMap,
    ) -> Effect<CookieAuthentication, E, R>
    where
        E: Send + 'static,
        R: Send + Sync + 'static,
        M: Fn(IdentityError) -> E,
        C::Native: SignInService,
        Users<C::Native>:
            identity_rs::identity::principal::security_stamp::SecurityStampUserAccessor,
        P: TicketSelection + PrincipalSelection,
        P::Service: CookieSessionService,
        P::Principal: PrincipalComponents<Users<C::Native>>,
    {
        identity! {
        identity: self, invoke_once(headers), native: |native, headers, authority, policies| async move {
                    use identity_rs::identity::principal::security_stamp::SecurityStampUserAccessor;
                    use identity_rs::identity::ticket::CookieSessionResult;
                    let principal = policies.principal_policy()?;
                    let validator = CurrentTicketValidator {
                        users: native.user_manager(),
                        policy: principal,
                        clock: authority.clock(),
                    };
                    match policies
                        .ticket_service()?
                        .authenticate_with_validator(&headers, &validator)
                        .await
                        .map_err(IdentityError::Ticket)?
                    {
                        CookieSessionResult::Missing => Ok(CookieAuthentication::Missing),
                        CookieSessionResult::Rejected { removal } => {
                            Ok(CookieAuthentication::Rejected { removal })
                        }
                        CookieSessionResult::Authenticated { ticket, renewal } => {
                            let user = native
                                .user_manager()
                                .find_user_by_principal(ticket.principal(), principal.claims_options())
                                .await
                                .map_err(IdentityError::Native)?
                                .ok_or(IdentityError::Unauthorized)?;
                            if !native
                                .user_manager()
                                .validate_security_stamp(
                                    &user,
                                    ticket.principal().find_first_value(
                                        &principal.claims_options().security_stamp_claim_type,
                                    ),
                                )
                                .await
                                .map_err(IdentityError::Native)?
                            {
                                return Err(IdentityError::StaleGrant);
                            }
                            let actor = authority.verify_user_grant(&user).await?;
                            Ok(CookieAuthentication::Authenticated {
                                ticket: Arc::new(ticket),
                                actor,
                                renewal,
                            })
                        }
                    }
                },
        }
    }
    /// Issue bearer mode only from trusted completion and exact current user state.
    /// Persist the native refresh digest before returning any secret credentials.
    pub fn sign_in_bearer<E, R>(&self, outcome: LoginOutcome) -> Effect<IssuedBearer, E, R>
    where
        E: Send + 'static,
        R: Send + Sync + 'static,
        M: Fn(IdentityError) -> E,
        C::Native: SignInService,
        Users<C::Native>: UserAuthenticationTokenManager,
        P: TokenSelection,
        P::Provider: UserTokenProvider<IdentityUser> + BearerIssuance,
    {
        identity! {
        identity: self, invoke_once(outcome), native: |native, outcome, authority, policies| async move {
                    let LoginOutcome::Completed { actor, .. } = outcome else {
                        return Err(IdentityError::Unauthorized);
                    };
                    let user =
                        current_user(native.user_manager(), authority.revalidate(&actor).await?).await?;
                    let provider = policies.token_provider()?;
                    let tokens = identity_rs::identity::tokens::issue_bearer_tokens(provider, &user)
                        .await
                        .map_err(IdentityError::Bearer)?;
                    let hash = identity_rs::identity::tokens::refresh_token_hash(tokens.refresh_token())
                        .map_err(IdentityError::Bearer)?;
                    authority.revalidate(&actor).await?;
                    acknowledged(
                        native
                            .user_manager()
                            .set_authentication_token(
                                &user,
                                identity_rs::identity::tokens::bearer_login_provider(),
                                identity_rs::identity::tokens::refresh_token_name(),
                                Some(hash),
                            )
                            .await,
                    )?;
                    Ok(IssuedBearer {
                        tokens,
                        lifetime: provider.access_lifetime_seconds(),
                    })
                },
        }
    }
    /// Verify native refresh evidence and atomically rotate exactly its current
    /// stored digest before returning replacements. Refresh never yields recent proof.
    pub fn refresh_bearer<E, R>(&self, token: String) -> Effect<(IssuedBearer, SelfGrant), E, R>
    where
        E: Send + 'static,
        R: Send + Sync + 'static,
        M: Fn(IdentityError) -> E,
        C::Native: SignInService,
        Users<C::Native>: UserAuthenticationTokenManager + UserManagerConfiguration,
        P: TokenSelection,
        P::Provider: UserTokenProvider<IdentityUser> + BearerIssuance,
    {
        identity! {
        identity: self, invoke_once(token), native: |native, token, authority, policies| async move {
                    let provider = policies.token_provider()?;
                    let (completed, expected) =
                        identity_rs::identity::CompletedAuthentication::verify_refresh(
                            native.user_manager(),
                            provider,
                            &token,
                        )
                        .await
                        .map_err(IdentityError::Native)?
                        .ok_or(IdentityError::Unauthorized)?;
                    let actor = authority.from_completed(&completed).await?;
                    let tokens =
                        identity_rs::identity::tokens::issue_bearer_tokens(provider, completed.user())
                            .await
                            .map_err(IdentityError::Bearer)?;
                    let replacement =
                        identity_rs::identity::tokens::refresh_token_hash(tokens.refresh_token())
                            .map_err(IdentityError::Bearer)?;
                    authority.revalidate(&actor).await?;
                    if !native
                        .user_manager()
                        .replace_authentication_token_if_matches(
                            completed.user(),
                            &identity_rs::identity::tokens::bearer_login_provider(),
                            &identity_rs::identity::tokens::refresh_token_name(),
                            &expected,
                            replacement,
                        )
                        .await
                        .map_err(IdentityError::Native)?
                    {
                        return Err(IdentityError::Unauthorized);
                    }
                    Ok((
                        IssuedBearer {
                            tokens,
                            lifetime: provider.access_lifetime_seconds(),
                        },
                        actor,
                    ))
                },
        }
    }
    /// Acknowledge bearer refresh revocation for current self; no browser cookie
    /// or CSRF behavior is invented for bearer-only mode.
    pub fn logout_bearer<E, R>(&self, grant: SelfGrant) -> Effect<(), E, R>
    where
        E: Send + 'static,
        R: Send + Sync + 'static,
        M: Fn(IdentityError) -> E,
        C::Native: SignInService,
        Users<C::Native>: UserAuthenticationTokenManager,
    {
        identity! {
        identity: self, invoke_once(grant), native: |native, grant, authority, _| async move {
                    let user =
                        current_user(native.user_manager(), authority.revalidate(&grant).await?).await?;
                    acknowledged(
                        native
                            .user_manager()
                            .remove_authentication_token(
                                &user,
                                &identity_rs::identity::tokens::bearer_login_provider(),
                                &identity_rs::identity::tokens::refresh_token_name(),
                            )
                            .await,
                    )
                },
        }
    }
    /// Persist selected stored mode only after native completion and selected
    /// principal/UTC session policy. The receipt is acknowledged storage state.
    pub fn store_session<E, R>(
        &self,
        outcome: LoginOutcome,
        id: uuid::Uuid,
    ) -> Effect<uuid::Uuid, E, R>
    where
        E: Send + 'static,
        R: Send + Sync + 'static,
        M: Fn(IdentityError) -> E,
        C::Native: SignInService,
        Users<C::Native>: UserLifecycleManager,
        S: SessionSelection,
        P: TicketSelection + PrincipalSelection,
        P::Service: SessionIssuance,
        P::Principal: PrincipalComponents<Users<C::Native>>,
    {
        let storage = self.retained_storage();
        identity! {
        identity: self, invoke_once((outcome, id)), native: move |native, (outcome, id), authority, policies| {
                        let storage = storage.clone();
                        async move {
                            let LoginOutcome::Completed { session, actor, .. } = outcome else {
                                return Err(IdentityError::Unauthorized);
                            };
                            let user =
                                current_user(native.user_manager(), authority.revalidate(&actor).await?)
                                    .await?;
                            let ticket = session_ticket(
                                native.user_manager(),
                                &user,
                                session.ticket().ok_or(IdentityError::Unauthorized)?,
                                policies.principal_policy()?,
                                policies.ticket_service()?,
                                authority.clock(),
                            )
                            .await?;
                            authority.revalidate(&actor).await?;
                            storage
                                .sessions_store()?
                                .store(
                                    user.id(),
                                    id,
                                    ticket
                                        .properties()
                                        .expires_utc()
                                        .ok_or(IdentityError::Configuration("stored session expiry"))?,
                                    &ticket,
                                )
                                .await
                        }
                    },
        }
    }
    /// Authenticate actual stored mode by native protected-ticket verification and
    /// current native stamp checks. A stored record alone cannot mint authority.
    pub fn authenticate_stored_session<E, R>(
        &self,
        owner: UserId,
        id: uuid::Uuid,
    ) -> Effect<Option<SelfGrant>, E, R>
    where
        E: Send + 'static,
        R: Send + Sync + 'static,
        M: Fn(IdentityError) -> E,
        C::Native: SignInService,
        Users<C::Native>:
            identity_rs::identity::principal::security_stamp::SecurityStampUserAccessor,
        S: SessionSelection,
        P: PrincipalSelection,
        P::Principal: PrincipalComponents<Users<C::Native>>,
    {
        let storage = self.retained_storage();
        identity! {
        identity: self, invoke_once((owner, id)), native: move |native, (owner, id), authority, policies| {
                        let storage = storage.clone();
                        async move {
                            use identity_rs::identity::principal::security_stamp::SecurityStampUserAccessor;
                            use identity_rs::identity::ticket::{
                                AuthenticationTicketValidator, TicketValidationOutcome,
                            };
                            let Some(ticket) = storage
                                .sessions_store()?
                                .load(owner, id, authority.clock().now())
                                .await?
                            else {
                                return Ok(None);
                            };
                            let principal = policies.principal_policy()?;
                            let validator = CurrentTicketValidator {
                                users: native.user_manager(),
                                policy: principal,
                                clock: authority.clock(),
                            };
                            let ticket = match validator
                                .validate_ticket(ticket)
                                .await
                                .map_err(IdentityError::Ticket)?
                            {
                                TicketValidationOutcome::Accepted(ticket)
                                | TicketValidationOutcome::Renewed(ticket) => ticket,
                                TicketValidationOutcome::Rejected => return Ok(None),
                            };
                            let Some(user) = native
                                .user_manager()
                                .find_user_by_principal(ticket.principal(), principal.claims_options())
                                .await
                                .map_err(IdentityError::Native)?
                            else {
                                return Ok(None);
                            };
                            if user.id() != owner {
                                return Err(IdentityError::Unauthorized);
                            }
                            authority.verify_user_grant(&user).await.map(Some)
                        }
                    },
        }
    }
    /// Revoke exactly an owned stored revision and await acknowledgment before logout.
    pub fn revoke_session<E, R>(
        &self,
        grant: SelfGrant,
        id: uuid::Uuid,
        expected: uuid::Uuid,
    ) -> Effect<bool, E, R>
    where
        E: Send + 'static,
        R: Send + Sync + 'static,
        M: Fn(IdentityError) -> E,
        S: SessionSelection,
    {
        let storage = self.retained_storage();
        identity! {
        identity: self, invoke_once((grant, id, expected)), native: move |_, (grant, id, expected), authority, _| {
                        let storage = storage.clone();
                        async move {
                            let user = authority.revalidate(&grant).await?;
                            storage
                                .sessions_store()?
                                .revoke(user.id(), id, expected)
                                .await
                        }
                    },
        }
    }
    /// Read selected consent/preferences after current self verification, then
    /// recheck authority immediately before emitting the application-owned document.
    pub fn preferences<E, R>(
        &self,
        grant: SelfGrant,
        key: uuid::Uuid,
    ) -> Effect<Option<VersionedDocument<<S::Consent as ConsentStore>::Document>>, E, R>
    where
        E: Send + 'static,
        R: Send + Sync + 'static,
        M: Fn(IdentityError) -> E,
        S: ConsentSelection,
    {
        let storage = self.retained_storage();
        identity! {
        identity: self, invoke_once((grant, key)), native: move |_, (grant, key), authority, _| {
                    let storage = storage.clone();
                    async move {
                        let user = authority.revalidate(&grant).await?;
                        let value = storage
                            .consent_store()?
                            .read(user.id(), key, authority.clock().now())
                            .await?;
                        authority.revalidate(&grant).await?;
                        Ok(value)
                    }
                },
        }
    }
    /// Persist one selected consent/preferences representation under exact self
    /// ownership and optional expected revision; no default preference is invented.
    pub fn update_preferences<E, R>(
        &self,
        grant: SelfGrant,
        key: uuid::Uuid,
        expected: Option<uuid::Uuid>,
        expires_at: Option<time::OffsetDateTime>,
        document: <S::Consent as ConsentStore>::Document,
    ) -> Effect<uuid::Uuid, E, R>
    where
        E: Send + 'static,
        R: Send + Sync + 'static,
        M: Fn(IdentityError) -> E,
        S: ConsentSelection,
    {
        let storage = self.retained_storage();
        identity! {
        identity: self, invoke_once((grant, key, expected, expires_at, document)), native: move |_, (grant, key, expected, expires_at, document), authority, _| {
                        let storage = storage.clone();
                        async move {
                            let user = authority.revalidate(&grant).await?;
                            storage
                                .consent_store()?
                                .write(user.id(), key, expected, expires_at, document)
                                .await
                        }
                    },
        }
    }
    /// Obtain this owner's selected complete security aggregate for trusted
    /// application workflows. It has no automatic HTTP serialization/view policy.
    pub fn account_snapshot<E, R>(
        &self,
        grant: SelfGrant,
    ) -> Effect<AccountSnapshot<S::Accounts>, E, R>
    where
        E: Send + 'static,
        R: Send + Sync + 'static,
        M: Fn(IdentityError) -> E,
        S: AccountSelection,
    {
        identity! {
        identity: self, invoke_capability_once(grant), native: |storage, _, grant, authority, _| async move {
                    let user = authority.revalidate(&grant).await?;
                    let snapshot = storage.accounts_store()?.load(user.id()).await?;
                    if let Some(value) = &snapshot {
                        let metadata = value.snapshot.metadata();
                        if metadata.id() != user.id()
                            || metadata.concurrency_stamp() != user.concurrency_stamp()
                            || metadata.security_stamp() != user.security_stamp()
                        {
                            return Err(IdentityError::StaleGrant);
                        }
                    }
                    authority.revalidate(&grant).await?;
                    Ok(snapshot)
                },
        }
    }
    /// Apply an application-owned native aggregate transform to the current
    /// recent-authorized owner and replace exactly its aggregate revision.
    /// The transform must retain selected native validation/crypto components;
    /// this API never treats a deserialized HTTP aggregate as credential proof.
    pub fn mutate_account_snapshot<I, E, R, F, Fut>(
        &self,
        grant: RecentSelfGrant,
        input: I,
        transform: F,
    ) -> Effect<<S::Accounts as AccountSnapshots>::Revision, E, R>
    where
        I: Send + 'static,
        E: Send + 'static,
        R: Send + Sync + 'static,
        M: Fn(IdentityError) -> E,
        S: AccountSelection,
        F: Fn(C::Native, <S::Accounts as AccountSnapshots>::Snapshot, I) -> Fut
            + Send
            + Sync
            + 'static,
        Fut: Future<Output = Result<<S::Accounts as AccountSnapshots>::Snapshot, IdentityError>>
            + Send
            + 'static,
    {
        let transform = Arc::new(transform);
        self.invoke_capability_once(
            (grant, input),
            move |storage, native, (grant, input), authority, _| {
                let transform = transform.clone();
                async move {
                    let user = authority.revalidate_recent(&grant).await?;
                    let value = storage
                        .accounts_store()?
                        .load(user.id())
                        .await?
                        .ok_or(IdentityError::Unauthorized)?;
                    if value.snapshot.metadata().id() != user.id()
                        || value.snapshot.metadata().security_stamp() != user.security_stamp()
                        || value.snapshot.metadata().concurrency_stamp() != user.concurrency_stamp()
                    {
                        return Err(IdentityError::StaleGrant);
                    }
                    let snapshot = transform(native, value.snapshot, input).await?;
                    if snapshot.metadata().id() != user.id() {
                        return Err(IdentityError::Unauthorized);
                    }
                    authority.revalidate_recent(&grant).await?;
                    storage
                        .accounts_store()?
                        .replace(value.revision, snapshot)
                        .await
                }
            },
        )
    }
    /// Delete the selected complete self aggregate only at its exact current
    /// revision after recent proof; this is one acknowledged aggregate transaction.
    pub fn delete_account<E, R>(&self, grant: RecentSelfGrant) -> Effect<(), E, R>
    where
        E: Send + 'static,
        R: Send + Sync + 'static,
        M: Fn(IdentityError) -> E,
        S: AccountSelection,
    {
        identity! {
        identity: self, invoke_capability_once(grant), native: |storage, _, grant, authority, _| async move {
                    let user = authority.revalidate_recent(&grant).await?;
                    let value = storage
                        .accounts_store()?
                        .load(user.id())
                        .await?
                        .ok_or(IdentityError::Unauthorized)?;
                    authority.revalidate_recent(&grant).await?;
                    storage
                        .accounts_store()?
                        .delete(user.id(), value.revision)
                        .await
                },
        }
    }
    /// Acknowledge selected durable delivery acceptance before handing the
    /// original request to its application-owned transport callback. Callback
    /// failure does not undo acceptance and no automatic retry is performed.
    pub fn deliver<A, E, R, F, Fut>(
        &self,
        grant: SelfGrant,
        id: uuid::Uuid,
        expires_at: Option<time::OffsetDateTime>,
        request: <S::Delivery as DurableRequests>::Request,
        transport: F,
    ) -> Effect<A, E, R>
    where
        A: Send + 'static,
        E: Send + 'static,
        R: Send + Sync + 'static,
        M: Fn(IdentityError) -> E,
        S: DeliverySelection,
        F: Fn(AcceptedDocument<<S::Delivery as DurableRequests>::Request>) -> Fut
            + Send
            + Sync
            + 'static,
        Fut: Future<Output = Result<A, IdentityError>> + Send + 'static,
    {
        let transport = Arc::new(transport);
        self.invoke_capability_once(
            (grant, id, expires_at, request),
            move |storage, _, (grant, id, expires_at, request), authority, _| {
                let transport = transport.clone();
                async move {
                    let user = authority.revalidate(&grant).await?;
                    let repository = storage.delivery_store()?;
                    if repository.domain() != RequestDomain::Delivery {
                        return Err(IdentityError::Configuration("delivery request domain"));
                    }
                    let accepted = repository
                        .accept(user.id(), id, expires_at, request)
                        .await?;
                    transport(accepted).await
                }
            },
        )
    }
    /// Acknowledge selected notification acceptance before the actual configured
    /// application callback receives its original moved payload.
    pub fn notify<A, E, R, F, Fut>(
        &self,
        grant: SelfGrant,
        id: uuid::Uuid,
        expires_at: Option<time::OffsetDateTime>,
        request: <S::Notifications as DurableRequests>::Request,
        dispatch: F,
    ) -> Effect<A, E, R>
    where
        A: Send + 'static,
        E: Send + 'static,
        R: Send + Sync + 'static,
        M: Fn(IdentityError) -> E,
        S: NotificationSelection,
        F: Fn(AcceptedDocument<<S::Notifications as DurableRequests>::Request>) -> Fut
            + Send
            + Sync
            + 'static,
        Fut: Future<Output = Result<A, IdentityError>> + Send + 'static,
    {
        let dispatch = Arc::new(dispatch);
        self.invoke_capability_once(
            (grant, id, expires_at, request),
            move |storage, _, (grant, id, expires_at, request), authority, _| {
                let dispatch = dispatch.clone();
                async move {
                    let user = authority.revalidate(&grant).await?;
                    let repository = storage.notification_store()?;
                    if repository.domain() != RequestDomain::Notification {
                        return Err(IdentityError::Configuration("notification request domain"));
                    }
                    let accepted = repository
                        .accept(user.id(), id, expires_at, request)
                        .await?;
                    dispatch(accepted).await
                }
            },
        )
    }
    /// Persist an export/delete request after recent self proof and pass its
    /// receipt to an application-owned acceptance hook. No inventory or worker
    /// engine is supplied and acceptance is not job completion.
    pub fn request_personal_data_job<A, E, R, F, Fut>(
        &self,
        grant: RecentSelfGrant,
        id: uuid::Uuid,
        expires_at: Option<time::OffsetDateTime>,
        request: <S::Jobs as DurableRequests>::Request,
        accept: F,
    ) -> Effect<A, E, R>
    where
        A: Send + 'static,
        E: Send + 'static,
        R: Send + Sync + 'static,
        M: Fn(IdentityError) -> E,
        S: JobSelection,
        F: Fn(AcceptedDocument<<S::Jobs as DurableRequests>::Request>) -> Fut
            + Send
            + Sync
            + 'static,
        Fut: Future<Output = Result<A, IdentityError>> + Send + 'static,
    {
        let accept = Arc::new(accept);
        self.invoke_capability_once(
            (grant, id, expires_at, request),
            move |storage, _, (grant, id, expires_at, request), authority, _| {
                let accept = accept.clone();
                async move {
                    let user = authority.revalidate_recent(&grant).await?;
                    let repository = storage.job_store()?;
                    if repository.domain() != RequestDomain::Job {
                        return Err(IdentityError::Configuration("job request domain"));
                    }
                    let accepted = repository
                        .accept(user.id(), id, expires_at, request)
                        .await?;
                    accept(accepted).await
                }
            },
        )
    }
    /// Inspect this owner's durable job request/status under current self authority.
    /// Corruption/availability remain failures, distinct from absent status.
    pub fn personal_data_job<E, R>(
        &self,
        grant: SelfGrant,
        id: uuid::Uuid,
    ) -> Effect<Option<VersionedDocument<<S::Jobs as DurableRequests>::Request>>, E, R>
    where
        E: Send + 'static,
        R: Send + Sync + 'static,
        M: Fn(IdentityError) -> E,
        S: JobSelection,
    {
        identity! {
        identity: self, invoke_capability_once((grant, id)), native: |storage, _, (grant, id), authority, _| async move {
                        let user = authority.revalidate(&grant).await?;
                        let repository = storage.job_store()?;
                        if repository.domain() != RequestDomain::Job {
                            return Err(IdentityError::Configuration("job request domain"));
                        }
                        let state = repository
                            .find(user.id(), id, authority.clock().now())
                            .await?;
                        authority.revalidate(&grant).await?;
                        Ok(state)
                    },
        }
    }
    /// Apply a selected native authorization policy before a route or route collection.
    /// Only this definition's privately verified grant supplies identity. Principal claims
    /// are generated from current native account/role sources, never request extensions.
    /// Native challenges remain 401 and insufficient roles/claims/permissions remain 403.
    pub fn authorization_stage<E, R, A>(
        &self,
        policy: A,
    ) -> impl Fn(effect_server::Request) -> Effect<(), E, R> + Send + Sync + 'static
    where
        E: Send + 'static,
        R: Send + Sync + 'static,
        M: Fn(IdentityError) -> E,
        C::Native: SignInService,
        Users<C::Native>: UserLifecycleManager,
        P: PrincipalSelection,
        P::Principal: PrincipalComponents<Users<C::Native>>,
        A: identity_rs::identity::axum::AuthorizationPolicy + 'static,
    {
        let definition = self.clone();
        let policy = Arc::new(policy);
        move |request| {
            let policy = policy.clone();
            definition.invoke_once(request, move |native, request, authority, policies| {
                let policy = policy.clone();
                async move {
                    use identity_rs::identity::axum::AuthorizationDecision;
                    let actor = request
                        .with_extensions(|extensions| extensions.get::<SelfGrant>().cloned());
                    let principal = if let Some(actor) = &actor {
                        let verified = authority.revalidate(actor).await?;
                        let user = current_user(native.user_manager(), verified).await?;
                        Some(
                            policies
                                .principal_policy()?
                                .create(native.user_manager(), &user)
                                .await
                                .map_err(IdentityError::Native)?,
                        )
                    } else {
                        None
                    };
                    let decision = policy.authorize(principal.as_ref()).await;
                    if let Some(actor) = &actor {
                        authority.revalidate(actor).await?;
                    }
                    match decision {
                        AuthorizationDecision::Allowed => Ok(()),
                        AuthorizationDecision::Challenged => {
                            Err(IdentityError::Policy(http::StatusCode::UNAUTHORIZED))
                        }
                        AuthorizationDecision::Forbidden => {
                            Err(IdentityError::Policy(http::StatusCode::FORBIDDEN))
                        }
                    }
                }
            })
        }
    }
    /// Trusted cookie authentication stage for Web/Server policy composition.
    /// Current private authority and native renewal/removal stay in the request scope.
    pub fn cookie_stage<E, R>(
        &self,
    ) -> impl Fn(effect_server::Request) -> Effect<bool, E, R> + Send + Sync + 'static
    where
        E: Send + 'static,
        R: Send + Sync + 'static,
        M: Fn(IdentityError) -> E,
        C::Native: SignInService,
        Users<C::Native>:
            identity_rs::identity::principal::security_stamp::SecurityStampUserAccessor,
        P: TicketSelection + PrincipalSelection,
        P::Service: CookieSessionService,
        P::Principal: PrincipalComponents<Users<C::Native>>,
    {
        let definition = self.clone();
        move |request: effect_server::Request| {
            effect_map! {
            program: definition
            .authenticate_cookie(request.headers().clone()), map: move |outcome| {
                let (authenticated, header) = match outcome {
                    CookieAuthentication::Missing => (false, None),
                    CookieAuthentication::Rejected { removal } => (false, Some(removal)),
                    CookieAuthentication::Authenticated { actor, renewal, .. } => {
                        request.insert_extension(actor);
                        (true, renewal)
                    }
                };
                if let Some(header) = header {
                    request.with_extensions(|extensions| {
                        if let Some(headers) =
                            extensions.get::<effect_server::ResponseHeaders>()
                        {
                            headers
                                .0
                                .lock()
                                .unwrap_or_else(std::sync::PoisonError::into_inner)
                                .append(http::header::SET_COOKIE, header);
                        }
                    });
                }
                authenticated
            }, }
        }
    }
    /// Authenticate access credentials with the selected D18 provider and the actual
    /// configured current-user manager, including protected personal-data adaptation.
    /// Native malformed/rejected proofs deny; original provider/storage failures remain errors.
    pub fn selected_bearer_stage<E, R>(
        &self,
    ) -> impl Fn(effect_server::Request) -> Effect<bool, E, R> + Send + Sync + 'static
    where
        E: Send + 'static,
        R: Send + Sync + 'static,
        M: Fn(IdentityError) -> E,
        C::Native: SignInService,
        Users<C::Native>: UserLifecycleManager,
        P: TokenSelection,
        P::Provider: UserTokenProvider<IdentityUser>,
    {
        let definition = self.clone();
        move |request: effect_server::Request| {
            identity! {
            identity: definition, invoke_once(request), native: |native, request, authority, policies| async move {
                            let values = request.headers().get_all(http::header::AUTHORIZATION);
                            if values.iter().count() > 1 {
                                return Err(IdentityError::Policy(http::StatusCode::BAD_REQUEST));
                            }
                            let Some(header) = values.iter().next() else {
                                return Ok(false);
                            };
                            let Ok(text) = header.to_str() else {
                                return Ok(false);
                            };
                            let Some((scheme, token)) = text.split_once(' ') else {
                                return Ok(false);
                            };
                            if !scheme.eq_ignore_ascii_case("Bearer") {
                                return Ok(false);
                            }
                            let parsed = match identity_rs::identity::tokens::parse_bearer_token(token) {
                                Ok(parsed) => parsed,
                                Err(identity_rs::identity::tokens::BearerTokenError::InvalidToken) => {
                                    return Ok(false);
                                }
                                Err(error) => return Err(IdentityError::Bearer(error)),
                            };
                            let Some(user) = native
                                .user_manager()
                                .find_by_id(&parsed.user_id())
                                .await
                                .map_err(IdentityError::Native)?
                            else {
                                return Ok(false);
                            };
                            match identity_rs::identity::tokens::validate_user_access_token(
                                policies.token_provider()?,
                                token,
                                &user,
                            )
                            .await
                            {
                                Ok(true) => {
                                    let actor = authority.verify_user_grant(&user).await?;
                                    request.insert_extension(actor);
                                    Ok(true)
                                }
                                Ok(false)
                                | Err(identity_rs::identity::tokens::BearerTokenError::InvalidToken) => {
                                    Ok(false)
                                }
                                Err(error) => Err(IdentityError::Bearer(error)),
                            }
                        },
            }
        }
    }
    /// Seal genuine native primary proof into the selected browser context. Storage
    /// acknowledgment consumes this owned receipt; copying grants cannot establish
    /// one-use. External metadata uses its native external HTTP group or the explicit
    /// external completion workflow, preserving the original verifier evidence.
    pub fn retain_pending_browser<E, R>(
        &self,
        pending: PendingLoginGrant,
        headers: http::HeaderMap,
    ) -> Effect<(), E, R>
    where
        E: Send + 'static,
        R: Send + Sync + 'static,
        M: Fn(IdentityError) -> E,
        P: BrowserSelection + TokenSelection,
        P::Browser: BrowserComponents<S::Backend>,
        P::Provider: UserTokenProvider<IdentityUser>,
    {
        identity! {
        identity: self, invoke_once((pending, headers)), native: |_, (pending, headers), authority, policies| async move {
                        authority.validate_pending(&pending).await?;
                        if pending.external.is_some() {
                            return Err(IdentityError::Configuration(
                                "external pending metadata requires its native external transport",
                            ));
                        }
                        let browser = policies
                            .browser_security()?
                            .bind_browser(authority.backend().clone(), authority.clock())?;
                        browser
                            .authorize(&headers)
                            .await
                            .map_err(IdentityError::Policy)?;
                        let binding = browser
                            .pending_binding(&headers)
                            .await
                            .map_err(IdentityError::Policy)?;
                        let proof = pending
                            .native()?
                            .protect_external_pending(policies.token_provider()?, &binding)
                            .await
                            .map_err(IdentityError::Bearer)?;
                        browser
                            .retain_pending(&headers, proof)
                            .await
                            .map_err(IdentityError::Policy)
                    },
        }
    }
    /// Atomically consume one native browser-bound primary proof and invoke native
    /// purpose/current-account/admission verification before minting pending authority.
    /// Rejection after consumption requires fresh primary authentication.
    pub fn resume_pending_browser<E, R>(
        &self,
        headers: http::HeaderMap,
    ) -> Effect<Option<PendingLoginGrant>, E, R>
    where
        E: Send + 'static,
        R: Send + Sync + 'static,
        M: Fn(IdentityError) -> E,
        C::Native: ExternalLoginSignInManager,
        P: BrowserSelection + TokenSelection,
        P::Browser: BrowserComponents<S::Backend>,
        P::Provider: UserTokenProvider<IdentityUser>,
    {
        identity! {
        identity: self, invoke_once(headers), native: |native, headers, authority, policies| async move {
                    let browser = policies
                        .browser_security()?
                        .bind_browser(authority.backend().clone(), authority.clock())?;
                    browser
                        .authorize(&headers)
                        .await
                        .map_err(IdentityError::Policy)?;
                    let Some((binding, proof)) = browser
                        .take_pending(&headers)
                        .await
                        .map_err(IdentityError::Policy)?
                    else {
                        return Ok(None);
                    };
                    let Some(pending) = native
                        .resume_external_pending(policies.token_provider()?, &proof, &binding)
                        .await
                        .map_err(IdentityError::Native)?
                    else {
                        return Ok(None);
                    };
                    let session = identity_rs::identity::SignInSession::from_verified(
                        identity_rs::identity::AuthenticationOutcome::AdditionalFactorRequired(pending),
                    );
                    Ok(Some(authority.pending(Arc::new(session))?))
                },
        }
    }
    /// Issue the selected native server-backed browser context and synchronizer
    /// material. Native response/body ownership and narrowed SQL outcomes survive
    /// the Effect boundary; no browser context is interpreted as authentication.
    pub fn issue_antiforgery<E, R>(
        &self,
        headers: http::HeaderMap,
    ) -> Effect<http::Response<axum::body::Body>, E, R>
    where
        E: Send + 'static,
        R: Send + Sync + 'static,
        M: Fn(IdentityError) -> E,
        P: BrowserSelection + TicketSelection,
        P::Browser: BrowserComponents<S::Backend>,
        P::Service: CookieSessionService,
    {
        identity! {
        identity: self, invoke_once(headers), native: |_, headers, authority, policies| async move {
                    let browser = policies
                        .browser_security()?
                        .bind_browser(authority.backend().clone(), authority.clock())?;
                    if browser.auth_cookie_name() != policies.ticket_service()?.cookie_name() {
                        return Err(IdentityError::Configuration(
                            "browser authentication cookie mismatch",
                        ));
                    }
                    Ok(browser.issue(&headers).await)
                },
        }
    }
    /// Selected native CSRF factory retains the same original SQL failure carrier
    /// as the whole workflow, including browser context errors narrowed to status.
    pub fn selected_antiforgery_stage<E, R>(
        &self,
    ) -> impl Fn(effect_server::Request) -> Effect<bool, E, R> + Send + Sync + 'static
    where
        E: Send + 'static,
        R: Send + Sync + 'static,
        M: Fn(IdentityError) -> E,
        P: BrowserSelection,
        P::Browser: BrowserComponents<S::Backend>,
    {
        let definition = self.clone();
        move |request: effect_server::Request| {
            identity! {
            identity: definition, invoke_once(request), native: |_, request, authority, policies| async move {
                            if request.method() == http::Method::GET
                                || request.method() == http::Method::HEAD
                                || request.method() == http::Method::OPTIONS
                            {
                                return Ok(true);
                            }
                            let actor = request
                                .with_extensions(|extensions| extensions.get::<SelfGrant>().cloned())
                                .ok_or(IdentityError::Unauthorized)?;
                            let browser = policies
                                .browser_security()?
                                .bind_browser(authority.backend().clone(), authority.clock())?;
                            let grant = authority
                                .protect_request(&actor, &request, &browser)
                                .await?;
                            request.insert_extension(grant);
                            Ok(true)
                        },
            }
        }
    }
    /// Remove the exact selected cookie and native browser context after current
    /// authorization. Browser invalidation must acknowledge before success.
    pub fn logout_cookie<E, R>(
        &self,
        grant: SelfGrant,
        headers: http::HeaderMap,
    ) -> Effect<http::HeaderValue, E, R>
    where
        E: Send + 'static,
        R: Send + Sync + 'static,
        M: Fn(IdentityError) -> E,
        C::Native: SignInService,
        Users<C::Native>: UserAuthenticationTokenManager,
        P: TicketSelection + BrowserSelection,
        P::Service: CookieSessionService,
        P::Browser: BrowserComponents<S::Backend>,
    {
        identity! {
        identity: self, invoke_once((grant, headers)), native: |native, (grant, headers), authority, policies| async move {
                        let user = current_user(native.user_manager(), authority.revalidate(&grant).await?)
                            .await?;
                        let browser = policies
                            .browser_security()?
                            .bind_browser(authority.backend().clone(), authority.clock())?;
                        if browser.auth_cookie_name() != policies.ticket_service()?.cookie_name() {
                            return Err(IdentityError::Configuration(
                                "browser authentication cookie mismatch",
                            ));
                        }
                        browser
                            .authorize(&headers)
                            .await
                            .map_err(IdentityError::Policy)?;
                        acknowledged(
                            native
                                .user_manager()
                                .remove_authentication_token(
                                    &user,
                                    &identity_rs::identity::tokens::bearer_login_provider(),
                                    &identity_rs::identity::tokens::refresh_token_name(),
                                )
                                .await,
                        )?;
                        browser
                            .invalidate(&headers)
                            .await
                            .map_err(IdentityError::Policy)?;
                        policies
                            .ticket_service()?
                            .sign_out_cookie()
                            .map_err(IdentityError::Ticket)
                    },
        }
    }
}
