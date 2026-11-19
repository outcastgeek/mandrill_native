//! Bounded consumers over the selected native pull contract.
use crate::memory::{Bytes, MemoryError, PreparedMemory, RegionId, SlotLease, SlotPool};
use crate::{
    Exit, PreparedChunk, PreparedChunkPool, PreparedChunkValues, PreparedStream, PreparedStreamPull,
};

/// Startup storage for the actual mutable accumulator of each admitted fold.
pub struct PreparedFoldPool<S: Send + 'static> {
    slots: SlotPool<'static, Option<S>>,
}

impl<S: Send + 'static> PreparedFoldPool<S> {
    /// Regional payload and slot bookkeeping. Captured pointees are separate.
    pub fn storage_bytes(folds: usize) -> Result<Bytes, MemoryError> {
        SlotPool::<Option<S>>::storage_bytes(folds)
    }

    /// Prepare empty accumulator slots before running a fold.
    pub fn prepare(
        memory: &PreparedMemory<'static>,
        region: RegionId,
        folds: usize,
    ) -> Result<Self, MemoryError> {
        Ok(Self {
            slots: memory.slots(region, folds, |_| Ok(None))?,
        })
    }
}

/// The completed accumulator and the regional slot that backs it.
pub struct PreparedFoldValue<S: Send + 'static> {
    state: SlotLease<'static, Option<S>>,
}

impl<S: Send + 'static> PreparedFoldValue<S> {
    /// Borrow the accumulator while retaining its actual fold slot.
    #[must_use]
    #[expect(clippy::expect_used, reason = "a published fold value always has an initialized accumulator")]
    pub fn value(&self) -> &S {
        self.state.as_ref().expect("prepared fold value is initialized")
    }
}

impl<S: Send + 'static> Drop for PreparedFoldValue<S> {
    fn drop(&mut self) {
        // Clear before releasing the actual slot, even if S::drop unwinds.
        drop(self.state.take());
    }
}

impl<P: PreparedStreamPull> PreparedStream<P> {
    /// Retain an output lease before the first upstream pull. Each batch is
    /// checked in full before transferring items into that output. A completed
    /// collection holds the slot until dropped.
    pub async fn collect(
        &mut self,
        memory: &PreparedChunkPool<P::Item>,
        on_capacity: impl FnOnce(MemoryError) -> P::Error + Send,
    ) -> Exit<PreparedChunkValues<P::Item>, P::Error> {
        let mut output = match memory.try_checkout() {
            Ok(output) => output,
            Err(error) => return Exit::Fail(on_capacity(error)),
        };
        loop {
            let chunk = match self.pull().await.into_failure() {
                Ok(chunk) => chunk,
                Err(failure) => return failure,
            };
            let PreparedChunk::Values(mut values) = chunk else {
                return Exit::Success(output);
            };
            let requested = values.len();
            let available = output.capacity() - output.len();
            if requested > available {
                drop(values);
                drop(output);
                return Exit::Fail(on_capacity(MemoryError::CollectionReservationFailed));
            }
            for value in values.take_items() {
                output.push_reserved(value);
            }
        }
    }

    /// Admit the actual accumulator before calling its initializer or pulling
    /// the source. A synchronous step mutates that same slot for each item in
    /// order. The enclosing run selects the cancellation and panic policy.
    pub async fn fold<S: Send + 'static>(
        &mut self,
        memory: &PreparedFoldPool<S>,
        initial: impl FnOnce() -> S + Send,
        mut step: impl FnMut(&mut S, P::Item) -> Result<(), P::Error> + Send,
        on_capacity: impl FnOnce(MemoryError) -> P::Error + Send,
    ) -> Exit<PreparedFoldValue<S>, P::Error> {
        let state = match memory.slots.try_checkout() {
            Ok(state) => state,
            Err(error) => return Exit::Fail(on_capacity(error)),
        };
        let mut output = PreparedFoldValue { state };
        let accumulator = output.state.insert(initial());
        loop {
            let chunk = match self.pull().await.into_failure() {
                Ok(chunk) => chunk,
                Err(failure) => return failure,
            };
            let PreparedChunk::Values(mut values) = chunk else {
                return Exit::Success(output);
            };
            for value in values.take_items() {
                if let Err(error) = step(accumulator, value) {
                    return Exit::Fail(error);
                }
            }
        }
    }
}
