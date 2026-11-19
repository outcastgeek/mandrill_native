//! Fixed metric series and independently retained snapshots.

use super::{
    MetricAggregate, MetricDomain, MetricError, MetricKind, MetricRecorder,
    MetricSample, MetricValue, validate_value,
};
use crate::memory::{
    Bytes, FixedVector, MemoryError, PreparedMemory, RegionId, SharedSlot, SlotLease, SlotPool,
    Text, TextFormatError,
};
use crate::stream::{PreparedChunkPool, PreparedChunkValues};
use std::alloc::Layout;
use std::cmp::Ordering;
use std::fmt;
use std::num::NonZeroUsize;
use std::ops::Range;
use std::sync::{Mutex, PoisonError};

/// Startup bounds for metric series, temporary formatting and retained snapshots.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PreparedMetricCapacity {
    /// Distinct series retained by this recorder.
    pub series: NonZeroUsize,
    /// Labels in one series, including zero for an unlabelled recorder.
    pub labels_per_series: usize,
    /// Combined name, label names and label value bytes in one series.
    pub bytes_per_series: Bytes,
    /// Concurrent drafts; resident series use a separate owner pool.
    pub drafts: NonZeroUsize,
    /// Concurrent independently retained snapshots.
    pub snapshots: NonZeroUsize,
}

impl PreparedMetricCapacity {
    /// Demand for table entries, text, label spans, drafts and snapshot records.
    /// The enclosing memory plan accounts for allocator headers and alignment.
    pub fn storage_bytes(self) -> Result<Bytes, MemoryError> {
        let table =
            Layout::array::<Entry>(self.series.get()).map_err(|_| MemoryError::InvalidLayout)?;
        let text = Layout::array::<u8>(self.bytes_per_series.get())
            .map_err(|_| MemoryError::InvalidLayout)?;
        let labels = Layout::array::<Label>(self.labels_per_series)
            .map_err(|_| MemoryError::InvalidLayout)?;
        let count = self
            .series
            .get()
            .checked_add(self.drafts.get())
            .ok_or(MemoryError::ArithmeticOverflow)?;
        let identity = Bytes::new(text.size()).checked_add(Bytes::new(labels.size()))?;
        Bytes::new(table.size())
            .checked_add(SlotPool::<Identity>::storage_bytes(self.series.get())?)?
            .checked_add(SlotPool::<Identity>::storage_bytes(self.drafts.get())?)?
            .checked_add(identity.checked_mul(count)?)?
            .checked_add(PreparedChunkPool::<PreparedMetricSeries>::storage_bytes(
                self.snapshots,
                self.series,
            )?)
    }
}

/// Typed failure from the selected recorder or its formatter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PreparedMetricError {
    /// Existing metric validation, capacity or numeric overflow.
    Metric(MetricError),
    /// The caller's prepared physical workspace could not admit the operation.
    Memory(MemoryError),
    /// The supplied formatter itself refused.
    Formatter(fmt::Error),
    /// The formatted draft already has a label with this name.
    DuplicateLabel,
}

impl From<MetricError> for PreparedMetricError {
    fn from(error: MetricError) -> Self {
        Self::Metric(error)
    }
}

impl From<MemoryError> for PreparedMetricError {
    fn from(error: MemoryError) -> Self {
        Self::Memory(error)
    }
}

impl From<TextFormatError> for PreparedMetricError {
    fn from(error: TextFormatError) -> Self {
        match error {
            TextFormatError::Memory(error) => Self::Memory(error),
            TextFormatError::Formatter(error) => Self::Formatter(error),
        }
    }
}

impl fmt::Display for PreparedMetricError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self, formatter)
    }
}

impl std::error::Error for PreparedMetricError {}

struct Label {
    name: Range<usize>,
    value: Range<usize>,
}

struct Identity {
    text: Text<'static>,
    labels: FixedVector<'static, Label>,
    name_length: usize,
    kind: MetricKind,
    domain: MetricDomain,
}

impl Identity {
    fn prepare(
        memory: &PreparedMemory<'static>,
        region: RegionId,
        capacity: PreparedMetricCapacity,
    ) -> Result<Self, MemoryError> {
        Ok(Self {
            text: memory.text(region, capacity.bytes_per_series)?,
            labels: FixedVector::prepare(memory.allocator(region)?, capacity.labels_per_series)?,
            name_length: 0,
            kind: MetricKind::Counter,
            domain: MetricDomain::General,
        })
    }

    fn reset(&mut self, kind: MetricKind, domain: MetricDomain) {
        self.text.clear();
        self.labels.clear();
        self.name_length = 0;
        self.kind = kind;
        self.domain = domain;
    }

    fn descriptor(&self) -> (&str, MetricKind, MetricDomain) {
        let name = self
            .text
            .as_str()
            .get(..self.name_length)
            .unwrap_or_default();
        (name, self.kind, self.domain)
    }

    fn labels(&self) -> impl Iterator<Item = (&str, &str)> + Clone {
        let text = self.text.as_str();
        self.labels.as_slice().iter().map(move |label| {
            (
                text.get(label.name.clone()).unwrap_or_default(),
                text.get(label.value.clone()).unwrap_or_default(),
            )
        })
    }

    fn append_label(
        &mut self,
        position: usize,
        name: &str,
        value: impl FnOnce(&mut Text<'static>) -> Result<(), PreparedMetricError>,
    ) -> Result<(), PreparedMetricError> {
        if self.labels.len() == self.labels.capacity() {
            return Err(MemoryError::CollectionReservationFailed.into());
        }
        let start = self.text.as_str().len();
        let mut text = std::mem::DropGuard::new(&mut self.text, move |text| {
            let _ = text.truncate(start);
        });
        text.try_push_str(name)?;
        let split = text.as_str().len();
        value(&mut text)?;
        let end = text.as_str().len();
        let _ = std::mem::DropGuard::dismiss(text);
        self.labels.push_reserved(Label {
            name: start..split,
            value: split..end,
        });
        self.labels
            .as_mut_slice()
            .split_at_mut(position)
            .1
            .rotate_right(1);
        Ok(())
    }
}

/// An affine formatted series identity. Formatting uses its fixed text owner.
pub struct PreparedMetricDraft(SlotLease<'static, Identity>);

impl PreparedMetricDraft {
    /// Add a label under the same byte and count bounds as a resident series.
    /// Refusal restores the draft's prior labels.
    pub fn push_label(
        &mut self,
        name: &str,
        value: fmt::Arguments<'_>,
    ) -> Result<(), PreparedMetricError> {
        if name.trim().is_empty() {
            return Err(MetricError::EmptyLabel.into());
        }
        let position = {
            let text = self.0.text.as_str();
            match self.0.labels.as_slice().binary_search_by(|label| {
                text.get(label.name.clone()).unwrap_or_default().cmp(name)
            }) {
                Ok(_) => return Err(PreparedMetricError::DuplicateLabel),
                Err(position) => position,
            }
        };
        self.0.append_label(position, name, |text| {
            text.try_write_fmt(value).map_err(Into::into)
        })
    }
}

fn compare_labels<'left, 'right>(
    mut left: impl Iterator<Item = (&'left str, &'left str)>,
    mut right: impl Iterator<Item = (&'right str, &'right str)>,
) -> Ordering {
    loop {
        match (left.next(), right.next()) {
            (Some((left_name, left_value)), Some((right_name, right_value))) => {
                let ordering = left_name
                    .cmp(right_name)
                    .then_with(|| left_value.cmp(right_value));
                if ordering != Ordering::Equal {
                    return ordering;
                }
            }
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
        }
    }
}

struct Entry {
    identity: SharedSlot<'static, Identity>,
    aggregate: MetricAggregate,
}

/// An aggregate and independently retained name and label owner.
pub struct PreparedMetricSeries {
    identity: SharedSlot<'static, Identity>,
    /// Count, gauge or timing data observed at snapshot time.
    pub aggregate: MetricAggregate,
}

impl PreparedMetricSeries {
    /// Borrow the validated name.
    #[must_use]
    pub fn name(&self) -> &str {
        self.identity.descriptor().0
    }

    /// Read the measurement kind.
    #[must_use]
    pub fn kind(&self) -> MetricKind {
        self.identity.kind
    }

    /// Read the domain of this series.
    #[must_use]
    pub fn domain(&self) -> MetricDomain {
        self.identity.domain
    }

    /// Borrow the label names and values in their identity order.
    pub fn labels(&self) -> impl Iterator<Item = (&str, &str)> + '_ {
        self.identity.labels()
    }
}

/// An affine snapshot of all current aggregates in descriptor and label order.
pub struct PreparedMetricSnapshot(PreparedChunkValues<PreparedMetricSeries>);

impl PreparedMetricSnapshot {
    /// Borrow records and their byte owners.
    #[must_use]
    pub fn series(&self) -> &[PreparedMetricSeries] {
        self.0.as_slice()
    }
}

/// A process-local recorder selected with finite, physically prepared storage.
#[derive(crate::EffectEnvironment)]
#[effect_environment(metric_recorder(provider = Self, get = self))]
pub struct PreparedMetricRecorder {
    entries: Mutex<FixedVector<'static, Entry>>,
    identities: SlotPool<'static, Identity>,
    drafts: SlotPool<'static, Identity>,
    snapshots: PreparedChunkPool<PreparedMetricSeries>,
}

impl PreparedMetricRecorder {
    /// Provision all series, formatting and snapshot owners before sealing.
    pub fn prepare(
        memory: &PreparedMemory<'static>,
        region: RegionId,
        capacity: PreparedMetricCapacity,
    ) -> Result<Self, MemoryError> {
        capacity.storage_bytes()?;
        Ok(Self {
            entries: Mutex::new(FixedVector::prepare(
                memory.allocator(region)?,
                capacity.series.get(),
            )?),
            identities: memory.slots(region, capacity.series.get(), |_| {
                Identity::prepare(memory, region, capacity)
            })?,
            drafts: memory.slots(region, capacity.drafts.get(), |_| {
                Identity::prepare(memory, region, capacity)
            })?,
            snapshots: PreparedChunkPool::prepare(
                memory,
                region,
                capacity.snapshots,
                capacity.series,
            )?,
        })
    }

    /// Reserve a draft before invoking formatting for its name.
    pub fn reserve_series(
        &self,
        name: fmt::Arguments<'_>,
        kind: MetricKind,
        domain: MetricDomain,
    ) -> Result<PreparedMetricDraft, PreparedMetricError> {
        let mut identity = self.drafts.try_checkout()?;
        identity.reset(kind, domain);
        identity.text.try_write_fmt(name)?;
        if identity.text.as_str().trim().is_empty() {
            return Err(MetricError::EmptyName.into());
        }
        identity.name_length = identity.text.as_str().len();
        Ok(PreparedMetricDraft(identity))
    }

    /// Consume a complete formatted draft and apply one measurement.
    pub fn record_prepared(
        &self,
        draft: PreparedMetricDraft,
        value: MetricValue,
    ) -> Result<(), PreparedMetricError> {
        if !draft.0.belongs_to(&self.drafts) {
            return Err(MemoryError::UnsupportedRunContract.into());
        }
        self.record_ordered(draft.0.descriptor(), draft.0.labels(), value)
    }

    /// Number of distinct retained series.
    #[must_use]
    pub fn series_count(&self) -> usize {
        self.entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len()
    }

    /// Retain the aggregates and immutable identities without copying their text.
    pub fn snapshot(&self) -> Result<PreparedMetricSnapshot, PreparedMetricError> {
        let mut snapshot = self.snapshots.try_checkout()?;
        let entries = self.entries.lock().unwrap_or_else(PoisonError::into_inner);
        for entry in entries.as_slice() {
            snapshot.push_reserved(PreparedMetricSeries {
                identity: entry.identity.try_clone()?,
                aggregate: entry.aggregate,
            });
        }
        Ok(PreparedMetricSnapshot(snapshot))
    }

    fn record_ordered<'a>(
        &self,
        descriptor: (&str, MetricKind, MetricDomain),
        labels: impl Iterator<Item = (&'a str, &'a str)> + Clone,
        value: MetricValue,
    ) -> Result<(), PreparedMetricError> {
        validate_value(descriptor.1, value)?;
        let mut entries = self.entries.lock().unwrap_or_else(PoisonError::into_inner);
        let position = entries.as_slice().binary_search_by(|entry| {
            match entry.identity.descriptor().cmp(&descriptor) {
                Ordering::Equal => compare_labels(entry.identity.labels(), labels.clone()),
                other @ (Ordering::Less | Ordering::Greater) => other,
            }
        });
        let index = match position {
            Ok(index) => {
                let entry = entries
                    .as_mut_slice()
                    .get_mut(index)
                    .ok_or(MemoryError::InvalidRange)?;
                return entry.aggregate.add(value).map_err(Into::into);
            }
            Err(index) => index,
        };
        if entries.len() == entries.capacity() {
            return Err(MetricError::CapacityReached.into());
        }
        let (name, kind, domain) = descriptor;
        let mut identity = self.identities.try_checkout()?;
        identity.reset(kind, domain);
        identity.text.try_push_str(name)?;
        identity.name_length = name.len();
        for (position, (name, value)) in labels.enumerate() {
            identity.append_label(position, name, |text| {
                text.try_push_str(value).map_err(Into::into)
            })?;
        }
        entries.push_reserved(Entry {
            identity: identity.share(),
            aggregate: MetricAggregate::first(value),
        });
        entries.as_mut_slice().split_at_mut(index).1.rotate_right(1);
        Ok(())
    }
}

impl MetricRecorder for PreparedMetricRecorder {
    type Error = PreparedMetricError;

    fn record(&self, sample: &MetricSample) -> Result<(), Self::Error> {
        let descriptor = sample.descriptor();
        let labels = sample
            .labels()
            .as_map()
            .iter()
            .map(|(key, value)| (key.as_ref(), value.as_ref()));
        self.record_ordered(
            (
                descriptor.name().as_str(),
                descriptor.kind(),
                descriptor.domain(),
            ),
            labels,
            sample.value(),
        )
    }
}
