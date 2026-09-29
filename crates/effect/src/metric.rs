//! Typed metric descriptions and provider-neutral recording.

use crate::{Effect, Exit};
use std::collections::BTreeMap;
use std::num::NonZeroUsize;
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// A typed failure to define or record a metric under its declared constraints.
pub enum MetricError {
    /// The required name is empty.
    EmptyName,
    /// The required label is empty.
    EmptyLabel,
    /// The supplied kind does not match the required or recorded value.
    WrongKind,
    /// A gauge update supplied a non-finite numeric value.
    NonFiniteGauge,
    /// The configured capacity bound was reached.
    CapacityReached,
    /// A bounded counter or arithmetic operation overflowed.
    Overflow,
}

impl std::fmt::Display for MetricError {
    ///
    /// # Parameters
    /// - `formatter` (`&mut std::fmt::Formatter<'_>`): Destination formatter used by the Rust formatting trait.
    ///
    /// # Returns
    /// Type: `std::fmt::Result`.
    /// Returns `std::fmt::Result`.
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(self, formatter)
    }
}

impl std::error::Error for MetricError {}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
/// A validated stable name for one metric.
pub struct MetricName(Arc<str>);

impl MetricName {
    /// Construct a `MetricName` from its required inputs and explicit policies.
    ///
    /// # Parameters
    /// - `name` (`impl Into<Arc<str>>`): Stable name used to register or locate this definition.
    ///
    /// # Returns
    /// Type: `Result<Self, MetricError>`.
    pub fn new(name: impl Into<Arc<str>>) -> Result<Self, MetricError> {
        let name = name.into();
        if name.trim().is_empty() {
            return Err(MetricError::EmptyName);
        }
        Ok(Self(name))
    }

    #[must_use]
    /// Borrow the textual representation without allocating a new string.
    ///
    /// # Returns
    /// Type: `&str`.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
/// The measurement class required by a metric descriptor.
pub enum MetricKind {
    /// The metric accumulates discrete increments.
    Counter,
    /// The metric records a finite current value.
    Gauge,
    /// The metric records elapsed-time observations.
    Latency,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
/// The identity scope used to group related metric series.
pub enum MetricDomain {
    /// The metric belongs to the general application domain.
    General,
    /// The operation addresses a queue.
    Queue,
    /// The operation must be retried under its declared policy.
    Retry,
    /// Commit the proposed state change.
    Commit,
}

/// A stable, provider-neutral series description. Name and policy are chosen
/// by the consumer; no backend, exporter or hidden sampling policy is implied.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct MetricDescriptor {
    name: MetricName,
    kind: MetricKind,
    domain: MetricDomain,
}

impl MetricDescriptor {
    #[must_use]
    /// # Parameters
    /// - `name` (`MetricName`): Stable name used to register or locate this definition.
    /// - `kind` (`MetricKind`): The kind supplied to this operation.
    /// - `domain` (`MetricDomain`): Stable identity or provider of the operation’s transaction domain.
    ///
    /// # Returns
    /// Type: `Self`.
    pub const fn new(name: MetricName, kind: MetricKind, domain: MetricDomain) -> Self {
        Self { name, kind, domain }
    }

    #[must_use]
    /// # Returns
    /// Type: `&MetricName`.
    pub const fn name(&self) -> &MetricName {
        &self.name
    }

    #[must_use]
    /// # Returns
    /// Type: `MetricKind`.
    pub const fn kind(&self) -> MetricKind {
        self.kind
    }

    #[must_use]
    /// # Returns
    /// Type: `MetricDomain`.
    pub const fn domain(&self) -> MetricDomain {
        self.domain
    }

    #[must_use]
    /// # Parameters
    /// - `name` (`MetricName`): Stable name used to register or locate this definition.
    ///
    /// # Returns
    /// Type: `Self`.
    pub const fn queue_depth(name: MetricName) -> Self {
        Self::new(name, MetricKind::Gauge, MetricDomain::Queue)
    }

    #[must_use]
    /// # Parameters
    /// - `name` (`MetricName`): Stable name used to register or locate this definition.
    ///
    /// # Returns
    /// Type: `Self`.
    pub const fn retry_attempts(name: MetricName) -> Self {
        Self::new(name, MetricKind::Counter, MetricDomain::Retry)
    }

    #[must_use]
    /// # Parameters
    /// - `name` (`MetricName`): Stable name used to register or locate this definition.
    ///
    /// # Returns
    /// Type: `Self`.
    pub const fn commit_latency(name: MetricName) -> Self {
        Self::new(name, MetricKind::Latency, MetricDomain::Commit)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
/// An immutable label set identifying one metric series.
pub struct MetricLabels(BTreeMap<Arc<str>, Arc<str>>);

impl MetricLabels {
    /// Construct a `MetricLabels` from its required inputs and explicit policies.
    ///
    /// # Parameters
    /// - `labels` (`BTreeMap<Arc<str>, Arc<str>>`): The labels supplied to this operation.
    ///
    /// # Returns
    /// Type: `Result<Self, MetricError>`.
    pub fn new(labels: BTreeMap<Arc<str>, Arc<str>>) -> Result<Self, MetricError> {
        if labels.keys().any(|key| key.trim().is_empty()) {
            return Err(MetricError::EmptyLabel);
        }
        Ok(Self(labels))
    }

    #[must_use]
    /// Borrow the metric labels as key/value entries.
    ///
    /// # Returns
    /// Type: `&BTreeMap<Arc<str>, Arc<str>>`.
    pub fn as_map(&self) -> &BTreeMap<Arc<str>, Arc<str>> {
        &self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
/// A finite numeric sample accepted by a gauge.
pub enum GaugeValue {
    /// The encoded value is an integer. Contains `i128`.
    Integer(i128),
    /// The encoded numeric value is floating point. Contains `f64`.
    Float(f64),
}

#[derive(Clone, Copy, Debug, PartialEq)]
/// A measurement submitted to a recorder under a metric descriptor.
pub enum MetricValue {
    /// The metric accumulates discrete increments. Contains `u64`.
    Counter(u64),
    /// The metric records a finite current value. Contains `GaugeValue`.
    Gauge(GaugeValue),
    /// The metric records elapsed-time observations. Contains `Duration`.
    Latency(Duration),
}

impl MetricValue {
    #[must_use]
    /// # Returns
    /// Type: `MetricKind`.
    pub const fn kind(self) -> MetricKind {
        match self {
            Self::Counter(_) => MetricKind::Counter,
            Self::Gauge(_) => MetricKind::Gauge,
            Self::Latency(_) => MetricKind::Latency,
        }
    }
}

/// A validated immutable measurement; it may be recorded more than once only
/// when the consumer intentionally executes its containing Effect again.
pub struct MetricSample {
    descriptor: MetricDescriptor,
    labels: MetricLabels,
    value: MetricValue,
}

impl MetricSample {
    /// Construct a `MetricSample` from its required inputs and explicit policies.
    ///
    /// # Parameters
    /// - `descriptor` (`MetricDescriptor`): The descriptor supplied to this operation.
    /// - `labels` (`MetricLabels`): The labels supplied to this operation.
    /// - `value` (`MetricValue`): Value read, transformed, encoded, or stored by this operation.
    ///
    /// # Returns
    /// Type: `Result<Self, MetricError>`.
    pub fn new(
        descriptor: MetricDescriptor,
        labels: MetricLabels,
        value: MetricValue,
    ) -> Result<Self, MetricError> {
        if descriptor.kind != value.kind() {
            return Err(MetricError::WrongKind);
        }
        if matches!(value, MetricValue::Gauge(GaugeValue::Float(number)) if !number.is_finite()) {
            return Err(MetricError::NonFiniteGauge);
        }
        Ok(Self {
            descriptor,
            labels,
            value,
        })
    }

    #[must_use]
    /// # Returns
    /// Type: `&MetricDescriptor`.
    pub const fn descriptor(&self) -> &MetricDescriptor {
        &self.descriptor
    }

    #[must_use]
    /// # Returns
    /// Type: `&MetricLabels`.
    pub const fn labels(&self) -> &MetricLabels {
        &self.labels
    }

    #[must_use]
    /// # Returns
    /// Type: `MetricValue`.
    pub const fn value(&self) -> MetricValue {
        self.value
    }
}

/// Implementations either accept one measurement or return their typed error.
/// They do not silently sample, drop, change kind or claim stronger storage
/// guarantees than their implementation provides.
pub trait MetricRecorder: Send + Sync + 'static {
    /// Provider-specific failure type returned by this capability.
    type Error: Send + 'static;

    /// # Parameters
    /// - `sample` (`&MetricSample`): The sample supplied to this operation.
    ///
    /// # Returns
    /// Type: `Result<(), Self::Error>`.
    fn record(&self, sample: &MetricSample) -> Result<(), Self::Error>;
}

/// Selects the metric recording capability in an application environment.
pub trait HasMetricRecorder {
    /// Metric recorder selected by this environment.
    type Recorder: MetricRecorder + ?Sized;

    /// # Returns
    /// Type: `&Self::Recorder`.
    fn metric_recorder(&self) -> &Self::Recorder;
}

impl<E, R> Effect<(), E, R>
where
    E: Send + 'static,
    R: HasMetricRecorder + Send + Sync + 'static,
    R::Recorder: MetricRecorder<Error = E>,
{
    /// Validate and submit one metric sample to the environment recorder.
    ///
    /// # Type parameters
    /// - `E`: Domain-error type.
    /// - `R`: Environment type.
    ///
    /// # Parameters
    /// - `sample` (`MetricSample`): The sample supplied to this operation.
    ///
    /// # Returns
    /// Type: `Self`.
    pub fn record_metric(sample: MetricSample) -> Self {
        Self::ready(move |env, _| match env.metric_recorder().record(&sample) {
            Ok(()) => Exit::Success(()),
            Err(error) => Exit::Fail(error),
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// Explicit resource bounds for metric.
pub enum MetricCapacity {
    /// No explicit capacity bound is imposed.
    Unbounded,
    /// The supplied positive capacity bound is enforced. Contains `NonZeroUsize`.
    Bounded(NonZeroUsize),
}

impl MetricCapacity {
    ///
    /// # Parameters
    /// - `current` (`usize`): The current supplied to this operation.
    ///
    /// # Returns
    /// Type: `bool`.
    /// Returns whether the requested condition holds.
    const fn permits(self, current: usize) -> bool {
        match self {
            Self::Unbounded => true,
            Self::Bounded(limit) => current < limit.get(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
/// The accumulated count, gauge, or timing data for a metric series.
pub enum MetricAggregate {
    /// The metric accumulates discrete increments. Contains `u128`.
    Counter(u128),
    /// The metric records a finite current value. Contains `GaugeValue`.
    Gauge(GaugeValue),
    /// The metric records elapsed-time observations.
    Latency {
        /// The number of recorded items or operations. Type: `u64`.
        count: u64,
        /// Accumulated duration across all recorded samples. Type: `Duration`.
        total: Duration,
        /// Smallest value permitted by this policy. Type: `Duration`.
        minimum: Duration,
        /// Largest value permitted by this policy. Type: `Duration`.
        maximum: Duration,
    },
}

impl MetricAggregate {
    ///
    /// # Parameters
    /// - `value` (`MetricValue`): Value read, transformed, encoded, or stored by this operation.
    ///
    /// # Returns
    /// Type: `Self`.
    fn first(value: MetricValue) -> Self {
        match value {
            MetricValue::Counter(delta) => Self::Counter(u128::from(delta)),
            MetricValue::Gauge(value) => Self::Gauge(value),
            MetricValue::Latency(value) => Self::Latency {
                count: 1,
                total: value,
                minimum: value,
                maximum: value,
            },
        }
    }

    ///
    /// # Parameters
    /// - `value` (`MetricValue`): Value read, transformed, encoded, or stored by this operation.
    ///
    /// # Returns
    /// Type: `Result<(), MetricError>`.
    fn add(&mut self, value: MetricValue) -> Result<(), MetricError> {
        match (self, value) {
            (Self::Counter(total), MetricValue::Counter(delta)) => {
                *total = total
                    .checked_add(u128::from(delta))
                    .ok_or(MetricError::Overflow)?;
            }
            (Self::Gauge(current), MetricValue::Gauge(value)) => *current = value,
            (
                Self::Latency {
                    count,
                    total,
                    minimum,
                    maximum,
                },
                MetricValue::Latency(value),
            ) => {
                let next_count = count.checked_add(1).ok_or(MetricError::Overflow)?;
                let next_total = total.checked_add(value).ok_or(MetricError::Overflow)?;
                *count = next_count;
                *total = next_total;
                *minimum = (*minimum).min(value);
                *maximum = (*maximum).max(value);
            }
            _ => return Err(MetricError::WrongKind),
        }
        Ok(())
    }
}

/// A descriptor, labels, and current aggregate for one recorded series.
pub struct MetricSeries {
    /// Metric name, kind, and domain for this series. Type: `MetricDescriptor`.
    pub descriptor: MetricDescriptor,
    /// Label set that distinguishes this metric series. Type: `MetricLabels`.
    pub labels: MetricLabels,
    /// Aggregate identity or accumulated metric data associated with this record. Type: `MetricAggregate`.
    pub aggregate: MetricAggregate,
}

#[derive(Default)]
/// Typed memory metrics used by metric.
struct MemoryMetrics {
    series: BTreeMap<MetricDescriptor, BTreeMap<MetricLabels, MetricAggregate>>,
    count: usize,
}

/// In-memory semantic implementation. Its capacity is explicit; full
/// cardinality returns an error instead of dropping measurements silently.
#[derive(Clone)]
pub struct MemoryMetricRecorder {
    state: Arc<Mutex<MemoryMetrics>>,
    capacity: MetricCapacity,
}

impl MemoryMetricRecorder {
    #[must_use]
    /// Construct a `MemoryMetricRecorder` from its required inputs and explicit policies.
    ///
    /// # Parameters
    /// - `capacity` (`MetricCapacity`): The caller-supplied capacity bound.
    ///
    /// # Returns
    /// Type: `Self`.
    pub fn new(capacity: MetricCapacity) -> Self {
        Self {
            state: Arc::new(Mutex::new(MemoryMetrics::default())),
            capacity,
        }
    }

    #[must_use]
    /// Count distinct metric series retained by this recorder.
    ///
    /// # Returns
    /// Type: `usize`.
    pub fn series_count(&self) -> usize {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .count
    }

    #[must_use]
    /// Read a consistent state and position for later observation or replay.
    ///
    /// # Returns
    /// Type: `Vec<MetricSeries>`.
    pub fn snapshot(&self) -> Vec<MetricSeries> {
        let state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state
            .series
            .iter()
            .flat_map(|(descriptor, group)| {
                group.iter().map(move |(labels, aggregate)| MetricSeries {
                    descriptor: descriptor.clone(),
                    labels: labels.clone(),
                    aggregate: *aggregate,
                })
            })
            .collect()
    }
}

impl MetricRecorder for MemoryMetricRecorder {
    /// Type alias for error in metric.
    type Error = MetricError;

    ///
    /// # Parameters
    /// - `sample` (`&MetricSample`): The sample supplied to this operation.
    ///
    /// # Returns
    /// Type: `Result<(), Self::Error>`.
    fn record(&self, sample: &MetricSample) -> Result<(), Self::Error> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(existing) = state
            .series
            .get_mut(&sample.descriptor)
            .and_then(|group| group.get_mut(&sample.labels))
        {
            return existing.add(sample.value);
        }
        if !self.capacity.permits(state.count) {
            return Err(MetricError::CapacityReached);
        }
        let next_count = state.count.checked_add(1).ok_or(MetricError::Overflow)?;
        state
            .series
            .entry(sample.descriptor.clone())
            .or_default()
            .insert(sample.labels.clone(), MetricAggregate::first(sample.value));
        state.count = next_count;
        Ok(())
    }
}

impl HasMetricRecorder for MemoryMetricRecorder {
    /// Type alias for recorder in metric.
    type Recorder = Self;

    ///
    /// # Returns
    /// Type: `&Self::Recorder`.
    fn metric_recorder(&self) -> &Self::Recorder {
        self
    }
}
