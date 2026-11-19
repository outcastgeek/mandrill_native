//! Fixed owned element arrays for managed JSON codecs.
use crate::memory::{
    Bytes, FixedVector, FramePool, MemoryError, PreparedMemory, RegionId, RetainedFrameValue,
    RuntimeFrame, SlotLease, SlotPool,
};
use std::ptr::NonNull;

/// Explicit simultaneous arrays and retained elements per array.
#[derive(Clone, Copy, Debug)]
pub struct PreparedJsonArrayCapacity {
    /// Maximum live array payload owners.
    pub arrays: usize,
    /// Physically retained elements per array.
    pub elements: usize,
}
/// Region backing and separately supplied actual retained-frame layout.
#[derive(Clone, Copy, Debug)]
pub struct PreparedJsonArrayDemand {
    /// Region vector slots and elements, excluding allocator overhead.
    pub region_bytes: Bytes,
    /// Actual caller-provided frame payload layout for one admitted array.
    pub frame_layout: std::alloc::Layout,
    /// Simultaneous frame count required at full array occupancy.
    pub frames: usize,
}
struct Payload<A: Send + 'static> {
    vector: NonNull<FixedVector<'static, A>>,
}
// SAFETY: the uniquely held frame/footer owns the vector slot, and the pointer
// never outlives that footer. Moving the frame transfers exclusive access.
unsafe impl<A: Send + 'static> Send for Payload<A> {}
// SAFETY: shared access exposes only immutable A references; mutation is exclusive.
unsafe impl<A: Send + Sync + 'static> Sync for Payload<A> {}
impl<A: Send + 'static> Drop for Payload<A> {
    fn drop(&mut self) {
        // SAFETY: this payload is destroyed before its retained vector footer;
        // no public mutable access or other payload owner exists.
        unsafe {
            self.vector.as_mut().clear();
        }
    }
}
/// A finite region-backed array pool with separately supplied retained frames.
pub struct PreparedJsonArrayPool<A: Send + 'static> {
    vectors: SlotPool<'static, FixedVector<'static, A>>,
    frames: FramePool<'static>,
}
/// Immutable typed array retaining its complete actual vector and frame owners.
pub struct PreparedJsonArray<A: Send + 'static> {
    payload: NonNull<Payload<A>>,
    _frame: RuntimeFrame,
}
impl<A: Send + std::fmt::Debug + 'static> std::fmt::Debug for PreparedJsonArray<A> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.as_slice().fmt(f)
    }
}
// SAFETY: the unique frame holds its live payload and vector footer throughout transfer.
unsafe impl<A: Send + 'static> Send for PreparedJsonArray<A> {}
// SAFETY: immutable views need A: Sync; all mutation requires unique array ownership.
unsafe impl<A: Send + Sync + 'static> Sync for PreparedJsonArray<A> {}
impl PreparedJsonArrayCapacity {
    /// Count actual vector slots and exact frame layout for this element type.
    pub fn demand<A: Send + 'static>(self) -> Result<PreparedJsonArrayDemand, MemoryError> {
        let elements = std::alloc::Layout::array::<A>(self.elements)
            .map_err(|_| MemoryError::InvalidLayout)?;
        let region_bytes = SlotPool::<FixedVector<'static, A>>::storage_bytes(self.arrays)?
            .checked_add(Bytes::new(elements.size()).checked_mul(self.arrays)?)?;
        Ok(PreparedJsonArrayDemand {
            region_bytes,
            frame_layout: std::alloc::Layout::new::<
                RetainedFrameValue<Payload<A>, SlotLease<'static, FixedVector<'static, A>>>,
            >(),
            frames: self.arrays,
        })
    }
}
impl<A: Send + 'static> PreparedJsonArrayPool<A> {
    /// Prepare fixed vectors; caller-supplied frame backing is separate.
    pub fn prepare(
        memory: &PreparedMemory<'static>,
        region: RegionId,
        capacity: PreparedJsonArrayCapacity,
        frames: FramePool<'static>,
    ) -> Result<Self, MemoryError> {
        let _ = capacity.demand::<A>()?;
        let vectors = memory.slots(region, capacity.arrays, |_| {
            FixedVector::prepare(memory.allocator(region)?, capacity.elements)
        })?;
        Ok(Self { vectors, frames })
    }
    /// Admit the vector and its exact destruction-ordered frame before decoding.
    pub(crate) fn reserve(&self) -> Result<PreparedJsonArray<A>, MemoryError> {
        let mut vector = self.vectors.checkout()?;
        let reservation = self
            .frames
            .reserve::<RetainedFrameValue<Payload<A>, SlotLease<'static, FixedVector<'static, A>>>>(
            )?;
        let pointer = NonNull::from(&mut *vector);
        let frame = reservation.retain_parts(Payload { vector: pointer }, vector);
        // SAFETY: this exact payload/footer pair was just initialized in the live frame.
        let payload = unsafe {
            RetainedFrameValue::<Payload<A>, SlotLease<'static, FixedVector<'static, A>>>::value_pointer(frame.pointer().cast())
        };
        Ok(PreparedJsonArray {
            payload,
            _frame: frame,
        })
    }
}
impl<A: Send + 'static> PreparedJsonArray<A> {
    /// Borrow all initialized values; the backing frame remains held by self.
    #[must_use]
    pub fn as_slice(&self) -> &[A] {
        // SAFETY: the frame owns this exact payload and its vector footer while self lives.
        unsafe { self.payload.as_ref().vector.as_ref().as_slice() }
    }
    /// Borrow initialized values in order.
    pub fn iter(&self) -> std::slice::Iter<'_, A> {
        self.as_slice().iter()
    }
    pub(crate) fn capacity(&self) -> usize {
        // SAFETY: as_slice's retained-frame ownership proof applies.
        unsafe { self.payload.as_ref().vector.as_ref().capacity() }
    }
    pub(crate) fn push(&mut self, value: A) -> Result<(), A> {
        // SAFETY: unique array ownership excludes other mutable or shared borrows.
        unsafe { self.payload.as_mut().vector.as_mut().push(value) }
    }
    pub(crate) fn as_mut_slice(&mut self) -> &mut [A] {
        // SAFETY: unique array ownership excludes shared and mutable aliases.
        unsafe { self.payload.as_mut().vector.as_mut().as_mut_slice() }
    }
    pub(crate) fn remove_first(&mut self) -> Option<A> {
        // SAFETY: exclusive ownership permits mutation of the retained vector.
        unsafe { self.payload.as_mut().vector.as_mut().remove_first() }
    }
    pub(crate) fn remove_at(&mut self, index: usize) -> Option<A> {
        // SAFETY: unique ownership permits mutation of the retained vector.
        unsafe { self.payload.as_mut().vector.as_mut().remove_at(index) }
    }
}

type ArrayFrame<A> = RetainedFrameValue<Payload<A>, SlotLease<'static, FixedVector<'static, A>>>;

/// Exclusive construction owner for an immutable shared prepared array.
pub struct PreparedJsonArrayBuilder<A: Send + Sync + 'static> {
    vector: Option<SlotLease<'static, FixedVector<'static, A>>>,
    frame: Option<crate::memory::FrameReservation<'static, ArrayFrame<A>>>,
}
/// Immutable shared array retaining its entire vector and destruction-ordered frame.
pub struct SharedPreparedJsonArray<A: Send + Sync + 'static> {
    frame: crate::memory::SharedRuntimeFrame,
    marker: std::marker::PhantomData<A>,
}
impl<A: Send + Sync + std::fmt::Debug + 'static> std::fmt::Debug for SharedPreparedJsonArray<A> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.as_slice().fmt(f)
    }
}
impl<A: Send + Sync + 'static> PreparedJsonArrayPool<A> {
    /// Reserve the same physical vector and exact frame for exclusive construction.
    pub fn reserve_shared(&self) -> Result<PreparedJsonArrayBuilder<A>, MemoryError> {
        let vector = self.vectors.checkout()?;
        let frame = self
            .frames
            .reserve::<RetainedFrameValue<Payload<A>, SlotLease<'static, FixedVector<'static, A>>>>(
            )?;
        Ok(PreparedJsonArrayBuilder {
            vector: Some(vector),
            frame: Some(frame),
        })
    }
}
impl<A: Send + Sync + 'static> PreparedJsonArrayBuilder<A> {
    /// Number of initialized elements.
    #[must_use]
    pub fn len(&self) -> usize {
        self.vector.as_ref().map_or(0, |v| v.len())
    }
    /// Whether no elements are initialized.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    /// Physical element capacity.
    #[must_use]
    pub fn capacity(&self) -> usize {
        self.vector.as_ref().map_or(0, |v| v.capacity())
    }
    /// Borrow the exclusively held initialized elements.
    #[must_use]
    pub fn as_slice(&self) -> &[A] {
        self.vector.as_ref().map_or(&[], |v| v.as_slice())
    }
    /// Remove the first element, preserving the order of the remainder.
    pub fn remove_first(&mut self) -> Option<A> {
        self.vector.as_mut().and_then(|v| v.remove_first())
    }
    /// Retire initialized elements while retaining the prepared slot.
    pub fn clear(&mut self) {
        if let Some(v) = &mut self.vector {
            v.clear();
        }
    }
    /// Insert one owned value, returning it if the prepared capacity is exhausted.
    pub fn try_push(&mut self, value: A) -> Result<(), A> {
        match &mut self.vector {
            Some(vector) => vector.push(value),
            None => Err(value),
        }
    }
    /// Publish without another admission; no mutable array access remains.
    #[allow(clippy::expect_used)] // A live builder owns both preadmitted parts until consuming publication.
    #[must_use]
    pub fn publish(mut self) -> SharedPreparedJsonArray<A> {
        let mut vector = self.vector.take().expect("live prepared array builder");
        let reservation = self.frame.take().expect("live prepared array frame");
        let pointer = NonNull::from(&mut *vector);
        let frame = reservation.share_parts(Payload { vector: pointer }, vector);
        SharedPreparedJsonArray {
            frame,
            marker: std::marker::PhantomData,
        }
    }
}
impl<A: Send + Sync + 'static> Drop for PreparedJsonArrayBuilder<A> {
    fn drop(&mut self) {
        if let (Some(mut vector), Some(reservation)) = (self.vector.take(), self.frame.take()) {
            let pointer = NonNull::from(&mut *vector);
            // The frame remains the destruction owner even for an abandoned partial build;
            // its footer holds the vector through element destructors and descendants.
            drop(reservation.share_parts(Payload { vector: pointer }, vector));
        }
    }
}
impl<A: Send + Sync + 'static> SharedPreparedJsonArray<A> {
    /// Borrow the immutable initialized elements while holding the complete frame.
    #[must_use]
    pub fn as_slice(&self) -> &[A] {
        // SAFETY: the shared frame retains this exact payload and footer until the last owner.
        unsafe {
            ArrayFrame::<A>::value_pointer(self.frame.pointer().cast())
                .as_ref()
                .vector
                .as_ref()
                .as_slice()
        }
    }
    /// Retain another actual frame owner, subject to physical owner limits.
    pub fn try_clone(&self) -> Result<Self, MemoryError> {
        Ok(Self {
            frame: self.frame.try_clone()?,
            marker: std::marker::PhantomData,
        })
    }
}
