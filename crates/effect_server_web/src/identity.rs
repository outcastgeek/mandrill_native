//! Optional composition over actual trusted Identity verifier/private-grant stages.
//!
//! Native tickets or arbitrary request extensions do not establish authentication.
//! These adapters delegate proof validation and server-backed mutation protection to
//! D18's selected manager/provider generation. Its invocation-owned backend retains
//! original SQL failures across narrowed native interfaces, and the configured user
//! manager retains selected personal-data protection. Web places those Effects in
//! native order without creating raw-store verifier shortcuts or default providers.

use crate::security::SecurityGuard;
use effect::{PreparedAdmissionError, PreparedProviderAdmission};
use effect_server_identity::capabilities::StoreSelection;
use effect_server_identity::components::{BrowserComponents, PrincipalComponents};
use effect_server_identity::native::identity::{
    manager::{SignInService, UserLifecycleManager},
    models::IdentityUser,
    principal::security_stamp::SecurityStampUserAccessor,
    ticket::CookieSessionService,
    tokens::UserTokenProvider,
};
use effect_server_identity::policies::{
    BrowserSelection, PrincipalSelection, TicketSelection, TokenSelection,
};
use effect_server_identity::{IdentityComponents, IdentityDefinition, IdentityError};
use std::num::NonZeroUsize;
use std::sync::Arc;

/// Hold one selected work control before invoking a trusted Identity stage.
/// The original verifier still establishes all authentication authority.
#[must_use]
pub fn admitted_stage<E: Send + 'static, R: Send + Sync + 'static>(
    stage: SecurityGuard<E, R>,
    owner: Arc<PreparedProviderAdmission>,
    on_capacity: impl Fn(PreparedAdmissionError) -> E + Send + Sync + 'static,
) -> SecurityGuard<E, R> {
    let on_capacity = Arc::new(on_capacity);
    Arc::new(move |request| {
        let stage = stage.clone();
        let on_capacity = on_capacity.clone();
        crate::admission::prepared_independent(
            owner.clone(),
            move || stage(request.clone()),
            move |error| on_capacity(error),
        )
    })
}

/// Adapt D18's selected native bearer provider and configured current-user manager
/// before application environment projection. Each invocation uses Identity's real
/// backend error capture and selected personal-data adaptation. Missing/rejected
/// proofs yield false; provider/SQL/cleanup failures retain their original Effect
/// outcome through the definition's error mapping. Successful verification alone
/// installs Identity's privately constructed SelfGrant.
#[must_use]
pub fn verified_bearer<S, C, P, M, E, R>(
    definition: &IdentityDefinition<S, C, P, M>,
) -> SecurityGuard<E, R>
where
    S: StoreSelection,
    C: IdentityComponents<S::Backend, P>,
    P: TokenSelection + Send + Sync + 'static,
    M: Fn(IdentityError) -> E + Send + Sync + 'static,
    C::Native: SignInService,
    <C::Native as SignInService>::UserManager: UserLifecycleManager,
    P::Provider: UserTokenProvider<IdentityUser>,
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    Arc::new(definition.selected_bearer_stage())
}
/// Build Web's bearer authentication hook with the definition's prepared
/// invocation and an explicit aggregate credential header byte limit.
#[must_use]
pub fn prepared_verified_bearer<S, C, P, M, E, R>(
    definition: &IdentityDefinition<S, C, P, M>,
    header_bytes: NonZeroUsize,
) -> SecurityGuard<E, R>
where
    S: StoreSelection,
    C: IdentityComponents<S::Backend, P>,
    P: TokenSelection + Send + Sync + 'static,
    M: Fn(IdentityError) -> E + Send + Sync + 'static,
    C::Native: SignInService,
    <C::Native as SignInService>::UserManager: UserLifecycleManager,
    P::Provider: UserTokenProvider<IdentityUser>,
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    Arc::new(definition.prepared_bearer_stage(header_bytes))
}
/// Adapt D18's selected cookie/ticket, principal and configured current-user manager
/// before request environment projection. Verified authority is private; native
/// renewal/removal headers append to Server's staged response metadata. Protected
/// account adaptation and original invocation SQL failures remain with Identity.
#[must_use]
pub fn verified_cookie<S, C, P, M, E, R>(
    definition: &IdentityDefinition<S, C, P, M>,
) -> SecurityGuard<E, R>
where
    S: StoreSelection,
    C: IdentityComponents<S::Backend, P>,
    P: TicketSelection + PrincipalSelection + Send + Sync + 'static,
    M: Fn(IdentityError) -> E + Send + Sync + 'static,
    C::Native: SignInService,
    <C::Native as SignInService>::UserManager: SecurityStampUserAccessor,
    P::Service: CookieSessionService,
    P::Principal: PrincipalComponents<<C::Native as SignInService>::UserManager>,
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    Arc::new(definition.cookie_stage())
}
/// Build Web's cookie authentication hook with the definition's prepared
/// invocation and an explicit aggregate credential header byte limit.
#[must_use]
pub fn prepared_verified_cookie<S, C, P, M, E, R>(
    definition: &IdentityDefinition<S, C, P, M>,
    header_bytes: NonZeroUsize,
) -> SecurityGuard<E, R>
where
    S: StoreSelection,
    C: IdentityComponents<S::Backend, P>,
    P: TicketSelection + PrincipalSelection + Send + Sync + 'static,
    M: Fn(IdentityError) -> E + Send + Sync + 'static,
    C::Native: SignInService,
    <C::Native as SignInService>::UserManager: SecurityStampUserAccessor,
    P::Service: CookieSessionService,
    P::Principal: PrincipalComponents<<C::Native as SignInService>::UserManager>,
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    Arc::new(definition.prepared_cookie_stage(header_bytes))
}
/// Adapt D18's selected server-backed browser factory after authentication and
/// before writes/cache handling. The factory binds the actual invocation backend,
/// retaining original SQL failures even when native browser APIs narrow errors to
/// HTTP status. Unsafe requests revalidate private actor authority and bind a
/// ProtectedRequestGrant to the exact method/URI; safe methods need no mutation proof.
#[must_use]
pub fn server_antiforgery<S, C, P, M, E, R>(
    definition: &IdentityDefinition<S, C, P, M>,
) -> SecurityGuard<E, R>
where
    S: StoreSelection,
    C: IdentityComponents<S::Backend, P>,
    P: BrowserSelection + Send + Sync + 'static,
    M: Fn(IdentityError) -> E + Send + Sync + 'static,
    P::Browser: BrowserComponents<S::Backend>,
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    Arc::new(definition.selected_antiforgery_stage())
}
