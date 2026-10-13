//! Request capability wiring for the finite order lookup; business endpoints stay in the example.
use super::{CafeError, CustomerId, cafe_telemetry};
use cafe_telemetry::TelemetryCapacity;
use effect::{
    Cancellation, HasBlockingExecutor, HasCancellation, HasClock, HasTaskSpawner, HasTracer,
    ParsePolicy,
};
use effect_otel::{BorrowedProviders, HasOtel, OtelBackend};
use effect_server::{HasRequest, HasTokio, Request, ServerError};
use std::{num::NonZeroUsize, sync::Arc};
/// Caller-owned Tokio I/O and scheduling capabilities beside application request state.
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
impl HasTaskSpawner for Cafe {
    /// Reuse the existing café task provider.
    type Tasks = effect_tokio::TokioTasks;
    /// Borrow the selected independent-child scheduler.
    fn task_spawner(&self) -> &Self::Tasks {
        &self.telemetry.tasks
    }
}
impl HasBlockingExecutor for Cafe {
    /// JSON native writes run on the existing independent blocking provider.
    type Executor = effect_tokio::TokioBlocking;
    /// Borrow the same provider retained through host drain.
    fn blocking_executor(&self) -> &Self::Executor {
        &self.telemetry.blocking
    }
}
impl HasClock for Cafe {
    /// Request projections preserve the existing monotonic origin.
    type Clock = effect_tokio::TokioClock;
    /// Borrow the selected native clock.
    fn clock(&self) -> &Self::Clock {
        &self.telemetry.clock
    }
}
impl HasTracer for Cafe {
    /// Both request and domain spans share the existing OTel-backed tracer.
    fn tracer(&self) -> &effect::Tracer {
        &self.telemetry.tracer
    }
}
impl HasOtel for Cafe {
    /// Borrow the selected native bridge without global discovery.
    fn otel(&self) -> &OtelBackend {
        &self.telemetry.otel
    }
}
impl effect_server::HasServerError for Cafe {
    /// JSON/input failures retain the café HTTP error channel.
    type Error = CafeError;
    /// Map the checked native cause once.
    fn server_error(&self, error: ServerError) -> CafeError {
        CafeError::Http(error)
    }
}
impl HasTokio for Cafe {
    /// Borrow the runtime kept alive by main through host drain.
    fn tokio(&self) -> &tokio::runtime::Handle {
        self.telemetry.runtime.handle()
    }
}
impl HasCancellation for Cafe {
    /// Select the cancellation authority belonging to this execution.
    fn cancellation(&self) -> &Cancellation {
        &self.cancellation
    }
}
impl HasRequest for Cafe {
    /// Preserve the real native request and its checked one-body owner.
    fn request(&self) -> &Request {
        &self.request
    }
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
