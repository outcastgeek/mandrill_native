//! Retained native components and concrete manager parameterization by selected stores.
//!
//! Every replacement service is supplied explicitly. Immutable retained wrappers
//! forward native algorithms without copying credentials or creating another crypto
//! engine. Native builder defaults are replaced before any operation can observe them.

use crate::{
    IdentityError, authority::IdentityClock, capabilities::NativeBackend,
    definition::IdentityComponents,
};
use identity_rs::identity::{
    IdentityOptions, IdentityResult,
    error::IdentityErrorDescriber,
    manager::{SignInManager, SignInPolicy, UserManager},
    models::{AuthenticatorKey, IdentityUser, UserId},
    normalization::LookupNormalizer,
    password::{
        PasswordHashError, PasswordHashString, PasswordHasher, PasswordValidator,
        PasswordVerificationResult,
    },
    store::UserStore,
    tokens::{AuthenticatorTokenContext, TokenProviderError, UserTokenProvider},
    validation::UserValidator,
};
use std::{num::NonZeroUsize, sync::Arc};

/// Retained explicitly selected service. Clone shares only its configuration Arc.
pub struct Retained<T>(Arc<T>);
impl<T> Retained<T> {
    /// Retain one explicitly selected generation for both native composition and D18.
    pub fn new(service: T) -> Self {
        Self(Arc::new(service))
    }
    /// Borrow its original native component without reacquiring a default.
    #[must_use]
    pub fn service(&self) -> &T {
        &self.0
    }
}
impl<T> Clone for Retained<T> {
    /// Retain the original native component without cloning application payloads.
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

/// Selected native browser construction with an invocation-owned real backend.
pub trait BrowserComponents<B: NativeBackend>: Send + Sync {
    /// Actual native server-backed origin/CSRF service.
    type Browser: identity_rs::identity::axum::browser::BrowserRequestSecurity;
    /// Purely compose this request's diagnostic store and compatible clock.
    fn bind_browser(
        &self,
        backend: B,
        clock: IdentityClock,
    ) -> Result<Self::Browser, IdentityError>;
}
/// Exact trusted-origin/cookie configuration retained once for native browser policy.
pub struct NativeBrowserPolicy {
    /// Original account-store configuration fence; a shared pool alone is insufficient.
    configuration: crate::capabilities::StorageConfiguration,
    /// Exact native-validated trusted origins retained from explicit application configuration.
    origins: Vec<String>,
    /// Exact selected authentication-cookie identity for browser-context binding.
    cookie: String,
}
impl NativeBrowserPolicy {
    /// Validate through the native policy constructor without acquiring persistence.
    pub fn new<B: NativeBackend + identity_rs::identity::store::BrowserContextStore>(
        backend: &B,
        origins: Vec<String>,
        cookie: String,
    ) -> Result<Self, IdentityError> {
        identity_rs::identity::axum::browser::BrowserSecurityPolicy::new(
            origins.clone(),
            backend.clone(),
            cookie.clone(),
        )
        .map_err(IdentityError::Policy)?;
        Ok(Self {
            configuration: backend.configuration(),
            origins,
            cookie,
        })
    }
}
impl<B: NativeBackend + identity_rs::identity::store::BrowserContextStore> BrowserComponents<B>
    for NativeBrowserPolicy
{
    type Browser = identity_rs::identity::axum::browser::BrowserSecurityPolicy<B>;
    /// Construct the native browser service over the exact scoped storage generation and clock.
    fn bind_browser(
        &self,
        backend: B,
        clock: IdentityClock,
    ) -> Result<Self::Browser, IdentityError> {
        if !self
            .configuration
            .same_configuration(&backend.configuration())
        {
            return Err(IdentityError::WrongConfiguration);
        }
        Ok(
            identity_rs::identity::axum::browser::BrowserSecurityPolicy::new(
                self.origins.clone(),
                backend,
                self.cookie.clone(),
            )
            .map_err(IdentityError::Policy)?
            .with_utc_now(clock.callable()),
        )
    }
}
/// Selected native principal generation over the invocation's actual account service.
pub trait PrincipalComponents<U: Send + Sync>: Send + Sync {
    /// Use native sources/algorithms; a public caller cannot turn the result into proof.
    fn create<'a>(
        &'a self,
        users: &'a U,
        user: &'a IdentityUser,
    ) -> impl std::future::Future<
        Output = Result<identity_rs::identity::principal::ClaimsPrincipal, IdentityResult>,
    > + Send
    + 'a;
    /// Exact native claim mapping retained by issuance and authentication.
    fn claims_options(&self) -> &identity_rs::identity::principal::ClaimsIdentityOptions;
}
/// Explicit native claim mapping; native principal sources remain current scoped managers.
pub struct NativePrincipalPolicy {
    /// Original selected native claim types/issuer and authentication mapping.
    options: identity_rs::identity::principal::ClaimsIdentityOptions,
}
impl NativePrincipalPolicy {
    /// Retain the caller-selected native claim mapping without reading account state.
    #[must_use]
    pub fn new(options: identity_rs::identity::principal::ClaimsIdentityOptions) -> Self {
        Self { options }
    }
}
impl<U: identity_rs::identity::principal::UserClaimsPrincipalSource<IdentityUser>>
    PrincipalComponents<U> for NativePrincipalPolicy
{
    /// Delegate current native principal construction to the retained configured source.
    async fn create<'a>(
        &'a self,
        users: &'a U,
        user: &'a IdentityUser,
    ) -> Result<identity_rs::identity::principal::ClaimsPrincipal, IdentityResult> {
        use identity_rs::identity::principal::UserClaimsPrincipalFactory;
        identity_rs::identity::principal::DefaultUserClaimsPrincipalFactory::new(users)
            .with_options(self.options.clone())
            .create(user)
            .await
    }
    /// Borrow the original native claim mapping shared by issuance and validation.
    fn claims_options(&self) -> &identity_rs::identity::principal::ClaimsIdentityOptions {
        &self.options
    }
}
/// Explicit credential advertisement policy; selected native providers enforce
/// their actual expiry. Applications align this bound with those provider settings.
pub trait BearerIssuance: Send + Sync {
    /// Caller-selected nonzero access-token lifetime in native seconds.
    fn access_lifetime_seconds(&self) -> std::num::NonZeroU64;
}
/// Native provider and its explicit transport lifetime, retained as one generation.
pub struct NativeBearerPolicy<T> {
    /// Original explicitly selected native purpose-bound provider.
    provider: T,
    /// Explicit matching transport advertisement, independent of cryptographic issuance.
    lifetime: std::num::NonZeroU64,
}
impl<T> NativeBearerPolicy<T> {
    /// Select the native crypto provider and matching advertised access duration.
    pub fn new(provider: T, lifetime: std::num::NonZeroU64) -> Self {
        Self { provider, lifetime }
    }
}
impl<T: Send + Sync> BearerIssuance for NativeBearerPolicy<T> {
    /// Return only the explicitly selected transport lifetime; native providers enforce expiry.
    fn access_lifetime_seconds(&self) -> std::num::NonZeroU64 {
        self.lifetime
    }
}
impl<T: UserTokenProvider<IdentityUser>> UserTokenProvider<IdentityUser> for NativeBearerPolicy<T> {
    /// Preserve selected native configuration rejection without substituting defaults.
    fn validate_configuration(&self) -> Result<(), TokenProviderError> {
        self.provider.validate_configuration()
    }
    /// Forward native purpose-bound issuance without copying or formatting credential bytes.
    async fn generate(
        &self,
        purpose: &str,
        user: &IdentityUser,
    ) -> Result<String, TokenProviderError> {
        self.provider.generate(purpose, user).await
    }
    /// Forward native purpose-bound verification and preserve provider failure separately from denial.
    async fn validate(
        &self,
        purpose: &str,
        token: &str,
        user: &IdentityUser,
    ) -> Result<bool, TokenProviderError> {
        self.provider.validate(purpose, token, user).await
    }
    /// Preserve the selected provider eligibility decision and actual failure.
    async fn can_generate_two_factor_token(
        &self,
        user: &IdentityUser,
    ) -> Result<bool, TokenProviderError> {
        self.provider.can_generate_two_factor_token(user).await
    }
}
/// Explicit selected ticket issuance policy, distinct from native-cookie transport.
pub trait SessionIssuance: Send + Sync {
    /// Positive selected native UTC duration, checked at component construction.
    fn lifetime(&self) -> time::Duration;
    /// Explicit selected renewal eligibility; this does not mint recent-self proof.
    fn allow_refresh(&self) -> bool;
}
/// Selected native cookie service plus explicit UTC lifetime/refresh preference.
pub struct NativeCookiePolicy<T> {
    /// Original selected native cookie protection and transport service.
    service: T,
    /// Explicit positive UTC ticket issuance lifetime.
    lifetime: time::Duration,
    /// Selected renewal eligibility; renewed authentication never supplies recent proof.
    refresh: bool,
}
impl<T> NativeCookiePolicy<T> {
    /// Validate the explicit session policy without issuing a cookie or reading UTC.
    pub fn new(service: T, lifetime: time::Duration, refresh: bool) -> Result<Self, IdentityError> {
        if lifetime <= time::Duration::ZERO {
            return Err(IdentityError::Configuration("session lifetime"));
        }
        Ok(Self {
            service,
            lifetime,
            refresh,
        })
    }
}
impl<T: Send + Sync> SessionIssuance for NativeCookiePolicy<T> {
    /// Return the original positive UTC lifetime selected for native ticket issuance.
    fn lifetime(&self) -> time::Duration {
        self.lifetime
    }
    /// Return selected renewal eligibility without minting fresh proof.
    fn allow_refresh(&self) -> bool {
        self.refresh
    }
}
impl<T: identity_rs::identity::ticket::CookieSessionService>
    identity_rs::identity::ticket::CookieSessionService for NativeCookiePolicy<T>
{
    type Format = T::Format;
    /// Borrow the original selected cookie identity used by native browser binding.
    fn cookie_name(&self) -> &str {
        self.service.cookie_name()
    }
    /// Preserve selected native configuration rejection without substituting defaults.
    fn validate_configuration(&self) -> Result<(), identity_rs::identity::ticket::TicketError> {
        self.service.validate_configuration()
    }
    /// Delegate selected native ticket protection and cookie rendering without another crypto format.
    async fn sign_in_cookie(
        &self,
        ticket: &identity_rs::identity::ticket::AuthenticationTicket,
    ) -> Result<http::HeaderValue, identity_rs::identity::ticket::TicketError> {
        self.service.sign_in_cookie(ticket).await
    }
    /// Return the selected native cookie removal header after the owning workflow acknowledges invalidation.
    fn sign_out_cookie(
        &self,
    ) -> Result<http::HeaderValue, identity_rs::identity::ticket::TicketError> {
        self.service.sign_out_cookie()
    }
    /// Delegate native format validation with the invocation-owned current-user verifier.
    async fn authenticate_with_validator<
        V: identity_rs::identity::ticket::AuthenticationTicketValidator,
    >(
        &self,
        headers: &http::HeaderMap,
        validator: &V,
    ) -> Result<
        identity_rs::identity::ticket::CookieSessionResult,
        identity_rs::identity::ticket::TicketError,
    > {
        self.service
            .authenticate_with_validator(headers, validator)
            .await
    }
}
impl<T> std::fmt::Debug for Retained<T> {
    /// Do not format native keys, options with secrets or implementation internals.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Retained(native component)")
    }
}
impl<T: UserValidator<IdentityUser>> UserValidator<IdentityUser> for Retained<T> {
    /// Delegate immutable native validation; options were applied once before retention.
    fn validate(&self, user: &IdentityUser) -> IdentityResult {
        self.0.validate(user)
    }
}
impl<
    T: identity_rs::identity::validation::RoleValidator<identity_rs::identity::models::IdentityRole>,
> identity_rs::identity::validation::RoleValidator<identity_rs::identity::models::IdentityRole>
    for Retained<T>
{
    /// Delegate the exact selected native role validation vocabulary.
    fn validate(&self, role: &identity_rs::identity::models::IdentityRole) -> IdentityResult {
        self.0.validate(role)
    }
}
/// Explicit native role composition over the same actual invocation backend.
pub struct NativeRoleComponents<V, D, LN> {
    /// Immutable selected native role validator.
    validator: Retained<V>,
    /// Original native validation/error vocabulary.
    errors: D,
    /// Explicit native equality/lookup policy, replaced before asynchronous work.
    normalizer: Retained<LN>,
}
impl<V, D, LN> NativeRoleComponents<V, D, LN> {
    /// Retain native validators, equality policy and public errors without I/O/default services.
    pub fn new(validator: V, errors: D, normalizer: LN) -> Self {
        Self {
            validator: Retained::new(validator),
            errors,
            normalizer: Retained::new(normalizer),
        }
    }
}
impl<B: NativeBackend, P, V, D, LN> IdentityComponents<B, P> for NativeRoleComponents<V, D, LN>
where
    V: identity_rs::identity::validation::RoleValidator<
            identity_rs::identity::models::IdentityRole,
        > + 'static,
    D: IdentityErrorDescriber + 'static,
    LN: LookupNormalizer + 'static,
{
    type Native = identity_rs::identity::manager::RoleManager<B, Retained<V>, D, Retained<LN>>;
    /// Native builder temporary normalization is replaced before any async operation.
    fn construct(
        &self,
        backend: B,
        _clock: IdentityClock,
        _policies: Arc<P>,
    ) -> Result<Self::Native, IdentityError> {
        Ok(
            identity_rs::identity::manager::RoleManager::with_components(
                backend,
                self.validator.clone(),
                self.errors.clone(),
            )
            .with_lookup_normalizer(self.normalizer.clone()),
        )
    }
}
impl<T: PasswordValidator<IdentityUser>> PasswordValidator<IdentityUser> for Retained<T> {
    /// Delegate the selected plaintext policy without formatting/copying its secret.
    fn validate(&self, user: &IdentityUser, password: Option<&str>) -> IdentityResult {
        self.0.validate(user, password)
    }
}
impl<T: PasswordHasher<IdentityUser>> PasswordHasher<IdentityUser> for Retained<T> {
    /// Delegate native selected hash work; native manager supplies bounded asynchronous execution.
    fn hash_password(
        &self,
        user: &IdentityUser,
        password: &str,
    ) -> Result<PasswordHashString, PasswordHashError> {
        self.0.hash_password(user, password)
    }
    /// Delegate native format/cost/constant-time verification rather than reimplementing it.
    fn verify_hashed_password(
        &self,
        user: &IdentityUser,
        hash: &str,
        password: &str,
    ) -> PasswordVerificationResult {
        self.0.verify_hashed_password(user, hash, password)
    }
}
impl<T: LookupNormalizer> LookupNormalizer for Retained<T> {
    /// Use the originally selected name equality policy for every operation.
    fn normalize_name(&self, name: Option<&str>) -> Option<String> {
        self.0.normalize_name(name)
    }
    /// Use the originally selected email equality policy for every operation.
    fn normalize_email(&self, email: Option<&str>) -> Option<String> {
        self.0.normalize_email(email)
    }
}
impl<T: SignInPolicy> SignInPolicy for Retained<T> {
    /// Preserve selected admission restrictions; native credential/factor proof remains mandatory.
    fn allows(&self, user: &IdentityUser) -> Result<bool, IdentityResult> {
        self.0.allows(user)
    }
}
impl<User: Sync, T: UserTokenProvider<User>> UserTokenProvider<User> for Retained<T> {
    /// Preserve native readiness/unsupported-purpose reporting without selecting a fallback.
    fn validate_configuration(&self) -> Result<(), TokenProviderError> {
        self.0.validate_configuration()
    }
    /// Issue only through the selected native purpose/user-bound algorithm.
    async fn generate(&self, purpose: &str, user: &User) -> Result<String, TokenProviderError> {
        self.0.generate(purpose, user).await
    }
    /// Infrastructure failures remain errors; rejected native proof remains false.
    async fn validate(
        &self,
        purpose: &str,
        token: &str,
        user: &User,
    ) -> Result<bool, TokenProviderError> {
        self.0.validate(purpose, token, user).await
    }
    /// Delegate native factor-material capability metadata, not authentication assurance.
    async fn can_generate_two_factor_token(&self, user: &User) -> Result<bool, TokenProviderError> {
        self.0.can_generate_two_factor_token(user).await
    }
}
impl<T: identity_rs::identity::protection::PersonalDataStorageProtector>
    identity_rs::identity::protection::PersonalDataStorageProtector for Retained<T>
{
    /// Delegate native authenticated encryption with exact account/field binding.
    async fn protect_field(
        &self,
        id: UserId,
        field: identity_rs::identity::protection::PersonalDataField,
        value: &str,
    ) -> Result<
        identity_rs::identity::protection::storage::EncryptedPersonalValue,
        identity_rs::identity::protection::PersonalDataProtectionError,
    > {
        self.0.protect_field(id, field, value).await
    }
    /// Delegate selected readable-key/authentication/strict decoding policy.
    async fn unprotect_field(
        &self,
        id: UserId,
        field: identity_rs::identity::protection::PersonalDataField,
        value: &identity_rs::identity::protection::storage::EncryptedPersonalValue,
    ) -> Result<String, identity_rs::identity::protection::PersonalDataProtectionError> {
        self.0.unprotect_field(id, field, value).await
    }
    /// Delegate the selected retained-key lookup versions; no plaintext index fallback exists.
    async fn lookup_keys(
        &self,
        field: identity_rs::identity::protection::PersonalDataField,
        value: &str,
    ) -> Result<
        Vec<identity_rs::identity::protection::storage::ProtectedLookupKey>,
        identity_rs::identity::protection::PersonalDataProtectionError,
    > {
        self.0.lookup_keys(field, value).await
    }
}
/// Explicit native user-store adaptation around the actual invocation backend.
pub trait UserStoreSelection<B: NativeBackend>: Send + Sync + 'static {
    /// Actual native user store passed to UserManager, including protection where selected.
    type Store: UserStore + 'static;
    /// Compose the selected store policy without I/O/default persistence acquisition.
    fn bind(&self, backend: B) -> Self::Store;
}
/// Explicit plaintext personal-field selection, never an implicit default provider.
pub struct PlainUserStore;
impl<B: NativeBackend> UserStoreSelection<B> for PlainUserStore {
    type Store = B;
    /// Pass through the actual invocation-owned native backend.
    fn bind(&self, backend: B) -> B {
        backend
    }
}
/// Explicit native protected-personal-data policy over the selected encrypted store.
pub struct ProtectedUserStore<P> {
    /// Native field protection/key retention service selected once.
    protector: Retained<P>,
}
impl<P> ProtectedUserStore<P> {
    /// Retain the selected native protector without generating/loading a hidden key provider.
    pub fn new(protector: P) -> Self {
        Self {
            protector: Retained(Arc::new(protector)),
        }
    }
}
impl<
    B: NativeBackend + identity_rs::identity::protection::storage::EncryptedPersonalDataStore,
    P: identity_rs::identity::protection::PersonalDataStorageProtector + 'static,
> UserStoreSelection<B> for ProtectedUserStore<P>
{
    type Store = identity_rs::identity::protection::ProtectedPersonalDataStore<B, Retained<P>>;
    /// Compose upstream's truthful protection wrapper over the exact selected encrypted backend.
    fn bind(&self, backend: B) -> Self::Store {
        identity_rs::identity::protection::ProtectedPersonalDataStore::new(
            backend,
            self.protector.clone(),
        )
    }
}
/// Concrete selected native user/sign-in services, retained before definition installation.
pub struct NativeUserComponents<SS, UV, PH, PV, LN, D, AP, SP> {
    /// Explicit plaintext/protected user-store policy.
    store: SS,
    /// Immutable validators were configured once with these native options.
    validator: Retained<UV>,
    /// Actual selected native hash algorithm/cost configuration.
    hasher: Retained<PH>,
    /// Actual selected plaintext password policy.
    password_validator: Retained<PV>,
    /// Native selected name/email equality policy.
    normalizer: Retained<LN>,
    /// Native interface explicitly requires Clone for its error vocabulary.
    errors: D,
    /// Immutable native eligibility/capability/expiry policy.
    options: IdentityOptions,
    /// Native compatible selected authenticator secret generator.
    generator: fn() -> Result<AuthenticatorKey, ()>,
    /// Selected native finite blocking-work bound.
    password_concurrency: NonZeroUsize,
    /// Actual native factor provider, selected once.
    authenticator: Retained<AP>,
    /// Application admission restriction, selected once.
    admission: Retained<SP>,
    /// Selected native authentication scheme identity.
    scheme: String,
}
impl<SS, UV: UserValidator<IdentityUser>, PH, PV: PasswordValidator<IdentityUser>, LN, D, AP, SP>
    NativeUserComponents<SS, UV, PH, PV, LN, D, AP, SP>
{
    /// Select all native services explicitly. Configure local validator options once
    /// before freezing retained generations; there is no hidden crypto/clock fallback.
    /// `user_services` follows the native user manager's store, validator, hasher,
    /// password validator, normalizer and error-describer order.
    pub fn new(
        user_services: (SS, UV, PH, PV, LN, D),
        options: IdentityOptions,
        generator: fn() -> Result<AuthenticatorKey, ()>,
        password_concurrency: NonZeroUsize,
        authenticator: AP,
        admission: SP,
        scheme: String,
    ) -> Result<Self, IdentityError> {
        let (store, mut validator, hasher, mut password_validator, normalizer, errors) =
            user_services;
        if scheme.trim().is_empty() {
            return Err(IdentityError::Configuration("authentication scheme"));
        }
        validator.configure_options(&options.user);
        password_validator.configure_options(options.password);
        Ok(Self {
            store,
            validator: Retained(Arc::new(validator)),
            hasher: Retained(Arc::new(hasher)),
            password_validator: Retained(Arc::new(password_validator)),
            normalizer: Retained(Arc::new(normalizer)),
            errors,
            options,
            generator,
            password_concurrency,
            authenticator: Retained(Arc::new(authenticator)),
            admission: Retained(Arc::new(admission)),
            scheme,
        })
    }
}
impl<B, P, SS, UV, PH, PV, LN, D, AP, SP> IdentityComponents<B, P>
    for NativeUserComponents<SS, UV, PH, PV, LN, D, AP, SP>
where
    B: NativeBackend,
    SS: UserStoreSelection<B>,
    UV: UserValidator<IdentityUser> + 'static,
    PH: PasswordHasher<IdentityUser>,
    PV: PasswordValidator<IdentityUser> + 'static,
    LN: LookupNormalizer + 'static,
    D: IdentityErrorDescriber + 'static,
    AP: for<'a> UserTokenProvider<AuthenticatorTokenContext<'a>> + 'static,
    SP: SignInPolicy,
{
    type Native = SignInManager<
        UserManager<SS::Store, Retained<UV>, Retained<PH>, Retained<PV>, Retained<LN>, D>,
        Retained<AP>,
    >;
    /// Demonstrably parameterize real native managers by the actual selected store;
    /// public native builders replace all clock/generator/admission/factor selections
    /// before any async method can observe their temporary constructor values.
    fn construct(
        &self,
        backend: B,
        clock: IdentityClock,
        _policies: Arc<P>,
    ) -> Result<Self::Native, IdentityError> {
        let users = UserManager::with_components_and_error_describer(
            self.store.bind(backend),
            self.validator.clone(),
            self.hasher.clone(),
            self.password_validator.clone(),
            self.normalizer.clone(),
            self.errors.clone(),
        )
        .with_utc_now(clock.callable())
        .with_authenticator_key_generator(self.generator)
        .with_password_work_limit(self.password_concurrency)
        .map_err(IdentityError::Native)?
        .with_options(self.options.clone())
        .map_err(IdentityError::Native)?;
        Ok(SignInManager::new(users)
            .with_authenticator_token_provider(self.authenticator.clone())
            .with_sign_in_policy(self.admission.clone())
            .with_authentication_scheme(self.scheme.clone()))
    }
}
