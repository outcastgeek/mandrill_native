//! Explicit distributed contexts and native propagation without global state.

mod prepared;

pub use prepared::{PreparedPropagationCapacity, PreparedPropagationCarriers};

use crate::NativeError;
use effect::http::{HttpRequest, HttpResponse, HttpTransport, TransportFuture};
use effect::{Cancellation, Defect};
use opentelemetry::baggage::BaggageExt;
use opentelemetry::propagation::{Extractor, Injector, TextMapPropagator};
use opentelemetry::trace::TraceContextExt;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;

/// Detached distributed identity, flags, tracestate and baggage.
///
/// This retains no live local span owner. Re-entry is explicit and does not
/// authorize application services; those continue to be selected through `R`.
#[derive(Clone, Default)]
pub struct OtelContext {
    /// Native immutable context with its live span replaced by a nonrecording one.
    native: opentelemetry::Context,
}

impl OtelContext {
    /// Capture only propagation identity from a native context.
    /// Baggage remains immutable; no ambient context is changed.
    #[must_use]
    pub fn from_native(context: &opentelemetry::Context) -> Self {
        let baggage = context.baggage().iter().map(|(key, (value, metadata))| {
            opentelemetry::baggage::KeyValueMetadata::new(
                key.clone(),
                value.clone(),
                metadata.clone(),
            )
        });
        Self {
            native: opentelemetry::Context::new()
                .with_remote_span_context(context.span().span_context().clone())
                .with_baggage(baggage),
        }
    }

    /// Borrow the exact native context for a custom carrier or host adapter.
    #[must_use]
    pub fn native(&self) -> &opentelemetry::Context {
        &self.native
    }

    /// Borrow distributed IDs; these are independent of `effect::SpanId`.
    #[must_use]
    pub fn span_context(&self) -> opentelemetry::trace::SpanContext {
        self.native.span().span_context().clone()
    }

    /// Scope an immutable baggage update, including optional native metadata.
    /// The caller owns selection/redaction of entries before propagation.
    #[must_use]
    pub fn with_baggage(
        &self,
        entries: impl IntoIterator<Item = opentelemetry::baggage::KeyValueMetadata>,
    ) -> Self {
        let existing = self
            .native
            .baggage()
            .iter()
            .map(|(key, (value, metadata))| {
                opentelemetry::baggage::KeyValueMetadata::new(
                    key.clone(),
                    value.clone(),
                    metadata.clone(),
                )
            });
        Self {
            native: self.native.with_baggage(existing.chain(entries)),
        }
    }
}

impl std::fmt::Debug for OtelContext {
    /// Report identity availability and baggage count without exporting values.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OtelContext")
            .field("valid", &self.span_context().is_valid())
            .field("baggage_entries", &self.native.baggage().len())
            .finish()
    }
}

/// Caller-selected official propagator; no process-global propagator is read.
#[derive(Clone)]
pub struct Propagation {
    /// SDK implementation supplies wire syntax and custom carrier semantics.
    native: Arc<dyn TextMapPropagator + Send + Sync>,
    carriers: Option<Arc<PreparedPropagationCarriers>>,
}

impl Propagation {
    /// Retain a W3C/composite or other native propagator selected by the caller.
    #[must_use]
    pub fn new(propagator: impl TextMapPropagator + Send + Sync + 'static) -> Self {
        Self {
            native: Arc::new(propagator),
            carriers: None,
        }
    }

    /// Select regional outbound staging and incoming Server HTTP carrier snapshots.
    #[must_use]
    pub fn with_prepared_carriers(mut self, carriers: Arc<PreparedPropagationCarriers>) -> Self {
        self.carriers = Some(carriers);
        self
    }

    /// Inject the explicit snapshot into a borrowed custom carrier.
    /// A propagator panic is returned as a checked native error.
    pub fn inject(
        &self,
        context: &OtelContext,
        carrier: &mut dyn Injector,
    ) -> Result<(), NativeError> {
        self.inject_with(carrier, |native, carrier| {
            native.inject_context(context.native(), carrier);
        })
    }

    fn inject_with(
        &self,
        carrier: &mut dyn Injector,
        inject: impl FnOnce(&dyn TextMapPropagator, &mut dyn Injector),
    ) -> Result<(), NativeError> {
        catch_unwind(AssertUnwindSafe(|| {
            if let Some(carriers) = &self.carriers {
                let mut staging = carriers.reserve()?;
                inject(self.native.as_ref(), &mut staging);
                staging.emit(carrier)?;
            } else {
                inject(self.native.as_ref(), carrier);
            }
            Ok(())
        }))
        .map_err(|panic| NativeError::Native(Defect::from_panic(panic)))?
    }

    /// Extract a carrier against an explicit base (including its baggage).
    /// Malformed/absent values follow the selected propagator's native behavior.
    /// Custom extractors own their carrier storage.
    pub fn extract(
        &self,
        base: &OtelContext,
        carrier: &dyn Extractor,
    ) -> Result<OtelContext, NativeError> {
        catch_unwind(AssertUnwindSafe(|| {
            OtelContext::from_native(&self.native.extract_with_context(base.native(), carrier))
        }))
        .map_err(|panic| NativeError::Native(Defect::from_panic(panic)))
    }

    #[cfg(feature = "server")]
    pub(crate) fn extract_headers(
        &self,
        base: &OtelContext,
        headers: &http::HeaderMap,
    ) -> Result<OtelContext, NativeError> {
        if let Some(carriers) = &self.carriers {
            let mut snapshot = carriers.reserve()?;
            snapshot.capture_headers(headers)?;
            self.extract(base, &snapshot)
        } else {
            self.extract(base, &crate::server::HeaderExtractor::new(headers))
        }
    }
}

/// Core HTTP carrier; writes only propagated metadata, never request payloads.
struct CoreHeaders<'a> {
    /// Fresh mutable headers owned by this outbound request invocation.
    headers: &'a mut Arc<effect::http::HttpHeaders>,
}

impl Injector for CoreHeaders<'_> {
    /// Replace the selected protocol field using the native propagator's value.
    fn set(&mut self, key: &str, value: String) {
        Arc::make_mut(self.headers).insert(Arc::from(key), Arc::from(value));
    }
}

/// One-way transport decorator for outbound core HTTP propagation.
///
/// Install under a poll-instrumented Effect boundary. Every send takes the native
/// context attached for the current poll; it never captures a startup thread's
/// ambient context. The original body and response owner move unchanged.
pub struct PropagatingTransport<T: HttpTransport + ?Sized> {
    /// Host-owned native transport, including its reactor and admission policy.
    transport: Arc<T>,
    /// Explicit native propagation policy for this transport.
    propagation: Propagation,
}

impl<T: HttpTransport + ?Sized> PropagatingTransport<T> {
    /// Retain the transport without constructing a client or runtime.
    #[must_use]
    pub fn new(transport: Arc<T>, propagation: Propagation) -> Self {
        Self {
            transport,
            propagation,
        }
    }
}

impl<T: HttpTransport + ?Sized> HttpTransport for PropagatingTransport<T> {
    /// Inject at native send construction, then transfer the original future.
    /// A carrier failure is a transport defect at the core HTTP error boundary.
    fn send(
        &self,
        mut request: HttpRequest,
        cancellation: Cancellation,
    ) -> TransportFuture<HttpResponse> {
        let mut carrier = CoreHeaders {
            headers: &mut request.options.headers,
        };
        if let Err(error) = self
            .propagation
            .inject_with(&mut carrier, |native, carrier| {
                // Drop the native thread-local borrow before invoking a user propagator.
                let context = OtelContext::from_native(&opentelemetry::Context::current());
                native.inject_context(context.native(), carrier);
            })
        {
            return Box::pin(async move { Err(error.into_defect()) });
        }
        self.transport.send(request, cancellation)
    }
}
