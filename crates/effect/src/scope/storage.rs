//! Shared collection contract for ordinary and prepared scope finalizers.
//! Prepared suffix transfer consumes an already-checked-out destination batch;
//! it never split_offs, grows or returns its physical batch before settlement.
use super::{Inner, Registered};
use crate::memory::{FixedVector, SharedSlot, SlotLease};
use std::ops::{Deref, DerefMut};

pub(super) struct SegmentLease<E: Send + 'static> {
    pub(super) inner: SharedSlot<'static, Inner<E>>,
}
impl<E: Send + 'static> Drop for SegmentLease<E> {
    fn drop(&mut self) {
        // One non-Clone token per pre-body segment charge. Its inner reference
        // prevents scope-row reuse until this charge and physical batch return.
        let mut state = self
            .inner
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.segments_in_use -= 1;
    }
}

pub(crate) struct PreparedBatch<E: Send + 'static> {
    // Field order keeps the scope alive through clearing and physical batch return.
    values: SlotLease<'static, FixedVector<'static, Registered<E>>>,
    scope: Option<SegmentLease<E>>,
}

pub(crate) struct SystemRegistration<E: Send + 'static> {
    sequence: usize,
    finalizer: Registered<E>,
}
impl<E: Send + 'static> Deref for PreparedBatch<E> {
    type Target = FixedVector<'static, Registered<E>>;
    fn deref(&self) -> &Self::Target {
        &self.values
    }
}
impl<E: Send + 'static> DerefMut for PreparedBatch<E> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.values
    }
}

#[derive(Default)]
pub(crate) enum FinalizerBatch<E: Send + 'static> {
    #[default]
    Empty,
    System {
        live: Vec<SystemRegistration<E>>,
        next_sequence: usize,
    },
    Prepared(PreparedBatch<E>),
}
impl<E: Send + 'static> FinalizerBatch<E> {
    pub(super) fn system() -> Self {
        Self::System {
            live: Vec::new(),
            next_sequence: 0,
        }
    }

    pub(super) fn system_checkpoint(&self) -> Option<usize> {
        match self {
            Self::System { next_sequence, .. } => Some(*next_sequence),
            Self::Empty | Self::Prepared(_) => None,
        }
    }

    pub(super) fn remove_tag(&mut self, tag: usize) -> Option<Registered<E>> {
        match self {
            Self::System { live, .. } => live
                .iter()
                .position(|item| item.finalizer.retirement_tag == Some(tag))
                .map(|index| live.remove(index).finalizer),
            Self::Empty | Self::Prepared(_) => None,
        }
    }
    pub(super) fn prepared(
        values: SlotLease<'static, FixedVector<'static, Registered<E>>>,
    ) -> Self {
        Self::Prepared(PreparedBatch {
            values,
            scope: None,
        })
    }
    pub(super) fn retain_scope(&mut self, scope: SegmentLease<E>) {
        if let Self::Prepared(batch) = self {
            batch.scope = Some(scope);
        }
    }
    pub(crate) fn len(&self) -> usize {
        match self {
            Self::Empty => 0,
            Self::System { live, .. } => live.len(),
            Self::Prepared(values) => values.len(),
        }
    }
    pub(crate) fn is_empty(&self) -> bool {
        self.len() == 0
    }
    /// None is the existing growable System lane; prepared/empty lanes expose
    /// real vacancies so acquisition promises cannot be stolen by registration.
    pub(super) fn available(&self) -> Option<usize> {
        match self {
            Self::System { .. } => None,
            Self::Empty => Some(0),
            Self::Prepared(values) => Some(values.capacity() - values.len()),
        }
    }
    pub(super) fn capacity(&self) -> usize {
        match self {
            Self::Empty => 0,
            Self::System { live, .. } => live.capacity(),
            Self::Prepared(values) => values.capacity(),
        }
    }
    pub(crate) fn pop(&mut self) -> Option<Registered<E>> {
        match self {
            Self::Empty => None,
            Self::System { live, .. } => live.pop().map(|entry| entry.finalizer),
            Self::Prepared(values) => values.pop(),
        }
    }
    pub(super) fn push(&mut self, value: Registered<E>) -> Result<(), Registered<E>> {
        match self {
            Self::Empty => Err(value),
            Self::System {
                live,
                next_sequence,
            } => {
                let Some(next) = next_sequence.checked_add(1) else {
                    return Err(value);
                };
                live.push(SystemRegistration {
                    sequence: *next_sequence,
                    finalizer: value,
                });
                *next_sequence = next;
                Ok(())
            }
            Self::Prepared(values) => values.push(value),
        }
    }
    /// Ordinary path only. Prepared callers must hold a destination acquired
    /// before entering the relevant body; no fallback to this allocating lane.
    pub(super) fn system_suffix(&mut self, checkpoint: usize) -> Option<Self> {
        match self {
            Self::System {
                live,
                next_sequence,
            } if checkpoint <= *next_sequence => {
                let first = live.partition_point(|entry| entry.sequence < checkpoint);
                Some(Self::System {
                    live: live.split_off(first),
                    next_sequence: *next_sequence,
                })
            }
            Self::System { .. } | Self::Empty | Self::Prepared(_) => None,
        }
    }
    pub(super) fn reserved_suffix(
        &mut self,
        checkpoint: usize,
        mut destination: Self,
    ) -> Result<Self, Self> {
        if !matches!(
            (&*self, &destination),
            (Self::Prepared(_), Self::Prepared(_))
        ) {
            return Err(destination);
        }
        let Some(count) = self.len().checked_sub(checkpoint) else {
            return Err(destination);
        };
        if !destination.is_empty() || destination.capacity() < count {
            return Err(destination);
        }
        while self.len() > checkpoint {
            if let Some(value) = self.pop() {
                // Capacity was checked for the complete transfer above. This
                // helper is private; no user callbacks or concurrent mutation can
                // change the two vectors while their scope mutex is held.
                match &mut destination {
                    Self::Prepared(values) => values.push_reserved(value),
                    Self::System { .. } | Self::Empty => return Err(destination),
                }
            }
        }
        // Pops reverse the tail. Restore original insertion order so the common
        // drain's subsequent pops still run finalizers in reverse registration.
        match &mut destination {
            Self::Prepared(values) => values.as_mut_slice().reverse(),
            Self::System { .. } | Self::Empty => {}
        }
        Ok(destination)
    }
}
impl<E: Send + 'static> Drop for FinalizerBatch<E> {
    fn drop(&mut self) {
        if let Self::Prepared(values) = self {
            // Abandonment invalidates resources synchronously before the batch
            // returns. User captures/destructors run outside the scope gate.
            values.clear();
        }
    }
}
