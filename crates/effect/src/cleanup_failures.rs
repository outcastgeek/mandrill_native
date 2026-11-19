//! Owned ordered cleanup failures. Ordinary clones allocate a new Global Vec;
//! cloning is not a prepared operation. Prepared buffers and metadata are charged
//! to the selected region before work. Payloads and their prepared descendants
//! retire before the frame footer returns the actual buffer to its pool.
use crate::memory::{
    FixedVector, FramePool, FrameReservation, MemoryError, PreparedMemory, RegionId,
    RetainedFrameValue, RuntimeFrame, SlotLease, SlotPool,
};
use crate::{CleanupFailure, Defect, DefectKind, LibraryDefect};
use std::{fmt, marker::PhantomData, ops::Deref, ptr::NonNull};

type Buffer<E> = FixedVector<'static, CleanupFailure<E>>;
type Record<E> = RetainedFrameValue<Clear<E>, SlotLease<'static, Buffer<E>>>;
struct Clear<E> {
    buffer: NonNull<Buffer<E>>,
}
// SAFETY: constructed only from the uniquely leased buffer of Send elements;
// the lease remains in the paired footer until payload descendants retire.
unsafe impl<E: Send> Send for Clear<E> {}
impl<E> Drop for Clear<E> {
    fn drop(&mut self) {
        // SAFETY: paired footer retains this unique initialized buffer through
        // payload and descendant destruction; no concurrent collection exists.
        unsafe {
            self.buffer.as_mut().clear();
        }
    }
}
struct Prepared<E> {
    // The typed pointer is valid only while this exact frame retains its footer.
    buffer: NonNull<Buffer<E>>,
    _frame: RuntimeFrame,
    marker: PhantomData<E>,
}
enum Storage<E> {
    Ordinary(Vec<CleanupFailure<E>>),
    Prepared(Prepared<E>),
}
/// Ordered cleanup entries. Ordinary `Clone` explicitly allocates an independent
/// Global Vec, and is not a prepared or allocation-free operation. Prepared
/// construction is fallible and uses physically reserved region-backed storage.
pub struct CleanupFailures<E> {
    storage: Storage<E>,
}
// SAFETY: a prepared variant is constructed only for Send + 'static E; it owns
// its sole typed buffer and its frame. Ordinary Vec has the same E bound.
unsafe impl<E: Send> Send for CleanupFailures<E> {}
// SAFETY: shared access only yields shared E references, never mutates buffers;
// final retirement requires exclusive ownership and prepared E was Send.
unsafe impl<E: Sync> Sync for CleanupFailures<E> {}
impl<E> CleanupFailures<E> {
    #[must_use]
    pub fn new() -> Self {
        Self {
            storage: Storage::Ordinary(Vec::new()),
        }
    }
    #[must_use]
    pub fn as_slice(&self) -> &[CleanupFailure<E>] {
        match &self.storage {
            Storage::Ordinary(values) => values,
            // SAFETY: the frame uniquely retains this buffer until self is dropped.
            Storage::Prepared(prepared) => unsafe { prepared.buffer.as_ref().as_slice() },
        }
    }
    #[must_use]
    pub fn len(&self) -> usize {
        self.as_slice().len()
    }
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.as_slice().is_empty()
    }
    pub fn iter(&self) -> std::slice::Iter<'_, CleanupFailure<E>> {
        self.as_slice().iter()
    }
    // Replace entries in their existing unique backing; the placeholder keeps
    // every slot initialized if a user mapper unwinds.
    pub(crate) fn map_same(&mut self, map: &impl Fn(E) -> E) {
        let values = match &mut self.storage {
            Storage::Ordinary(values) => values.as_mut_slice(),
            // SAFETY: this owner exclusively retains the prepared buffer.
            Storage::Prepared(prepared) => unsafe { prepared.buffer.as_mut().as_mut_slice() },
        };
        for entry in values {
            let old = std::mem::replace(entry, CleanupFailure::ReleaseInterrupted);
            *entry = match old {
                CleanupFailure::ReleaseFailure(error) => CleanupFailure::ReleaseFailure(map(error)),
                other @ (CleanupFailure::ReleaseDefect(_) | CleanupFailure::ReleaseInterrupted) => {
                    other
                }
            };
        }
    }
    /// Prepared capacity reserves its last slot for a typed overflow summary.
    /// Overflowed values are dropped outside the pool's metadata lock.
    pub fn push(&mut self, failure: CleanupFailure<E>) {
        match &mut self.storage {
            Storage::Ordinary(values) => values.push(failure),
            Storage::Prepared(prepared) => {
                // SAFETY: &mut self exclusively owns the reserved live buffer.
                let values = unsafe { prepared.buffer.as_mut() };
                let retained = values.capacity() - 1;
                let incoming = match &failure {
                    CleanupFailure::ReleaseDefect(defect) => match defect.kind() {
                        DefectKind::Library(LibraryDefect::CleanupOverflow {
                            omitted,
                            saturated,
                        }) => Some((omitted, saturated)),
                        DefectKind::Custom | DefectKind::CallbackPanic | DefectKind::Library(_) => {
                            None
                        }
                    },
                    CleanupFailure::ReleaseFailure(_) | CleanupFailure::ReleaseInterrupted => None,
                };
                let current = values.as_slice().last().and_then(|last| match last {
                    CleanupFailure::ReleaseDefect(defect) => match defect.kind() {
                        DefectKind::Library(LibraryDefect::CleanupOverflow {
                            omitted,
                            saturated,
                        }) => Some((omitted, saturated)),
                        DefectKind::Custom | DefectKind::CallbackPanic | DefectKind::Library(_) => {
                            None
                        }
                    },
                    CleanupFailure::ReleaseFailure(_) | CleanupFailure::ReleaseInterrupted => None,
                });
                let real_count = values.len() - usize::from(current.is_some());
                if incoming.is_none() && real_count < retained {
                    // The marker, if present, remains last; this exclusive buffer
                    // has a proved free slot and cannot allocate.
                    if current.is_some() {
                        values.insert_before_last_reserved(failure);
                    } else {
                        values.push_reserved(failure);
                    }
                    return;
                }
                let (increment, incoming_saturated) = incoming.unwrap_or((1, false));
                let (omitted, saturated) = match current {
                    Some((old, old_saturated)) => {
                        let (sum, overflow) = old.overflowing_add(increment);
                        (
                            if overflow { usize::MAX } else { sum },
                            old_saturated || incoming_saturated || overflow,
                        )
                    }
                    None => (increment, incoming_saturated),
                };
                let marker = CleanupFailure::ReleaseDefect(Defect::library(
                    LibraryDefect::CleanupOverflow { omitted, saturated },
                ));
                if current.is_some() {
                    if let Some(last) = values.as_mut_slice().last_mut() {
                        *last = marker;
                    } else {
                        // The previous inspection proves this branch unreachable;
                        // retain the marker even if that invariant is violated.
                        values.push_reserved(marker);
                    }
                } else {
                    // All real entries use at most retained slots; the separately
                    // reserved final marker slot is physically available.
                    values.push_reserved(marker);
                }
                drop(failure);
            }
        }
    }
}
impl<E> Default for CleanupFailures<E> {
    fn default() -> Self {
        Self::new()
    }
}
impl<E> From<Vec<CleanupFailure<E>>> for CleanupFailures<E> {
    fn from(values: Vec<CleanupFailure<E>>) -> Self {
        Self {
            storage: Storage::Ordinary(values),
        }
    }
}
impl<E> Deref for CleanupFailures<E> {
    type Target = [CleanupFailure<E>];
    fn deref(&self) -> &Self::Target {
        self.as_slice()
    }
}
impl<E> Extend<CleanupFailure<E>> for CleanupFailures<E> {
    fn extend<I: IntoIterator<Item = CleanupFailure<E>>>(&mut self, iter: I) {
        for value in iter {
            self.push(value);
        }
    }
}
impl<E> FromIterator<CleanupFailure<E>> for CleanupFailures<E> {
    fn from_iter<I: IntoIterator<Item = CleanupFailure<E>>>(iter: I) -> Self {
        Self::from(iter.into_iter().collect::<Vec<_>>())
    }
}
impl<E: Clone> Clone for CleanupFailures<E> {
    fn clone(&self) -> Self {
        Self::from(self.as_slice().to_vec())
    }
}
impl<E: fmt::Debug> fmt::Debug for CleanupFailures<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.as_slice().fmt(f)
    }
}
impl<E: PartialEq> PartialEq for CleanupFailures<E> {
    fn eq(&self, other: &Self) -> bool {
        self.as_slice() == other.as_slice()
    }
}
impl<E: Eq> Eq for CleanupFailures<E> {}
enum IteratorStorage<E> {
    Ordinary(std::vec::IntoIter<CleanupFailure<E>>),
    Prepared(Prepared<E>),
}
/// Owned ordered iterator. Prepared removal shifts a bounded number of entries.
pub struct CleanupFailuresIntoIter<E> {
    values: IteratorStorage<E>,
}
// SAFETY: ownership and read-only access have the same bounds as CleanupFailures.
unsafe impl<E: Send> Send for CleanupFailuresIntoIter<E> {}
// SAFETY: shared iterator access never mutates the retained buffer.
unsafe impl<E: Sync> Sync for CleanupFailuresIntoIter<E> {}
impl<E> Iterator for CleanupFailuresIntoIter<E> {
    type Item = CleanupFailure<E>;
    fn next(&mut self) -> Option<Self::Item> {
        match &mut self.values {
            IteratorStorage::Ordinary(values) => values.next(),
            // SAFETY: iterator exclusively owns the retained prepared collection.
            IteratorStorage::Prepared(prepared) => unsafe {
                prepared.buffer.as_mut().remove_first()
            },
        }
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        let len = match &self.values {
            IteratorStorage::Ordinary(values) => values.len(),
            // SAFETY: the iterator retains the paired frame and buffer.
            IteratorStorage::Prepared(prepared) => unsafe { prepared.buffer.as_ref().len() },
        };
        (len, Some(len))
    }
}
impl<E> ExactSizeIterator for CleanupFailuresIntoIter<E> {}
impl<E> IntoIterator for CleanupFailures<E> {
    type Item = CleanupFailure<E>;
    type IntoIter = CleanupFailuresIntoIter<E>;
    fn into_iter(self) -> Self::IntoIter {
        let values = match self.storage {
            Storage::Ordinary(values) => IteratorStorage::Ordinary(values.into_iter()),
            Storage::Prepared(prepared) => IteratorStorage::Prepared(prepared),
        };
        CleanupFailuresIntoIter { values }
    }
}
impl<'a, E> IntoIterator for &'a CleanupFailures<E> {
    type Item = &'a CleanupFailure<E>;
    type IntoIter = std::slice::Iter<'a, CleanupFailure<E>>;
    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

/// Startup-provisioned bounded buffers for independent cleanup outcomes.
pub struct PreparedCleanupPool<E: Send + 'static> {
    buffers: SlotPool<'static, Buffer<E>>,
}
impl<E: Send + 'static> PreparedCleanupPool<E> {
    pub fn prepare(
        memory: &PreparedMemory<'static>,
        region: RegionId,
        count: usize,
        retained_entries: usize,
    ) -> Result<Self, MemoryError> {
        Self::prepare_with_return_listeners(memory, region, count, retained_entries, 0)
    }
    /// Prepare cleanup buffers and an explicit finite number of return listeners.
    pub fn prepare_with_return_listeners(
        memory: &PreparedMemory<'static>,
        region: RegionId,
        count: usize,
        retained_entries: usize,
        listeners: usize,
    ) -> Result<Self, MemoryError> {
        let entries = retained_entries
            .checked_add(1)
            .ok_or(MemoryError::ArithmeticOverflow)?;
        Ok(Self {
            buffers: memory.slots_with_return_listeners(region, count, listeners, |_| {
                FixedVector::prepare(memory.allocator(region)?, entries)
            })?,
        })
    }
    /// Selected-region payload and slot metadata for this pool. Allocator
    /// control and alignment are accounted by PreparedMemory itself.
    pub fn storage_bytes(
        count: usize,
        retained_entries: usize,
    ) -> Result<crate::memory::Bytes, MemoryError> {
        let entries = retained_entries
            .checked_add(1)
            .ok_or(MemoryError::ArithmeticOverflow)?;
        let layout = std::alloc::Layout::array::<CleanupFailure<E>>(entries)
            .map_err(|_| MemoryError::InvalidLayout)?;
        SlotPool::<Buffer<E>>::storage_bytes(count)?
            .checked_add(crate::memory::Bytes::new(layout.size()).checked_mul(count)?)
    }
    pub fn storage_bytes_with_return_listeners(
        count: usize,
        retained_entries: usize,
        listeners: usize,
    ) -> Result<crate::memory::Bytes, MemoryError> {
        let entries = retained_entries
            .checked_add(1)
            .ok_or(MemoryError::ArithmeticOverflow)?;
        let layout = std::alloc::Layout::array::<CleanupFailure<E>>(entries)
            .map_err(|_| MemoryError::InvalidLayout)?;
        SlotPool::<Buffer<E>>::storage_bytes_with_return_listeners(count, listeners)?
            .checked_add(crate::memory::Bytes::new(layout.size()).checked_mul(count)?)
    }
    pub fn try_clone(&self) -> Result<Self, MemoryError> {
        Ok(Self {
            buffers: self.buffers.try_clone()?,
        })
    }
    #[must_use]
    pub fn capacity(&self) -> usize {
        self.buffers.capacity()
    }
    /// Demand for the separate pre-reserved destruction frame.
    #[must_use]
    pub fn frame_layout() -> std::alloc::Layout {
        std::alloc::Layout::new::<Record<E>>()
    }
    /// Reserves both physical buffer and frame before finalizer/business work.
    pub fn reserve(&self, frames: &FramePool<'static>) -> Result<CleanupFailures<E>, MemoryError> {
        let frame: FrameReservation<'static, Record<E>> = frames.reserve()?;
        let mut buffer = self.buffers.try_checkout()?;
        let pointer = NonNull::from(&mut *buffer);
        let frame = frame.retain_parts(Clear { buffer: pointer }, buffer);
        Ok(CleanupFailures {
            storage: Storage::Prepared(Prepared {
                buffer: pointer,
                _frame: frame,
                marker: PhantomData,
            }),
        })
    }
}

/// One pre-admitted accumulator. The primary remains installed in the owner
/// during every overflowed secondary value's destructor.
pub struct PreparedCleanupAccumulator<A, E: Send + 'static> {
    primary: AccumulatorPrimary<A, E>,
    first: Option<CleanupFailure<E>>,
    rest: CleanupFailures<E>,
    previous: Option<CleanupFailures<E>>,
}
enum AccumulatorPrimary<A, E> {
    Success(A),
    Succeeded,
    Failed(E),
    Defected(Defect),
    Interrupted,
}
// Advance each owned item before invoking its destructor. Drop resumes the
// remaining sequence on one unwind; a second panic has normal Rust semantics.
struct RestDrain<'a, E> {
    destination: &'a mut CleanupFailures<E>,
    source: CleanupFailuresIntoIter<E>,
    active: bool,
}
impl<E> RestDrain<'_, E> {
    fn run(&mut self) {
        for failure in self.source.by_ref() {
            self.destination.push(failure);
        }
        self.active = false;
    }
}
impl<E> Drop for RestDrain<'_, E> {
    fn drop(&mut self) {
        if self.active {
            self.run();
        }
    }
}
struct FailureDrain<'a, A, E: Send + 'static> {
    destination: &'a mut PreparedCleanupAccumulator<A, E>,
    discard: Option<crate::PrimaryOutcome<E>>,
    head: Option<CleanupFailure<E>>,
    first: Option<CleanupFailure<E>>,
    rest: Option<CleanupFailuresIntoIter<E>>,
    active: bool,
}
impl<A, E: Send + 'static> FailureDrain<'_, A, E> {
    fn run(&mut self) {
        self.destination.flush_previous();
        drop(self.discard.take());
        if let Some(failure) = self.head.take() {
            self.destination.push(failure);
        }
        if let Some(failure) = self.first.take() {
            self.destination.push(failure);
        }
        if let Some(rest) = &mut self.rest {
            for failure in rest.by_ref() {
                self.destination.push(failure);
            }
        }
        self.active = false;
    }
}
impl<A, E: Send + 'static> Drop for FailureDrain<'_, A, E> {
    fn drop(&mut self) {
        if self.active {
            self.run();
        }
    }
}
fn primary_failure<E>(primary: crate::PrimaryOutcome<E>) -> Option<CleanupFailure<E>> {
    match primary {
        crate::PrimaryOutcome::Succeeded => None,
        crate::PrimaryOutcome::Failed(error) => Some(CleanupFailure::ReleaseFailure(error)),
        crate::PrimaryOutcome::Defected(defect) => Some(CleanupFailure::ReleaseDefect(defect)),
        crate::PrimaryOutcome::WasInterrupted => Some(CleanupFailure::ReleaseInterrupted),
    }
}
impl<E: Send + 'static> PreparedCleanupPool<E> {
    /// Reserve actual backing before invoking work that may require cleanup.
    #[allow(clippy::result_large_err)] // Returning the original outcome avoids allocating an error box.
    pub fn reserve_accumulator<A>(
        &self,
        frames: &FramePool<'static>,
        primary: crate::Exit<A, E>,
    ) -> Result<PreparedCleanupAccumulator<A, E>, (MemoryError, crate::Exit<A, E>)> {
        match self.reserve(frames) {
            Ok(rest) => Ok(PreparedCleanupAccumulator::from_reserved(primary, rest)),
            Err(error) => Err((error, primary)),
        }
    }
}
impl<A, E: Send + 'static> PreparedCleanupAccumulator<A, E> {
    /// Start with an owned buffer previously obtained from a prepared pool.
    /// The caller controls the buffer's storage and capacity policy.
    pub fn from_reserved(primary: crate::Exit<A, E>, rest: CleanupFailures<E>) -> Self {
        let (primary, first, previous) = match primary {
            crate::Exit::Success(value) => (AccumulatorPrimary::Success(value), None, None),
            crate::Exit::Fail(error) => (AccumulatorPrimary::Failed(error), None, None),
            crate::Exit::Die(defect) => (AccumulatorPrimary::Defected(defect), None, None),
            crate::Exit::Interrupted => (AccumulatorPrimary::Interrupted, None, None),
            crate::Exit::FinalizationFailed(details) => {
                let primary = match details.primary {
                    crate::PrimaryOutcome::Succeeded => AccumulatorPrimary::Succeeded,
                    crate::PrimaryOutcome::Failed(error) => AccumulatorPrimary::Failed(error),
                    crate::PrimaryOutcome::Defected(defect) => AccumulatorPrimary::Defected(defect),
                    crate::PrimaryOutcome::WasInterrupted => AccumulatorPrimary::Interrupted,
                };
                (primary, Some(details.first), Some(details.rest))
            }
        };
        Self {
            primary,
            first,
            rest,
            previous,
        }
    }
    /// Rejoin an owned outcome and its still-reserved spare buffer. A finalization
    /// may instead carry any valid owned prepared buffer directly in its `rest`
    /// field. Its existing capacity and selected storage remain in effect.
    #[allow(clippy::result_large_err, clippy::type_complexity)]
    pub fn rejoin(
        outcome: crate::Exit<A, E>,
        spare: Option<CleanupFailures<E>>,
    ) -> Result<Self, (crate::Exit<A, E>, Option<CleanupFailures<E>>)> {
        if let Some(rest) = spare {
            if !matches!(rest.storage, Storage::Prepared(_)) || !rest.is_empty() {
                return Err((outcome, Some(rest)));
            }
            return Ok(Self::from_reserved(outcome, rest));
        }
        match outcome {
            crate::Exit::FinalizationFailed(details)
                if matches!(details.rest.storage, Storage::Prepared(_)) =>
            {
                let primary = match details.primary {
                    crate::PrimaryOutcome::Succeeded => AccumulatorPrimary::Succeeded,
                    crate::PrimaryOutcome::Failed(error) => AccumulatorPrimary::Failed(error),
                    crate::PrimaryOutcome::Defected(defect) => AccumulatorPrimary::Defected(defect),
                    crate::PrimaryOutcome::WasInterrupted => AccumulatorPrimary::Interrupted,
                };
                Ok(Self {
                    primary,
                    first: Some(details.first),
                    rest: details.rest,
                    previous: None,
                })
            }
            outcome @ (crate::Exit::Success(_)
            | crate::Exit::Fail(_)
            | crate::Exit::Die(_)
            | crate::Exit::Interrupted
            | crate::Exit::FinalizationFailed(_)) => Err((outcome, None)),
        }
    }
    /// Materialize the outcome without releasing its pre-reserved buffer.
    /// The spare is present exactly when the outcome has no finalization carrier.
    pub fn into_parts(mut self) -> (crate::Exit<A, E>, Option<CleanupFailures<E>>) {
        self.flush_previous();
        match self.first {
            None => {
                let exit = match self.primary {
                    AccumulatorPrimary::Success(value) => crate::Exit::Success(value),
                    AccumulatorPrimary::Failed(error) => crate::Exit::Fail(error),
                    AccumulatorPrimary::Defected(defect) => crate::Exit::Die(defect),
                    AccumulatorPrimary::Interrupted => crate::Exit::Interrupted,
                    AccumulatorPrimary::Succeeded => {
                        crate::Exit::Die(Defect::library(LibraryDefect::MissingInterpreterOutcome))
                    }
                };
                (exit, Some(self.rest))
            }
            Some(first) => {
                let primary = match self.primary {
                    AccumulatorPrimary::Success(value) => {
                        drop(value);
                        crate::PrimaryOutcome::Succeeded
                    }
                    AccumulatorPrimary::Succeeded => crate::PrimaryOutcome::Succeeded,
                    AccumulatorPrimary::Failed(error) => crate::PrimaryOutcome::Failed(error),
                    AccumulatorPrimary::Defected(defect) => crate::PrimaryOutcome::Defected(defect),
                    AccumulatorPrimary::Interrupted => crate::PrimaryOutcome::WasInterrupted,
                };
                (
                    crate::Exit::FinalizationFailed(crate::Finalization {
                        primary,
                        first,
                        rest: self.rest,
                    }),
                    None,
                )
            }
        }
    }
    fn flush_previous(&mut self) {
        if let Some(previous) = self.previous.take() {
            RestDrain {
                destination: &mut self.rest,
                source: previous.into_iter(),
                active: true,
            }
            .run();
        }
    }
    fn push(&mut self, failure: CleanupFailure<E>) {
        self.flush_previous();
        if self.first.is_some() {
            self.rest.push(failure);
            return;
        }
        self.first = Some(failure);
        if matches!(self.primary, AccumulatorPrimary::Success(_)) {
            let previous = std::mem::replace(&mut self.primary, AccumulatorPrimary::Succeeded);
            // Primary and first failure are installed before user destruction.
            drop(previous);
        }
    }
    fn drain_failures(
        &mut self,
        discard: Option<crate::PrimaryOutcome<E>>,
        head: Option<CleanupFailure<E>>,
        first: Option<CleanupFailure<E>>,
        rest: Option<CleanupFailures<E>>,
    ) {
        FailureDrain {
            destination: self,
            discard,
            head,
            first,
            rest: rest.map(IntoIterator::into_iter),
            active: true,
        }
        .run();
    }
    /// Append without a temporary Vec; the owner continues remaining entries on unwind.
    pub fn append_cleanup(&mut self, cleanup: crate::Exit<(), E>) {
        match cleanup {
            crate::Exit::Success(()) => self.flush_previous(),
            crate::Exit::Fail(error) => self.drain_failures(
                None,
                Some(CleanupFailure::ReleaseFailure(error)),
                None,
                None,
            ),
            crate::Exit::Die(defect) => self.drain_failures(
                None,
                Some(CleanupFailure::ReleaseDefect(defect)),
                None,
                None,
            ),
            crate::Exit::Interrupted => {
                self.drain_failures(None, Some(CleanupFailure::ReleaseInterrupted), None, None)
            }
            crate::Exit::FinalizationFailed(details) => self.drain_failures(
                None,
                primary_failure(details.primary),
                Some(details.first),
                Some(details.rest),
            ),
        }
    }
    /// Append a race loser's cleanup; include its primary only when ownership was lost.
    pub(crate) fn append_loser(&mut self, loser: crate::Exit<A, E>, include_primary: bool) {
        match loser {
            crate::Exit::FinalizationFailed(details) => {
                let (discard, head) = if include_primary {
                    (None, primary_failure(details.primary))
                } else {
                    (Some(details.primary), None)
                };
                self.drain_failures(discard, head, Some(details.first), Some(details.rest));
            }
            crate::Exit::Success(value) => {
                self.flush_previous();
                drop(value);
            }
            crate::Exit::Fail(error) => {
                if include_primary {
                    self.drain_failures(
                        None,
                        Some(CleanupFailure::ReleaseFailure(error)),
                        None,
                        None,
                    );
                } else {
                    self.flush_previous();
                    drop(error);
                }
            }
            crate::Exit::Die(defect) => {
                if include_primary {
                    self.drain_failures(
                        None,
                        Some(CleanupFailure::ReleaseDefect(defect)),
                        None,
                        None,
                    );
                } else {
                    self.flush_previous();
                    drop(defect);
                }
            }
            crate::Exit::Interrupted => {
                if include_primary {
                    self.drain_failures(None, Some(CleanupFailure::ReleaseInterrupted), None, None);
                } else {
                    self.flush_previous();
                }
            }
        }
    }
    /// Move the original primary and ordered, bounded secondary outcome onward.
    pub fn finish(mut self) -> crate::Exit<A, E> {
        self.flush_previous();
        match self.first {
            None => match self.primary {
                AccumulatorPrimary::Success(value) => crate::Exit::Success(value),
                AccumulatorPrimary::Failed(error) => crate::Exit::Fail(error),
                AccumulatorPrimary::Defected(defect) => crate::Exit::Die(defect),
                AccumulatorPrimary::Interrupted => crate::Exit::Interrupted,
                AccumulatorPrimary::Succeeded => {
                    crate::Exit::Die(Defect::library(LibraryDefect::MissingInterpreterOutcome))
                }
            },
            Some(first) => {
                let primary = match self.primary {
                    AccumulatorPrimary::Success(value) => {
                        drop(value);
                        crate::PrimaryOutcome::Succeeded
                    }
                    AccumulatorPrimary::Succeeded => crate::PrimaryOutcome::Succeeded,
                    AccumulatorPrimary::Failed(error) => crate::PrimaryOutcome::Failed(error),
                    AccumulatorPrimary::Defected(defect) => crate::PrimaryOutcome::Defected(defect),
                    AccumulatorPrimary::Interrupted => crate::PrimaryOutcome::WasInterrupted,
                };
                crate::Exit::FinalizationFailed(crate::Finalization {
                    primary,
                    first,
                    rest: self.rest,
                })
            }
        }
    }
}

pub(crate) fn append_reserved<A, E: Send + 'static>(
    primary: crate::Exit<A, E>,
    cleanup: crate::Exit<(), E>,
    rest: CleanupFailures<E>,
) -> crate::Exit<A, E> {
    let mut accumulator = PreparedCleanupAccumulator::from_reserved(primary, rest);
    accumulator.append_cleanup(cleanup);
    accumulator.finish()
}

/// Caller-selected region-backed cleanup buffers and independent cleanup frames.
pub struct PreparedCleanupConfig<E: Send + 'static> {
    pub(crate) pool: PreparedCleanupPool<E>,
    pub(crate) frames: FramePool<'static>,
}
impl<E: Send + 'static> PreparedCleanupConfig<E> {
    #[must_use]
    pub fn new(pool: PreparedCleanupPool<E>, frames: FramePool<'static>) -> Self {
        Self { pool, frames }
    }
    pub(crate) fn validate_frames(&self) -> Result<(), MemoryError> {
        let required = PreparedCleanupPool::<E>::frame_layout();
        let available = self.frames.layout();
        if required.align() > available.align() {
            return Err(MemoryError::UnsupportedAlignment {
                requested: required.align(),
                supported: available.align(),
            });
        }
        if required.size() > available.size() {
            return Err(MemoryError::ValueTooLarge {
                requested: crate::memory::Bytes::new(required.size()),
                capacity: crate::memory::Bytes::new(available.size()),
            });
        }
        Ok(())
    }
    pub fn try_clone(&self) -> Result<Self, MemoryError> {
        Ok(Self {
            pool: self.pool.try_clone()?,
            frames: self.frames.try_clone()?,
        })
    }
    /// Reserve physical cleanup backing before starting the associated work.
    pub fn reserve_failures(&self) -> Result<CleanupFailures<E>, MemoryError> {
        self.pool.reserve(&self.frames)
    }
    /// Reserve one listener in each actual pool for a single waiting host.
    pub fn subscribe_returns(&self) -> Result<CleanupReturnWaiter<E>, MemoryError> {
        Ok(CleanupReturnWaiter {
            buffers: self.pool.buffers.subscribe_returns()?,
            frames: self.frames.subscribe_returns()?,
            pool: self.pool.try_clone()?,
            frame_pool: self.frames.try_clone()?,
        })
    }
    #[allow(clippy::result_large_err)] // Failure preserves the original outcome without allocation.
    /// Reserve an accumulator before the work whose failures it will retain.
    pub fn reserve<A>(
        &self,
        primary: crate::Exit<A, E>,
    ) -> Result<PreparedCleanupAccumulator<A, E>, (MemoryError, crate::Exit<A, E>)> {
        self.pool.reserve_accumulator(&self.frames, primary)
    }
}

/// One physically provisioned, cancellation-safe host waiting authority.
pub struct CleanupReturnWaiter<E: Send + 'static> {
    buffers: crate::memory::ReturnSubscription<'static, Buffer<E>>,
    frames: crate::memory::FrameReturnSubscription<'static>,
    pool: PreparedCleanupPool<E>,
    frame_pool: FramePool<'static>,
}
struct CleanupWaitRegistration<'a, E: Send + 'static>(&'a mut CleanupReturnWaiter<E>);
impl<E: Send + 'static> Drop for CleanupWaitRegistration<'_, E> {
    fn drop(&mut self) {
        let frames = self.0.frames.take_waker();
        let buffers = self.0.buffers.take_waker();
        drop(frames);
        drop(buffers);
    }
}
#[derive(Clone, Copy)]
enum MissingCleanup {
    Frame,
    Buffer,
}
impl<E: Send + 'static> CleanupReturnWaiter<E> {
    fn reserve_one(&self) -> Result<CleanupFailures<E>, (MissingCleanup, MemoryError)> {
        let frame: FrameReservation<'static, Record<E>> = self
            .frame_pool
            .reserve_blocking()
            .map_err(|error| (MissingCleanup::Frame, error))?;
        let mut buffer = self
            .pool
            .buffers
            .checkout()
            .map_err(|error| (MissingCleanup::Buffer, error))?;
        let pointer = NonNull::from(&mut *buffer);
        let frame = frame.retain_parts(Clear { buffer: pointer }, buffer);
        Ok(CleanupFailures {
            storage: Storage::Prepared(Prepared {
                buffer: pointer,
                _frame: frame,
                marker: PhantomData,
            }),
        })
    }
    /// Try a complete pair; all partial reservations return before failure.
    pub fn try_reserve_pair(&self) -> Result<[CleanupFailures<E>; 2], MemoryError> {
        let first = self.reserve_one().map_err(|(_, error)| error)?;
        let second = self.reserve_one().map_err(|(_, error)| error)?;
        Ok([first, second])
    }
    /// Wait without holding resources, registering before checking both pools.
    pub async fn wait_pair_available(&mut self) -> Result<(), MemoryError> {
        let registration = CleanupWaitRegistration(self);
        std::future::poll_fn(move |cx| registration.0.poll_pair_available(cx)).await
    }
    fn poll_pair_available(
        &mut self,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), MemoryError>> {
        self.frames.register(cx.waker());
        self.buffers.register(cx.waker());
        if self.frame_pool.reusable_capacity() < 2 || self.pool.buffers.reusable_capacity() < 2 {
            return std::task::Poll::Ready(Err(MemoryError::NoSlot));
        }
        let frames = match self.frame_pool.available() {
            Ok(value) => value,
            Err(error) => return std::task::Poll::Ready(Err(error)),
        };
        let buffers = match self.pool.buffers.available() {
            Ok(value) => value,
            Err(error) => return std::task::Poll::Ready(Err(error)),
        };
        if frames >= 2 && buffers >= 2 {
            std::task::Poll::Ready(Ok(()))
        } else {
            std::task::Poll::Pending
        }
    }
}
