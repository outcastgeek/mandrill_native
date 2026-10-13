//! Local maintenance, scoped session admission, finite deadlines and core rate decisions.
//!
//! A local write hold is not distributed authority. Rejection bodies are selected
//! by application Effects; capacity/backend/uncertain failures are distinct from
//! policy denial. Session reservations are registered with the request/body scope.

use crate::WebError;
use crate::{
    effect_finalizer, effect_future, effect_map_error, effect_resource, effect_sync,
    effect_try_sync, effect_using,
};
use effect::persistence::{
    HasRateLimitStore, RateLimitDecision, RateLimitError, RateLimitStore, RateLimiter, UtcMillis,
};
use effect::{
    AdmissionCapacity, AdmissionError, AdmissionPermit, Effect, Exit, HasClock, HasTaskSpawner,
    ProviderAdmission, Scope,
};
use effect::{KeyDomain, StableKey};
use effect::{effect, effect_timeout};
use http::{HeaderMap, HeaderValue};
use std::num::{NonZeroU64, NonZeroUsize};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Operation classification supplied at route/group declaration, not inferred from
/// an arbitrary method when the application knows it creates a session or write.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[must_use]
pub enum AdmissionClass {
    /// Deliberately read-only business work.
    Read,
    /// Business state mutation.
    Mutation,
    /// New long-lived session creation.
    NewSession,
    /// Explicit operational exclusion selected by the application.
    Operational,
}
/// Process-local maintenance selection; it does not stop another replica or background writer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[must_use]
pub enum MaintenanceMode {
    /// Normal local admission.
    Open,
    /// Reads/explicit operations proceed; writes/new sessions are held.
    ReadOnly,
    /// New non-operational traffic is rejected while retained work drains.
    Draining,
}
/// Explicit whole-second retry guidance; HTTP Retry-After uses seconds rather than
/// an invented default copied from the reference implementation.
#[derive(Clone, Copy)]
#[must_use]
pub struct RetryAfter(NonZeroU64);
impl RetryAfter {
    /// Select positive retry guidance in whole seconds.
    pub const fn seconds(seconds: NonZeroU64) -> Self {
        Self(seconds)
    }
    /// Render the RFC delta-seconds field from the caller's explicit value.
    pub fn header(self) -> Result<HeaderValue, WebError> {
        HeaderValue::from_str(&self.0.to_string())
            .map_err(|_| WebError::Configuration("Retry-After seconds".into()))
    }
}
/// A typed local rejection decision, supplied to an application renderer.
#[derive(Clone, Copy)]
#[must_use]
pub struct MaintenanceRejection {
    /// Current local mode causing rejection.
    pub mode: MaintenanceMode,
    /// Explicit classification of the rejected operation.
    pub class: AdmissionClass,
    /// Caller-selected whole-second retry guidance.
    pub retry_after: RetryAfter,
}
impl MaintenanceRejection {
    /// Apply native no-store/retry metadata after the application builds its body.
    pub fn headers(self, headers: &mut HeaderMap) -> Result<(), WebError> {
        headers.insert(
            http::header::CACHE_CONTROL,
            HeaderValue::from_static("no-store"),
        );
        headers.insert(http::header::RETRY_AFTER, self.retry_after.header()?);
        if self.mode == MaintenanceMode::ReadOnly {
            headers.insert("x-write-hold", HeaderValue::from_static("active"));
        }
        Ok(())
    }
}
/// Shared local selection. State reads are short synchronous metadata operations;
/// no lock is held while awaiting an Effect or body delivery.
#[derive(Clone)]
#[must_use]
pub struct Maintenance {
    /// Process-local selected mode.
    state: Arc<Mutex<MaintenanceMode>>,
    /// Deliberate policy value used for denial guidance.
    retry_after: RetryAfter,
}
impl Maintenance {
    /// Construct a local control with explicit initial state and retry policy.
    pub fn new(initial: MaintenanceMode, retry_after: RetryAfter) -> Self {
        Self {
            state: Arc::new(Mutex::new(initial)),
            retry_after,
        }
    }
    /// Read current local selection without acquiring application/global authority.
    pub fn mode(&self) -> MaintenanceMode {
        *self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
    /// Lazily publish local mode; the application retains authority to call this handle.
    pub fn set<E: Send + 'static, R: Send + Sync + 'static>(
        &self,
        mode: MaintenanceMode,
    ) -> Effect<(), E, R> {
        let control = self.clone();
        effect_sync! {
            body: move || {
            *control
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = mode
        }, }
    }
    /// Classify local policy without synthesizing a user-facing response.
    pub fn decision(&self, class: AdmissionClass) -> Result<(), MaintenanceRejection> {
        let mode = self.mode();
        if class == AdmissionClass::Operational
            || mode == MaintenanceMode::Open
            || (mode == MaintenanceMode::ReadOnly && class == AdmissionClass::Read)
        {
            Ok(())
        } else {
            Err(MaintenanceRejection {
                mode,
                class,
                retry_after: self.retry_after,
            })
        }
    }
    /// Select admitted work or application-supplied Effect rejection under the same
    /// interpreter. A rejection Effect's failure remains the original typed outcome.
    pub fn admit<A: Send + 'static, E: Send + 'static, R: Send + Sync + 'static>(
        &self,
        class: AdmissionClass,
        admitted: impl Fn() -> Effect<A, E, R> + Send + Sync + 'static,
        reject: impl Fn(MaintenanceRejection) -> Effect<A, E, R> + Send + Sync + 'static,
    ) -> Effect<A, E, R> {
        let control = self.clone();
        effect! {
        bind;
        program: effect_sync! { body { control.decision(class) }, },
        body: move |decision| match decision {
                       Ok(()) => admitted(),
                       Err(decision) => reject(decision),
                   },
        }
    }
    /// Local traffic readiness reports drain state; dependency probes remain separate.
    pub fn traffic_ready<E: Send + 'static, R: Send + Sync + 'static>(&self) -> Effect<bool, E, R> {
        let control = self.clone();
        effect_sync! { body: move || control.mode() != MaintenanceMode::Draining, }
    }
}
/// Session admission delegates immediate reservation/draining to the core provider.
#[derive(Clone)]
#[must_use]
pub struct SessionAdmission {
    /// Existing provider admission owner; no second semaphore.
    owner: ProviderAdmission,
}
impl SessionAdmission {
    /// Select a positive local maximum of live sessions, with no arbitrary default.
    pub fn new(maximum: NonZeroUsize) -> Self {
        Self {
            owner: ProviderAdmission::new(AdmissionCapacity::Bounded(maximum)),
        }
    }
    /// Count sessions whose request/body cleanup has not yet released its reservation.
    #[must_use]
    pub fn active(&self) -> usize {
        self.owner.active()
    }
    /// Register one permit in the actual request/body scope before endpoint work.
    /// A closed scope releases immediately through protected core finalization.
    pub fn acquire<E: Send + 'static, R: Send + Sync + 'static>(
        &self,
        scope: Scope<E>,
        on_error: impl Fn(AdmissionError) -> E + Send + Sync + 'static,
    ) -> Effect<(), E, R> {
        let owner = self.owner.clone();
        effect! {
        bind;
        program: effect_try_sync! { body: move || owner.admit(), on_error: on_error, },
        body: move |permit| {
                   let permit = Arc::new(Mutex::new(Some(permit)));
                   effect_finalizer! {
                       release;
                       scope: scope,
                       release: move || {
                       let permit = permit
                           .lock()
                           .unwrap_or_else(std::sync::PoisonError::into_inner)
                           .take();
                           effect_resource! { release_sync; body: move || drop(permit) }
                       },
                   }
               },
        }
    }
    /// Bind an immediate admission result into the existing endpoint scope. Denial
    /// is a typed value so a caller can execute its finite rejection renderer before
    /// response commitment; accepted permits remain owned until request cleanup.
    pub(crate) fn reservation<E: Send + 'static, R: Send + Sync + 'static>(
        &self,
    ) -> effect::ResourceBinding<Result<AdmissionPermit, AdmissionError>, E, R> {
        let owner = self.owner.clone();
        effect_using! {
            binding;
            acquire: effect_sync! { body: move || owner.admit() },
        }
    }
    /// Close new local sessions and join every already owned reservation.
    pub fn shutdown<E: Send + 'static, R: Send + Sync + 'static>(&self) -> Effect<(), E, R> {
        let owner = self.owner.clone();
        effect_future! {
            exit_with_context; body: move |_, _| {
            let drain = owner.stop();
            async move {
                drain.wait().await;
                Exit::Success(())
            }
        }, }
    }
}
/// Apply a deadline only to deliberately finite work. Core timeout cancels and
/// joins the loser including asynchronous cleanup; no detached native future.
pub fn finite<A, E, R>(
    program: Effect<A, E, R>,
    deadline: Duration,
    on_timeout: impl Fn() -> E + Send + Sync + 'static,
) -> Result<Effect<A, E, R>, WebError>
where
    A: Send + 'static,
    E: Send + 'static,
    R: HasClock + HasTaskSpawner + Send + Sync + 'static,
{
    if deadline.is_zero() {
        return Err(WebError::Configuration("finite deadline".into()));
    }
    Ok(effect_timeout! {
        program: program,
        duration: deadline,
        on_timeout: on_timeout,
    })
}
/// A rate-limit decision through the selected core store and explicit UTC instant.
/// Denied/ImpossibleCost/AdaptationRequired invoke the selected rejection Effect. Store
/// capacity/backend/uncertainty remain typed E via `on_error`.
pub fn enforce_rate<A, E, R, D>(
    limiter: &RateLimiter<D>,
    partition: StableKey<D>,
    now: UtcMillis,
    cost: NonZeroU64,
    on_error: impl Fn(RateLimitError<<R::Store as RateLimitStore>::Error>) -> E + Send + Sync + 'static,
    allowed: impl Fn() -> Effect<A, E, R> + Send + Sync + 'static,
    reject: impl Fn(RateLimitDecision) -> Effect<A, E, R> + Send + Sync + 'static,
) -> Effect<A, E, R>
where
    A: Send + 'static,
    E: Send + 'static,
    R: HasRateLimitStore + Send + Sync + 'static,
    D: KeyDomain,
{
    effect! {
    bind;
    program: effect_map_error! {
        program: limiter
           .acquire(partition, now, cost), map: on_error, },
    body: move |decision| match decision {
               RateLimitDecision::Allowed { .. } => allowed(),
               RateLimitDecision::Denied { .. }
               | RateLimitDecision::ImpossibleCost { .. }
               | RateLimitDecision::AdaptationRequired { .. } => reject(decision),
           },
    }
}
/// Execute one independent finite producer with an existing core reservation held
/// until its Effect and nested cleanup finish. Used by disabled origin caches.
pub(crate) fn independent<A: Send + 'static, E: Send + 'static, R: Send + Sync + 'static>(
    owner: ProviderAdmission,
    work: impl Fn() -> Effect<A, E, R> + Send + Sync + 'static,
    capacity: impl Fn(AdmissionError) -> E + Send + Sync + 'static,
) -> Effect<A, E, R> {
    effect_using! {
        acquire: effect_try_sync! { body { owner.admit() }, on_error: capacity, },
        body: move |_permit| work(),
    }
}
