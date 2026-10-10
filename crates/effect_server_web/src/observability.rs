//! Optional OTel composition over the delivered Server request/context bridge.
//!
//! This module owns no exporter, span engine or provider runtime. Context extraction
//! is installed by Server's outer selection envelope before authentication and
//! request projection; its single span wraps body delivery, scope finalization and
//! terminal observation using the existing one-way OTel-to-Server integration.

use effect::Effect;
use effect_otel::server::{RequestObservation, RequestSpan};
use effect_otel::{HasOtel, NativeError, Propagation};
use effect_server::{Request, Server};
use std::sync::Arc;

/// Reusable selected OTel request composition, borrowing host-owned SDK services
/// through R. Provider acquisition/shutdown remains in effect_otel's scoped owner.
#[must_use]
pub struct Observability<E: Send + 'static, R: HasOtel + Send + Sync + 'static> {
    /// Official selected propagator stack; no custom carrier parser exists in Web.
    propagation: Propagation,
    /// Explicit safe application span metadata, excluding raw request payloads.
    metadata: Arc<dyn Fn(&Request) -> RequestSpan + Send + Sync>,
    /// Explicit final-body signal routing; avoids implicit event/log duplication.
    observe: Arc<dyn Fn(RequestObservation) -> Effect<(), E, R> + Send + Sync>,
    /// Checked native propagation errors remain typed application failures.
    errors: Arc<dyn Fn(NativeError) -> E + Send + Sync>,
}
impl<E: Send + 'static, R: HasOtel + Send + Sync + 'static> Clone for Observability<E, R> {
    /// Share immutable native selection/callbacks without creating another provider.
    fn clone(&self) -> Self {
        Self {
            propagation: self.propagation.clone(),
            metadata: self.metadata.clone(),
            observe: self.observe.clone(),
            errors: self.errors.clone(),
        }
    }
}
impl<E: Send + 'static, R: HasOtel + Send + Sync + 'static> Observability<E, R> {
    /// Retain selected official propagation, safe metadata, signal routing and errors.
    pub fn new(
        propagation: Propagation,
        metadata: impl Fn(&Request) -> RequestSpan + Send + Sync + 'static,
        observe: impl Fn(RequestObservation) -> Effect<(), E, R> + Send + Sync + 'static,
        errors: impl Fn(NativeError) -> E + Send + Sync + 'static,
    ) -> Self {
        Self {
            propagation,
            metadata: Arc::new(metadata),
            observe: Arc::new(observe),
            errors: Arc::new(errors),
        }
    }
    /// Install the actual effect_otel::server `around_selection` bridge. It extracts
    /// the parent and opens the same span before authentication/RequestBinding;
    /// request ownership, selected environment and joined async cleanup remain
    /// entirely with Server/Effect through body delivery and final observation.
    #[must_use]
    pub fn install(&self, server: Server<E, R>) -> Server<E, R> {
        let metadata = self.metadata.clone();
        let observe = self.observe.clone();
        let errors = self.errors.clone();
        effect_otel::server::install(
            server,
            self.propagation.clone(),
            move |request| metadata(request),
            move |summary| observe(summary),
            move |error| errors(error),
        )
    }
}
