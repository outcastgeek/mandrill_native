use crate::{Effect, Exit};
use std::collections::BTreeMap;
use std::num::NonZeroUsize;
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MetricError {
    EmptyName,
    EmptyLabel,
    WrongKind,
    NonFiniteGauge,
    CapacityReached,
    Overflow,
}

impl std::fmt::Display for MetricError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(self, formatter)
    }
}

impl std::error::Error for MetricError {}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct MetricName(Arc<str>);

impl MetricName {
    pub fn new(name: impl Into<Arc<str>>) -> Result<Self, MetricError> {
        let name = name.into();
        if name.trim().is_empty() {
            return Err(MetricError::EmptyName);
        }
        Ok(Self(name))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum MetricKind {
    Counter,
    Gauge,
    Latency,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum MetricDomain {
    General,
    Queue,
    Retry,
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
    pub const fn new(name: MetricName, kind: MetricKind, domain: MetricDomain) -> Self {
        Self { name, kind, domain }
    }

    #[must_use]
    pub const fn name(&self) -> &MetricName {
        &self.name
    }

    #[must_use]
    pub const fn kind(&self) -> MetricKind {
        self.kind
    }

    #[must_use]
    pub const fn domain(&self) -> MetricDomain {
        self.domain
    }

    #[must_use]
    pub const fn queue_depth(name: MetricName) -> Self {
        Self::new(name, MetricKind::Gauge, MetricDomain::Queue)
    }

    #[must_use]
    pub const fn retry_attempts(name: MetricName) -> Self {
        Self::new(name, MetricKind::Counter, MetricDomain::Retry)
    }

    #[must_use]
    pub const fn commit_latency(name: MetricName) -> Self {
        Self::new(name, MetricKind::Latency, MetricDomain::Commit)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct MetricLabels(BTreeMap<Arc<str>, Arc<str>>);

impl MetricLabels {
    pub fn new(labels: BTreeMap<Arc<str>, Arc<str>>) -> Result<Self, MetricError> {
        if labels.keys().any(|key| key.trim().is_empty()) {
            return Err(MetricError::EmptyLabel);
        }
        Ok(Self(labels))
    }

    #[must_use]
    pub fn as_map(&self) -> &BTreeMap<Arc<str>, Arc<str>> {
        &self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum GaugeValue {
    Integer(i128),
    Float(f64),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MetricValue {
    Counter(u64),
    Gauge(GaugeValue),
    Latency(Duration),
}

impl MetricValue {
    #[must_use]
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
    pub const fn descriptor(&self) -> &MetricDescriptor {
        &self.descriptor
    }

    #[must_use]
    pub const fn labels(&self) -> &MetricLabels {
        &self.labels
    }

    #[must_use]
    pub const fn value(&self) -> MetricValue {
        self.value
    }
}

/// Implementations either accept one measurement or return their typed error.
/// They do not silently sample, drop, change kind or claim stronger storage
/// guarantees than their implementation provides.
pub trait MetricRecorder: Send + Sync + 'static {
    type Error: Send + 'static;

    fn record(&self, sample: &MetricSample) -> Result<(), Self::Error>;
}

pub trait HasMetricRecorder {
    type Recorder: MetricRecorder + ?Sized;

    fn metric_recorder(&self) -> &Self::Recorder;
}

impl<E, R> Effect<(), E, R>
where
    E: Send + 'static,
    R: HasMetricRecorder + Send + Sync + 'static,
    R::Recorder: MetricRecorder<Error = E>,
{
    pub fn record_metric(sample: MetricSample) -> Self {
        Self::ready(move |env, _| match env.metric_recorder().record(&sample) {
            Ok(()) => Exit::Success(()),
            Err(error) => Exit::Fail(error),
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MetricCapacity {
    Unbounded,
    Bounded(NonZeroUsize),
}

impl MetricCapacity {
    const fn permits(self, current: usize) -> bool {
        match self {
            Self::Unbounded => true,
            Self::Bounded(limit) => current < limit.get(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MetricAggregate {
    Counter(u128),
    Gauge(GaugeValue),
    Latency {
        count: u64,
        total: Duration,
        minimum: Duration,
        maximum: Duration,
    },
}

impl MetricAggregate {
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

pub struct MetricSeries {
    pub descriptor: MetricDescriptor,
    pub labels: MetricLabels,
    pub aggregate: MetricAggregate,
}

#[derive(Default)]
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
    pub fn new(capacity: MetricCapacity) -> Self {
        Self {
            state: Arc::new(Mutex::new(MemoryMetrics::default())),
            capacity,
        }
    }

    #[must_use]
    pub fn series_count(&self) -> usize {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .count
    }

    #[must_use]
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
    type Error = MetricError;

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
    type Recorder = Self;

    fn metric_recorder(&self) -> &Self::Recorder {
        self
    }
}
