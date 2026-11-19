//! Native parent/service drain integrated with the typed physical backend.
use effect::memory::{FramePool, FrameReservation, MemoryError, PreparedMemory, RegionId};
use effect::prepared_provider::{
    PreparedCancellation, PreparedExecutionLease, PreparedProvider, PreparedReceipt,
    PreparedSubmission, PreparedSubmitError, PreparedSubmitter, ProviderBudget,
    ProviderNativeEnvelope, ProviderRefusal, ProviderStartupError, ProviderWorkerContext,
    ProviderWorkingSet,
};
use native_async_runtime::{AdmissionError, PreparedOperation, RuntimeHandle};
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

#[derive(Clone)]
struct NativeContext(RuntimeHandle);
impl ProviderWorkerContext for NativeContext {
    type Guard<'a> = native_async_runtime::PreparedRuntimeGuard<'a>;
    fn enter(&self) -> Self::Guard<'_> {
        self.0.enter_prepared_worker()
    }
}

/// The create callback runs on the native-compatible worker, not on the
/// submitting thread. Its concrete future is initialized in place on first
/// poll and never moved after pinning. The separate execution lease retains
/// native drain through future/capture drop and completion notification,
/// independently of a cancelled/dropped receiver or retained result owner.
pub struct PreparedNativeWork<C, F> {
    future: Option<F>,
    create: Option<C>,
    cancellation: Option<PreparedCancellation>,
}
impl<C, F, T> Future for PreparedNativeWork<C, F>
where
    C: FnOnce(PreparedCancellation) -> F,
    F: Future<Output = T>,
{
    type Output = T;
    fn poll(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<T> {
        // SAFETY: future is the only structurally pinned field. We install it
        // before exposing Pin and never remove/move it, including on Ready.
        // Other fields are not exposed through pinned projections.
        let this = unsafe { self.get_unchecked_mut() };
        if this.future.is_none()
            && let (Some(create), Some(cancellation)) =
                (this.create.take(), this.cancellation.take())
        {
            this.future = Some(create(cancellation));
        }
        match this.future.as_mut() {
            // SAFETY: Some(F) was initialized at its final pinned address above.
            Some(future) => unsafe { Pin::new_unchecked(future) }.poll(context),
            None => Poll::Pending,
        }
    }
}

/// Separate prepared shutdown boundary. Native RuntimeOwner stop/drain also
/// observes its accepted work via the existing parent/service permits. Keep
/// both owners alive through settlement; stop prepared submissions before
/// destroying the native runtime. Retained results can outlive both drains.
pub struct PreparedNativeTasks<C: 'static, F: 'static, T: 'static, D: 'static> {
    provider: PreparedProvider<PreparedNativeWork<C, F>, T, D>,
    native: RuntimeHandle,
    settlements: FramePool<'static>,
}
impl<C, F, T, D> PreparedNativeTasks<C, F, T, D>
where
    C: FnOnce(PreparedCancellation) -> F + Send + 'static,
    F: Future<Output = T> + Send + 'static,
    T: Send + 'static,
    D: ProviderWorkingSet,
{
    pub fn prepare(
        native: RuntimeHandle,
        memory: &PreparedMemory<'static>,
        region: RegionId,
        capacity: usize,
        envelope: ProviderNativeEnvelope,
    ) -> Result<Self, ProviderStartupError> {
        if native.worker_count() <= std::num::NonZeroUsize::MIN.get() {
            return Err(ProviderStartupError::Reservation(
                ProviderRefusal::InsufficientWorkers,
            ));
        }
        let settlements = memory.frames(
            region,
            capacity,
            std::alloc::Layout::new::<PreparedOperation>(),
        )?;
        let provider = PreparedProvider::prepare_with_context(
            memory,
            region,
            capacity,
            envelope,
            NativeContext(native.clone()),
        )?;
        Ok(Self {
            provider,
            native,
            settlements,
        })
    }
    pub fn submitter(&self) -> Result<PreparedNativeSubmitter<C, F, T, D>, MemoryError> {
        Ok(PreparedNativeSubmitter {
            physical: self.provider.submitter()?,
            native: self.native.clone(),
            settlements: self.settlements.try_clone()?,
        })
    }
    pub fn stop(&self) {
        self.provider.stop();
    }
    pub fn join(&mut self) {
        self.provider.join();
    }
    #[must_use]
    pub fn worker_panics(&self) -> &effect::prepared_provider::ProviderWorkerPanics {
        self.provider.worker_panics()
    }
    /// Inline region demand plus exact native parent/service settlement frames;
    /// excludes region allocator overhead and separately budgeted native domains.
    pub fn storage_bytes(
        capacity: usize,
        workers: usize,
    ) -> Result<effect::memory::Bytes, MemoryError> {
        PreparedProvider::<PreparedNativeWork<C, F>, T, D>::storage_bytes(capacity, workers)?
            .checked_add(FramePool::storage_bytes(
                capacity,
                std::alloc::Layout::new::<PreparedOperation>(),
            )?)
    }
}

pub struct PreparedNativeSubmitter<C: 'static, F: 'static, T: 'static, D: 'static> {
    physical: PreparedSubmitter<PreparedNativeWork<C, F>, T, D>,
    native: RuntimeHandle,
    settlements: FramePool<'static>,
}
#[derive(Debug)]
pub enum PreparedNativeRefusal {
    Physical(ProviderRefusal),
    Native(AdmissionError),
}
pub struct PreparedNativeSubmission<C: 'static, F: 'static, T: 'static, D: 'static> {
    operation: PreparedOperation,
    settlement: FrameReservation<'static, PreparedOperation>,
    // Rust drops fields in declaration order. Release native parent/service
    // permits and settlement-frame rollback before the physical preparation
    // owner decrements preparing and wakes provider shutdown.
    physical: PreparedSubmission<PreparedNativeWork<C, F>, T, D>,
}
impl<C, F, T, D> PreparedNativeSubmitter<C, F, T, D> {
    pub fn try_clone(&self) -> Result<Self, MemoryError> {
        Ok(Self {
            physical: self.physical.try_clone()?,
            native: self.native.clone(),
            settlements: self.settlements.try_clone()?,
        })
    }
    #[must_use]
    pub fn active(&self) -> usize {
        self.physical.active()
    }
    /// All physical cells precede native admission and either refusal rolls back
    /// the entire reservation. Neither callback nor future has been built yet.
    pub fn reserve(&self) -> Result<PreparedNativeSubmission<C, F, T, D>, PreparedNativeRefusal> {
        let physical = self
            .physical
            .reserve()
            .map_err(PreparedNativeRefusal::Physical)?;
        let settlement = self
            .settlements
            .reserve::<PreparedOperation>()
            .map_err(|error| PreparedNativeRefusal::Physical(error.into()))?;
        let operation = self
            .native
            .reserve_prepared()
            .map_err(PreparedNativeRefusal::Native)?;
        Ok(PreparedNativeSubmission {
            physical,
            operation,
            settlement,
        })
    }
}
impl<C, F, T, D> PreparedNativeSubmission<C, F, T, D>
where
    C: FnOnce(PreparedCancellation) -> F + Send + 'static,
    F: Future<Output = T> + Send + 'static,
    T: Send + 'static,
    D: ProviderWorkingSet,
{
    pub fn submit<X>(
        self,
        budget: impl FnOnce() -> Result<ProviderBudget<D>, X>,
        captures: impl FnOnce(&D) -> C,
    ) -> Result<PreparedReceipt<PreparedNativeWork<C, F>, T, D>, PreparedSubmitError<X>> {
        let operation = self.operation;
        self.physical
            .with_execution_lease(PreparedExecutionLease::new(self.settlement, operation))
            .submit(budget, move |budget, cancellation| PreparedNativeWork {
                future: None,
                create: Some(captures(budget)),
                cancellation: Some(cancellation),
            })
    }
}
