//! Optional OpenTelemetry signals over Effect's existing tracing lifetime.
//!
//! Put an [`OtelBackend`] in `R` through [`HasOtel`], expose its core tracer
//! through `effect::HasTracer`, and install [`Otel::instrument`] at the host
//! boundary. Both `span!` and [`otel!`] then use `Effect::with_span`; local Effect
//! IDs are never used as distributed IDs. No global SDK, subscriber, collector,
//! sampler, runtime, payload serializer or exporter is installed here.
//!
//! The caller builds native SDK providers with its resource, sampler, readers,
//! processors and exporters. [`BorrowedProviders`] does not shut those providers
//! down. [`OwnedProviders`] has an explicit, joined, typed shutdown and can be
//! acquired in an Effect scope. SDK background tasks retain the runtime contract
//! of the caller's chosen processors/exporters.

extern crate self as effect_otel;

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

mod context;
mod metrics;
mod providers;
#[cfg(feature = "server")]
pub mod server;
mod signals;

pub use context::{OtelContext, PropagatingTransport, Propagation};
/// Describe a lazy span on the same lifetime path as `effect::span!`.
///
/// Required `name: Into<Arc<str>>` and exactly one `body { ... }` or
/// `effect: Effect<A,E,R>` select the program. Optional `attributes: Vec<KeyValue>`,
/// `links: Vec<Link>`, `kind: SpanKind` and `parent: OtelContext` are evaluated once
/// in declaration order and emitted at execution. `parent_option: Option<OtelContext>`
/// retains an optional parent and conflicts with `parent`. `?` inside an inline
/// body binds Effects using the existing DSL; Results need explicit conversion. Use lazy
/// `Otel::tag`, `tag_bool`, `tag_int`, `tag_number`, `event`, `event_with`, `count`,
/// `log`, `status` and `link` operations with `?` at their intended positions.
/// `R: HasOtel` supplies the same core tracer used by ordinary `span!`; business
/// values/errors move without blanket Clone bounds. Invalid/duplicate fields or
/// conflicting body forms produce source diagnostics. Construction exports nothing.
pub use effect_macros::otel;
pub use metrics::{
    LatencyUnit, MetricDefinition, MetricPolicy, NativeInstrument, NativeInstrumentKind,
    OtelMetricError, OtelMetricRecorder,
};
/// Native context, kinds, links and status keep the SDK's exact representations.
pub use opentelemetry::trace::{Link, SpanContext, SpanKind, Status};
/// Native types remain available for explicit, checked interoperation.
pub use opentelemetry::{KeyValue, Value};
pub use providers::{
    BorrowedProviders, LifecycleCause, LifecycleError, LifecycleFailure, LifecyclePhase,
    OwnedProviders, ProviderFactories, ShutdownPolicy, Signal,
};
pub use signals::{HasOtel, LogRouting, NativeError, Otel, OtelBackend, Redaction, SpanOptions};

/// Hygienic macro lowering support; not an alternate application interpreter.
#[doc(hidden)]
pub mod __private {
    /// Existing Effect implementation used by generated domain bodies.
    pub use effect;
}
