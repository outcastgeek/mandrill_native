use super::SharedResult;
use crate::memory::{FixedVector, MemoryError, SlotLease, SlotPool};

type ResultVector<A, E> = FixedVector<'static, SharedResult<A, E>>;
pub(in crate::request_resolver) type ResultPool<A, E> = SlotPool<'static, ResultVector<A, E>>;

pub(in crate::request_resolver) trait ResultCollector<A: Send + 'static, E: Send + 'static>:
    Sized + Send + 'static
{
    fn reserve(pool: Option<&ResultPool<A, E>>, count: usize) -> Result<Self, MemoryError>;
    fn ready(&mut self, _count: usize) {}
    fn push(&mut self, result: SharedResult<A, E>);
    fn finish(self) -> Self {
        self
    }
}

impl<A: Send + 'static, E: Send + 'static> ResultCollector<A, E> for Vec<SharedResult<A, E>> {
    fn reserve(_: Option<&ResultPool<A, E>>, _: usize) -> Result<Self, MemoryError> {
        Ok(Vec::new())
    }

    fn ready(&mut self, count: usize) {
        self.reserve_exact(count);
    }

    fn push(&mut self, result: SharedResult<A, E>) {
        self.push(result);
    }
}

impl<A: Send + 'static, E: Send + 'static> ResultCollector<A, E> for Option<SharedResult<A, E>> {
    fn reserve(_: Option<&ResultPool<A, E>>, _: usize) -> Result<Self, MemoryError> {
        Ok(None)
    }

    fn push(&mut self, result: SharedResult<A, E>) {
        *self = Some(result);
    }
}

/// The owned ordered results of a prepared resolve-many call. Its backing slot
/// returns to the resolver when this iterator is dropped.
pub struct PreparedResolvedBatch<A: Send + 'static, E: Send + 'static> {
    slot: Option<SlotLease<'static, ResultVector<A, E>>>,
}

impl<A: Send + 'static, E: Send + 'static> ResultCollector<A, E> for PreparedResolvedBatch<A, E> {
    fn reserve(pool: Option<&ResultPool<A, E>>, count: usize) -> Result<Self, MemoryError> {
        let pool = pool.ok_or(MemoryError::MissingRuntimeStorage)?;
        if count == 0 {
            return Ok(Self { slot: None });
        }
        let slot = pool.try_checkout()?;
        Ok(Self { slot: Some(slot) })
    }

    fn push(&mut self, result: SharedResult<A, E>) {
        if let Some(slot) = &mut self.slot {
            slot.push_reserved(result);
        }
    }

    fn finish(mut self) -> Self {
        if let Some(slot) = &mut self.slot {
            slot.as_mut_slice().reverse();
        }
        self
    }
}

impl<A: Send + 'static, E: Send + 'static> Iterator for PreparedResolvedBatch<A, E> {
    type Item = SharedResult<A, E>;

    fn next(&mut self) -> Option<Self::Item> {
        self.slot.as_mut()?.pop()
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let count = self.slot.as_ref().map_or(0, |slot| slot.len());
        (count, Some(count))
    }
}

impl<A: Send + 'static, E: Send + 'static> ExactSizeIterator for PreparedResolvedBatch<A, E> {}
impl<A: Send + 'static, E: Send + 'static> std::iter::FusedIterator
    for PreparedResolvedBatch<A, E>
{
}

impl<A: Send + 'static, E: Send + 'static> Drop for PreparedResolvedBatch<A, E> {
    fn drop(&mut self) {
        if let Some(slot) = &mut self.slot {
            slot.clear();
        }
    }
}
