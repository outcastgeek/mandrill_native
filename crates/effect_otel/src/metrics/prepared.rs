use super::{
    MetricDefinition, MetricPolicy, NativeInstrument, NativeInstrumentKind, OtelMetricError,
};
use effect::memory::{
    Bytes, FixedVector, MemoryError, PreparedMemory, RegionId, SharedSlot, SlotLease, SlotPool,
    Text,
};
use effect::{MetricDescriptor, MetricDomain, MetricKind, MetricLabels};
use opentelemetry::KeyValue;
use std::{alloc::Layout, num::NonZeroUsize, ops::Range, sync::Mutex};

type SeriesOwner = SharedSlot<'static, Series>;
type DescriptorOwner = SlotLease<'static, Descriptor>;

/// Physical byte and label bounds for the prepared OTel metric registry.
#[derive(Clone, Copy, Debug)]
pub struct PreparedOtelMetricCapacity {
    /// Additional series slots for simultaneous recording attempts and existing-series lookups.
    pub drafts: NonZeroUsize,
    /// Label count in each independently retained series.
    pub labels_per_series: NonZeroUsize,
    /// Total key/value bytes in each series.
    pub label_bytes_per_series: NonZeroUsize,
}

impl PreparedOtelMetricCapacity {
    fn series_slots(self, policy: &MetricPolicy) -> Result<usize, MemoryError> {
        policy
            .descriptors
            .get()
            .checked_mul(policy.series_per_descriptor.get())
            .and_then(|rows| rows.checked_add(self.drafts.get()))
            .ok_or(MemoryError::ArithmeticOverflow)
    }

    /// Checked regional bytes for names, units, tables, label text, and SDK handle arrays.
    pub fn storage_bytes(self, policy: &MetricPolicy) -> Result<Bytes, MemoryError> {
        fn bytes<T>(items: usize) -> Result<Bytes, MemoryError> {
            Layout::array::<T>(items)
                .map(|layout| Bytes::new(layout.size()))
                .map_err(|_| MemoryError::InvalidLayout)
        }
        let descriptors = policy.descriptors.get();
        let series = self.series_slots(policy)?;
        let labels = self.labels_per_series.get();
        let label_storage = bytes::<u8>(self.label_bytes_per_series.get())?
            .checked_add(bytes::<Label>(labels)?)?
            .checked_add(bytes::<KeyValue>(labels)?)?;
        let descriptors_storage = bytes::<DescriptorOwner>(descriptors)?
            .checked_add(SlotPool::<Descriptor>::storage_bytes(descriptors)?)?
            .checked_add(
                bytes::<SeriesOwner>(policy.series_per_descriptor.get())?
                    .checked_mul(descriptors)?,
            )?;
        descriptors_storage
            .checked_add(SlotPool::<()>::storage_bytes(self.drafts.get())?)?
            .checked_add(SlotPool::<Series>::storage_bytes(series)?)?
            .checked_add(label_storage.checked_mul(series)?)
    }
}

struct Label {
    key: Range<usize>,
    value: Range<usize>,
}

pub(super) struct Series {
    text: Text<'static>,
    labels: FixedVector<'static, Label>,
    attributes: FixedVector<'static, KeyValue>,
}

impl Series {
    fn prepare(
        memory: &PreparedMemory<'static>,
        region: RegionId,
        capacity: PreparedOtelMetricCapacity,
    ) -> Result<Self, MemoryError> {
        let labels = capacity.labels_per_series.get();
        Ok(Self {
            text: memory.text(region, Bytes::new(capacity.label_bytes_per_series.get()))?,
            labels: FixedVector::prepare(memory.allocator(region)?, labels)?,
            attributes: FixedVector::prepare(memory.allocator(region)?, labels)?,
        })
    }

    fn labels(&self) -> impl Iterator<Item = (&str, &str)> {
        let text = self.text.as_str();
        self.labels.as_slice().iter().map(move |label| {
            (
                text.get(label.key.clone()).unwrap_or_default(),
                text.get(label.value.clone()).unwrap_or_default(),
            )
        })
    }

    pub(super) fn attributes(&self) -> &[KeyValue] {
        self.attributes.as_slice()
    }
}

pub(super) struct SeriesDraft {
    slot: SlotLease<'static, Series>,
    _admission: SlotLease<'static, ()>,
}

impl SeriesDraft {
    pub(super) fn stage(&mut self, labels: &MetricLabels) -> Result<(), OtelMetricError> {
        if labels.as_map().len() > self.slot.labels.capacity() {
            return Err(OtelMetricError::CapacityReached);
        }
        for (key, value) in labels.as_map() {
            let start = self.slot.text.as_str().len();
            self.slot.text.try_push_str(key).map_err(capacity_error)?;
            let split = self.slot.text.as_str().len();
            self.slot.text.try_push_str(value).map_err(capacity_error)?;
            let end = self.slot.text.as_str().len();
            self.slot
                .labels
                .push(Label {
                    key: start..split,
                    value: split..end,
                })
                .map_err(|_| OtelMetricError::CapacityReached)?;
        }
        Ok(())
    }

    fn publish(mut self) -> Result<(SeriesOwner, SeriesOwner), OtelMetricError> {
        let series = &mut *self.slot;
        let text = series.text.as_str();
        for label in series.labels.as_slice() {
            let key = text.get(label.key.clone()).unwrap_or_default();
            let value = text.get(label.value.clone()).unwrap_or_default();
            // The SDK requires owned strings, but the region owns the indexed KeyValue
            // array. The native conversions run once per new series.
            series
                .attributes
                .push(KeyValue::new(key.to_owned(), value.to_owned()))
                .map_err(|_| OtelMetricError::CapacityReached)?;
        }
        let owner = self.slot.share();
        let observation = owner.try_clone().map_err(capacity_error)?;
        Ok((owner, observation))
    }
}

struct Descriptor {
    name: [u8; super::NATIVE_NAME_MAX_BYTES],
    name_len: usize,
    unit: [u8; super::NATIVE_UNIT_MAX_BYTES],
    unit_len: usize,
    portable: Option<(MetricKind, MetricDomain)>,
    definition_kind: NativeInstrumentKind,
    latency_unit: Option<super::LatencyUnit>,
    native: Option<NativeInstrument>,
    series: FixedVector<'static, SeriesOwner>,
}

impl Descriptor {
    fn name(&self) -> &[u8] {
        self.name.get(..self.name_len).unwrap_or_default()
    }

    fn set_identity(
        &mut self,
        name: &str,
        portable: Option<(MetricKind, MetricDomain)>,
        definition: &MetricDefinition,
    ) -> Result<(), OtelMetricError> {
        self.name
            .get_mut(..name.len())
            .ok_or(OtelMetricError::InvalidDefinition)?
            .copy_from_slice(name.as_bytes());
        self.unit
            .get_mut(..definition.unit.len())
            .ok_or(OtelMetricError::InvalidDefinition)?
            .copy_from_slice(definition.unit.as_bytes());
        self.name_len = name.len();
        self.unit_len = definition.unit.len();
        self.portable = portable;
        self.definition_kind = definition.kind;
        self.latency_unit = definition.latency_unit;
        Ok(())
    }

    fn matches(
        &self,
        portable: Option<(MetricKind, MetricDomain)>,
        definition: &MetricDefinition,
    ) -> bool {
        self.portable == portable
            && self.definition_kind == definition.kind
            && self.latency_unit == definition.latency_unit
            && self.unit.get(..self.unit_len) == Some(definition.unit.as_bytes())
    }

    fn admit_series(&mut self, draft: SeriesDraft) -> Result<SeriesOwner, OtelMetricError> {
        if let Some(series) = self
            .series
            .as_slice()
            .iter()
            .find(|series| series.labels().eq(draft.slot.labels()))
        {
            return series.try_clone().map_err(capacity_error);
        }
        if self.series.len() == self.series.capacity() {
            return Err(OtelMetricError::CapacityReached);
        }
        let (resident, observation) = draft.publish()?;
        self.series
            .push(resident)
            .map_err(|_| OtelMetricError::CapacityReached)?;
        Ok(observation)
    }
}

pub(super) struct Registry {
    entries: Mutex<FixedVector<'static, DescriptorOwner>>,
    descriptors: SlotPool<'static, Descriptor>,
    series: SlotPool<'static, Series>,
    drafts: SlotPool<'static, ()>,
}

impl Registry {
    pub(super) fn prepare(
        memory: &PreparedMemory<'static>,
        region: RegionId,
        policy: &MetricPolicy,
        capacity: PreparedOtelMetricCapacity,
    ) -> Result<Self, MemoryError> {
        capacity.storage_bytes(policy)?;
        let descriptors = policy.descriptors.get();
        Ok(Self {
            entries: Mutex::new(FixedVector::prepare(
                memory.allocator(region)?,
                descriptors,
            )?),
            descriptors: memory.slots(region, descriptors, |_| {
                Ok(Descriptor {
                    name: [0; super::NATIVE_NAME_MAX_BYTES],
                    name_len: 0,
                    unit: [0; super::NATIVE_UNIT_MAX_BYTES],
                    unit_len: 0,
                    portable: None,
                    definition_kind: NativeInstrumentKind::CounterU64,
                    latency_unit: None,
                    native: None,
                    series: FixedVector::prepare(
                        memory.allocator(region)?,
                        policy.series_per_descriptor.get(),
                    )?,
                })
            })?,
            series: memory.slots(region, capacity.series_slots(policy)?, |_| {
                Series::prepare(memory, region, capacity)
            })?,
            drafts: memory.slots(region, capacity.drafts.get(), |_| Ok(()))?,
        })
    }

    pub(super) fn draft(&self) -> Result<SeriesDraft, OtelMetricError> {
        let admission = self.drafts.try_checkout().map_err(capacity_error)?;
        let mut slot = self.series.try_checkout().map_err(capacity_error)?;
        slot.text.clear();
        slot.labels.clear();
        slot.attributes.clear();
        Ok(SeriesDraft {
            slot,
            _admission: admission,
        })
    }

    pub(super) fn admit(
        &self,
        name: &str,
        descriptor: Option<&MetricDescriptor>,
        definition: &MetricDefinition,
        draft: Option<SeriesDraft>,
        build: impl FnOnce() -> NativeInstrument,
    ) -> Result<(NativeInstrument, Option<SeriesOwner>), OtelMetricError> {
        let portable = descriptor.map(|descriptor| (descriptor.kind(), descriptor.domain()));
        let mut entries = self
            .entries
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(entry) = entries
            .as_mut_slice()
            .iter_mut()
            .find(|entry| entry.name() == name.as_bytes())
        {
            if !entry.matches(portable, definition) {
                return Err(OtelMetricError::ConflictingDefinition);
            }
            let native = entry
                .native
                .clone()
                .ok_or(OtelMetricError::CapacityReached)?;
            let observation = draft.map(|draft| entry.admit_series(draft)).transpose()?;
            return Ok((native, observation));
        }

        let mut entry = self.descriptors.try_checkout().map_err(capacity_error)?;
        entry.set_identity(name, portable, definition)?;
        entries
            .push(entry)
            .map_err(|_| OtelMetricError::CapacityReached)?;
        drop(entries);

        // Keep the concrete descriptor reservation visible without calling the SDK
        // under the registry gate. Panic or refusal retires the pending generation.
        let rollback = std::mem::DropGuard::new((), |()| self.remove_pending(name));
        let native = build();
        let observation = draft.map(SeriesDraft::publish).transpose()?;
        let mut entries = self
            .entries
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let entry = entries
            .as_mut_slice()
            .iter_mut()
            .find(|entry| entry.name() == name.as_bytes())
            .ok_or(OtelMetricError::CapacityReached)?;
        let observation = if let Some((resident, observation)) = observation {
            entry
                .series
                .push(resident)
                .map_err(|_| OtelMetricError::CapacityReached)?;
            Some(observation)
        } else {
            None
        };
        entry.native = Some(native.clone());
        std::mem::DropGuard::dismiss(rollback);
        Ok((native, observation))
    }

    fn remove_pending(&self, name: &str) {
        let mut entries = self
            .entries
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let retired = entries
            .as_slice()
            .iter()
            .position(|entry| entry.name() == name.as_bytes())
            .and_then(|index| {
                entries.as_mut_slice().split_at_mut(index).1.rotate_left(1);
                entries.pop()
            });
        drop(entries);
        drop(retired);
    }
}

pub(super) const fn capacity_error(_: MemoryError) -> OtelMetricError {
    OtelMetricError::CapacityReached
}
