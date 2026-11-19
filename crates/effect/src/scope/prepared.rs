//! Counted prepared scope ownership over the existing Scope phase/close engine.
//! Finalizer batches and suffix destinations are provisioned before body entry.
use super::storage::SegmentLease;
use super::{
    FinalizerBatch, Inner, Phase, Registered, RegistrationError, Scope, ScopeCloseStatus, ScopeLink,
};
use crate::internal::runtime::{self, Context, Finish, Step};
use crate::internal::runtime_storage::Consume;
use crate::memory::{
    Bytes, FixedVector, FramePool, MemoryError, PreparedMemory, RegionId, SharedSlot, SlotPool,
};
use crate::{Defect, Effect, Exit, LibraryDefect, Release};
use std::alloc::Layout;
use std::marker::PhantomData;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Arc, Mutex};

pub(super) mod closed;
pub use closed::PreparedCloseWait;
mod drain;
pub use drain::{PreparedScopeClose, PreparedScopeCloseDemand};
mod pool;
mod resource;
mod start;
pub use pool::PreparedScopePool;
pub use resource::{
    PreparedCleanupDemand, PreparedFutureResourceBinding, PreparedFutureResourcePlan,
    PreparedResourceBinding, PreparedResourceDemand, PreparedResourcePlan, PreparedResourceValue,
};

/// A root batch plus this many simultaneous detached suffix batches. Each batch
/// has its own fixed capacity; all storage is provisioned, not grown on transfer.
#[derive(Clone, Copy, Debug)]
pub struct PreparedScopeLimits {
    pub finalizers: usize,
    pub segments: usize,
}
#[derive(Clone, Copy, Debug)]
pub struct PreparedScopeDemand {
    /// Exact requested collection/control payloads, before region allocation
    /// headers/padding. Region reports include the complete backing charges.
    pub storage_payload: Bytes,
    pub allocation_requests: usize,
    pub maximum_retained_finalizers: usize,
    /// Two buffers per concurrently executing scoped owner; direct close, release,
    /// and resource admissions reserve their own additional buffers.
    pub cleanup_accumulators: usize,
    /// Checkout and body-construction catch sites per concurrent scoped pool run.
    pub panic_sites: usize,
}
impl PreparedScopeLimits {
    /// Concrete typed layout demand for generated configuration/preparation code.
    /// Callback frame/factory demand remains separately selected and checked.
    pub fn demand<E: Send + 'static>(self) -> Result<PreparedScopeDemand, MemoryError> {
        self.pool_demand::<E>(1)
    }
    pub fn pool_demand<E: Send + 'static>(
        self,
        scopes: usize,
    ) -> Result<PreparedScopeDemand, MemoryError> {
        let batches = self
            .segments
            .checked_add(1)
            .and_then(|count| count.checked_mul(scopes))
            .ok_or(MemoryError::ArithmeticOverflow)?;
        let values = Layout::array::<Registered<E>>(self.finalizers)
            .map_err(|_| MemoryError::InvalidLayout)?;
        let storage_payload = Bytes::new(values.size())
            .checked_mul(batches)?
            .checked_add(SlotPool::<FixedVector<'static, Registered<E>>>::storage_bytes(batches)?)?
            .checked_add(SlotPool::<Inner<E>>::storage_bytes(scopes)?)?;
        Ok(PreparedScopeDemand {
            storage_payload,
            allocation_requests: if self.finalizers == 0 {
                2
            } else {
                batches
                    .checked_add(2)
                    .ok_or(MemoryError::ArithmeticOverflow)?
            },
            panic_sites: scopes
                .checked_mul(2)
                .ok_or(MemoryError::ArithmeticOverflow)?,
            cleanup_accumulators: scopes
                .checked_mul(2)
                .ok_or(MemoryError::ArithmeticOverflow)?,
            maximum_retained_finalizers: self
                .finalizers
                .checked_mul(batches)
                .ok_or(MemoryError::ArithmeticOverflow)?,
        })
    }
}

/// Shareable handle, not execution ownership. Dropping a handle does not extend
/// or close the scope; the dedicated owner retains canonical cleanup authority.
pub struct PreparedScope<E: Send + 'static> {
    inner: SharedSlot<'static, Inner<E>>,
    batches: SlotPool<'static, FixedVector<'static, Registered<E>>>,
    callbacks: FramePool<'static>,
    cleanup: Option<crate::PreparedCleanupConfig<E>>,
    panic_pool: Option<crate::PreparedPanicPool>,
}
pub(crate) use resource::entry::Entry as PreparedScopeEntry;
impl<E: Send + 'static> PreparedScope<E> {
    pub(crate) fn reserve_panic(
        &self,
    ) -> Result<Option<crate::PreparedPanicReservation>, MemoryError> {
        self.panic_pool
            .as_ref()
            .map(crate::PreparedPanicPool::try_reserve)
            .transpose()
    }
    pub(crate) fn reserve_cleanup_buffer(&self) -> Result<crate::CleanupFailures<E>, MemoryError> {
        let cleanup = self
            .cleanup
            .as_ref()
            .ok_or(MemoryError::UnsupportedRunContract)?;
        cleanup.pool.reserve(&cleanup.frames)
    }

    pub(crate) fn reserve_entry_preflight(&self) -> Result<PreparedScopeEntry<E>, MemoryError> {
        PreparedScopeEntry::reserve(self).map_err(|error| match error {
            resource::entry::PreflightError::Capacity(error) => error,
            resource::entry::PreflightError::Closed => MemoryError::Closed,
        })
    }

    pub fn try_clone(&self) -> Result<Self, MemoryError> {
        Ok(Self {
            inner: self.inner.try_clone()?,
            batches: self.batches.try_clone()?,
            callbacks: self.callbacks.try_clone()?,
            panic_pool: self
                .panic_pool
                .as_ref()
                .map(crate::PreparedPanicPool::try_clone)
                .transpose()?,
            cleanup: self
                .cleanup
                .as_ref()
                .map(crate::PreparedCleanupConfig::try_clone)
                .transpose()?,
        })
    }
    /// Acquire the suffix destination and retained scope handle BEFORE executing
    /// a nested body/acquisition. Release uses this exact physical destination.
    pub fn checkpoint(&self) -> Result<PreparedCheckpoint<E>, MemoryError> {
        let inner = self.inner.try_clone()?;
        let mut state = self
            .inner
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.phase != Phase::Open {
            return Err(MemoryError::Closed);
        }
        if state.segments_in_use >= state.segment_limit.unwrap_or(0) {
            return Err(MemoryError::NoSlot);
        }
        let batch = FinalizerBatch::prepared(self.batches.try_checkout()?);
        state.segments_in_use += 1;
        Ok(PreparedCheckpoint {
            scope: SegmentLease { inner },
            batch,
            index: state.finalizers.len(),
        })
    }
    /// Generated admission registers only empty/unacquired guarded records here.
    /// Refusal is outside business E and before acquisition/factory invocation.
    pub(crate) fn register_preflight(&self, finalizer: Registered<E>) -> Result<(), MemoryError> {
        Scope::<E>::register_inner(&self.inner, finalizer).map_err(|error| match error {
            RegistrationError::Full(finalizer) => {
                drop(finalizer);
                MemoryError::NoSlot
            }
            RegistrationError::Closed(finalizer) => {
                drop(finalizer);
                MemoryError::Closed
            }
        })
    }
    pub async fn closed(&self) -> ScopeCloseStatus {
        Scope::<E>::closed_inner(&self.inner).await
    }
    /// Admit one close before returning its single-use Effect. A diagnostic
    /// refusal is returned directly and never invokes the capacity mapper.
    pub fn admit_close<R, M>(
        &self,
        on_capacity: M,
    ) -> Result<Effect<ScopeCloseStatus, E, R>, MemoryError>
    where
        R: Send + Sync + 'static,
        M: Fn(MemoryError) -> E + Send + Sync + 'static,
    {
        let panic = self
            .reserve_panic()?
            .ok_or(MemoryError::UnsupportedRunContract)?;
        let scope = self.try_clone()?;
        let candidate = Mutex::new(Some((scope, panic)));
        Ok(Effect::new(move |_, _, finish| {
            let candidate = candidate
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take();
            let Some((scope, panic)) = candidate else {
                return runtime::finish(
                    finish,
                    Exit::Die(Defect::from(LibraryDefect::ScopeOutcomeConsumed)),
                );
            };
            let buffer = match scope.reserve_cleanup_buffer() {
                Ok(buffer) => buffer,
                Err(error) => {
                    return runtime::finish(
                        finish,
                        capacity_outcome_prepared(error, &on_capacity, panic),
                    );
                }
            };
            let inner = match scope.inner.try_clone() {
                Ok(inner) => inner,
                Err(error) => {
                    return runtime::finish(
                        finish,
                        capacity_outcome_prepared(error, &on_capacity, panic),
                    );
                }
            };
            let observed = match scope.inner.try_clone() {
                Ok(inner) => ScopeLink::Prepared(inner),
                Err(error) => {
                    return runtime::finish(
                        finish,
                        capacity_outcome_prepared(error, &on_capacity, panic),
                    );
                }
            };
            drop(panic);
            ScopeLink::Prepared(inner).join_close_prepared(
                Exit::Success(()),
                runtime::continuation(move |_| {
                    runtime::finish(finish, Exit::Success(observed.status()))
                }),
                buffer,
            )
        }))
    }
    /// Concurrent/late close follows the same join/acknowledgment engine. Capacity
    /// for extra sharing is mapped explicitly; the dedicated owner's canonical
    /// close handles were already acquired before its body was invoked.
    pub fn close<R, M>(self, on_capacity: M) -> Effect<ScopeCloseStatus, E, R>
    where
        R: Send + Sync + 'static,
        M: Fn(MemoryError) -> E + Send + Sync + 'static,
    {
        Effect::new(move |_, _, finish| {
            let buffer = match self.reserve_cleanup_buffer() {
                Ok(buffer) => buffer,
                Err(error) => return failed(finish, error, &on_capacity),
            };
            let scope = match self.inner.try_clone() {
                Ok(scope) => scope,
                Err(error) => return failed(finish, error, &on_capacity),
            };
            let observed = match self.inner.try_clone() {
                Ok(scope) => ScopeLink::Prepared(scope),
                Err(error) => return failed(finish, error, &on_capacity),
            };
            ScopeLink::Prepared(scope).join_close_prepared(
                Exit::Success(()),
                runtime::continuation(move |_| {
                    runtime::finish(finish, Exit::Success(observed.status()))
                }),
                buffer,
            )
        })
    }
}

/// Pre-execution scope admission failure, preserving caught checkout defects.
#[derive(Debug)]
pub enum PreparedScopeAdmissionError {
    Memory(MemoryError),
    Defect(Defect),
}

/// Dedicated owner of one prepared scope, including never-started/abandoned body
/// cleanup. This is one scope handoff, not a reusable opaque PreparedRun recipe.
pub struct PreparedScopeOwner<E: Send + 'static> {
    scope: PreparedScope<E>,
    settled: bool,
}
impl<E: Send + 'static> Drop for PreparedScopeOwner<E> {
    fn drop(&mut self) {
        if !self.settled {
            Scope::<E>::abandon_inner(&self.scope.inner);
        }
    }
}
impl<E: Send + 'static> PreparedScopeOwner<E> {
    pub(crate) fn reserve_cleanup_buffer(&self) -> Result<crate::CleanupFailures<E>, MemoryError> {
        let cleanup = self
            .scope
            .cleanup
            .as_ref()
            .ok_or(MemoryError::UnsupportedRunContract)?;
        cleanup.pool.reserve(&cleanup.frames)
    }

    pub fn prepare(
        memory: &PreparedMemory<'static>,
        metadata: RegionId,
        limits: PreparedScopeLimits,
        callbacks: FramePool<'static>,
    ) -> Result<Self, MemoryError> {
        PreparedScopePool::prepare(memory, metadata, 1, limits, callbacks)?.checkout()
    }
    /// Prepare a dedicated scope with explicitly selected cleanup storage.
    pub fn prepare_with_cleanup(
        memory: &PreparedMemory<'static>,
        metadata: RegionId,
        limits: PreparedScopeLimits,
        callbacks: FramePool<'static>,
        cleanup: crate::PreparedCleanupConfig<E>,
    ) -> Result<Self, MemoryError> {
        PreparedScopePool::prepare(memory, metadata, 1, limits, callbacks)?
            .with_cleanup(cleanup)
            .checkout()
    }
    /// Attach an explicitly prepared pool before starting this dedicated scope.
    pub fn with_panic_diagnostics(
        mut self,
        pool: crate::PreparedPanicPool,
    ) -> Result<Self, MemoryError> {
        if pool.capacity() < 1 {
            return Err(MemoryError::NoSlot);
        }
        self.scope.panic_pool = Some(pool);
        Ok(self)
    }
    pub fn handle(&self) -> Result<PreparedScope<E>, MemoryError> {
        self.scope.try_clone()
    }
    /// Acquire all owner/close handles before calling the body. Description clones
    /// coordinate this single ownership transfer; a consumed owner is a defect,
    /// as in existing single-use release/ownership adapters, not hidden capacity.
    /// Result covers checked owner sharing, NOT fallible provisioning of the
    /// ordinary Effect description Arc, which remains an explicit legacy domain.
    pub fn into_effect<A, R, F>(self, body: F) -> Result<Effect<A, E, R>, MemoryError>
    where
        A: Send + 'static,
        R: Send + Sync + 'static,
        F: FnOnce(PreparedScope<E>) -> Effect<A, E, R> + Send + 'static,
    {
        let panic = self.scope.reserve_panic()?;
        let cleanup_buffer = self.reserve_cleanup_buffer()?;
        let close_buffer = self.reserve_cleanup_buffer()?;
        let handle = self.scope.try_clone()?;
        let closing = ScopeLink::Prepared(self.scope.inner.try_clone()?);
        let candidate = Mutex::new(Some((
            self,
            handle,
            closing,
            body,
            cleanup_buffer,
            close_buffer,
            panic,
        )));
        Ok(Effect::new(move |env, context, finish| {
            let candidate = candidate
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take();
            let Some((owner, handle, closing, body, cleanup_buffer, close_buffer, panic)) =
                candidate
            else {
                return runtime::finish(
                    finish,
                    Exit::Die(Defect::from(LibraryDefect::ScopeOutcomeConsumed)),
                );
            };
            start::start(
                (owner, handle, closing, cleanup_buffer, close_buffer, panic),
                move |scope| Ok(body(scope)),
                env,
                context,
                finish,
            )
        }))
    }
}

/// An owned suffix destination, acquired before a nested body. Its batch remains
/// held through every awaited finalizer; no post-body allocation or split_off.
pub struct PreparedCheckpoint<E: Send + 'static> {
    // Unused/drop-only checkpoints also return the physical batch before their
    // quota/inner owner; successful releases transfer that order into the batch.
    batch: FinalizerBatch<E>,
    scope: SegmentLease<E>,
    index: usize,
}
impl<E: Send + 'static> PreparedCheckpoint<E> {
    pub fn release<R: Send + Sync + 'static>(self) -> Effect<(), E, R> {
        let candidate = Mutex::new(Some(self));
        Effect::new(move |env, context, finish| {
            let candidate = candidate
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take();
            let Some(checkpoint) = candidate else {
                return runtime::finish(
                    finish,
                    Exit::Die(Defect::from(LibraryDefect::ScopeFinalizerConsumed)),
                );
            };
            let work = {
                let mut state = checkpoint
                    .scope
                    .inner
                    .state
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                if state.phase != Phase::Open || checkpoint.index > state.finalizers.len() {
                    Err(checkpoint.batch)
                } else {
                    state
                        .finalizers
                        .reserved_suffix(checkpoint.index, checkpoint.batch)
                }
            };
            match work {
                Ok(mut work) => {
                    // Transfer the already-held inner owner into the batch, not
                    // merely this constructor closure. Field/drop order retains
                    // scope state through terminal cleanup AND abandoned batches.
                    work.retain_scope(checkpoint.scope);
                    crate::internal::macro_future::release_work(work, Exit::Success(())).execute(
                        env,
                        context.protected(),
                        finish,
                    )
                }
                Err(_) => runtime::finish(
                    finish,
                    Exit::Die(Defect::from(LibraryDefect::ScopeRegistrationClosed)),
                ),
            }
        })
    }
}

/// Inert typed finalizer descriptor. Layout queries do not invoke the factory;
/// generated configuration can account storage before preparation/native work.
pub struct PreparedReleasePlan<E, R, F>
where
    E: Send + 'static,
    R: Send + Sync + 'static,
    F: Fn() -> Release<E, R> + Send + Sync + 'static,
{
    factory: F,
    marker: PhantomData<fn() -> (E, R)>,
}
impl<E, R, F> PreparedReleasePlan<E, R, F>
where
    E: Send + 'static,
    R: Send + Sync + 'static,
    F: Fn() -> Release<E, R> + Send + Sync + 'static,
{
    #[must_use]
    pub fn new(factory: F) -> Self {
        Self {
            factory,
            marker: PhantomData,
        }
    }
    #[must_use]
    pub fn callback_layout(&self) -> Layout {
        Layout::new::<ReleaseOperation<E, R, F>>()
    }
    pub fn factory_storage_payload(&self) -> Result<Bytes, MemoryError> {
        SlotPool::<F>::storage_bytes(1)
    }
    pub fn frames(
        &self,
        memory: &PreparedMemory<'static>,
        region: RegionId,
        count: usize,
    ) -> Result<FramePool<'static>, MemoryError> {
        memory.frames(region, count, self.callback_layout())
    }
    pub fn prepare(
        self,
        memory: &PreparedMemory<'static>,
        metadata: RegionId,
    ) -> Result<PreparedRelease<E, R, F>, MemoryError> {
        PreparedRelease::prepare(memory, metadata, self.factory)
    }
}

/// Prepared reusable release factory. Its captures and sharing metadata are in
/// counted storage, not a new Arc hidden in fallible preparation.
pub struct PreparedRelease<E, R, F>
where
    E: Send + 'static,
    R: Send + Sync + 'static,
    F: Fn() -> Release<E, R> + Send + Sync + 'static,
{
    factory: SharedSlot<'static, F>,
    marker: PhantomData<fn() -> (E, R)>,
}
struct ReleaseOperation<E, R, F>
where
    E: Send + 'static,
    R: Send + Sync + 'static,
    F: Fn() -> Release<E, R> + Send + Sync + 'static,
{
    factory: SharedSlot<'static, F>,
    environment: Arc<R>,
    context: Context,
    panic: Option<crate::PreparedPanicReservation>,
}
impl<E, R, F> Consume<Finish<(), E>, Step> for ReleaseOperation<E, R, F>
where
    E: Send + 'static,
    R: Send + Sync + 'static,
    F: Fn() -> Release<E, R> + Send + Sync + 'static,
{
    fn consume(self, finish: Finish<(), E>) -> Step {
        match catch_unwind(AssertUnwindSafe(|| (self.factory)())) {
            Ok(release) => release.execute(self.environment, self.context, finish),
            Err(payload) => runtime::finish(
                finish,
                Exit::Die(runtime::panic_defect(self.panic, payload)),
            ),
        }
    }
}
impl<E, R, F> PreparedRelease<E, R, F>
where
    E: Send + 'static,
    R: Send + Sync + 'static,
    F: Fn() -> Release<E, R> + Send + Sync + 'static,
{
    pub fn prepare(
        memory: &PreparedMemory<'static>,
        metadata: RegionId,
        factory: F,
    ) -> Result<Self, MemoryError> {
        let mut factory = Some(factory);
        let values = memory.slots(metadata, 1, |_| factory.take().ok_or(MemoryError::NoSlot))?;
        Ok(Self {
            factory: values.try_checkout()?.share(),
            marker: PhantomData,
        })
    }
    pub fn try_clone(&self) -> Result<Self, MemoryError> {
        Ok(Self {
            factory: self.factory.try_clone()?,
            marker: PhantomData,
        })
    }
    /// Compiler-derived callback layout; no user-written raw frame layout needed.
    pub fn frames(
        &self,
        memory: &PreparedMemory<'static>,
        region: RegionId,
        count: usize,
    ) -> Result<FramePool<'static>, MemoryError> {
        memory.frames(region, count, Layout::new::<ReleaseOperation<E, R, F>>())
    }
    /// Admit one registration before returning its single-use Effect. A missing
    /// diagnostic slot is a pre-execution error; no mapper or release runs here.
    pub fn admit_registration<M>(
        &self,
        scope: PreparedScope<E>,
        on_capacity: M,
    ) -> Result<Effect<(), E, R>, MemoryError>
    where
        M: Fn(MemoryError) -> E + Send + Sync + 'static,
    {
        let release_panic = scope
            .reserve_panic()?
            .ok_or(MemoryError::UnsupportedRunContract)?;
        let mapper_panic = scope
            .reserve_panic()?
            .ok_or(MemoryError::UnsupportedRunContract)?;
        let cleanup_buffer = scope.reserve_cleanup_buffer()?;
        let factory = self.factory.try_clone()?;
        let reservation = scope.callbacks.reserve::<ReleaseOperation<E, R, F>>()?;
        let candidate = Mutex::new(Some((
            scope,
            release_panic,
            mapper_panic,
            cleanup_buffer,
            factory,
            reservation,
        )));
        Ok(Effect::new(move |environment, context, finish| {
            let candidate = candidate
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take();
            let Some((scope, release_panic, mapper_panic, cleanup_buffer, factory, reservation)) =
                candidate
            else {
                return runtime::finish(
                    finish,
                    Exit::Die(Defect::from(LibraryDefect::ScopeOutcomeConsumed)),
                );
            };
            let operation = ReleaseOperation {
                factory,
                environment,
                context: context.protected(),
                panic: Some(release_panic),
            };
            let finalizer = Registered::from_prepared(reservation, operation);
            match Scope::<E>::register_inner(&scope.inner, finalizer) {
                Ok(()) => runtime::finish(finish, Exit::Success(())),
                Err(RegistrationError::Full(finalizer)) => {
                    let primary =
                        capacity_outcome_prepared(MemoryError::NoSlot, &on_capacity, mapper_panic);
                    finalizer.execute_with(move |frame| {
                        runtime::continuation_retained(
                            move |cleanup| {
                                runtime::finish(
                                    finish,
                                    crate::cleanup_failures::append_reserved(
                                        primary,
                                        cleanup,
                                        cleanup_buffer,
                                    ),
                                )
                            },
                            frame,
                        )
                    })
                }
                Err(RegistrationError::Closed(finalizer)) => finalizer.execute_with(move |frame| {
                    runtime::continuation_retained(
                        move |cleanup| {
                            runtime::finish(
                                finish,
                                crate::cleanup_failures::append_reserved(
                                    Exit::Die(Defect::from(LibraryDefect::ScopeRegistrationClosed)),
                                    cleanup,
                                    cleanup_buffer,
                                ),
                            )
                        },
                        frame,
                    )
                }),
            }
        }))
    }

    /// Register cleanup work. Refused registration awaits this finalizer
    /// immediately, retaining the mapped capacity failure as primary (the same
    /// ownership rule as closed-scope registration). Resource acquisition still
    /// needs a pre-held abandonment/registration bundle BEFORE native work.
    pub fn register<M>(self, scope: PreparedScope<E>, on_capacity: M) -> Effect<(), E, R>
    where
        M: Fn(MemoryError) -> E + Send + Sync + 'static,
    {
        Effect::new(move |environment, context, finish| {
            let panic = match scope.reserve_panic() {
                Ok(panic) => panic,
                Err(error) => return failed(finish, error, &on_capacity),
            };
            let cleanup_buffer = match scope.reserve_cleanup_buffer() {
                Ok(buffer) => buffer,
                Err(error) => return failed(finish, error, &on_capacity),
            };
            let factory = match self.factory.try_clone() {
                Ok(factory) => factory,
                Err(error) => {
                    let primary = capacity_outcome(error, &on_capacity);
                    let cleanup_finish = runtime::continuation(move |cleanup| {
                        runtime::finish(
                            finish,
                            crate::cleanup_failures::append_reserved(
                                primary,
                                cleanup,
                                cleanup_buffer,
                            ),
                        )
                    });
                    return match catch_unwind(AssertUnwindSafe(|| (self.factory)())) {
                        Ok(release) => {
                            release.execute(environment, context.protected(), cleanup_finish)
                        }
                        Err(payload) => runtime::finish(
                            cleanup_finish,
                            Exit::Die(runtime::panic_defect(panic, payload)),
                        ),
                    };
                }
            };
            let operation = ReleaseOperation {
                factory,
                environment,
                context: context.protected(),
                panic,
            };
            let reservation = match scope.callbacks.reserve::<ReleaseOperation<E, R, F>>() {
                Ok(reservation) => reservation,
                Err(error) => {
                    let primary = capacity_outcome(error, &on_capacity);
                    return operation.consume(runtime::continuation(move |cleanup| {
                        runtime::finish(
                            finish,
                            crate::cleanup_failures::append_reserved(
                                primary,
                                cleanup,
                                cleanup_buffer,
                            ),
                        )
                    }));
                }
            };
            let finalizer = Registered::from_prepared(reservation, operation);
            match Scope::<E>::register_inner(&scope.inner, finalizer) {
                Ok(()) => runtime::finish(finish, Exit::Success(())),
                Err(RegistrationError::Full(finalizer)) => {
                    let primary = capacity_outcome(MemoryError::NoSlot, &on_capacity);
                    finalizer.execute_with(move |frame| {
                        runtime::continuation_retained(
                            move |cleanup| {
                                runtime::finish(
                                    finish,
                                    crate::cleanup_failures::append_reserved(
                                        primary,
                                        cleanup,
                                        cleanup_buffer,
                                    ),
                                )
                            },
                            frame,
                        )
                    })
                }
                Err(RegistrationError::Closed(finalizer)) => finalizer.execute_with(move |frame| {
                    runtime::continuation_retained(
                        move |cleanup| {
                            runtime::finish(
                                finish,
                                crate::cleanup_failures::append_reserved(
                                    Exit::Die(Defect::from(LibraryDefect::ScopeRegistrationClosed)),
                                    cleanup,
                                    cleanup_buffer,
                                ),
                            )
                        },
                        frame,
                    )
                }),
            }
        })
    }
}

fn failed<A: Send + 'static, E: Send + 'static>(
    finish: Finish<A, E>,
    error: MemoryError,
    mapper: &impl Fn(MemoryError) -> E,
) -> Step {
    runtime::finish(finish, capacity_outcome(error, mapper))
}
fn capacity_outcome<A, E>(error: MemoryError, mapper: &impl Fn(MemoryError) -> E) -> Exit<A, E> {
    catch_unwind(AssertUnwindSafe(|| mapper(error)))
        .map(Exit::Fail)
        .unwrap_or_else(|panic| Exit::Die(Defect::from_panic(panic)))
}

fn capacity_outcome_prepared<A, E>(
    error: MemoryError,
    mapper: &impl Fn(MemoryError) -> E,
    panic: crate::PreparedPanicReservation,
) -> Exit<A, E> {
    capacity_outcome_with(error, mapper, Some(panic))
}
fn capacity_outcome_with<A, E>(
    error: MemoryError,
    mapper: &impl Fn(MemoryError) -> E,
    panic: Option<crate::PreparedPanicReservation>,
) -> Exit<A, E> {
    catch_unwind(AssertUnwindSafe(|| mapper(error)))
        .map(Exit::Fail)
        .unwrap_or_else(|payload| Exit::Die(runtime::panic_defect(panic, payload)))
}
