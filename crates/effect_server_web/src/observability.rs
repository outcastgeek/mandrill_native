//! Optional OTel composition over the delivered Server request/context bridge.
//!
//! This module owns no exporter, span engine or provider runtime. Context extraction
//! is installed by Server's outer selection envelope before authentication and
//! request projection; its single span wraps body delivery, scope finalization and
//! terminal observation using the existing one-way OTel-to-Server integration.

use effect::{Effect, PreparedAdmissionError, PreparedProviderAdmission};
use effect_otel::server::{RequestHooks, RequestObservation, RequestSpan};
use effect_otel::{HasOtel, NativeError, Propagation};
use effect_server::{Request, Server};
use std::sync::Arc;

type ObservationAdmission<E> = (
    Arc<PreparedProviderAdmission>,
    Arc<dyn Fn(PreparedAdmissionError) -> E + Send + Sync>,
);

/// Reusable selected OTel request composition, borrowing host-owned SDK services
/// through R. Provider acquisition/shutdown remains in effect_otel's scoped owner.
#[must_use]
pub struct Observability<E: Send + 'static, R: HasOtel + Send + Sync + 'static> {
    /// Official selected propagator stack; no custom carrier parser exists in Web.
    propagation: Propagation,
    /// Selected safe span metadata, terminal observation and typed error mapping.
    hooks: RequestHooks<E, R>,
    admission: Option<ObservationAdmission<E>>,
}
impl<E: Send + 'static, R: HasOtel + Send + Sync + 'static> Clone for Observability<E, R> {
    /// Share immutable native selection/callbacks without creating another provider.
    fn clone(&self) -> Self {
        Self {
            propagation: self.propagation.clone(),
            hooks: self.hooks.clone(),
            admission: self.admission.clone(),
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
            hooks: RequestHooks::new(metadata, observe, errors),
            admission: None,
        }
    }

    /// Select prepared request-span controls before native extraction.
    pub fn with_prepared_admission(
        mut self,
        control: Arc<PreparedProviderAdmission>,
        on_capacity: impl Fn(PreparedAdmissionError) -> E + Send + Sync + 'static,
    ) -> Self {
        self.admission = Some((control, Arc::new(on_capacity)));
        self
    }
    /// Install the actual effect_otel::server `around_selection` bridge. It extracts
    /// the parent and opens the same span before authentication/RequestBinding;
    /// request ownership, selected environment and joined async cleanup remain
    /// entirely with Server/Effect through body delivery and final observation.
    #[must_use]
    pub fn install(&self, server: Server<E, R>) -> Server<E, R> {
        let server = if let Some((control, on_capacity)) = self.admission.clone() {
            effect_server::server! {
                source: server,
                around_selection(move |_, complete| {
                    let on_capacity = on_capacity.clone();
                    crate::admission::prepared_independent(
                        control.clone(), move || complete.clone(), move |error| on_capacity(error),
                    )
                }),
            }
        } else {
            server
        };
        effect_otel::otel! {
            install_request_hooks;
            hooks: self.hooks,
            server: server,
            propagation: self.propagation.clone(),
        }
    }
}
