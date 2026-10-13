//! Configure one concrete café Identity definition; no consumer capability bounds escape.

use super::*;
use effect::{effect_map, effect_try_sync, service};
use effect_server::{endpoint, pipeline, reply, router};
use effect_server_identity::authority::{
    GrantPolicy, IdentityClock, PermissionDecision, PermissionOperation, PermissionTarget,
    ProtectedRequestGrant, SelfGrant,
};
use effect_server_identity::components::{
    BrowserComponents, NativeBrowserPolicy, NativeCookiePolicy, NativePrincipalPolicy, Retained,
};
use effect_server_identity::native::identity::{
    IdentityOptions, IdentityResult,
    error::DefaultIdentityErrorDescriber,
    manager::{SignInManager, SignInPolicy, UserManager},
    models::IdentityUser,
    normalization::UpperInvariantLookupNormalizer,
    password::{
        DefaultPasswordHasher, DefaultPasswordValidator, OsRandomBytes, PasswordHasherOptions,
    },
    principal::ClaimsIdentityOptions,
    ticket::{
        CookieSameSite, CookieSessionManager, CookieSessionOptions, DataProtectionTicketFormat,
        TicketDataFormatOptions, default_ticket_purposes,
    },
    tokens::{AuthenticatorTokenProvider, data_protection::DataProtectionKeyRing},
    validation::DefaultUserValidator,
};
use effect_server_identity::{
    IdentityError, IdentityStoragePolicy, SqlxIdentityStore, identity, identity_endpoints,
    identity_store,
};
use effect_server_identity::{
    definition::{LoginInput, LoginOutcome, RegistrationInput},
    native::identity::{
        axum::{RoleClaimAuthorizationPolicy, browser::BrowserRequestSecurity},
        models::{
            ClaimType, ClaimValue, IdentityClaim, IdentityRole, NewRole, NormalizedUserName,
            UserName,
        },
        store::RoleStore,
    },
    policies::BrowserSelection,
};
use effect_server_web::csp::{CspDirective, CspSources, ResponseNonce};
use effect_server_web::{csp, security};

/// Native sign-in admission cannot waive password, factor or current-user checks.
#[derive(Debug)]
struct CafeAdmission;
impl SignInPolicy for CafeAdmission {
    /// This café accepts named accounts after native credential verification.
    fn allows(&self, user: &IdentityUser) -> Result<bool, IdentityResult> {
        Ok(user.user_name().is_some())
    }
}
/// Explicit local customer policy, separate from host drain and cache budgets.
pub(super) struct CustomerPolicy {
    pub capacity: NonZeroUsize,
    pub ring: DataProtectionKeyRing,
    pub cookie: String,
    pub origin: String,
    pub secure: bool,
    pub session_seconds: NonZeroU64,
    pub recent_seconds: i64,
    pub hash_iterations: u32,
    pub bytes: NonZeroUsize,
}
/// Concrete application outputs; the native definition stays at its inference boundary.
pub(super) struct CustomerServices {
    pub security: Security<CafeError, Environment>,
    pub routes: Router<CafeError, Environment>,
    pub fixture: Effect<(), CafeError>,
}
/// Acquire the native store and compose real password/session/browser workflows.
pub(super) async fn configure(
    sql: SqlClient,
    policy: CustomerPolicy,
) -> Result<CustomerServices, Box<dyn std::error::Error>> {
    let capacity = policy.capacity;
    let cookie = policy.cookie;
    let origin = policy.origin;
    let secure = policy.secure;
    let session = Duration::from_secs(policy.session_seconds.get());
    let native_window = time::Duration::seconds(i64::try_from(policy.session_seconds.get())?);
    let ticket_format = DataProtectionTicketFormat::with_options(
        policy.ring,
        TicketDataFormatOptions::new()
            .with_purposes(default_ticket_purposes("Cafe.Customer"))
            .with_ticket_lifespan(session)
            .with_utc_now(OffsetDateTime::now_utc),
    );
    let tickets = NativeCookiePolicy::new(
        CookieSessionManager::with_options(
            ticket_format,
            CookieSessionOptions::new()
                .with_name(cookie.clone())
                .with_secure(secure)
                .with_http_only(true)
                .with_same_site(CookieSameSite::Strict)
                .with_utc_now(OffsetDateTime::now_utc),
        ),
        native_window,
        true,
    )?;
    let backend = SqlxIdentityStore::from_kernel(
        sql.kernel().clone(),
        IdentityStoragePolicy {
            challenge_capacity: capacity,
            utc_now: OffsetDateTime::now_utc,
        },
    )?;
    backend.initialize().await?;
    let role_name = NormalizedUserName::parse("CUSTOMER")?;
    if backend
        .find_role_by_normalized_name(&role_name)
        .await?
        .is_none()
    {
        let role = IdentityRole::with_name(UserName::parse("Customer")?)
            .with_name_and_normalized_name(UserName::parse("Customer")?, role_name);
        backend.create_role(NewRole::from_role(role)?).await?;
    }
    let browser = NativeBrowserPolicy::new(&backend, vec![origin], cookie)?;
    let validator = Retained::new(DefaultUserValidator::new());
    let passwords = Retained::new(DefaultPasswordValidator::new());
    let hasher = Retained::new(DefaultPasswordHasher::with_options_and_random(
        PasswordHasherOptions::new(policy.hash_iterations)?,
        OsRandomBytes,
    ));
    let normalizer = Retained::new(UpperInvariantLookupNormalizer::new());
    let authenticator = Retained::new(AuthenticatorTokenProvider::new());
    let admission = Retained::new(CafeAdmission);
    let recent_window = time::Duration::seconds(policy.recent_seconds);
    let definition = identity! {
        storage: identity_store! { backend: backend }?,
        native_factory |backend, clock, _policies| {
            let users = UserManager::with_components_and_error_describer(
                backend,
                validator.clone(),
                hasher.clone(),
                passwords.clone(),
                normalizer.clone(),
                DefaultIdentityErrorDescriber,
            )
            .with_utc_now(clock.callable())
            .with_password_work_limit(capacity)
            .map_err(IdentityError::Native)?
            .with_options(IdentityOptions::default())
            .map_err(IdentityError::Native)?;
            Ok(SignInManager::new(users)
                .with_authenticator_token_provider(authenticator.clone())
                .with_sign_in_policy(admission.clone())
                .with_authentication_scheme("Cafe.Customer"))

        },
        tickets: tickets, browser: browser, principal_factory: NativePrincipalPolicy::new(ClaimsIdentityOptions::default()),
        errors |error| { cafe_error(CafeProblem::Identity(Box::new(error))) }, clock: IdentityClock::new(OffsetDateTime::now_utc),
        grant_policy: GrantPolicy { self_lifetime: native_window, recent_lifetime: recent_window }, admission: capacity,
    }?;
    let mut selected_security = security! {
        csp: csp! { directive(CspDirective::DefaultSrc, CspSources::self_origin()); directive(CspDirective::ScriptSrc, CspSources::nonce()); directive(CspDirective::StyleSrc, CspSources::nonce()); },
        nonce: || {
        effect_try_sync! { body { ResponseNonce::new(uuid::Uuid::new_v4().simple().to_string()) }, on_error |error| { cafe_error(CafeProblem::Web(error)) }, }
        },
        authentication: definition.cookie_stage(),
        header(http::header::X_CONTENT_TYPE_OPTIONS, http::HeaderValue::from_static("nosniff"));
    }?;
    let verify = selected_security
        .authentication
        .take()
        .ok_or("selected native authentication stage")?;
    // Authentication verifies optional credentials globally. Route policies decide
    // which groups require them; public entry routes need no path allowlist.
    selected_security.authentication = Some(Arc::new(
        move |request| effect_map! { program: verify(request), map |_verified| { true }, },
    ));
    let login_accounts = definition.clone();
    let login_guard = move |_accounts, request: Request| {
        identity! {
            identity: login_accounts.clone(), authorize_once(request),
            authorize |request, authority, policies| {
                let browser = policies.browser_security()?.bind_browser(authority.backend().clone(), authority.clock())?;
                browser.authorize(request.headers()).await.map_err(IdentityError::Policy)?;
                Ok(())
            },
        }
    };
    // The request guard protects exactly the actor's pickup resource and action.
    // It revalidates browser proof for collection and fresh permission before SQL.
    let accounts = definition.clone();
    let resource_guard = move |action: &'static str, collection: bool| {
        let accounts = accounts.clone();
        let guard: Effect<(), CafeError, Environment> = effect! {
            let accounts = accounts.clone();
            let request = service! { env |env: &Environment| { env.request().clone() }, }?;
            identity! {
                identity: accounts, authorize_once(request),
                authorize |request, authority, policies| {
                    let actor = request.with_extensions(|extensions| extensions.get::<SelfGrant>().cloned()).ok_or(IdentityError::Unauthorized)?;
                    if collection {
                        let browser = policies.browser_security()?.bind_browser(authority.backend().clone(), authority.clock())?;
                        let proof = request.with_extensions(|extensions| extensions.get::<ProtectedRequestGrant>().cloned()).ok_or(IdentityError::Unauthorized)?;
                        authority.revalidate_protected(&proof, &request, &browser).await?;
                    }
                    let action: Arc<str> = Arc::from(action);
                    let owner = actor.user_id();
                    let resource: Arc<str> = Arc::from(format!("pickup/{owner}"));
                    let expected_resource = resource.clone();
                    let expected_action = action.clone();
                    let expires_at = actor.expires_at();
                    let decide = move |current, action: Arc<str>, resource: Arc<str>| {
                        let expected_resource = expected_resource.clone();
                        let expected_action = expected_action.clone();
                        async move {
                            Ok(PermissionDecision {
                            allowed: current == owner && action == expected_action && resource == expected_resource,
                            revision: uuid::Uuid::from_u128(1), expires_at,
                            target: PermissionTarget::Application(resource),
                            operation: PermissionOperation::Application(action),
                        }) }
                    };
                    let permission = authority.permission(&actor, action.clone(), resource.clone(), &decide).await?;
                    authority.revalidate_permission(&permission, &action, &resource, decide).await?;
                    Ok(())
                },
            }?;
        };
        guard
    };
    let read_guard = resource_guard("pickup.read", false);
    let collect_guard = resource_guard("pickup.collect", true);
    let show_sql = sql.clone();
    let collect_sql = sql.clone();
    let bytes = policy.bytes;
    let pickup_routes = identity_endpoints! {
        identity: definition.clone(), prefix: "/",
        route {
            method: http::Method::GET, path: "/pickup",
            policy: RoleClaimAuthorizationPolicy::require_permission("pickup.read"),
            guard |_accounts, _request| { let guard = read_guard.clone(); guard?; },
            input |request| { request },
            workflow |_accounts, _request| { let sql = show_sql.clone(); pickup::show::<Environment>(sql)? },
            render |response| { response },
        },
        route {
            method: http::Method::POST, path: "/identity/pickup/collect",
            policy: RoleClaimAuthorizationPolicy::require_permission("pickup.collect"),
            guard |_accounts, _request| { let guard = collect_guard.clone(); guard?; },
            input |request| { request },
            workflow |_accounts, _request| { let sql = collect_sql.clone(); pickup::collect::<Environment>(sql)? },
            render |response| { response },
        },
    }?;
    let antiforgery = definition.selected_antiforgery_stage();
    let pickup_routes = router! {
        source: pickup_routes,
        pipe_through(pipeline! {
            before: move |request| effect_map! { program: antiforgery(request), map |_verified| { () }, },
        }),
    }?;
    let routes = identity_endpoints! {
        identity: definition.clone(), prefix: "/",
        login {
            method: http::Method::POST, path: "/identity/login",
            guard: login_guard,
            input |_request| { pickup::credentials::<Environment>(bytes)? },
            workflow |accounts, submitted| {
                let outcome = accounts.login(LoginInput { user_name: submitted.username, password: submitted.password, persistent: false, lockout_on_failure: true })?;
                if matches!(outcome, LoginOutcome::Completed { .. }) { Some(accounts.sign_in_cookie(outcome)?) } else { None }
            },
            render |cookie| { customer_routes::login_reply(cookie)? },
        },
        antiforgery {
            method: http::Method::GET, path: "/identity/csrf",
            input |request| { request.headers().clone() },
            workflow |accounts, headers| { accounts.issue_antiforgery(headers)? },
            render |response| { reply! { native(response) } },
        },
        protect(RoleClaimAuthorizationPolicy::require_role("CUSTOMER"), pickup_routes);
        get("/", endpoint! { effect(pickup::welcome::<Environment>()) });
    }?;
    let account = move |name: &'static str| {
        let accounts = definition.clone();
        effect! {
            let accounts = accounts.clone();
            let existing = identity! {
                identity: accounts.clone(), invoke_once(name),
                native |native, name, _authority, _policies| {
                    let user = native.user_manager().find_by_name(name).await.map_err(IdentityError::Native)?;
                    Ok(user.map(|user| user.id()))
                },
            }?;
            let id = match existing {
                Some(id) => id,
                None => {
                    let name = effect_try_sync! { once; body { UserName::parse(name) }, on_error |_error| { cafe_error(CafeProblem::Identity(Box::new(IdentityError::Configuration("demo account name")))) }, }?;
                    accounts.register(RegistrationInput { user: IdentityUser::with_user_name(name), password: pickup::DEMO_PASSWORD.to_owned() })?.id
                }
            };
            identity! {
                identity: accounts, invoke_once(id),
                native |native, id, _authority, _policies| {
                    let users = native.user_manager();
                    let user = users.find_by_id(&id).await.map_err(IdentityError::Native)?.ok_or(IdentityError::Unauthorized)?;
                    if !users.is_in_role(&user, "Customer").await.map_err(IdentityError::Native)? {
                        let result = users.add_to_role(&user, "Customer").await;
                        if !result.succeeded() { return Err(IdentityError::Native(result)); }
                    }
                    let claims = users.get_claims(&user).await.map_err(IdentityError::Native)?;
                    for permission in ["pickup.read", "pickup.collect"] {
                        let claim = IdentityClaim::new(ClaimType::parse("permission").map_err(|_| IdentityError::Configuration("permission claim type"))?, ClaimValue::parse(permission).map_err(|_| IdentityError::Configuration("pickup permission"))?);
                        if !claims.contains(&claim) {
                            let result = users.add_claim(&user, claim).await;
                            if !result.succeeded() { return Err(IdentityError::Native(result)); }
                        }
                    }
                    Ok(id)
                },
            }?
        }
    };
    let fixture = customer_routes::seed(account, sql);
    Ok(CustomerServices {
        security: selected_security,
        routes,
        fixture,
    })
}
