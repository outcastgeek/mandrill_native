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

/// D18 inert definition. Required fields: `storage`, `configure_native` (typed
/// IdentityComponents), `errors` (application mapping), `clock` (IdentityClock),
/// `grant_policy` and `admission` (NonZeroUsize). Optional native replacements:
/// `passwords`, `authenticator`, `external`, `passkeys`, `email_confirmation`,
/// `browser`, `renderers`, `validators`, `normalizer`, `principal_factory`,
/// `tokens`, `tickets`, `key_ring`. The factory receives this retained policy bag
/// and the actual invocation store; it must use selected services without defaults.
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
    pub use effect;
    pub use effect_server;
}
