//! Stable-address initialized frames. The underlying mutable slot is private:
//! once placed, a pinned value can only be projected as Pin or dropped in place.
use super::pool::ReturnSubscription;
use super::{
    Allocation, Bytes, MemoryError, PreparedMemory, RegionId, SharedSlot, SlotLease, SlotPool,
};
use crate::internal::destruction::{DestructionNode, enqueue_prepared};
use std::alloc::Layout;
use std::cell::UnsafeCell;
use std::future::Future;
use std::marker::PhantomData;
use std::mem::{ManuallyDrop, MaybeUninit};
use std::pin::Pin;
use std::ptr::NonNull;
use std::task::{Context, Poll};

struct Cell<'a> {
    storage: Allocation<'a>,
    initialized: UnsafeCell<bool>,
    destroy: unsafe fn(NonNull<u8>),
    footer_live: UnsafeCell<bool>,
    footer: unsafe fn(NonNull<u8>),
    node: UnsafeCell<DestructionNode>,
    queued: UnsafeCell<MaybeUninit<SlotLease<'a, Cell<'a>>>>,
}
// SAFETY: a live payload is installed only under a unique frame reservation for
// Send values. Queue metadata is accessed only by the intrusive drain protocol;
// no shared reference permits payload mutation or moving initialized values.
unsafe impl Send for Cell<'_> {}
unsafe fn empty(_: NonNull<u8>) {}
unsafe fn destroy_node(node: NonNull<DestructionNode>) {
    // SAFETY: RuntimeFrame installed a static Cell context and a retained lease;
    // shared Cell access mutates only the initialized flag through UnsafeCell.
    unsafe { (&*DestructionNode::context(node).cast::<Cell<'static>>()).clear() };
}
unsafe fn footer_node(node: NonNull<DestructionNode>) {
    // SAFETY: queued cell remains retained after its payload descendants.
    unsafe { (&*DestructionNode::context(node).cast::<Cell<'static>>()).clear_footer() };
}
unsafe fn release_node(node: NonNull<DestructionNode>) {
    // SAFETY: queued owner was initialized once before enqueue, and this callback
    // consumes it only after payload destruction and unlinking the node.
    unsafe {
        let cell = &*DestructionNode::context(node).cast::<Cell<'static>>();
        let owner = (*cell.queued.get()).assume_init_read();
        drop(owner);
    }
}
unsafe fn destroy<T>(pointer: NonNull<u8>) {
    // SAFETY: initialized state records exactly this T at this aligned address;
    // the unique frame owner clears state before invoking destruction once.
    unsafe { std::ptr::drop_in_place(pointer.cast::<T>().as_ptr()) };
}
impl Cell<'_> {
    fn clear_footer(&self) {
        // SAFETY: unique cell drain/final destruction only; clear before Drop.
        let live = unsafe { &mut *self.footer_live.get() };
        if *live {
            *live = false;
            unsafe { (self.footer)(self.storage.pointer) };
        }
    }
    fn clear(&self) {
        // SAFETY: only the unique live frame, its drain, or final pool destruction
        // accesses initialized state; none exposes Cell through a shared API.
        let initialized = unsafe { &mut *self.initialized.get() };
        if *initialized {
            *initialized = false;
            // SAFETY: unique private cell ownership, initialized type/drop function
            // and matching pointer. Clearing first also protects unwind/reentrancy.
            unsafe { (self.destroy)(self.storage.pointer) };
        }
    }
}
impl Drop for Cell<'_> {
    fn drop(&mut self) {
        self.clear();
        self.clear_footer();
    }
}

/// A physically prepared, finite set of stable-address frame payloads. It may be
/// used with borrowed backing directly, or owned backing for static callbacks.
/// This is storage, not a proof of an arbitrary Effect's complete run demand.
pub struct FramePool<'a> {
    cells: SlotPool<'a, Cell<'a>>,
    layout: Layout,
}
impl<'a> FramePool<'a> {
    /// Actual fixed-cell bookkeeping plus payload backing (allocator control
    /// metadata remains charged by PreparedMemory itself).
    pub fn storage_bytes(count: usize, layout: Layout) -> Result<Bytes, MemoryError> {
        SlotPool::<Cell<'a>>::storage_bytes(count)?
            .checked_add(Bytes::new(layout.size()).checked_mul(count)?)
    }
    pub fn storage_bytes_with_return_listeners(
        count: usize,
        layout: Layout,
        listeners: usize,
    ) -> Result<Bytes, MemoryError> {
        SlotPool::<Cell<'a>>::storage_bytes_with_return_listeners(count, listeners)?
            .checked_add(Bytes::new(layout.size()).checked_mul(count)?)
    }
    pub(crate) fn subscribe_returns(&self) -> Result<FrameReturnSubscription<'a>, MemoryError> {
        Ok(FrameReturnSubscription {
            listener: self.cells.subscribe_returns()?,
        })
    }
    pub(crate) fn available(&self) -> Result<usize, MemoryError> {
        self.cells.available()
    }
    pub(crate) fn reusable_capacity(&self) -> usize {
        self.cells.reusable_capacity()
    }
    #[must_use]
    pub fn active(&self) -> usize {
        self.cells.active()
    }

    pub(crate) fn same_pool(&self, other: &Self) -> bool {
        self.cells.same_pool(&other.cells)
    }
    pub fn try_clone(&self) -> Result<Self, MemoryError> {
        Ok(Self {
            cells: self.cells.try_clone()?,
            layout: self.layout,
        })
    }
    #[must_use]
    pub fn layout(&self) -> Layout {
        self.layout
    }
    #[must_use]
    pub fn capacity(&self) -> usize {
        self.cells.capacity()
    }
    pub fn close(&self) {
        self.cells.close();
    }
    /// Check actual concrete layout and check out physical storage BEFORE
    /// constructing captures/futures or committing a broader admission bundle.
    pub fn reserve<T: Send>(&self) -> Result<FrameReservation<'a, T>, MemoryError> {
        let layout = Layout::new::<T>();
        validate_layout(layout, self.layout)?;
        Ok(FrameReservation {
            cell: self.cells.try_checkout()?,
            value: PhantomData,
        })
    }

    pub(crate) fn reserve_layout(&self, layout: Layout) -> Result<FrameVacancy<'a>, MemoryError> {
        validate_layout(layout, self.layout)?;
        Ok(FrameVacancy { cell: self.cells.try_checkout()?, layout })
    }
    pub(crate) fn reserve_blocking<T: Send>(&self) -> Result<FrameReservation<'a, T>, MemoryError> {
        let layout = Layout::new::<T>();
        if layout.align() > self.layout.align() {
            return Err(MemoryError::UnsupportedAlignment {
                requested: layout.align(),
                supported: self.layout.align(),
            });
        }
        if layout.size() > self.layout.size() {
            return Err(MemoryError::ValueTooLarge {
                requested: Bytes::new(layout.size()),
                capacity: Bytes::new(self.layout.size()),
            });
        }
        Ok(FrameReservation {
            cell: self.cells.checkout()?,
            value: PhantomData,
        })
    }
}

fn validate_layout(required: Layout, available: Layout) -> Result<(), MemoryError> {
    if required.align() > available.align() {
        return Err(MemoryError::UnsupportedAlignment {
            requested: required.align(), supported: available.align(),
        });
    }
    if required.size() > available.size() {
        return Err(MemoryError::ValueTooLarge {
            requested: Bytes::new(required.size()), capacity: Bytes::new(available.size()),
        });
    }
    Ok(())
}

// This empty lease retains an already checked region cell before the concrete
// future is made. Placement rechecks the same layout without another checkout.
pub(crate) struct FrameVacancy<'a> {
    cell: SlotLease<'a, Cell<'a>>,
    layout: Layout,
}

impl<'a> FrameVacancy<'a> {
    pub(crate) fn typed<T: Send>(self, _value: &T) -> Result<FrameReservation<'a, T>, MemoryError> {
        validate_layout(Layout::new::<T>(), self.layout)?;
        Ok(FrameReservation { cell: self.cell, value: PhantomData })
    }
}
pub(crate) struct FrameReturnSubscription<'a> {
    listener: ReturnSubscription<'a, Cell<'a>>,
}
impl FrameReturnSubscription<'_> {
    pub(crate) fn register(&self, waker: &std::task::Waker) {
        self.listener.register(waker);
    }
    pub(crate) fn take_waker(&self) -> Option<std::task::Waker> {
        self.listener.take_waker()
    }
}

/// Separate physical work/cleanup frame capacity for one runtime storage policy.
/// Control frames cannot alias work frames, so saturation cannot consume the
/// cleanup reserve. This is not yet a complete prepared interpreter descriptor.
pub struct RunFrames {
    pub(crate) work: FramePool<'static>,
    pub(crate) cleanup: FramePool<'static>,
}
impl RunFrames {
    pub fn new(work: FramePool<'static>, cleanup: FramePool<'static>) -> Result<Self, MemoryError> {
        if work.cells.same_pool(&cleanup.cells) {
            return Err(MemoryError::ControlPoolAlias);
        }
        Ok(Self { work, cleanup })
    }
    pub fn try_clone(&self) -> Result<Self, MemoryError> {
        Ok(Self {
            work: self.work.try_clone()?,
            cleanup: self.cleanup.try_clone()?,
        })
    }
}

/// A checked physical reservation for one concrete T. No payload exists yet;
/// dropping it returns the empty slot. Placement itself cannot exhaust storage.
pub struct FrameReservation<'a, T: Send> {
    cell: SlotLease<'a, Cell<'a>>,
    value: PhantomData<T>,
}
impl<'a, T: Send> FrameReservation<'a, T> {
    /// Initialize at the final address. Safe access afterwards is pinned only.
    #[must_use]
    pub fn initialize(self, value: T) -> PinnedValue<'a, T> {
        PinnedValue {
            frame: self.place(value),
            value: PhantomData,
        }
    }
    fn write(&mut self, value: T) {
        let pointer = self.cell.storage.pointer;
        // SAFETY: reserve checked T size/alignment against this private cell;
        // no other lease/view can access it. T is moved once before any pin exists.
        unsafe { pointer.cast::<T>().as_ptr().write(value) };
        self.cell.destroy = destroy::<T>;
        self.cell.footer = empty;
        *self.cell.footer_live.get_mut() = false;
        *self.cell.initialized.get_mut() = true;
    }
    fn place(mut self, value: T) -> ErasedFrame<'a> {
        self.write(value);
        ErasedFrame { cell: self.cell }
    }
}
impl<T: Send + Sync + 'static> FrameReservation<'static, T> {
    pub(crate) fn share(mut self, value: T) -> SharedRuntimeFrame {
        self.write(value);
        SharedRuntimeFrame {
            cell: ManuallyDrop::new(self.cell.share()),
        }
    }
}

impl FramePool<'static> {
    pub(crate) fn owns_shared(&self, frame: &SharedRuntimeFrame) -> bool {
        self.cells.owns_shared(&frame.cell)
    }
}

/// Immutable typed frame ownership. Last release enters the SAME intrusive
/// payload destruction lane and retains the unique cell until destruction ends.
/// Used by pre-provisioned graph environment/resource records, not ordinary Clone.
pub(crate) struct SharedRuntimeFrame {
    cell: ManuallyDrop<SharedSlot<'static, Cell<'static>>>,
}
// SAFETY: only the typed share constructor creates this wrapper, for Sync T;
// Typed projection requires the caller to prove its exact T. Cell metadata is mutated only
// after the atomic/locked last-owner transfer or by the common queued drain.
unsafe impl Send for SharedRuntimeFrame {}
unsafe impl Sync for SharedRuntimeFrame {}
impl SharedRuntimeFrame {
    pub(crate) fn try_clone(&self) -> Result<Self, MemoryError> {
        Ok(Self {
            cell: ManuallyDrop::new(self.cell.try_clone()?),
        })
    }
    pub(crate) fn pointer(&self) -> NonNull<u8> {
        self.cell.storage.pointer
    }
}
impl Drop for SharedRuntimeFrame {
    fn drop(&mut self) {
        // SAFETY: move this view once; no field access follows this transfer.
        let cell = unsafe { ManuallyDrop::take(&mut self.cell) };
        if let Some(cell) = cell.into_last() {
            drop(RuntimeFrame {
                cell: ManuallyDrop::new(cell),
            });
        }
    }
}

impl<T: Send + 'static> FrameReservation<'static, T> {
    pub(crate) fn erase(mut self, value: T) -> RuntimeFrame {
        self.write(value);
        RuntimeFrame {
            cell: ManuallyDrop::new(self.cell),
        }
    }
}

/// An opaque unit future stored in one caller-prepared frame.
///
/// The frame owns the actual future until this owner is destroyed, including
/// after Ready. Its fixed inline layout does not bound allocations reachable
/// through the future's captures.
pub struct PreparedUnitFuture {
    frame: RuntimeFrame,
    poll: fn(&mut RuntimeFrame, &mut Context<'_>) -> Poll<()>,
    complete: bool,
}
// Moving the owner only moves a frame handle; the stored future stays in place.
impl Unpin for PreparedUnitFuture {}
impl PreparedUnitFuture {
    /// Initialize an already reserved, exact-typed frame with its future.
    #[must_use]
    pub fn initialize<F: Future<Output = ()> + Send + 'static>(reservation: FrameReservation<'static, F>, future: F) -> Self {
        fn poll_exact<F: Future<Output = ()> + Send + 'static>(frame: &mut RuntimeFrame, cx: &mut Context<'_>) -> Poll<()> {
            // SAFETY: initialize pairs this function with precisely F in the
            // unique frame. The frame's stable address is retained through poll
            // and destruction; this exclusive borrow prevents another projection.
            unsafe { Pin::new_unchecked(&mut *frame.pointer().cast::<F>().as_ptr()) }.poll(cx)
        }
        Self { frame: reservation.erase(future), poll: poll_exact::<F>, complete: false }
    }
    /// Reserve an actual matching frame, returning the original future unchanged
    /// if admission is refused. No future is polled before successful admission.
    pub fn try_admit<F: Future<Output = ()> + Send + 'static>(pool: &FramePool<'static>, future: F) -> Result<Self, (MemoryError, F)> {
        match pool.reserve::<F>() { Ok(reservation) => Ok(Self::initialize(reservation, future)), Err(error) => Err((error, future)) }
    }
}
impl Future for PreparedUnitFuture {
    type Output = ();
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        let this = self.get_mut();
        if this.complete { return Poll::Ready(()); }
        let result = (this.poll)(&mut this.frame, cx);
        if result.is_ready() { this.complete = true; }
        result
    }
}

// The context guard belongs to the exact in-place payload destructor, including
// when FlatDrop defers this payload beyond its outer owner's destruction.
struct ScopedUnitFuture<F, C: crate::prepared_provider::ProviderWorkerContext> {
    future: ManuallyDrop<F>,
    context: C,
}
impl<F, C: crate::prepared_provider::ProviderWorkerContext> Drop for ScopedUnitFuture<F, C> {
    fn drop(&mut self) {
        let future = std::mem::DropGuard::new(&mut self.future, |future| {
            // SAFETY: this is the single initialized future and this destructor
            // executes only once at its stable address.
            unsafe { ManuallyDrop::drop(future); }
        });
        let _entered = self.context.enter();
        drop(future);
    }
}
/// Exact pre-admission for a pinned unit future, worker context, and retained
/// footer. The footer outlives the future and all prepared destruction descendants.
pub struct PreparedScopedUnitFutureReservation<F, C, G>
where F: Future<Output = ()> + Send + 'static, C: crate::prepared_provider::ProviderWorkerContext, G: Send + 'static {
    frame: FrameReservation<'static, RetainedFrameValue<ScopedUnitFuture<F, C>, G>>,
}
impl<F, C, G> PreparedScopedUnitFutureReservation<F, C, G>
where F: Future<Output = ()> + Send + 'static, C: crate::prepared_provider::ProviderWorkerContext, G: Send + 'static {
    /// Exact payload/footer frame layout; captured pointees remain separate.
    #[must_use]
    pub fn layout() -> Layout { Layout::new::<RetainedFrameValue<ScopedUnitFuture<F, C>, G>>() }
    /// Reserve the actual frame before constructing or publishing the future.
    pub fn reserve(pool: &FramePool<'static>) -> Result<Self, MemoryError> { Ok(Self { frame: pool.reserve()? }) }
    /// Initialize the reserved frame without moving the future after placement.
    #[must_use]
    pub fn initialize(self, future: F, context: C, footer: G) -> PreparedUnitFuture {
        fn poll_exact<F, C, G>(frame: &mut RuntimeFrame, cx: &mut Context<'_>) -> Poll<()>
        where F: Future<Output = ()> + Send + 'static, C: crate::prepared_provider::ProviderWorkerContext, G: Send + 'static {
            // SAFETY: this private function is paired only with this exact
            // retained type; the unique frame keeps the payload pinned in place.
            let value = unsafe { RetainedFrameValue::<ScopedUnitFuture<F, C>, G>::value_pointer(frame.pointer().cast()) };
            // SAFETY: exclusive frame access owns this exact live payload.
            let value = unsafe { &mut *value.as_ptr() };
            let _entered = value.context.enter();
            // SAFETY: F is never moved from its stable initialized frame before
            // in-place destruction, and this poll has the unique projection.
            unsafe { Pin::new_unchecked(&mut *value.future) }.poll(cx)
        }
        let frame = self.frame.retain_parts(ScopedUnitFuture { future: ManuallyDrop::new(future), context }, footer);
        PreparedUnitFuture { frame, poll: poll_exact::<F, C, G>, complete: false }
    }
}

/// Two-phase payload: footer remains initialized through all prepared payload
/// descendants. It is private, never projected as a pinned aggregate.
pub(crate) struct RetainedFrameValue<T, G> {
    value: ManuallyDrop<T>,
    footer: ManuallyDrop<G>,
}
impl<T, G> RetainedFrameValue<T, G> {
    pub(crate) unsafe fn footer_pointer(pointer: NonNull<Self>) -> NonNull<G> {
        // SAFETY: caller proves exact live retained record and keeps its owner.
        // This does not expose the aggregate or project a pinned payload.
        unsafe {
            NonNull::new_unchecked(std::ptr::addr_of_mut!((*pointer.as_ptr()).footer).cast::<G>())
        }
    }
    pub(crate) unsafe fn value_pointer(pointer: NonNull<Self>) -> NonNull<T> {
        // SAFETY: caller proves exact live RetainedFrameValue and retains it.
        unsafe {
            NonNull::new_unchecked(std::ptr::addr_of_mut!((*pointer.as_ptr()).value).cast::<T>())
        }
    }
}
unsafe fn retained_value<T, G>(pointer: NonNull<u8>) {
    // SAFETY: paired exact reservation; only payload is retired in this phase.
    unsafe {
        ManuallyDrop::drop(&mut (*pointer.cast::<RetainedFrameValue<T, G>>().as_ptr()).value)
    };
}
unsafe fn retained_footer<T, G>(pointer: NonNull<u8>) {
    // SAFETY: same reservation, footer live until all payload descendants finish.
    unsafe {
        ManuallyDrop::drop(&mut (*pointer.cast::<RetainedFrameValue<T, G>>().as_ptr()).footer)
    };
}
impl<T: Send + 'static, G: Send + 'static> FrameReservation<'static, RetainedFrameValue<T, G>> {
    fn write_parts(&mut self, value: T, footer: G) {
        self.write(RetainedFrameValue {
            value: ManuallyDrop::new(value),
            footer: ManuallyDrop::new(footer),
        });
        self.cell.destroy = retained_value::<T, G>;
        self.cell.footer = retained_footer::<T, G>;
        *self.cell.footer_live.get_mut() = true;
    }
    pub(crate) fn share_parts(mut self, value: T, footer: G) -> SharedRuntimeFrame
    where
        T: Sync,
        G: Sync,
    {
        self.write_parts(value, footer);
        SharedRuntimeFrame {
            cell: ManuallyDrop::new(self.cell.share()),
        }
    }
    pub(crate) fn retain_parts(mut self, value: T, footer: G) -> RuntimeFrame {
        self.write_parts(value, footer);
        self.cell.mark_retained();
        RuntimeFrame {
            cell: ManuallyDrop::new(self.cell),
        }
    }
}

/// Static runtime erasure additionally participates in iterative destruction.
/// The reserved record retains its own physical lease until the drain releases it.
pub(crate) struct RuntimeFrame {
    cell: ManuallyDrop<SlotLease<'static, Cell<'static>>>,
}
impl RuntimeFrame {
    pub(crate) fn pointer(&self) -> NonNull<u8> {
        self.cell.storage.pointer
    }
    /// Replace a moved-out, never-pinned callback payload with its settlement
    /// owner. Caller proves empty state and size/alignment against the original
    /// typed reservation; no other projection or pin may exist.
    pub(crate) unsafe fn replace_unpinned<T: Send + 'static>(&mut self, value: T) {
        // SAFETY: caller's typed enclosing-payload proof provides capacity and
        // alignment. The original payload was moved out once, never pinned.
        unsafe { self.pointer().cast::<T>().as_ptr().write(value) };
        self.cell.destroy = destroy::<T>;
        *self.cell.initialized.get_mut() = true;
    }
    pub(crate) unsafe fn take<T>(&mut self) -> T {
        *self.cell.initialized.get_mut() = false;
        // SAFETY: caller proves exact T and that the callback was never pinned.
        unsafe { self.pointer().cast::<T>().as_ptr().read() }
    }
}
impl Drop for RuntimeFrame {
    fn drop(&mut self) {
        // SAFETY: the context derives from shared Cell access, not a temporary
        // exclusive reborrow invalidated by subsequent whole-Cell shared access.
        // Only UnsafeCell metadata is mutated while that context is retained.
        // Moving the SlotLease handle into queued storage does not move its Cell:
        // the private pool owns the Cell at a fixed address and the moved lease
        // retains it until unlink/release. Payload drops outside registry locks,
        // before that physical lease returns to the pool.
        unsafe {
            let owner = ManuallyDrop::take(&mut self.cell);
            owner.mark_retained();
            let cell = &*owner;
            let context = std::ptr::from_ref(cell).cast_mut().cast::<()>();
            let node = NonNull::new_unchecked(cell.node.get());
            let queued = cell.queued.get();
            (*node.as_ptr()).set_context(context);
            (*queued).write(owner);
            enqueue_prepared(node);
        }
    }
}

/// Unique private frame ownership for erased runtime adapters. No DerefMut or
/// public raw pointer escape can move a pinned future. Drop clears payload in
/// place BEFORE the SlotLease field returns the cell to its pool.
pub(crate) struct ErasedFrame<'a> {
    cell: SlotLease<'a, Cell<'a>>,
}
impl ErasedFrame<'_> {
    pub(crate) fn pointer(&self) -> NonNull<u8> {
        self.cell.storage.pointer
    }
    /// Used only for never-pinned one-shot callbacks, not future frames.
    pub(crate) unsafe fn take<T>(&mut self) -> T {
        *self.cell.initialized.get_mut() = false;
        // SAFETY: caller proves exact concrete T and that it was never pinned.
        unsafe { self.pointer().cast::<T>().as_ptr().read() }
    }
}
impl Drop for ErasedFrame<'_> {
    fn drop(&mut self) {
        self.cell.clear();
    }
}

/// An initialized pinned value retaining backing through completion/destruction.
/// Moving this small handle never moves its payload. No mutable unpinned access
/// is exposed; extraction is available only for Unpin values.
pub struct PinnedValue<'a, T: Send> {
    frame: ErasedFrame<'a>,
    value: PhantomData<T>,
}
// Moving the handle cannot move the independently allocated payload.
impl<T: Send> Unpin for PinnedValue<'_, T> {}
// SAFETY: T is accessed through pin projections; immutable sharing requires Sync.
unsafe impl<T: Send + Sync> Sync for PinnedValue<'_, T> {}
impl<T: Send> PinnedValue<'_, T> {
    #[must_use]
    pub fn as_pin_ref(&self) -> Pin<&T> {
        // SAFETY: frame is initialized as T and cannot be moved until it is dropped
        // in place (or extracted under T: Unpin). This borrow retains the owner.
        unsafe { Pin::new_unchecked(self.frame.pointer().cast::<T>().as_ref()) }
    }
    #[must_use]
    pub fn as_pin_mut(&mut self) -> Pin<&mut T> {
        // SAFETY: same stable initialized address, with unique owner access.
        unsafe { Pin::new_unchecked(self.frame.pointer().cast::<T>().as_mut()) }
    }
    #[must_use]
    pub fn into_inner(mut self) -> T
    where
        T: Unpin,
    {
        // SAFETY: this is the initialized T and Unpin permits moving it. Clearing
        // state prevents double destruction when the empty frame owner drops.
        unsafe { self.frame.take::<T>() }
    }
}
impl<F: Future + Send> Future for PinnedValue<'_, F> {
    type Output = F::Output;
    fn poll(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        self.get_mut().as_pin_mut().poll(context)
    }
}

unsafe fn track_cell(pointer: NonNull<u8>, retained: bool) {
    // SAFETY: installed only for Cell pools; live slot and its allocation are
    // retained by the pool lock. No payload code or destruction occurs here.
    unsafe {
        pointer
            .cast::<Cell<'_>>()
            .as_ref()
            .storage
            .set_retained(retained)
    };
}

impl<'a> PreparedMemory<'a> {
    pub fn frames(
        &self,
        region: RegionId,
        count: usize,
        layout: Layout,
    ) -> Result<FramePool<'a>, MemoryError> {
        self.frames_with_return_listeners(region, count, layout, 0)
    }
    /// Prepare fixed frame cells with a caller-selected number of return listeners.
    pub fn frames_with_return_listeners(
        &self,
        region: RegionId,
        count: usize,
        layout: Layout,
        listeners: usize,
    ) -> Result<FramePool<'a>, MemoryError> {
        let cells = SlotPool::prepare_tracked_with_listeners(
            self,
            region,
            count,
            listeners,
            |_| {
                Ok(Cell {
                    storage: self.allocate(region, layout)?,
                    initialized: UnsafeCell::new(false),
                    destroy: empty,
                    footer_live: UnsafeCell::new(false),
                    footer: empty,
                    node: UnsafeCell::new(DestructionNode::new(
                        destroy_node,
                        footer_node,
                        release_node,
                    )),
                    queued: UnsafeCell::new(MaybeUninit::uninit()),
                })
            },
            track_cell,
        )?;
        Ok(FramePool { cells, layout })
    }
}

/// Pre-admitted payload and footer storage. The footer retires only after the
/// payload and its prepared destruction descendants have retired.
pub struct RetainedValueReservation<T: Send + 'static, G: Send + 'static> {
    frame: FrameReservation<'static, RetainedFrameValue<T, G>>,
}
impl<T: Send + 'static, G: Send + 'static> RetainedValueReservation<T, G> {
    #[must_use]
    pub fn layout() -> Layout {
        Layout::new::<RetainedFrameValue<T, G>>()
    }
    pub fn reserve(frames: &FramePool<'static>) -> Result<Self, MemoryError> {
        Ok(Self {
            frame: frames.reserve()?,
        })
    }
    /// Install both owners without further allocation or admission.
    #[must_use]
    pub fn publish(self, value: T, footer: G) -> RetainedValue<T, G> {
        let frame = self.frame.retain_parts(value, footer);
        // SAFETY: this exact typed frame was initialized immediately above.
        let value = unsafe { RetainedFrameValue::<T, G>::value_pointer(frame.pointer().cast()) };
        RetainedValue {
            value,
            _frame: frame,
            marker: PhantomData,
        }
    }
}
/// Borrow-only payload owner with a destruction-ordered retained footer.
pub struct RetainedValue<T: Send + 'static, G: Send + 'static> {
    value: NonNull<T>,
    _frame: RuntimeFrame,
    marker: PhantomData<(T, G)>,
}
// SAFETY: the frame and payload have unique owned lifetime; both T and G are Send.
unsafe impl<T: Send + 'static, G: Send + 'static> Send for RetainedValue<T, G> {}
// SAFETY: shared access exposes only &T; final frame destruction is exclusive.
unsafe impl<T: Send + Sync + 'static, G: Send + Sync + 'static> Sync for RetainedValue<T, G> {}
impl<T: Send + 'static, G: Send + 'static> std::ops::Deref for RetainedValue<T, G> {
    type Target = T;
    fn deref(&self) -> &T {
        // SAFETY: frame retains the exact initialized payload throughout self.
        unsafe { self.value.as_ref() }
    }
}
