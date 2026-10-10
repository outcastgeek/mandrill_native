//! Bounded native instrument identities and exact portable metric adaptation.

use effect::{
    GaugeValue, MetricDescriptor, MetricLabels, MetricRecorder, MetricSample, MetricValue,
};
use opentelemetry::KeyValue;
use opentelemetry::metrics::{Counter, Gauge, Histogram, Meter, UpDownCounter};
use std::collections::{BTreeMap, BTreeSet};
use std::num::NonZeroUsize;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Arc, Mutex};

/// Checked metric failures never silently narrow integers or change identities.
#[derive(Clone, Debug)]
pub enum OtelMetricError {
    /// Owned provider shutdown has closed new measurement admission.
    ProviderClosed,
    /// Empty/invalid instrument names or units are rejected before SDK construction.
    InvalidDefinition,
    /// One instrument name was reused with different kind/unit/domain semantics.
    ConflictingDefinition,
    /// Caller-selected descriptor or series cardinality capacity was reached.
    CapacityReached,
    /// The core value does not match the selected native numeric kind.
    WrongKind,
    /// Exact native representation is unavailable for this integer/duration.
    PrecisionLoss,
    /// A native instrument or policy callback panicked.
    Native(effect::Defect),
}
impl std::fmt::Display for OtelMetricError {
    /// Errors report structure without label or application payload values.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for OtelMetricError {}

/// Numeric instrument identity, also used for checked native instruments.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NativeInstrumentKind {
    /// Monotonic exact unsigned count, used by core counters.
    CounterU64,
    /// Native floating point monotonic count beyond core's integer kind.
    CounterF64,
    /// Native signed increment/decrement instrument.
    UpDownI64,
    /// Native floating point increment/decrement instrument.
    UpDownF64,
    /// Exact signed current measurement.
    GaugeI64,
    /// Exact unsigned current measurement.
    GaugeU64,
    /// Floating point current measurement.
    GaugeF64,
    /// Exact unsigned distribution, used for core duration values.
    HistogramU64,
    /// Native floating point distribution beyond core duration mapping.
    HistogramF64,
}

/// OTel/UCUM duration unit with an exact integer scaling contract.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LatencyUnit {
    /// SDK histogram observations are exact nanoseconds (`ns`).
    Nanoseconds,
    /// Exact microseconds (`us`); sub-microsecond values return `PrecisionLoss`.
    Microseconds,
    /// Exact milliseconds (`ms`); fractional milliseconds are rejected.
    Milliseconds,
    /// Exact seconds (`s`); fractional seconds are rejected.
    Seconds,
}
impl LatencyUnit {
    /// UCUM spellings are protocol identities, not operational policy defaults.
    fn unit(self) -> &'static str {
        match self {
            Self::Nanoseconds => "ns",
            Self::Microseconds => "us",
            Self::Milliseconds => "ms",
            Self::Seconds => "s",
        }
    }
    /// Convert without rounding; native `u64` bounds remain checked.
    fn exact(self, duration: std::time::Duration) -> Result<u64, OtelMetricError> {
        let nanos = duration.as_nanos();
        let scale = match self {
            Self::Nanoseconds => 1,
            Self::Microseconds => 1_000,
            Self::Milliseconds => 1_000_000,
            Self::Seconds => 1_000_000_000,
        };
        if !nanos.is_multiple_of(scale) {
            return Err(OtelMetricError::PrecisionLoss);
        }
        u64::try_from(nanos / scale).map_err(|_| OtelMetricError::PrecisionLoss)
    }
}

/// Explicit native numeric/unit mapping for one portable descriptor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MetricDefinition {
    /// Native instrument numeric/kind identity.
    pub kind: NativeInstrumentKind,
    /// Native unit; empty means explicitly dimensionless under the OTel API.
    pub unit: Arc<str>,
    /// Required exact scaling when mapping a core latency descriptor.
    pub latency_unit: Option<LatencyUnit>,
}

/// Select a checked native numeric/unit definition for one portable descriptor.
type DefineMetric =
    dyn Fn(&MetricDescriptor) -> Result<MetricDefinition, OtelMetricError> + Send + Sync;
/// Redact portable labels before admitting a series or exporting a measurement.
type RedactMetricLabels =
    dyn Fn(&MetricLabels) -> Result<MetricLabels, OtelMetricError> + Send + Sync;

/// Caller-owned instrument and cardinality policies, with no hidden quotas.
#[derive(Clone)]
pub struct MetricPolicy {
    /// Maximum different instrument names retained by this recorder.
    pub descriptors: NonZeroUsize,
    /// Maximum distinct label sets admitted per instrument identity.
    pub series_per_descriptor: NonZeroUsize,
    /// Stable definition callback; repeated incompatible answers are rejected.
    define: Arc<DefineMetric>,
    /// Labels pass redaction before cardinality admission and export.
    redact: Arc<RedactMetricLabels>,
}
impl MetricPolicy {
    /// Supply explicit limits, exact numeric/unit mapping and label redaction.
    #[must_use]
    pub fn new(
        descriptors: NonZeroUsize,
        series_per_descriptor: NonZeroUsize,
        define: impl Fn(&MetricDescriptor) -> Result<MetricDefinition, OtelMetricError>
        + Send
        + Sync
        + 'static,
        redact: impl Fn(&MetricLabels) -> Result<MetricLabels, OtelMetricError> + Send + Sync + 'static,
    ) -> Self {
        Self {
            descriptors,
            series_per_descriptor,
            define: Arc::new(define),
            redact: Arc::new(redact),
        }
    }
}

/// Checked synchronous native instrument families beyond the portable core.
#[derive(Clone)]
pub enum NativeInstrument {
    /// Exact unsigned monotonic observations.
    CounterU64(Counter<u64>),
    /// Native floating point monotonic observations.
    CounterF64(Counter<f64>),
    /// Native signed deltas.
    UpDownI64(UpDownCounter<i64>),
    /// Native floating point deltas.
    UpDownF64(UpDownCounter<f64>),
    /// Native exact signed gauge.
    GaugeI64(Gauge<i64>),
    /// Native exact unsigned gauge.
    GaugeU64(Gauge<u64>),
    /// Native floating point gauge.
    GaugeF64(Gauge<f64>),
    /// Native exact unsigned histogram.
    HistogramU64(Histogram<u64>),
    /// Native floating point histogram.
    HistogramF64(Histogram<f64>),
}

/// A single stable descriptor registration, including its bounded series set.
struct Entry {
    /// Portable identity includes domain; native registrations omit it deliberately.
    descriptor: Option<MetricDescriptor>,
    /// Numeric/unit fingerprint prevents SDK name reuse with incompatible data.
    definition: MetricDefinition,
    /// SDK-owned instrument handle; cloning shares its aggregation, not samples.
    instrument: NativeInstrument,
    /// Exact admitted label sets are bounded by the caller's policy.
    series: BTreeSet<MetricLabels>,
}

/// Core recorder adaptation using the official SDK meter and immutable policies.
#[derive(Clone)]
pub struct OtelMetricRecorder {
    /// Meter selected by the application's SDK provider and instrumentation scope.
    meter: Meter,
    /// Checked SDK admission is shared with owned provider shutdown.
    providers: crate::BorrowedProviders,
    /// No library-selected cardinality, numeric conversion or units.
    policy: MetricPolicy,
    /// One registry serializes name compatibility and series admission together.
    entries: Arc<Mutex<BTreeMap<String, Entry>>>,
}
impl OtelMetricRecorder {
    /// Select a native meter scope without constructing a provider/exporter/runtime.
    /// Retained admission prevents a silent measurement after owned close starts.
    pub fn new(
        providers: crate::BorrowedProviders,
        scope: opentelemetry::InstrumentationScope,
        policy: MetricPolicy,
    ) -> Result<Self, OtelMetricError> {
        use opentelemetry::metrics::MeterProvider;
        let _permit = providers.admit().map_err(admission_error)?;
        let meter = catch_unwind(AssertUnwindSafe(|| {
            providers.meter().meter_with_scope(scope)
        }))
        .map_err(|panic| OtelMetricError::Native(effect::Defect::from_panic(panic)))?;
        Ok(Self {
            meter,
            providers,
            policy,
            entries: Arc::new(Mutex::new(BTreeMap::new())),
        })
    }

    /// Register/check native synchronous instruments in the same identity registry.
    /// The returned SDK handle uses native measurement semantics. Explicit native
    /// observations own their label admission policy; portable `record` enforces
    /// this recorder's bounds. Observable instruments can be configured through
    /// `Otel::use_providers` with a separate instrumentation scope owned by the host.
    pub fn native(
        &self,
        name: &str,
        definition: MetricDefinition,
    ) -> Result<NativeInstrument, OtelMetricError> {
        let _permit = self.providers.admit().map_err(admission_error)?;
        catch_unwind(AssertUnwindSafe(|| {
            self.admit(name, None, definition, None)
        }))
        .map_err(|panic| OtelMetricError::Native(effect::Defect::from_panic(panic)))?
    }

    /// Use a registered native instrument while owned shutdown awaits this callback.
    /// This is the checked interoperation path for deltas/distributions beyond
    /// the portable kinds; numeric values and SDK measurement APIs remain native.
    pub fn use_native<A>(
        &self,
        name: &str,
        definition: MetricDefinition,
        action: impl FnOnce(&NativeInstrument) -> Result<A, OtelMetricError>,
    ) -> Result<A, OtelMetricError> {
        let _permit = self.providers.admit().map_err(admission_error)?;
        catch_unwind(AssertUnwindSafe(|| {
            let instrument = self.admit(name, None, definition, None)?;
            action(&instrument)
        }))
        .map_err(|panic| OtelMetricError::Native(effect::Defect::from_panic(panic)))?
    }

    /// Check definition/name/series as one mutation before publishing an instrument.
    fn admit(
        &self,
        name: &str,
        descriptor: Option<&MetricDescriptor>,
        definition: MetricDefinition,
        labels: Option<&MetricLabels>,
    ) -> Result<NativeInstrument, OtelMetricError> {
        validate_definition(name, &definition.unit)?;
        if let Some(unit) = definition.latency_unit
            && (definition.kind != NativeInstrumentKind::HistogramU64
                || definition.unit.as_ref() != unit.unit())
        {
            return Err(OtelMetricError::InvalidDefinition);
        }
        let mut entries = self
            .entries
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(entry) = entries.get_mut(name) {
            if entry.definition != definition || entry.descriptor.as_ref() != descriptor {
                return Err(OtelMetricError::ConflictingDefinition);
            }
            if let Some(labels) = labels {
                admit_series(entry, labels, self.policy.series_per_descriptor)?;
            }
            return Ok(entry.instrument.clone());
        }
        if entries.len() == self.policy.descriptors.get() {
            return Err(OtelMetricError::CapacityReached);
        }
        let instrument = self.build(name.to_owned(), &definition);
        let mut entry = Entry {
            descriptor: descriptor.cloned(),
            definition,
            instrument: instrument.clone(),
            series: BTreeSet::new(),
        };
        if let Some(labels) = labels {
            admit_series(&mut entry, labels, self.policy.series_per_descriptor)?;
        }
        entries.insert(name.to_owned(), entry);
        Ok(instrument)
    }

    /// Native SDK builders preserve numeric identity and caller-supplied units.
    fn build(&self, name: String, definition: &MetricDefinition) -> NativeInstrument {
        let unit = definition.unit.to_string();
        match definition.kind {
            NativeInstrumentKind::CounterU64 => {
                NativeInstrument::CounterU64(self.meter.u64_counter(name).with_unit(unit).build())
            }
            NativeInstrumentKind::CounterF64 => {
                NativeInstrument::CounterF64(self.meter.f64_counter(name).with_unit(unit).build())
            }
            NativeInstrumentKind::UpDownI64 => NativeInstrument::UpDownI64(
                self.meter.i64_up_down_counter(name).with_unit(unit).build(),
            ),
            NativeInstrumentKind::UpDownF64 => NativeInstrument::UpDownF64(
                self.meter.f64_up_down_counter(name).with_unit(unit).build(),
            ),
            NativeInstrumentKind::GaugeI64 => {
                NativeInstrument::GaugeI64(self.meter.i64_gauge(name).with_unit(unit).build())
            }
            NativeInstrumentKind::GaugeU64 => {
                NativeInstrument::GaugeU64(self.meter.u64_gauge(name).with_unit(unit).build())
            }
            NativeInstrumentKind::GaugeF64 => {
                NativeInstrument::GaugeF64(self.meter.f64_gauge(name).with_unit(unit).build())
            }
            NativeInstrumentKind::HistogramU64 => NativeInstrument::HistogramU64(
                self.meter.u64_histogram(name).with_unit(unit).build(),
            ),
            NativeInstrumentKind::HistogramF64 => NativeInstrument::HistogramF64(
                self.meter.f64_histogram(name).with_unit(unit).build(),
            ),
        }
    }
}

/// Native SDK 0.32 instrument-name contract; these are protocol bounds, not quotas.
const NATIVE_NAME_MAX_BYTES: usize = 255;
/// Native SDK 0.32 instrument-unit contract, measured in ASCII bytes.
const NATIVE_UNIT_MAX_BYTES: usize = 63;

/// Reject configurations which the native SDK would replace with a no-op handle.
/// The SDK's private `metrics::meter` validation is reproduced at this checked
/// boundary because native builder construction does not return its error.
fn validate_definition(name: &str, unit: &str) -> Result<(), OtelMetricError> {
    if name.is_empty()
        || name.len() > NATIVE_NAME_MAX_BYTES
        || !name.starts_with(|c: char| c.is_ascii_alphabetic())
        || name
            .contains(|c: char| !c.is_ascii_alphanumeric() && !matches!(c, '_' | '.' | '-' | '/'))
        || unit.len() > NATIVE_UNIT_MAX_BYTES
        || !unit.is_ascii()
    {
        return Err(OtelMetricError::InvalidDefinition);
    }
    Ok(())
}

impl MetricRecorder for OtelMetricRecorder {
    /// Policy/backend failures remain in the core's explicitly selected error channel.
    type Error = OtelMetricError;
    /// Redact, validate exact representation, admit identity/series and observe once.
    fn record(&self, sample: &MetricSample) -> Result<(), Self::Error> {
        let _permit = self.providers.admit().map_err(admission_error)?;
        catch_unwind(AssertUnwindSafe(|| {
            let definition = (self.policy.define)(sample.descriptor())?;
            let labels = (self.policy.redact)(sample.labels())?;
            // Validate representation before consuming registry/series capacity.
            let measurement = exact(sample.value(), &definition)?;
            let native = self.admit(
                sample.descriptor().name().as_str(),
                Some(sample.descriptor()),
                definition,
                Some(&labels),
            )?;
            let attributes = labels
                .as_map()
                .iter()
                .map(|(k, v)| KeyValue::new(k.to_string(), v.to_string()))
                .collect::<Vec<_>>();
            match (native, measurement) {
                (NativeInstrument::CounterU64(native), Measurement::U64(value)) => {
                    native.add(value, &attributes)
                }
                (NativeInstrument::GaugeI64(native), Measurement::I64(value)) => {
                    native.record(value, &attributes)
                }
                (NativeInstrument::GaugeU64(native), Measurement::U64(value)) => {
                    native.record(value, &attributes)
                }
                (NativeInstrument::GaugeF64(native), Measurement::F64(value)) => {
                    native.record(value, &attributes)
                }
                (NativeInstrument::HistogramU64(native), Measurement::U64(value)) => {
                    native.record(value, &attributes)
                }
                _ => return Err(OtelMetricError::WrongKind),
            }
            Ok(())
        }))
        .map_err(|panic| OtelMetricError::Native(effect::Defect::from_panic(panic)))?
    }
}

/// Numeric observations after checked conversion; no application payload is held.
enum Measurement {
    /// Exact unsigned native quantity.
    U64(u64),
    /// Exact signed native quantity.
    I64(i64),
    /// Original finite floating point value.
    F64(f64),
}
/// Convert core kinds without an `i128`/duration-to-f64 precision shortcut.
fn exact(
    value: MetricValue,
    definition: &MetricDefinition,
) -> Result<Measurement, OtelMetricError> {
    match (value, definition.kind) {
        (MetricValue::Counter(v), NativeInstrumentKind::CounterU64) => Ok(Measurement::U64(v)),
        (MetricValue::Gauge(GaugeValue::Integer(v)), NativeInstrumentKind::GaugeI64) => {
            i64::try_from(v)
                .map(Measurement::I64)
                .map_err(|_| OtelMetricError::PrecisionLoss)
        }
        (MetricValue::Gauge(GaugeValue::Integer(v)), NativeInstrumentKind::GaugeU64) => {
            u64::try_from(v)
                .map(Measurement::U64)
                .map_err(|_| OtelMetricError::PrecisionLoss)
        }
        (MetricValue::Gauge(GaugeValue::Float(v)), NativeInstrumentKind::GaugeF64)
            if v.is_finite() =>
        {
            Ok(Measurement::F64(v))
        }
        (MetricValue::Latency(v), NativeInstrumentKind::HistogramU64) => definition
            .latency_unit
            .ok_or(OtelMetricError::InvalidDefinition)?
            .exact(v)
            .map(Measurement::U64),
        _ => Err(OtelMetricError::WrongKind),
    }
}
/// Admit redacted labels under one instrument's explicit cardinality bound.
fn admit_series(
    entry: &mut Entry,
    labels: &MetricLabels,
    limit: NonZeroUsize,
) -> Result<(), OtelMetricError> {
    if !entry.series.contains(labels) && entry.series.len() == limit.get() {
        return Err(OtelMetricError::CapacityReached);
    }
    entry.series.insert(labels.clone());
    Ok(())
}

/// Preserve closed admission separately from overflow or native diagnostics.
fn admission_error(error: crate::NativeError) -> OtelMetricError {
    match error {
        crate::NativeError::ProviderClosed => OtelMetricError::ProviderClosed,
        other @ (crate::NativeError::NoActiveSpan
        | crate::NativeError::Parent(_)
        | crate::NativeError::Native(_)) => OtelMetricError::Native(other.into_defect()),
    }
}
