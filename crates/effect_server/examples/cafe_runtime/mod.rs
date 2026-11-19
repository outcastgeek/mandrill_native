//! Request capability wiring for the finite order lookup; business endpoints stay in the example.
use super::{CafeError, CustomerId, cafe_telemetry};
use cafe_telemetry::TelemetryCapacity;
use effect::{Cancellation, ParsePolicy};
use effect_otel::BorrowedProviders;
use effect_server::{EffectEnvironment, Request};
use std::{num::NonZeroUsize, sync::Arc};
/// Caller-owned Tokio I/O and scheduling capabilities beside application request state.
#[derive(EffectEnvironment)]
#[effect_environment(
    request = request,
    tokio = self.telemetry.runtime.handle(),
    server_error(error = CafeError, map = CafeError::Http),
    cancellation = cancellation,
    clock(provider = effect_tokio::TokioClock, get = &self.telemetry.clock),
    task_spawner(provider = effect_tokio::TokioTasks, get = &self.telemetry.tasks),
    blocking_executor(provider = effect_tokio::TokioBlocking, get = &self.telemetry.blocking),
    tracer = effect::HasTracer::tracer(&self.telemetry),
    otel = effect_otel::HasOtel::otel(&self.telemetry),
)]
pub(crate) struct Cafe {
    /// Existing café provider composition, shared by root and request projections.
    pub(super) telemetry: cafe_telemetry::Environment,
    /// Caller-supplied JSON budget and explicit record parse policy.
    pub(super) json: ParsePolicy,
    /// Each selected request has its own cancellation source.
    pub(super) cancellation: Cancellation,
    /// Actual body/metadata owner for this execution.
    pub(super) request: Request,
    /// Trusted result of the explicit local demonstration policy.
    pub(super) customer: Option<CustomerId>,
}
/// Root capability state uses caller-owned providers and explicit telemetry/JSON storage bounds.
pub(crate) fn environment(
    runtime: Arc<tokio::runtime::Runtime>,
    providers: BorrowedProviders,
    json: ParsePolicy,
    capacity: TelemetryCapacity,
) -> Result<Arc<Cafe>, String> {
    let cancellation = Cancellation::new();
    Ok(Arc::new(Cafe {
        telemetry: cafe_telemetry::Environment::new(runtime, providers, capacity)?,
        json,
        request: Request::from_native(
            http::Request::new(axum::body::Body::empty()),
            cancellation.clone(),
        ),
        cancellation,
        customer: None,
    }))
}
/// The caller selects JSON bytes; structural bounds describe this record's three scalar fields.
pub(crate) fn order_policy(bytes: NonZeroUsize) -> ParsePolicy {
    ParsePolicy {
        limits: effect::ParseLimits {
            bytes,
            depth: NonZeroUsize::MIN.saturating_add(1),
            nodes: NonZeroUsize::MIN.saturating_add(3),
            collection: NonZeroUsize::MIN.saturating_add(2),
            issues: NonZeroUsize::MIN,
        },
        errors: effect::SchemaErrorMode::First,
        unknown_fields: effect::UnknownFields::Reject,
        duplicate_fields: effect::DuplicateFields::Reject,
    }
}
/// Drain the reused scheduling providers before the caller shuts down its native SDK originals.
pub(crate) async fn drain_providers(root: &Cafe) {
    root.telemetry.tasks.stop().wait().await;
    root.telemetry.blocking.stop().wait().await;
}
