//! Effect Identity persistence and explicitly selected native Identity services.
//!
//! With `sqlx-turso`, native managers use the local SQLx replacement stores. Each
//! mutation owns one acknowledged transaction; a manager workflow may compose
//! multiple commits. Apply the namespaced schema through the application's shared
//! SQLx migration catalog before serving requests. Native memory/Turso/sync stores
//! are not selected by this crate.
//!
//! Optional record capabilities have explicit codecs, finite policy bounds and a
//! storage configuration fence. Missing capabilities never create fallback stores.
//! Native ticket formats, protection providers and cryptographic algorithms stay
//! selected application services; persisted state alone does not establish proof.

extern crate self as effect_server_identity;

/// Typed callback blocks using this facade's existing Effect dependency.
pub use effect::{
    codec, effect_all, effect_all_concurrent, effect_blocking, effect_cpu, effect_die,
    effect_finalizer, effect_for_each, effect_for_each_concurrent, effect_from_option,
    effect_future, effect_map, effect_map_error, effect_on_interrupt, effect_permit,
    effect_provide_some, effect_provided, effect_race, effect_recover, effect_ref, effect_repeat,
    effect_resource, effect_retry, effect_sandbox, effect_schema_parse, effect_scope,
    effect_scope_acquire, effect_socket, effect_stream_for_each, effect_sync, effect_timeout,
    effect_try_blocking, effect_try_cpu, effect_try_future, effect_try_sync,
    effect_uninterruptible, effect_using, effect_with_layer, layer, pool, request_resolver,
    schedule, sink, weak_cache,
};

/// D18 inert definition and typed callback workflows. Definition fields: `storage`,
/// `configure_native` (typed
/// IdentityComponents), `errors` (application mapping), `clock` (IdentityClock),
/// `grant_policy` and `admission` (NonZeroUsize). Optional native replacements:
/// `passwords`, `authenticator`, `external`, `passkeys`, `email_confirmation`,
/// `browser`, `renderers`, `validators`, `normalizer`, `principal_factory`,
/// `tokens`, `tickets`, `key_ring`. The factory receives this retained policy bag
/// and the actual invocation store; it must use selected services without defaults.
/// Instead of `configure_native`, `native_factory |backend, clock, policies| { ... }`
/// or `native_factory: helper` infers a statically typed reusable native constructor
/// returning `Result<N, IdentityError>`. `errors |error| { ... }` is a pure mapping;
/// `errors: helper` retains its extracted typed function.
///
/// With `identity: definition`, select exactly one callback-bearing operation:
///
/// | Operation and ordered inputs | Native async Result roles |
/// | --- | --- |
/// | `invoke()` / `invoke_once(input)` / `native_http(request)` | `native` with 4 parameters, matching the existing method |
/// | `invoke_capability_once(input)` | `native` with 5 parameters (storage first) |
/// | `authorize_once(input)` | `authorize` with input, scoped authority, policies |
/// | `accounts_page(scope, after, limit)` / `roles_page(scope, after, limit)` | `decide(actor, action, resource)`, `allow(actor, record)` |
/// | `administer_account(grant, action, target, input)` / `administer_role(grant, operation_kind, input)` | `decide`, `operation(native, current, input)` |
/// | `create_role(grant, action, resource, name)` | `decide` |
/// | `rename_role(grant, name)`, `delete_role(grant)`, claim add/remove, membership add/remove | `decide`; other inputs retain the corresponding method's order |
/// | `mutate_account_snapshot(grant, input)` | `transform(native, snapshot, input)` |
/// | `deliver(grant, id, expires, request)` / `notify(...)` / `request_personal_data_job(...)` | `transport(accepted)` / `dispatch(accepted)` / `accept(accepted)` |
///
/// Inline roles use named braces: `decide |actor, action, resource| { ... }`.
/// Their body is native async Rust: `.await` and `?` preserve `IdentityError`.
/// Explicit leading handle copies run before the future; payloads move once.
/// `decide: helper` retains an extracted typed Future factory. Field and helper
/// expressions evaluate once in written order. Native execution remains deferred.
/// No role invents policy, retries, native services or another Effect interpreter.
/// The selected method maps expected errors and restores defects/interruption/cleanup.
///
/// Ordinary account/password/factor/passkey/external, email/phone, cookie/bearer/
/// stored-session, preference/snapshot/job-status workflows already return Effects;
/// bind them directly in `effect!` or an endpoint `workflow` block. Their original
/// selected native-manager/store/policy bounds still apply. Browser pending state,
/// antiforgery issuance and logout retain their native acknowledgement boundaries.
///
/// `authorize_once` exposes the exact existing authority: `revalidate`,
/// `revalidate_recent`, `protect_request`/`revalidate_protected`, permission issuance/
/// revalidation, scope issuance/per-record revalidation, and role issuance/revalidation.
/// Keep method/URI, actor, action/resource, policy revision/expiry and target/operation
/// checks at the protected operation. Query `decide` and `allow` roles run for every
/// emitted record; they do not replace these checks with authentication alone.
/// `authorization_stage(policy)` selects the native request authorization stage.
/// `authorization_guard(policy), guard: callback` checks that policy before the
/// supplied resource guard. Both forms require the selected `identity` definition.
/// The café consumers demonstrate actual native sign-in, scoped visibility and
/// request/resource guards without consumer generic parameter lists.
pub use effect_macros::identity;
/// D19 inert scoped endpoint assembly. Required `identity` and `prefix`; call-shaped
/// groups `registration`, `login`, `recovery`, `authenticator`, `passkeys`, `external`,
/// `management`, `logout`, `refresh`, `antiforgery` accept typed IdentityGroup factories.
/// WorkflowGroup composes selected input/native workflow/renderer Effects;
/// WorkflowRoutes retains multiple heterogeneous routes in one semantic group.
/// NativeHttpGroup retains public native protocol codecs and original response bodies
/// through the definition's invocation-scoped native_http adapter. Custom
/// `get(route, endpoint)`/`post(route, endpoint)` retain typed Server captures. Also
/// accepts `include`, `pipe_through`, and inline/extracted Effect `on_error` renderers.
/// `protect(native_policy, routes)` protects an entire Router collection, including
/// included and nested routes, using the existing native `AuthorizationPolicy`.
/// Existing child pipelines survive; outer group and inner endpoint policies must
/// both pass. Use `RoleClaimAuthorizationPolicy` for native role, claim, claim-type
/// and permission requirements, or select an application native policy implementation.
/// Separate protected collections can select different roles/permissions; public
/// routes remain outside those collections. No path allowlist is required.
/// `policy: native_policy` inside a route adds its own requirements before its
/// resource guard, input, workflow and renderer. A denied policy does not invoke
/// those callbacks. `require_route_permission(method, path)` matches an exact
/// method/path permission value; it does not match a route template.
/// Authentication stages supply only private proof from the selected verifier.
/// Authorization regenerates claims from current native sources after checking
/// grant owner, expiry and current account stamps; arbitrary principal/ticket
/// extensions cannot grant access. Native challenges return 401 and insufficient
/// authorization returns 403. Keep action/resource grant checks at the protected
/// operation as well. `identity! { identity: &definition, authorization_stage(policy) }`
/// also composes with Server `pipeline!` before stages for existing route modules.
/// Each group also accepts `login |identity| { ... }`-style native blocks returning
/// `Result<Router<E, R>, EndpointError>`; input inference uses the installed definition.
/// Extracted `login: helper` groups retain their existing typed IdentityGroup contract.
///
/// Semantic groups may instead contain one typed route:
/// `login { method: Method::POST, path: "/login", input |request| { ... },
/// workflow |identity, input| { ... }, render |output| { ... }, }`.
/// Repeatable `route { ... }` blocks add application routes under the same prefix.
/// Optional `guard |identity, request| { ... }` is an Effect body that must finish
/// successfully before input decoding, native work or rendering. Guard failures
/// preserve expected errors, defects, interruption and ordered cleanup. Use actual
/// private Identity grants and current authority checks for endpoint/resource policy.
/// Inline bodies consume each request/input/output once; `input: helper`,
/// `workflow: helper`, `render: helper`, `guard: helper` retain extracted typed recipes.
/// Method/path and extracted helper expressions evaluate once during construction;
/// request callbacks execute later inside Server's existing request/body scope.
pub use effect_macros::identity_endpoints;
/// D17 typed capability selection. Required `backend: NativeBackend`; optional
/// `accounts`, `sessions`, `keys`, `roles`, `permissions`, `challenges`, `queries`,
/// `audit`, `consent`, `delivery`, `notifications`, `jobs`, `personal_data`,
/// `native_user_queries`, `native_role_queries` are actual `StorageBound` services.
/// Fields evaluate once in declaration order; selection is inert and returns Result.
pub use effect_macros::identity_store;

#[cfg(feature = "sqlx-turso")]
pub mod store;
#[cfg(feature = "sqlx-turso")]
pub use store::{IdentityStoragePolicy, SqlxBrowserContextStore, SqlxIdentityStore};
pub mod error;
pub use error::{CapabilityName, IdentityError};
pub mod authority;
pub mod capabilities;
pub mod definition;
pub mod policies;
pub use definition::{ConfiguredComponents, IdentityComponents, IdentityDefinition};
pub mod components;
pub mod endpoints;
pub mod services;
pub mod workflows;

/// Public native interoperability retains the actual selected provider types.
pub mod native {
    pub use http;
    pub use identity_rs::*;
}

/// Hygienic macro paths for consumers with renamed dependencies.
#[doc(hidden)]
pub mod __private {
    pub use crate::definition::{definition_with_factory, error_mapper};
    pub use effect;
    pub use effect_server;
}
