//! Ordinary and startup-provisioned tables shared by cache and pool operations.
use crate::memory::{FixedVector, MemoryError, SlotLease, SlotPool};
use std::collections::VecDeque;

pub(crate) struct PreparedIndexLoan<T: Send + 'static>(SlotLease<'static, FixedVector<'static, T>>);
impl<T: Send + 'static> Drop for PreparedIndexLoan<T> {
    fn drop(&mut self) {
        self.0.clear();
    }
}

pub(crate) enum WorkList<T: Send + 'static> {
    Ordinary(VecDeque<T>),
    Prepared(PreparedIndexLoan<T>),
}
impl<T: Send + 'static> Default for WorkList<T> {
    fn default() -> Self {
        Self::Ordinary(VecDeque::new())
    }
}
impl<T: Send + 'static> WorkList<T> {
    pub(crate) fn checkout(
        slots: &SlotPool<'static, FixedVector<'static, T>>,
    ) -> Result<Self, MemoryError> {
        Ok(Self::Prepared(PreparedIndexLoan(slots.try_checkout()?)))
    }

    pub(crate) fn len(&self) -> usize {
        match self {
            Self::Ordinary(entries) => entries.len(),
            Self::Prepared(entries) => entries.0.len(),
        }
    }

    pub(crate) fn iter(&self) -> impl Iterator<Item = &T> {
        let (first, second) = match self {
            Self::Ordinary(entries) => entries.as_slices(),
            Self::Prepared(entries) => (entries.0.as_slice(), &[][..]),
        };
        first.iter().chain(second)
    }

    pub(crate) fn drain(&mut self) -> impl Iterator<Item = T> + '_ {
        let (ordinary, prepared) = match self {
            Self::Ordinary(entries) => (Some(entries.drain(..)), None),
            Self::Prepared(entries) => (None, Some(entries.0.drain())),
        };
        ordinary
            .into_iter()
            .flatten()
            .chain(prepared.into_iter().flatten())
    }
    pub(crate) fn iter_mut(&mut self) -> impl Iterator<Item = &mut T> {
        let (first, second) = match self {
            Self::Ordinary(entries) => entries.as_mut_slices(),
            Self::Prepared(entries) => (entries.0.as_mut_slice(), &mut [][..]),
        };
        first.iter_mut().chain(second)
    }

    pub(crate) fn get(&self, index: usize) -> Option<&T> {
        match self {
            Self::Ordinary(entries) => entries.get(index),
            Self::Prepared(entries) => entries.0.as_slice().get(index),
        }
    }

    pub(crate) fn remove(&mut self, index: usize) -> Option<T> {
        match self {
            Self::Ordinary(entries) => entries.remove(index),
            Self::Prepared(entries) => entries.0.remove_at(index),
        }
    }

    pub(crate) fn retain(&mut self, keep: impl FnMut(&T) -> bool) {
        match self {
            Self::Ordinary(entries) => entries.retain(keep),
            Self::Prepared(entries) => entries.0.retain(keep),
        }
    }

    pub(crate) fn remaining(&self) -> usize {
        match self {
            Self::Ordinary(_) => usize::MAX,
            Self::Prepared(entries) => entries.0.capacity() - entries.0.len(),
        }
    }

    pub(crate) fn push_reserved(&mut self, entry: T) {
        match self {
            Self::Ordinary(entries) => entries.push_back(entry),
            Self::Prepared(entries) => entries.0.push_reserved(entry),
        }
    }

    pub(crate) fn pop_front(&mut self) -> Option<T> {
        self.remove(0)
    }
}
