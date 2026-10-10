//! Typed Web policies over Effect, the existing Server owner and core capabilities.
//!
//! Descriptions are inert. Runtime work selects services through `R`; native HTTP
//! hosting requires the Server Tokio adapter while other Effect providers remain
//! usable in the same environment. Cache values and request bodies retain their
//! original owners instead of gaining blanket `Clone` bounds.

extern crate self as effect_server_web;

#[cfg(test)]
mod contract_tests;

pub mod admission;
pub mod application;
pub mod availability;
pub mod cache;
pub mod csp;
pub mod dashboard;
#[cfg(feature = "sqlx-turso")]
pub mod database;
pub mod diagnostics;
#[cfg(feature = "identity")]
pub mod identity;
pub mod localization;
pub mod management;
#[cfg(feature = "otel")]
pub mod observability;
pub mod output_cache;
pub mod rendering;
pub mod security;

/// D25 checked service/error/deadline/parallelism policy with repeated `readiness`,
/// `health`, `liveness`, `process_liveness` and corresponding optional probe calls.
/// Nonempty readiness is required. Optional `traffic_ready` shares the collection
/// deadline, and `dashboard` accepts only prepared operator-authorized routes.
pub use effect_macros::availability;
/// D22 checked `directive(name, sources);`, `append(name, sources);`, and
/// `replace(name, sources);` operations. Source lists are typed `CspSources`;
/// exclusive none and directive/value incompatibilities return typed errors.
/// Obvious duplicate/exclusive literal descriptions fail at their source span.
pub use effect_macros::csp;
/// D24 the D23 ownership/policy forms plus `dependencies` (identity equality) and
/// `maximum_dependencies`. The loader returns `(value, dependency identities)`;
/// with optional inline/extracted `select_dependencies`, it returns the original
/// value and the pure selector borrows key/value to choose membership identities;
/// include empty collection membership dependencies. Active invalidation delegates
/// fencing/cancellation/joining to the same existing core DependencyCache owner.
pub use effect_macros::dependent_read_through;
/// D27 checked `catalogue`, `cultures`, inline/extracted `select_by`, and `missing`
/// fields with optional positive `reload` duration. Host scope owns acquisition/
/// reload, and native selection pins one immutable generation before resolving R.
pub use effect_macros::localization;
/// D26 `message: Message<M>` and repeated literal `field(name, typed_value);`.
/// NamedArgument's typed ownership chain must equal M::Arguments; Message::bind
/// checks the contract's names/order. Raw unvalidated interpolation is unavailable.
pub use effect_macros::message_arguments;
/// D23 existing core scoped-cache acquisition. Required fields: `scope`, `retain`
/// (checked Retain), `admission`, `keys` (equality), `cache_failures`, and `loader`.
/// Optional `project` maps the acquired handle. Inline loader `|key| { ... }`
/// receives the existing Arc key; each returned Effect consumes that invocation's
/// body once. Inline/extracted callbacks preserve non-Clone loaded values.
pub use effect_macros::read_through;
/// D21 checked native policies: browser/CSP/nonce/forwarding/CORS/CSRF/authentication/
/// authorization/compression/static/body-limit/method-override/HSTS. Repeated
/// `header(name, value);` keeps native repeated fields. Dynamic validation returns
/// Result; inline nonce and request guards use the ordinary Effect body grammar
/// through a fresh one-shot binder per invocation, preserving owned non-Clone results.
pub use effect_macros::security;
/// D20 inert complete application. Required fields: `policy`, `web_errors`,
/// `startup`, `environment`, `listen`, `protocols`, `routes`. Optional selections:
/// `security`, `localization`, `observe`, `otel`, `availability`, `output_cache`,
/// `configure_builder`, `configure_app`. Inline `startup |scope| { ... }` and
/// `environment |capabilities, request| { ... }` bind the existing Effect grammar;
/// extracted callback fields retain the same C/E/R contracts. Inline borrowed
/// observers project owned metadata in their pure prefix before deferred binds.
/// Each inline invocation returns a one-shot Effect that moves owned values without
/// Clone bounds; the enclosing callback remains reusable with fresh invocation inputs.
pub use effect_macros::web;

/// Expansion support for renamed downstream dependencies. This module is not an
/// application service locator; macros use the same Effect interpreter.
#[doc(hidden)]
pub mod __private {
    /// The existing interpreter and lowering support, re-exported hygienically.
    pub use effect;
}

/// Configuration/policy failures before native response commitment. Application
/// code maps these explicitly into its own error type; payloads are never logged.
#[derive(Debug, Clone, PartialEq, Eq)]
#[must_use]
pub enum WebError {
    /// A named policy has inconsistent or malformed inputs.
    Configuration(String),
    /// Native metadata could not be parsed safely under the selected policy.
    Metadata(String),
    /// Selected localization acquisition/selection failed before response commitment.
    Localization(localization::LocalizationError),
    /// Selected authentication/authorization/CSRF stage rejected this request.
    Rejected(&'static str),
    /// Selected availability configuration failed before accepting traffic.
    Availability(availability::AvailabilityError),
    /// A finite response exceeded its caller-selected encoded byte bound.
    ResponseLimit {
        /// Maximum encoded bytes supplied by the caller.
        maximum: usize,
        /// Actual encoded bytes observed.
        observed: usize,
    },
}
impl std::fmt::Display for WebError {
    /// Render only a safe category and policy name, never a credential/body.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Configuration(name) => write!(f, "invalid web policy: {name}"),
            Self::Metadata(name) => write!(f, "invalid web metadata: {name}"),
            Self::Availability(error) => write!(f, "availability: {error}"),
            Self::Localization(error) => write!(f, "localization: {error}"),
            Self::Rejected(stage) => write!(f, "web policy rejected: {stage}"),
            Self::ResponseLimit { maximum, observed } => {
                write!(f, "finite response {observed} bytes exceeds {maximum}")
            }
        }
    }
}
impl std::error::Error for WebError {}

/// Re-enter the existing interpreter with one materialized outcome. Single-use
/// ownership keeps non-Clone values/errors intact when serialized publication must
/// record status before returning the original failure/defect/interruption.
pub(crate) fn restore_exit<A: Send + 'static, E: Send + 'static, R: Send + Sync + 'static>(
    exit: effect::Exit<A, E>,
) -> effect::Effect<A, E, R> {
    let exit = std::sync::Mutex::new(Some(exit));
    effect::Effect::from_exit_future_with_context(move |_, _| {
        let exit = exit
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        async move {
            match exit {
                Some(exit) => exit,
                None => effect::Exit::Die(effect::Defect::new(
                    "materialized web outcome already consumed",
                )),
            }
        }
    })
}
