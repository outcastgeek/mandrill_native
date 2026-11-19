use crate::internal::runtime::prepared_graph::FutureContext;
use crate::memory::{
    Bytes, FixedVector, MemoryError, PreparedMemory, RegionId, SlotLease, SlotPool,
};
use crate::{Exit, PrimaryOutcomeRef};
use futures_util::future::{Either, select};
use std::alloc::Layout;
use std::future::Future;
use std::num::NonZeroUsize;
use std::pin::pin;

/// Reusable storage for a finite number of independently retained stream batches.
/// Slots hold inline items. A byte batch uses `PreparedChunkPool<u8>`; items
/// with their own backing retain that backing through their own owners.
pub struct PreparedChunkPool<A: Send + 'static> {
    slots: SlotPool<'static, FixedVector<'static, A>>,
}

impl<A: Send + 'static> PreparedChunkPool<A> {
    /// Slot controls and actual element storage, excluding region allocator
    /// headers and alignment overhead.
    pub fn storage_bytes(batches: NonZeroUsize, items: NonZeroUsize) -> Result<Bytes, MemoryError> {
        let layout = Layout::array::<A>(items.get()).map_err(|_| MemoryError::InvalidLayout)?;
        SlotPool::<FixedVector<'static, A>>::storage_bytes(batches.get())?
            .checked_add(Bytes::new(layout.size()).checked_mul(batches.get())?)
    }

    /// Prepare every reusable batch in the supplied region before sealing.
    pub fn prepare(
        memory: &PreparedMemory<'static>,
        region: RegionId,
        batches: NonZeroUsize,
        items: NonZeroUsize,
    ) -> Result<Self, MemoryError> {
        Self::storage_bytes(batches, items)?;
        let slots = memory.slots(region, batches.get(), |_| {
            FixedVector::prepare(memory.allocator(region)?, items.get())
        })?;
        Ok(Self { slots })
    }

    /// Retain the same storage without creating new batches.
    pub fn try_clone(&self) -> Result<Self, MemoryError> {
        Ok(Self {
            slots: self.slots.try_clone()?,
        })
    }

    /// Reserve an empty batch. Refusal leaves the upstream source untouched.
    pub fn try_checkout(&self) -> Result<PreparedChunkValues<A>, MemoryError> {
        self.slots
            .try_checkout()
            .map(|values| PreparedChunkValues { values })
    }
}

/// An affine batch owner. The slot and its contents survive the stream and
/// return to the pool when the downstream owner drops this batch.
pub struct PreparedChunkValues<A: Send + 'static> {
    values: SlotLease<'static, FixedVector<'static, A>>,
}

impl<A: Send + 'static> PreparedChunkValues<A> {
    #[must_use]
    pub fn capacity(&self) -> usize {
        self.values.capacity()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.values.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    #[must_use]
    pub fn as_slice(&self) -> &[A] {
        self.values.as_slice()
    }

    pub(crate) fn as_mut_slice(&mut self) -> &mut [A] {
        self.values.as_mut_slice()
    }

    pub(crate) fn remove_at(&mut self, index: usize) -> Option<A> {
        self.values.remove_at(index)
    }

    /// Append one value, returning ownership of it if the batch is full.
    pub fn try_push(&mut self, value: A) -> Result<(), A> {
        self.values.push(value)
    }

    /// The caller has checked free space for the whole transfer.
    pub(crate) fn push_reserved(&mut self, value: A) {
        self.values.push_reserved(value);
    }

    /// Move the first item out while preserving the order of remaining items.
    pub fn pop_front(&mut self) -> Option<A> {
        self.values.remove_first()
    }

    /// Move all items in order. Dropping the iterator drops unread items. The
    /// empty backing remains checked out until the batch owner is dropped.
    pub fn take_items(&mut self) -> impl Iterator<Item = A> + '_ {
        let count = self.values.len();
        self.values.drain_prefix(count)
    }
}

impl<A: Send + 'static> Drop for PreparedChunkValues<A> {
    fn drop(&mut self) {
        self.values.clear();
    }
}

/// A prepared batch or a successful end of input.
pub enum PreparedChunk<A: Send + 'static> {
    Values(PreparedChunkValues<A>),
    End,
}

/// A serial, typed pull interface for admitted native futures. The mutable
/// borrow permits one outstanding pull. Concrete pull futures can compose
/// inside the caller's existing prepared future frame without boxing.
pub trait PreparedStreamPull: Send {
    type Item: Send + 'static;
    type Error: Send + 'static;

    fn pull(&mut self)
    -> impl Future<Output = Exit<PreparedChunk<Self::Item>, Self::Error>> + Send;
}

/// A prepared pull pipeline with its source and pending batch state owned by
/// the enclosing invocation. Dropping it releases any upstream batch it holds.
pub struct PreparedStream<P> {
    source: P,
}

impl<P> PreparedStream<P> {
    #[must_use]
    pub const fn new(source: P) -> Self {
        Self { source }
    }
}

impl PreparedStream<()> {
    /// Pull batches from an owned iterator. Checkout precedes the first call
    /// to `next` for each batch, so a refused pull can be retried unchanged.
    #[must_use]
    pub fn from_iterable<I, E, M>(
        iterator: I,
        memory: PreparedChunkPool<I::Item>,
        on_capacity: M,
    ) -> PreparedStream<impl PreparedStreamPull<Item = I::Item, Error = E>>
    where
        I: Iterator + Send,
        I::Item: Send + 'static,
        E: Send + 'static,
        M: Fn(MemoryError) -> E + Send,
    {
        PreparedStream::new(PreparedIterator {
            iterator,
            memory,
            on_capacity,
            ended: false,
        })
    }
}

impl<P: PreparedStreamPull> PreparedStream<P> {
    pub async fn pull(&mut self) -> Exit<PreparedChunk<P::Item>, P::Error> {
        self.source.pull().await
    }

    /// Use the enclosing prepared native operation's existing cancellation
    /// listener for every pull. Apply to the outermost stage to interrupt
    /// pending asynchronous transformations as well as source reads.
    /// Interruption drops the pending pull future. A native source that needs
    /// asynchronous settlement must keep that work owned by its scope or
    /// provider and settle it cooperatively.
    #[must_use]
    pub fn with_context(
        self,
        context: FutureContext<P::Error>,
    ) -> PreparedStream<impl PreparedStreamPull<Item = P::Item, Error = P::Error>> {
        crate::stream! {
            pull_prepared;
            pull: ContextualPull { active: Some((self.source, context)) },
        }
    }

    /// Transform a batch in source order. The output slot is acquired before
    /// requesting input. Input remains admitted through each asynchronous
    /// transformation and the output is published only when all items succeed.
    /// A batch larger than the destination remains staged, with no callbacks
    /// run. Use `rechunk` first when the source may produce larger batches.
    #[must_use]
    pub fn map<B, F, Fut, M>(
        self,
        memory: PreparedChunkPool<B>,
        transform: F,
        on_capacity: M,
    ) -> PreparedStream<impl PreparedStreamPull<Item = B, Error = P::Error>>
    where
        B: Send + 'static,
        F: FnMut(P::Item) -> Fut + Send,
        Fut: Future<Output = Exit<B, P::Error>> + Send,
        M: Fn(MemoryError) -> P::Error + Send,
    {
        PreparedStream::new(PreparedMap {
            source: self.source,
            memory,
            transform,
            on_capacity,
            pending: None,
            ended: false,
        })
    }

    /// Split upstream batches into bounded downstream batches. While the
    /// remainder is held, this stage does not pull another upstream batch.
    /// This is a demand-driven stage and does not start a background producer.
    #[must_use]
    pub fn rechunk<M>(
        self,
        memory: PreparedChunkPool<P::Item>,
        on_capacity: M,
    ) -> PreparedStream<impl PreparedStreamPull<Item = P::Item, Error = P::Error>>
    where
        M: Fn(MemoryError) -> P::Error + Send,
    {
        PreparedStream::new(Rechunk {
            source: self.source,
            memory,
            on_capacity,
            pending: None,
            ended: false,
        })
    }
}

struct ContextualPull<P: PreparedStreamPull> {
    active: Option<(P, FutureContext<P::Error>)>,
}

impl<P: PreparedStreamPull> PreparedStreamPull for ContextualPull<P> {
    type Item = P::Item;
    type Error = P::Error;

    async fn pull(&mut self) -> Exit<PreparedChunk<P::Item>, P::Error> {
        let Some((source, context)) = &mut self.active else {
            return Exit::Interrupted;
        };
        let outcome = {
            let cancelled = pin!(context.cancelled());
            let pulled = pin!(async { source.pull().await });
            match select(cancelled, pulled).await {
                Either::Left(_) => Exit::Interrupted,
                Either::Right((outcome, _)) => outcome,
            }
        };
        // Preserve a source failure even if cancellation raced with the pull.
        let outcome = if matches!(&outcome, Exit::Success(_)) && context.is_requested() {
            Exit::Interrupted
        } else {
            outcome
        };
        if matches!(outcome.primary(), PrimaryOutcomeRef::WasInterrupted) {
            self.active = None;
        }
        outcome
    }
}

struct PreparedIterator<I: Iterator, M>
where
    I::Item: Send + 'static,
{
    iterator: I,
    memory: PreparedChunkPool<I::Item>,
    on_capacity: M,
    ended: bool,
}

impl<I, M, E> PreparedStreamPull for PreparedIterator<I, M>
where
    I: Iterator + Send,
    I::Item: Send + 'static,
    M: Fn(MemoryError) -> E + Send,
    E: Send + 'static,
{
    type Item = I::Item;
    type Error = E;

    async fn pull(&mut self) -> Exit<PreparedChunk<Self::Item>, E> {
        if self.ended {
            return Exit::Success(PreparedChunk::End);
        }
        let mut output = match self.memory.try_checkout() {
            Ok(output) => output,
            Err(error) => return Exit::Fail((self.on_capacity)(error)),
        };
        for _ in 0..output.capacity() {
            let Some(value) = self.iterator.next() else {
                self.ended = true;
                break;
            };
            output.push_reserved(value);
        }
        if output.is_empty() {
            Exit::Success(PreparedChunk::End)
        } else {
            Exit::Success(PreparedChunk::Values(output))
        }
    }
}

struct PreparedMap<P: PreparedStreamPull, B: Send + 'static, F, M> {
    source: P,
    memory: PreparedChunkPool<B>,
    transform: F,
    on_capacity: M,
    pending: Option<PreparedChunkValues<P::Item>>,
    ended: bool,
}

impl<P, B, F, Fut, M> PreparedStreamPull for PreparedMap<P, B, F, M>
where
    P: PreparedStreamPull,
    B: Send + 'static,
    F: FnMut(P::Item) -> Fut + Send,
    Fut: Future<Output = Exit<B, P::Error>> + Send,
    M: Fn(MemoryError) -> P::Error + Send,
{
    type Item = B;
    type Error = P::Error;

    async fn pull(&mut self) -> Exit<PreparedChunk<B>, P::Error> {
        if self.ended {
            return Exit::Success(PreparedChunk::End);
        }
        let mut output = match self.memory.try_checkout() {
            Ok(output) => output,
            Err(error) => return Exit::Fail((self.on_capacity)(error)),
        };
        let mut input = match self.pending.take() {
            Some(input) => input,
            None => match self.source.pull().await.into_failure() {
                Ok(PreparedChunk::Values(input)) => input,
                Ok(PreparedChunk::End) => {
                    self.ended = true;
                    return Exit::Success(PreparedChunk::End);
                }
                Err(failure) => return failure,
            },
        };
        if input.len() > output.capacity() {
            self.pending = Some(input);
            drop(output);
            return Exit::Fail((self.on_capacity)(MemoryError::CollectionReservationFailed));
        }
        for value in input.take_items() {
            match (self.transform)(value).await.into_failure() {
                Ok(mapped) => output.push_reserved(mapped),
                Err(failure) => return failure,
            }
        }
        Exit::Success(PreparedChunk::Values(output))
    }
}

struct Rechunk<P: PreparedStreamPull, M> {
    source: P,
    memory: PreparedChunkPool<P::Item>,
    on_capacity: M,
    pending: Option<PreparedChunkValues<P::Item>>,
    ended: bool,
}

impl<P, M> PreparedStreamPull for Rechunk<P, M>
where
    P: PreparedStreamPull,
    M: Fn(MemoryError) -> P::Error + Send,
{
    type Item = P::Item;
    type Error = P::Error;

    async fn pull(&mut self) -> Exit<PreparedChunk<P::Item>, P::Error> {
        if self.ended {
            return Exit::Success(PreparedChunk::End);
        }
        let mut output = match self.memory.try_checkout() {
            Ok(output) => output,
            Err(error) => return Exit::Fail((self.on_capacity)(error)),
        };
        let mut input = match self.pending.take() {
            Some(input) => input,
            None => match self.source.pull().await.into_failure() {
                Ok(PreparedChunk::Values(input)) => input,
                Ok(PreparedChunk::End) => {
                    self.ended = true;
                    return Exit::Success(PreparedChunk::End);
                }
                Err(failure) => return failure,
            },
        };
        for value in input.values.drain_prefix(output.capacity()) {
            output.push_reserved(value);
        }
        if !input.is_empty() {
            self.pending = Some(input);
        }
        Exit::Success(PreparedChunk::Values(output))
    }
}
