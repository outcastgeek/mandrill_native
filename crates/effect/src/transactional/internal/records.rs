use crate::memory::{
    Bytes, FixedVector, MemoryError, PreparedMemory, RegionId, SlotLease, SlotPool,
};
use std::alloc::Layout;
use std::collections::BTreeMap;
use std::num::NonZeroU64;

type Table<V> = FixedVector<'static, (NonZeroU64, V)>;

pub(super) struct RecordPool<V: Send + 'static> {
    slots: SlotPool<'static, Table<V>>,
}

impl<V: Send + 'static> RecordPool<V> {
    pub(super) fn storage_bytes(slots: usize, records: usize) -> Result<Bytes, MemoryError> {
        let layout =
            Layout::array::<(NonZeroU64, V)>(records).map_err(|_| MemoryError::InvalidLayout)?;
        SlotPool::<Table<V>>::storage_bytes(slots)?
            .checked_add(Bytes::new(layout.size()).checked_mul(slots)?)
    }

    pub(super) fn prepare(
        memory: &PreparedMemory<'static>,
        region: RegionId,
        slots: usize,
        records: usize,
    ) -> Result<Self, MemoryError> {
        let slots = memory.slots(region, slots, |_| {
            FixedVector::prepare(memory.allocator(region)?, records)
        })?;
        Ok(Self { slots })
    }

    pub(super) fn reserve(&self) -> Result<Records<V>, MemoryError> {
        self.slots
            .try_checkout()
            .map(|lease| Records::Prepared(Reserved { lease }))
    }
}

pub(super) struct Reserved<V: Send + 'static> {
    lease: SlotLease<'static, Table<V>>,
}

impl<V: Send + 'static> Drop for Reserved<V> {
    fn drop(&mut self) {
        self.lease.clear();
    }
}

pub(super) enum Records<V: Send + 'static> {
    Ordinary(BTreeMap<NonZeroU64, V>),
    Prepared(Reserved<V>),
}

impl<V: Send + 'static> Default for Records<V> {
    fn default() -> Self {
        Self::Ordinary(BTreeMap::new())
    }
}

impl<V: Send + 'static> Records<V> {
    pub(super) fn len(&self) -> usize {
        match self {
            Self::Ordinary(values) => values.len(),
            Self::Prepared(values) => values.lease.len(),
        }
    }

    pub(super) fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub(super) fn clear(&mut self) {
        match self {
            Self::Ordinary(values) => values.clear(),
            Self::Prepared(values) => values.lease.clear(),
        }
    }

    pub(super) fn get(&self, id: &NonZeroU64) -> Option<&V> {
        match self {
            Self::Ordinary(values) => values.get(id),
            Self::Prepared(values) => {
                let (index, _) = position(values.lease.as_slice(), id);
                index
                    .and_then(|index| values.lease.as_slice().get(index))
                    .map(|(_, value)| value)
            }
        }
    }

    pub(super) fn get_mut(&mut self, id: &NonZeroU64) -> Option<&mut V> {
        match self {
            Self::Ordinary(values) => values.get_mut(id),
            Self::Prepared(values) => {
                let (index, _) = position(values.lease.as_slice(), id);
                index
                    .and_then(|index| values.lease.as_mut_slice().get_mut(index))
                    .map(|(_, value)| value)
            }
        }
    }

    pub(super) fn reserve_entries(&self, count: usize) -> Result<(), MemoryError> {
        if let Self::Prepared(values) = self
            && count > values.lease.capacity() - values.lease.len()
        {
            return Err(MemoryError::CollectionReservationFailed);
        }
        Ok(())
    }

    // Callers checked one free destination before publishing this record.
    pub(super) fn insert_reserved(&mut self, id: NonZeroU64, value: V) {
        match self {
            Self::Ordinary(values) => {
                values.insert(id, value);
            }
            Self::Prepared(values) => {
                let (_, index) = position(values.lease.as_slice(), &id);
                values.lease.push_reserved((id, value));
                if let Some(suffix) = values.lease.as_mut_slice().get_mut(index..) {
                    suffix.rotate_right(1);
                }
            }
        }
    }

    pub(super) fn remove(&mut self, id: &NonZeroU64) -> Option<V> {
        match self {
            Self::Ordinary(values) => values.remove(id),
            Self::Prepared(values) => {
                let (index, _) = position(values.lease.as_slice(), id);
                index
                    .and_then(|index| values.lease.remove_at(index))
                    .map(|(_, value)| value)
            }
        }
    }

    pub(super) fn iter(&self) -> impl Iterator<Item = (&NonZeroU64, &V)> {
        let (ordinary, prepared) = match self {
            Self::Ordinary(values) => (Some(values.iter()), None),
            Self::Prepared(values) => (None, Some(values.lease.as_slice().iter())),
        };
        ordinary.into_iter().flatten().chain(
            prepared
                .into_iter()
                .flatten()
                .map(|(id, value)| (id, value)),
        )
    }

    pub(super) fn iter_mut(&mut self) -> impl Iterator<Item = (&NonZeroU64, &mut V)> {
        let (ordinary, prepared) = match self {
            Self::Ordinary(values) => (Some(values.iter_mut()), None),
            Self::Prepared(values) => (None, Some(values.lease.as_mut_slice().iter_mut())),
        };
        ordinary.into_iter().flatten().chain(
            prepared
                .into_iter()
                .flatten()
                .map(|(id, value)| (&*id, value)),
        )
    }
}

fn position<V>(values: &[(NonZeroU64, V)], id: &NonZeroU64) -> (Option<usize>, usize) {
    match values.binary_search_by_key(id, |(id, _)| *id) {
        Ok(index) => (Some(index), index),
        Err(index) => (None, index),
    }
}
