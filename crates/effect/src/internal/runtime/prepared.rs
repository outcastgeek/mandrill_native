//! Explicit prepared native root companions over the SAME Step/drive protocol.
//! No infallible Clone or opaque Effect->prepared claim is introduced.
#![doc = include_str!("PREPARED.md")]
#![doc = include_str!("OUTPUT_OWNERS.md")]
#![doc = include_str!("CONSTRUCTION.md")]
mod bridge;
mod cancellation;
pub use cancellation::{PreparedCancellation, PreparedCancellationSignalWait};
mod construction;
mod context;
mod instrumentation;
mod retention;
pub use instrumentation::{PreparedPollHook, PreparedPollInstrumentation};
pub use construction::{
    ConstructionShape, ConstructionSite, ConstructionVacancy, PreparedConstructionPool,
    construction_site,
};
pub(crate) use retention::{NativeGraphOrigins, OriginSnapshot, OriginSnapshotReservation};
use retention::{OriginGroup, OutputLease, Owners};
pub use retention::{PreparedOutputOwnerReservation, PreparedOutputOwnerView};
#[doc(hidden)]
pub mod graph;
use super::{DriveContext, Finish, PollContext, Publication, Step, drive, publish_flags};
use crate::internal::runtime_storage::{Consume, FutureReservation, Once};
use crate::memory::{
    Bytes, FramePool, MemoryError, MemoryPolicy, PreparedMemory, RegionId, SharedSlot, SlotPool,
};
use crate::{ClockTick, Defect, Exit, LibraryDefect, PreparedTraceContext, TraceContext};
pub use context::PreparedCancellationObservation;
use context::{Domain, Id, Kind, ListenerId};
use std::alloc::Layout;
use std::future::Future;
use std::marker::PhantomData;
use std::ops::Deref;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context as TaskContext, Poll};

pub(crate) fn panic_defect(
    reservation: Option<crate::PreparedPanicReservation>,
    payload: Box<dyn std::any::Any + Send>,
) -> Defect {
    match reservation {
        Some(reservation) => reservation.capture(payload),
        None => Defect::from_panic(payload),
    }
}

/// Checked boundary for this concrete root node. Native dependency allocations
/// still need their own qualified contract. Opaque Effect graphs are refused at
/// configuration, not silently executed on a System backend under a strict name.
#[derive(Clone, Copy, Debug)]
pub enum DynamicRunContract {
    ConcreteFuture {
        clock_ticks: usize,
        contexts: usize,
        listeners: usize,
        bridges: usize,
    },
    GeneratedGraph {
        clock_ticks: usize,
        contexts: usize,
        listeners: usize,
        bridges: usize,
    },
    OpaqueEffect,
}
#[derive(Clone, Copy, Debug)]
pub struct PreparedRunDemand {
    pub frame_layout: Layout,
    pub frames_per_run: usize,
    pub record_payload: Bytes,
    /// Factory or polling panic slot for each admitted run.
    pub panic_sites: usize,
}
struct Root<A: Send + 'static, E: Send + 'static> {
    output: Mutex<Option<Exit<A, E>>>,
    owners: Mutex<Owners>,
    construction: Mutex<Option<construction::Lane>>,
    consumers: AtomicUsize,
    output_readers: AtomicUsize,
    closed: AtomicBool,
    contexts: Domain,
    bridges: bridge::Storage,
    trace: TraceContext,
    instrumentation: Option<PreparedPollInstrumentation>,
    panic_pool: Option<crate::PreparedPanicPool>,
}
impl<A: Send + 'static, E: Send + 'static> Root<A, E> {
    fn take(&self) -> Exit<A, E> {
        self.output
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
            .unwrap_or_else(|| Exit::Die(Defect::from(LibraryDefect::MissingInterpreterOutcome)))
    }
    fn store(&self, value: Exit<A, E>) {
        let old = self
            .output
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .replace(value);
        drop(old);
    }
}
/// Prepared-specific owned context. Copies are checked, not infallible Clone;
/// the ordinary NativeContext Clone/System API is unchanged. Child/protected/
/// parallel/listener graph descriptors remain separate, not implicit System calls.
pub struct PreparedRunContext<A: Send + 'static, E: Send + 'static> {
    root: SharedSlot<'static, Root<A, E>>,
    id: Id,
    origin_group: Option<SharedSlot<'static, OriginGroup>>,
    prepared_trace: Option<PreparedTraceContext>,
}
impl<A: Send + 'static, E: Send + 'static> PreparedRunContext<A, E> {
    fn with_origin_of(&self, selected: &Self) -> Result<Self, MemoryError> {
        if !self.root.same_slot(&selected.root) {
            return Err(MemoryError::UnsupportedRunContract);
        }
        let origin_group = selected.origin_group.as_ref().map(SharedSlot::try_clone).transpose()?;
        let mut context = self.try_clone()?;
        context.origin_group = origin_group;
        Ok(context)
    }
    pub fn try_clone(&self) -> Result<Self, MemoryError> {
        let root = self.root.try_clone()?;
        let origin_group = self
            .origin_group
            .as_ref()
            .map(SharedSlot::try_clone)
            .transpose()?;
        let prepared_trace = self.retain_trace()?;
        root.contexts.retain(self.id)?;
        Ok(Self {
            root,
            id: self.id,
            origin_group,
            prepared_trace,
        })
    }
    fn derive(&self, kind: Kind, cancellation: Option<Id>) -> Result<Self, MemoryError> {
        if self.root.closed.load(Ordering::Acquire) {
            return Err(MemoryError::Closed);
        }
        let root = self.root.try_clone()?;
        let origin_group = self
            .origin_group
            .as_ref()
            .map(SharedSlot::try_clone)
            .transpose()?;
        let prepared_trace = self.retain_trace()?;
        let id = root
            .contexts
            .create(kind, Some(self.id), cancellation, None)?;
        if root.closed.load(Ordering::Acquire) {
            root.contexts.release(id);
            return Err(MemoryError::Closed);
        }
        Ok(Self {
            root,
            id,
            origin_group,
            prepared_trace,
        })
    }
    /// Mask parent cancellation and retain the caller's clock frontier.
    pub fn protected(&self) -> Result<Self, MemoryError> {
        self.derive(Kind::Protected, None)
    }
    /// Fiber metadata inherits cancellation, with its own clock frontier.
    pub fn child(&self) -> Result<Self, MemoryError> {
        self.derive(Kind::Child, None)
    }
    /// Explicit admitted same-root cancellation source; no implicit System bridge.
    pub fn parallel<SA: Send + 'static, SE: Send + 'static>(
        &self,
        cancellation: &PreparedRunContext<SA, SE>,
    ) -> Result<Self, MemoryError> {
        self.derive_remote(Kind::Parallel, cancellation, true)
    }
    /// A child in this root with a retained cross-root parent cancellation
    /// origin. Requests remain local; the parent source is never requested.
    pub fn child_from<SA: Send + 'static, SE: Send + 'static>(
        &self,
        parent: &PreparedRunContext<SA, SE>,
    ) -> Result<Self, MemoryError> {
        self.derive_remote(Kind::Child, parent, true)
    }
    /// Mask cancellation while retaining an explicitly admitted foreign clock.
    pub fn protected_from<SA: Send + 'static, SE: Send + 'static>(
        &self,
        parent: &PreparedRunContext<SA, SE>,
    ) -> Result<Self, MemoryError> {
        self.derive_remote(Kind::Protected, parent, false)
    }
    fn derive_remote<SA: Send + 'static, SE: Send + 'static>(
        &self,
        kind: Kind,
        source: &PreparedRunContext<SA, SE>,
        cancellation: bool,
    ) -> Result<Self, MemoryError> {
        if self.root.closed.load(Ordering::Acquire) {
            return Err(MemoryError::Closed);
        }
        let root = self.root.try_clone()?;
        let origin_group = self
            .origin_group
            .as_ref()
            .map(SharedSlot::try_clone)
            .transpose()?;
        let prepared_trace = self.retain_trace()?;
        let id = if self.root.same_slot(&source.root) {
            let parent = if matches!(kind, Kind::Protected) {
                source.id
            } else {
                self.id
            };
            root.contexts
                .create(kind, Some(parent), Some(source.id), None)?
        } else {
            let remote = bridge::prepare(self, source, cancellation)?;
            root.contexts
                .create(kind, Some(self.id), None, Some(remote))?
        };
        let context = Self {
            root,
            id,
            origin_group,
            prepared_trace,
        };
        if context.root.closed.load(Ordering::Acquire) {
            return Err(MemoryError::Closed);
        }
        Ok(context)
    }
    pub(crate) fn reserve_panic(
        &self,
    ) -> Result<Option<crate::PreparedPanicReservation>, MemoryError> {
        self.root
            .panic_pool
            .as_ref()
            .map(crate::PreparedPanicPool::try_reserve)
            .transpose()
    }
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.root.contexts.cancelled(self.id)
    }
    #[must_use]
    pub fn request_cancellation(&self) -> bool {
        self.root.contexts.request(self.id)
    }
    #[must_use]
    pub fn trace(&self) -> &TraceContext {
        &self.root.trace
    }
    /// Borrow this run's physically retained prepared snapshot.
    #[must_use]
    pub fn prepared_trace(&self) -> Option<&PreparedTraceContext> {
        self.prepared_trace.as_ref()
    }
    fn retain_trace(&self) -> Result<Option<PreparedTraceContext>, MemoryError> {
        self.prepared_trace().map(PreparedTraceContext::try_clone).transpose()
    }
    pub fn retain_tick(&self, tick: ClockTick) -> Result<(), (MemoryError, ClockTick)> {
        self.root.contexts.retain_tick(self.id, tick)
    }
    pub fn cancelled(&self) -> Result<PreparedCancellationWait<A, E>, MemoryError> {
        let context = self.try_clone()?;
        let listener = self.root.contexts.listener(self.id)?;
        Ok(PreparedCancellationWait {
            context,
            listener: Some(listener),
            result: None,
        })
    }
    /// Same publication rules with explicit prepared ownership.
    pub fn publish<T>(&self, outcome: Exit<T, E>) -> Exit<T, E> {
        let (fiber, protected) = self.root.contexts.flags(self.id);
        publish_flags(outcome, self.is_cancelled(), fiber, protected)
    }
    pub fn record_protected_failure<T>(&self, outcome: &Exit<T, E>) {
        self.root
            .contexts
            .protected_failure(self.id, matches!(outcome, Exit::Fail(_)));
    }
    /// Construct an inline typed native child. Its factory context is acquired
    /// before any poll/invocation; the parent Future's compiler layout includes
    /// every inline child capture. This is not a detached native task spawn.
    pub fn native<T, F, P>(self, factory: F) -> PreparedNativeAdmission<T, A, E, F, P>
    where
        F: FnOnce(Self) -> P,
        P: Future<Output = Exit<T, E>>,
    {
        match self.reserve_native() {
            Ok(reservation) => Ok(reservation.native(factory)),
            Err(refusal) => Err(refusal),
        }
    }
    /// Obtain every child execution owner during the root's pre-body storage
    /// recipe. Returning this token in L couples real ownership to graph demand.
    pub fn reserve_native(self) -> Result<PreparedNativeReservation<A, E>, (MemoryError, Self)> {
        let factory_context = match self.try_clone() {
            Ok(context) => context,
            Err(error) => return Err((error, self)),
        };
        Ok(PreparedNativeReservation {
            factory_context,
            context: self,
        })
    }
}
/// Complete metadata execution reservation; no callback/capture is invoked here.
pub struct PreparedNativeReservation<A: Send + 'static, E: Send + 'static> {
    factory_context: PreparedRunContext<A, E>,
    context: PreparedRunContext<A, E>,
}
impl<A: Send + 'static, E: Send + 'static> PreparedNativeReservation<A, E> {
    pub fn native<T, F, P>(self, factory: F) -> PreparedNativeFuture<T, A, E, F, P>
    where
        F: FnOnce(PreparedRunContext<A, E>) -> P,
        P: Future<Output = Exit<T, E>>,
    {
        PreparedNativeFuture {
            factory: Some(factory),
            pending: None,
            done: false,
            factory_context: Some(self.factory_context),
            context: self.context,
        }
    }
}
/// Admission refuses before invoking or polling the child factory.
pub type PreparedNativeAdmission<T, A, E, F, P> =
    Result<PreparedNativeFuture<T, A, E, F, P>, (MemoryError, PreparedRunContext<A, E>)>;
/// Inline native sub-operation, not a second interpreter. Its owned context and
/// captured Future remain live through actual Ready/drop. Panics continue to the
/// existing enclosing prepared callback boundary; bounded panic diagnostics there
/// remain a separately tracked unfinished obligation.
pub struct PreparedNativeFuture<
    T,
    A: Send + 'static,
    E: Send + 'static,
    F,
    P: Future<Output = Exit<T, E>>,
> {
    // Rust drops fields in declaration order. User captures/pinned Future must
    // finish destruction before either prepared node/root owner can be released.
    factory: Option<F>,
    pending: Option<P>,
    done: bool,
    factory_context: Option<PreparedRunContext<A, E>>,
    context: PreparedRunContext<A, E>,
}
impl<T, A: Send + 'static, E: Send + 'static, F, P> Future for PreparedNativeFuture<T, A, E, F, P>
where
    F: FnOnce(PreparedRunContext<A, E>) -> P,
    P: Future<Output = Exit<T, E>>,
{
    type Output = Exit<T, E>;
    fn poll(self: Pin<&mut Self>, cx: &mut TaskContext<'_>) -> Poll<Self::Output> {
        // SAFETY: only pending's initialized P is structurally pinned. It is
        // installed once from None and never moved thereafter. Context/factory
        // options are unpinned ownership handles, not projections into P.
        let this = unsafe { self.get_unchecked_mut() };
        if this.done {
            return Poll::Pending;
        }
        if this.pending.is_none() {
            if this.context.is_cancelled() {
                this.done = true;
                return Poll::Ready(Exit::Interrupted);
            }
            let (Some(factory), Some(context)) = (this.factory.take(), this.factory_context.take())
            else {
                this.done = true;
                return Poll::Ready(Exit::Die(Defect::from(
                    LibraryDefect::FutureFactoryConsumed,
                )));
            };
            this.context
                .root
                .contexts
                .protected_failure(this.context.id, false);
            this.pending = Some(factory(context));
        }
        let Some(pending) = this.pending.as_mut() else {
            return Poll::Pending;
        };
        match unsafe { Pin::new_unchecked(pending) }.poll(cx) {
            Poll::Pending => {
                this.context.acknowledge_clock();
                Poll::Pending
            }
            Poll::Ready(outcome) => {
                this.done = true;
                this.context.acknowledge_clock();
                Poll::Ready(this.context.publish(outcome))
            }
        }
    }
}
/// Joining consumes both already admitted inline children and waits for both
/// native Ready outcomes; observer cancellation/drop is not an acknowledgment.
pub async fn join_prepared_native<T, U, A: Send + 'static, E: Send + 'static, F, G, P, Q>(
    left: PreparedNativeFuture<T, A, E, F, P>,
    right: PreparedNativeFuture<U, A, E, G, Q>,
) -> (Exit<T, E>, Exit<U, E>)
where
    F: FnOnce(PreparedRunContext<A, E>) -> P,
    G: FnOnce(PreparedRunContext<A, E>) -> Q,
    P: Future<Output = Exit<T, E>>,
    Q: Future<Output = Exit<U, E>>,
{
    futures_util::future::join(left, right).await
}
impl<A: Send + 'static, E: Send + 'static> Drop for PreparedRunContext<A, E> {
    fn drop(&mut self) {
        self.root.contexts.release(self.id);
    }
}
impl<A: Send + 'static, E: Send + 'static> DriveContext for PreparedRunContext<A, E> {
    fn poll_context(&self) -> PollContext {
        PollContext {
            trace: self.root.trace.clone(),
            instrumentation: None,
        }
    }
    fn prepared_instrumentation(
        &self,
    ) -> Option<(&PreparedPollInstrumentation, Option<&PreparedTraceContext>)> {
        Some((self.root.instrumentation.as_ref()?, self.prepared_trace()))
    }
    fn acknowledge_clock(&self) {
        self.root.contexts.acknowledge(self.id);
    }
}
/// Physically reserved observation; completion/drop never acknowledges native work.
pub struct PreparedCancellationWait<A: Send + 'static, E: Send + 'static> {
    context: PreparedRunContext<A, E>,
    listener: Option<ListenerId>,
    result: Option<PreparedCancellationObservation>,
}
impl<A: Send + 'static, E: Send + 'static> Unpin for PreparedCancellationWait<A, E> {}
impl<A: Send + 'static, E: Send + 'static> Future for PreparedCancellationWait<A, E> {
    type Output = PreparedCancellationObservation;
    fn poll(self: Pin<&mut Self>, cx: &mut TaskContext<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        if let Some(result) = this.result {
            return Poll::Ready(result);
        }
        let Some(listener) = this.listener else {
            return Poll::Pending;
        };
        match this
            .context
            .root
            .contexts
            .observe(this.context.id, listener, cx.waker())
        {
            Some(result) => {
                this.context.root.contexts.unlink(listener);
                this.listener = None;
                this.result = Some(result);
                Poll::Ready(result)
            }
            None => Poll::Pending,
        }
    }
}
impl<A: Send + 'static, E: Send + 'static> Drop for PreparedCancellationWait<A, E> {
    fn drop(&mut self) {
        if let Some(listener) = self.listener.take() {
            self.context.root.contexts.unlink(listener);
        }
    }
}
struct Loss<A: Send + 'static, E: Send + 'static> {
    root: SharedSlot<'static, Root<A, E>>,
}
impl<A: Send + 'static, E: Send + 'static> Drop for Loss<A, E> {
    fn drop(&mut self) {
        self.root.closed.store(true, Ordering::Release);
        self.root.contexts.close();
    }
}
// Reader authority is separate from lifetime-only output origin consumers.
// The claimed sentinel prevents a new reader after owned transfer begins.
const OUTPUT_CLAIMED: usize = usize::MAX;
struct OutputReader<A: Send + 'static, E: Send + 'static> {
    root: SharedSlot<'static, Root<A, E>>,
    armed: bool,
}
impl<A: Send + 'static, E: Send + 'static> OutputReader<A, E> {
    fn try_clone(&self) -> Result<Self, MemoryError> {
        let root = self.root.try_clone()?;
        root.output_readers
            .try_update(Ordering::AcqRel, Ordering::Acquire, |count| {
                if count == 0 || count >= OUTPUT_CLAIMED - 1 {
                    None
                } else {
                    Some(count + 1)
                }
            })
            .map_err(|count| {
                if count == 0 || count == OUTPUT_CLAIMED {
                    MemoryError::Closed
                } else {
                    MemoryError::OwnerLimit
                }
            })?;
        Ok(Self { root, armed: true })
    }
    fn claim(&mut self) -> bool {
        if self
            .root
            .output_readers
            .compare_exchange(1, OUTPUT_CLAIMED, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            self.armed = false;
            true
        } else {
            false
        }
    }
}
impl<A: Send + 'static, E: Send + 'static> Drop for OutputReader<A, E> {
    fn drop(&mut self) {
        if self.armed {
            self.root.output_readers.fetch_sub(1, Ordering::AcqRel);
        }
    }
}
/// Root output remains charged until the last consumer. Contexts/cancellation
/// observers/pending continuations independently retain the same generation.
pub struct PreparedOutput<A: Send + 'static, E: Send + 'static> {
    reader: OutputReader<A, E>,
    root: OutputLease<A, E>,
}
/// Three physically retained output consumers reserved before business polling.
pub struct ReservedOutputOrigins<A: Send + 'static, E: Send + 'static> {
    first: OutputLease<A, E>,
    second: OutputLease<A, E>,
    third: OutputLease<A, E>,
}
impl<A: Send + 'static, E: Send + 'static> PreparedOutput<A, E> {
    /// Transfer an owned outcome together with the exact pre-reserved consumers.
    /// # Safety
    /// Moved outcome values must remain attached to at least one returned lease
    /// through their actual destruction and prepared descendants.
    #[allow(clippy::result_large_err, clippy::type_complexity)]
    pub unsafe fn map_owned_with_origins<T>(
        mut self,
        origins: ReservedOutputOrigins<A, E>,
        map: impl FnOnce(Exit<A, E>) -> T,
    ) -> Result<
        (LeasedRunResult<T, A, E>, [LeasedRunResult<(), A, E>; 3]),
        (Self, ReservedOutputOrigins<A, E>),
    > {
        let same = self.root.root.same_slot(&origins.first.root)
            && self.root.root.same_slot(&origins.second.root)
            && self.root.root.same_slot(&origins.third.root);
        if !same || !self.reader.claim() {
            return Err((self, origins));
        }
        let root = self.root;
        let value = map(root.take());
        Ok((
            LeasedRunResult { value, root },
            [
                LeasedRunResult {
                    value: (),
                    root: origins.first,
                },
                LeasedRunResult {
                    value: (),
                    root: origins.second,
                },
                LeasedRunResult {
                    value: (),
                    root: origins.third,
                },
            ],
        ))
    }
    pub fn try_clone(&self) -> Result<Self, MemoryError> {
        let root = self.root.try_clone()?;
        let reader = self.reader.try_clone()?;
        Ok(Self { reader, root })
    }
    pub fn with<T>(&self, access: impl FnOnce(&Exit<A, E>) -> T) -> Result<T, Defect> {
        let value = self
            .root
            .output
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        value
            .as_ref()
            .map(access)
            .ok_or_else(|| Defect::from(LibraryDefect::MissingInterpreterOutcome))
    }
    /// Borrow the original outcome while producing a consumer-coupled derived
    /// value. Safe callbacks never receive the original owned Exit.
    pub fn map<T>(
        self,
        map: impl FnOnce(&Exit<A, E>) -> T,
    ) -> Result<LeasedRunResult<T, A, E>, Self> {
        if self.root.consumers.load(Ordering::Acquire) != 1 {
            return Err(self);
        }
        let value = match self.with(map) {
            Ok(value) => value,
            Err(_) => return Err(self),
        };
        Ok(LeasedRunResult {
            value,
            root: self.root,
        })
    }
    /// Transform an owned outcome under an explicit external ownership contract.
    /// # Safety
    /// The callback must not detach the original outcome/pointees from the
    /// returned consumer. Every moved value must be destroyed/settled before
    /// that consumer retires, or independently retain its actual origins. Sending
    /// the original elsewhere and returning unit does NOT satisfy this contract.
    pub unsafe fn map_owned_unchecked<T>(
        mut self,
        map: impl FnOnce(Exit<A, E>) -> T,
    ) -> Result<LeasedRunResult<T, A, E>, Self> {
        if self.root.consumers.load(Ordering::Acquire) != 1 || !self.reader.claim() {
            return Err(self);
        }
        let root = self.root;
        let value = map(root.take());
        Ok(LeasedRunResult { value, root })
    }
}

pub struct LeasedRunResult<T, A: Send + 'static, E: Send + 'static> {
    value: T,
    root: OutputLease<A, E>,
}
impl<T, A: Send + 'static, E: Send + 'static> Deref for LeasedRunResult<T, A, E> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.value
    }
}
impl<T, A: Send + 'static, E: Send + 'static> LeasedRunResult<T, A, E> {
    /// Retain the same prepared origin without copying or exposing this value.
    pub fn try_retain(&self) -> Result<LeasedRunResult<(), A, E>, MemoryError> {
        Ok(LeasedRunResult {
            value: (),
            root: self.root.try_clone()?,
        })
    }
    /// # Safety
    /// The returned lease must remain attached to the moved value and its
    /// prepared descendants until their actual destruction or independent ownership.
    pub unsafe fn into_parts_unchecked(self) -> (T, LeasedRunResult<(), A, E>) {
        (
            self.value,
            LeasedRunResult {
                value: (),
                root: self.root,
            },
        )
    }
    /// Safe derived mapping borrows, rather than giving away, the owned value.
    pub fn map<U>(self, map: impl FnOnce(&T) -> U) -> LeasedRunResult<U, A, E> {
        LeasedRunResult {
            value: map(&self.value),
            root: self.root,
        }
    }
    /// # Safety
    /// All original owned T/pointees must remain coupled to the resulting
    /// consumer until actual destruction/settlement, or carry independent actual
    /// origin ownership. The callback must not send T elsewhere and return unit.
    pub unsafe fn map_owned_unchecked<U>(
        self,
        map: impl FnOnce(T) -> U,
    ) -> LeasedRunResult<U, A, E> {
        LeasedRunResult {
            value: map(self.value),
            root: self.root,
        }
    }
}
/// The consumer and its Ready output each need ownership. Acquire the transfer
/// handle BEFORE any poll, not as a fallible clone after the business result.
pub struct LeasedRunFuture<F: Future, A: Send + 'static, E: Send + 'static> {
    value: F,
    _retained_root: OutputLease<A, E>,
    transfer: Option<OutputLease<A, E>>,
}
impl<F: Future, A: Send + 'static, E: Send + 'static> LeasedRunResult<F, A, E> {
    pub fn prepare_future(self) -> Result<LeasedRunFuture<F, A, E>, (MemoryError, Self)> {
        let transfer = match self.root.try_clone() {
            Ok(root) => root,
            Err(error) => return Err((error, self)),
        };
        Ok(LeasedRunFuture {
            value: self.value,
            _retained_root: self.root,
            transfer: Some(transfer),
        })
    }
}
impl<F: Future, A: Send + 'static, E: Send + 'static> Future for LeasedRunFuture<F, A, E> {
    type Output = LeasedRunResult<F::Output, A, E>;
    fn poll(self: Pin<&mut Self>, context: &mut TaskContext<'_>) -> Poll<Self::Output> {
        // SAFETY: only value is structurally pinned, never moved by polling. Both
        // original and pre-held transfer owner survive Ready until their consumers
        // drop; completed F captures are not prematurely uncharged.
        let this = unsafe { self.get_unchecked_mut() };
        if this.transfer.is_none() {
            return Poll::Pending;
        }
        match unsafe { Pin::new_unchecked(&mut this.value) }.poll(context) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(value) => match this.transfer.take() {
                Some(root) => Poll::Ready(LeasedRunResult { value, root }),
                None => Poll::Pending,
            },
        }
    }
}
struct Store<A: Send + 'static, E: Send + 'static> {
    root: SharedSlot<'static, Root<A, E>>,
}
impl<A: Send + 'static, E: Send + 'static> Consume<Exit<A, E>, Step> for Store<A, E> {
    fn consume(self, outcome: Exit<A, E>) -> Step {
        self.root.store(outcome);
        Step::Done
    }
}
struct Start<A, E, R, F, P, G, L>
where
    A: Send + 'static,
    E: Send + 'static,
    R: Send + Sync + 'static,
    F: Send + Sync + 'static,
    G: Send + Sync + 'static,
    L: Send + 'static,
    P: Future<Output = Exit<A, E>> + Send + 'static,
{
    recipe: SharedSlot<'static, Recipe<F, G>>,
    environment: Arc<R>,
    input: L,
    context: PreparedRunContext<A, E>,
    reservation: FutureReservation<P, A, E>,
    finish: Finish<A, E>,
}
impl<A, E, R, F, P, G, L> Consume<(), Step> for Start<A, E, R, F, P, G, L>
where
    A: Send + 'static,
    E: Send + 'static,
    R: Send + Sync + 'static,
    F: Fn(Arc<R>, PreparedRunContext<A, E>, L) -> P + Send + Sync + 'static,
    G: Fn(&Arc<R>, &PreparedRunContext<A, E>) -> Result<L, MemoryError> + Send + Sync + 'static,
    L: Send + 'static,
    P: Future<Output = Exit<A, E>> + Send + 'static,
{
    fn consume(self, (): ()) -> Step {
        if self.context.is_cancelled() {
            return self.reservation.complete(Exit::Interrupted, self.finish);
        }
        self.reservation.start(
            move || (self.recipe.factory)(self.environment, self.context, self.input),
            self.finish,
        )
    }
}
type RunTypes<A, E, R, P, L> = fn() -> (A, E, R, P, L);
type Preparation<A, E, R, F, P, G, L> = Result<PreparedRun<A, E, R, F, P, G, L>, MemoryError>;
struct Recipe<F, G> {
    factory: F,
    reserve: G,
}
pub struct PreparedRunPlan<A, E, R, F, P, G, L> {
    factory: F,
    reserve: G,
    contract: DynamicRunContract,
    panic_pool: Option<crate::PreparedPanicPool>,
    prepared_trace: Option<PreparedTraceContext>,
    instrumentation: Option<PreparedPollInstrumentation>,
    marker: PhantomData<RunTypes<A, E, R, P, L>>,
}
pub struct PreparedRun<
    A: Send + 'static,
    E: Send + 'static,
    R: Send + Sync + 'static,
    F: Send + Sync + 'static,
    P,
    G: Send + Sync + 'static,
    L,
> {
    rows: SlotPool<'static, Root<A, E>>,
    recipe: SharedSlot<'static, Recipe<F, G>>,
    frames: FramePool<'static>,
    policy: MemoryPolicy,
    prepared_trace: Option<PreparedTraceContext>,
    marker: PhantomData<fn(Arc<R>, L) -> P>,
}
pub struct PreparedInvocation<A: Send + 'static, E: Send + 'static> {
    step: Step,
    context: PreparedRunContext<A, E>,
    output: PreparedOutput<A, E>,
    loss: Loss<A, E>,
}
/// Nameable owned driver for native provider/transport admission. Payloads stay
/// in their existing region frames; this is the SAME one-transition poll_drive,
/// not another runtime or an ordinary Effect adapter.
pub struct PreparedDriver<A: Send + 'static, E: Send + 'static> {
    invocation: Option<PreparedInvocation<A, E>>,
    poll_context: PollContext,
}
impl<A: Send, E: Send> Unpin for PreparedDriver<A, E> {}
impl<A: Send + 'static, E: Send + 'static> Future for PreparedDriver<A, E> {
    type Output = PreparedOutput<A, E>;
    fn poll(self: Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        let Some(source) = this.invocation.as_mut() else {
            return Poll::Pending;
        };
        let PreparedInvocation {
            step,
            context,
            output,
            ..
        } = source;
        if super::poll_drive(
            step,
            context,
            &mut this.poll_context,
            Publication::Publish,
            &mut || {
                let value = output.root.take();
                output.root.store(context.publish(value));
            },
            cx,
        )
        .is_pending()
        {
            return Poll::Pending;
        }
        let Some(source) = this.invocation.take() else {
            return Poll::Pending;
        };
        let PreparedInvocation {
            step,
            context,
            output,
            loss,
        } = source;
        drop(loss);
        drop(step);
        drop(context);
        Poll::Ready(output)
    }
}
/// Unique driver claims its output reader before business polling.
#[doc(hidden)]
pub struct PreparedOwnedDriver<A: Send + 'static, E: Send + 'static> {
    driver: PreparedDriver<A, E>,
    origins: Option<ReservedOutputOrigins<A, E>>,
}
impl<A: Send, E: Send> Unpin for PreparedOwnedDriver<A, E> {}
impl<A: Send + 'static, E: Send + 'static> Future for PreparedOwnedDriver<A, E> {
    type Output = (
        LeasedRunResult<Exit<A, E>, A, E>,
        [LeasedRunResult<(), A, E>; 3],
    );
    fn poll(self: Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        if this.origins.is_none() {
            return Poll::Pending;
        }
        let output = match Pin::new(&mut this.driver).poll(cx) {
            Poll::Pending => return Poll::Pending,
            Poll::Ready(output) => output,
        };
        let Some(origins) = this.origins.take() else {
            return Poll::Pending;
        };
        let root = output.root;
        let value = root.take();
        Poll::Ready((
            LeasedRunResult { value, root },
            [
                LeasedRunResult {
                    value: (),
                    root: origins.first,
                },
                LeasedRunResult {
                    value: (),
                    root: origins.second,
                },
                LeasedRunResult {
                    value: (),
                    root: origins.third,
                },
            ],
        ))
    }
}
impl<A: Send + 'static, E: Send + 'static> PreparedInvocation<A, E> {
    /// Reserve three output origins before the invocation is polled.
    pub fn reserve_output_origins(&self) -> Result<ReservedOutputOrigins<A, E>, MemoryError> {
        Ok(ReservedOutputOrigins {
            first: self.output.root.try_clone()?,
            second: self.output.root.try_clone()?,
            third: self.output.root.try_clone()?,
        })
    }
    /// Claim the unique output reader and its exact pre-reserved origins before
    /// polling. Refusal returns both owners without starting business work.
    #[doc(hidden)]
    #[allow(clippy::result_large_err)]
    pub fn into_owned_driver(
        mut self,
        origins: ReservedOutputOrigins<A, E>,
    ) -> Result<PreparedOwnedDriver<A, E>, (Self, ReservedOutputOrigins<A, E>)> {
        let root = &self.output.root.root;
        if !root.same_slot(&origins.first.root)
            || !root.same_slot(&origins.second.root)
            || !root.same_slot(&origins.third.root)
            || !self.output.reader.claim()
        {
            return Err((self, origins));
        }
        Ok(PreparedOwnedDriver {
            driver: self.into_driver(),
            origins: Some(origins),
        })
    }
    /// Move admitted control into a nameable future without allocating/growing.
    #[must_use]
    pub fn into_driver(self) -> PreparedDriver<A, E> {
        let poll_context = self.context.poll_context();
        PreparedDriver {
            invocation: Some(self),
            poll_context,
        }
    }
    pub fn context(&self) -> Result<PreparedRunContext<A, E>, MemoryError> {
        self.context.try_clone()
    }
    pub async fn run(self) -> PreparedOutput<A, E> {
        let Self {
            step,
            context,
            output,
            loss,
        } = self;
        drive(step, &context, Publication::Publish, || {
            let value = output.root.take();
            output.root.store(context.publish(value));
        })
        .await;
        drop(loss);
        output
    }
}
impl<A, E, R, F, P, G, L> PreparedRunPlan<A, E, R, F, P, G, L>
where
    A: Send + 'static,
    E: Send + 'static,
    R: Send + Sync + 'static,
    F: Fn(Arc<R>, PreparedRunContext<A, E>, L) -> P + Send + Sync + 'static,
    G: Fn(&Arc<R>, &PreparedRunContext<A, E>) -> Result<L, MemoryError> + Send + Sync + 'static,
    L: Send + 'static,
    P: Future<Output = Exit<A, E>> + Send + 'static,
{
    pub fn new(factory: F, reserve: G, contract: DynamicRunContract) -> Self {
        Self {
            factory,
            reserve,
            contract,
            panic_pool: None,
            prepared_trace: None,
            instrumentation: None,
            marker: PhantomData,
        }
    }
    /// Supply explicitly prepared diagnostics for this concrete-future plan.
    #[must_use]
    pub fn with_panic_diagnostics(mut self, pool: crate::PreparedPanicPool) -> Self {
        self.panic_pool = Some(pool);
        self
    }
    /// Retain a default trace snapshot for invocations of this plan.
    #[must_use]
    pub fn with_prepared_trace(mut self, trace: PreparedTraceContext) -> Self {
        self.prepared_trace = Some(trace);
        self
    }
    /// Select a startup-prepared hook for each concrete root transition.
    #[must_use]
    pub fn with_prepared_poll_instrumentation(mut self, hook: PreparedPollInstrumentation) -> Self {
        self.instrumentation = Some(hook);
        self
    }
    pub fn demand(&self, capacity: usize) -> Result<PreparedRunDemand, MemoryError> {
        let DynamicRunContract::ConcreteFuture {
            clock_ticks,
            contexts,
            listeners,
            bridges,
        } = self.contract
        else {
            return Err(MemoryError::UnsupportedRunContract);
        };
        let (pending, completion) = FutureReservation::<P, A, E>::layouts();
        let layouts = [
            pending,
            completion,
            Layout::new::<Start<A, E, R, F, P, G, L>>(),
            Layout::new::<Store<A, E>>(),
            PreparedOutputOwnerReservation::<SharedSlot<'static, Recipe<F, G>>, A, E>::layout(),
            PreparedOutputOwnerReservation::<Arc<R>, A, E>::layout(),
        ];
        let size = layouts.iter().map(Layout::size).max().unwrap_or(0);
        let align = layouts.iter().map(Layout::align).max().unwrap_or(1);
        let frame_layout = Layout::from_size_align(size, align)
            .map_err(|_| MemoryError::InvalidLayout)?
            .pad_to_align();
        let bridge_payload = bridge::Storage::payload::<A, E>(bridges)?.checked_mul(capacity)?;
        let contexts_payload =
            Domain::payload(contexts, listeners, clock_ticks)?.checked_mul(capacity)?;
        Ok(PreparedRunDemand {
            frame_layout,
            frames_per_run: 6,
            panic_sites: 1,
            record_payload: SlotPool::<Root<A, E>>::storage_bytes(capacity)?
                .checked_add(SlotPool::<Recipe<F, G>>::storage_bytes(1)?)?
                .checked_add(contexts_payload)?
                .checked_add(bridge_payload)?,
        })
    }

    pub fn prepare(
        self,
        memory: &PreparedMemory<'static>,
        region: RegionId,
        capacity: usize,
        frames: FramePool<'static>,
    ) -> Preparation<A, E, R, F, P, G, L> {
        let demand = self.demand(capacity)?;
        let needed = demand
            .panic_sites
            .checked_mul(capacity)
            .ok_or(MemoryError::ArithmeticOverflow)?;
        if let Some(pool) = &self.panic_pool
            && pool.capacity() < needed
        {
            return Err(MemoryError::NoSlot);
        }
        let DynamicRunContract::ConcreteFuture {
            clock_ticks,
            contexts,
            listeners,
            bridges,
        } = self.contract
        else {
            return Err(MemoryError::UnsupportedRunContract);
        };
        let rows = memory.slots(region, capacity, |_| {
            Ok(Root {
                output: Mutex::new(None),
                owners: Mutex::new(Owners::new()),
                construction: Mutex::new(None),
                consumers: AtomicUsize::new(0),
                output_readers: AtomicUsize::new(0),
                closed: AtomicBool::new(true),
                trace: TraceContext::default(),
                instrumentation: self
                    .instrumentation
                    .as_ref()
                    .map(PreparedPollInstrumentation::try_clone)
                    .transpose()?,
                panic_pool: self
                    .panic_pool
                    .as_ref()
                    .map(crate::PreparedPanicPool::try_clone)
                    .transpose()?,
                contexts: Domain::prepare(memory, region, contexts, listeners, clock_ticks)?,
                bridges: bridge::Storage::prepare::<A, E>(memory, region, bridges)?,
            })
        })?;
        let mut factory = Some(Recipe {
            factory: self.factory,
            reserve: self.reserve,
        });
        let recipes = memory.slots(region, 1, |_| factory.take().ok_or(MemoryError::NoSlot))?;
        Ok(PreparedRun {
            rows,
            recipe: recipes.try_checkout()?.share(),
            frames,
            policy: memory.policy(),
            prepared_trace: self.prepared_trace,
            marker: PhantomData,
        })
    }
}
impl<A, E, R, F, P, G, L> PreparedRun<A, E, R, F, P, G, L>
where
    A: Send + 'static,
    E: Send + 'static,
    R: Send + Sync + 'static,
    F: Fn(Arc<R>, PreparedRunContext<A, E>, L) -> P + Send + Sync + 'static,
    G: Fn(&Arc<R>, &PreparedRunContext<A, E>) -> Result<L, MemoryError> + Send + Sync + 'static,
    L: Send + 'static,
    P: Future<Output = Exit<A, E>> + Send + 'static,
{
    #[must_use]
    pub fn policy(&self) -> MemoryPolicy {
        self.policy
    }
    #[must_use]
    pub fn active(&self) -> usize {
        self.rows.active()
    }
    pub fn close(&self) {
        self.rows.close();
    }
    pub fn admit(&self, environment: Arc<R>) -> Result<PreparedInvocation<A, E>, MemoryError> {
        self.admit_using_trace(environment, self.prepared_trace.as_ref())
    }
    /// Retain a caller-prepared trace for this invocation in place of the default.
    pub fn admit_with_prepared_trace(
        &self,
        environment: Arc<R>,
        trace: &PreparedTraceContext,
    ) -> Result<PreparedInvocation<A, E>, MemoryError> {
        self.admit_using_trace(environment, Some(trace))
    }
    fn admit_using_trace(
        &self,
        environment: Arc<R>,
        trace: Option<&PreparedTraceContext>,
    ) -> Result<PreparedInvocation<A, E>, MemoryError> {
        let recipe = self.recipe.try_clone()?;
        let mut row = self.rows.try_checkout()?;
        // Unique physical row, no context/cancel/output/pending owner from an old
        // generation remains. Complete invocation-local state is reset here.
        row.closed.store(true, Ordering::Release);
        let old_output = row
            .output
            .get_mut()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        if row.bridges.in_use() {
            return Err(MemoryError::Busy);
        }
        row.contexts.reset()?;
        drop(old_output);
        row.trace = TraceContext::default();
        row.closed.store(false, Ordering::Relaxed);
        *row.consumers.get_mut() = 1;
        *row.output_readers.get_mut() = 1;
        let reservation = FutureReservation::<P, A, E>::reserve(&self.frames)?;
        let start = self.frames.reserve::<Start<A, E, R, F, P, G, L>>()?;
        let store = self.frames.reserve::<Store<A, E>>()?;
        let prepared_trace = trace.map(PreparedTraceContext::try_clone).transpose()?;
        let id = row.contexts.create(Kind::Root, None, None, None)?;
        // Install the first node's Drop owner with an infallible transfer of the
        // already-owned row. Every later context copy is explicitly checked.
        let context = PreparedRunContext {
            root: row.share(),
            id,
            origin_group: None,
            prepared_trace,
        };
        let reservation = reservation.with_panic(context.reserve_panic()?);
        let root = context.root.try_clone()?;
        let factory_context = context.try_clone()?;
        let callback = root.try_clone()?;
        let loss = Loss {
            root: root.try_clone()?,
        };
        let output = PreparedOutput {
            reader: OutputReader {
                root: context.root.try_clone()?,
                armed: true,
            },
            root: OutputLease { root },
        };
        let recipe_owner =
            context.reserve_output_owner::<SharedSlot<'static, Recipe<F, G>>>(&self.frames)?;
        recipe_owner
            .attach(recipe.try_clone()?)
            .map_err(|(error, _)| error)?;
        let environment_owner = context.reserve_output_owner::<Arc<R>>(&self.frames)?;
        environment_owner
            .attach(Arc::clone(&environment))
            .map_err(|(error, _)| error)?;
        // Explicit physical data/foreign-resource reservation contract. The
        // factory is never invoked when this checked preflight refuses.
        let input = (recipe.reserve)(&environment, &context)?;
        let finish = Once::prepared(store, Store { root: callback });
        let step = Step::More(Once::prepared(
            start,
            Start {
                recipe,
                environment,
                input,
                context: factory_context,
                reservation,
                finish,
            },
        ));
        Ok(PreparedInvocation {
            step,
            context,
            output,
            loss,
        })
    }
}
