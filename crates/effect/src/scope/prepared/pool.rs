//! Reusable prepared scope rows. An old observer/segment retains its generation's
//! shared row; it cannot be recycled underneath a later run.
use super::{PreparedScope, PreparedScopeLimits, PreparedScopeOwner, capacity_outcome, failed};
use crate::internal::runtime;
use crate::memory::{FixedVector, FramePool, MemoryError, PreparedMemory, RegionId, SlotPool};
use crate::scope::{FinalizerBatch, Inner, Phase, Registered, Scope, link::ScopeLink};
use crate::{Effect, Exit};
use std::panic::{AssertUnwindSafe, catch_unwind};

/// Fixed reusable scope capacity for repeated/concurrent descriptions. Admission
/// acquires actual row, root batch and owner handles before calling the body.
pub struct PreparedScopePool<E: Send + 'static> {
    scopes: SlotPool<'static, Inner<E>>,
    batches: SlotPool<'static, FixedVector<'static, Registered<E>>>,
    callbacks: FramePool<'static>,
    limits: PreparedScopeLimits,
    cleanup: Option<crate::PreparedCleanupConfig<E>>,
    panic_pool: Option<crate::PreparedPanicPool>,
}
impl<E: Send + 'static> PreparedScopePool<E> {
    pub fn prepare(
        memory: &PreparedMemory<'static>,
        metadata: RegionId,
        scopes: usize,
        limits: PreparedScopeLimits,
        callbacks: FramePool<'static>,
    ) -> Result<Self, MemoryError> {
        let _demand = limits.pool_demand::<E>(scopes)?;
        let batches = limits
            .segments
            .checked_add(1)
            .and_then(|count| count.checked_mul(scopes))
            .ok_or(MemoryError::ArithmeticOverflow)?;
        let batches = memory.slots(metadata, batches, |_| {
            FixedVector::prepare(memory.allocator(metadata)?, limits.finalizers)
        })?;
        let scopes = memory.slots(metadata, scopes, |_| {
            let mut inner = Scope::<E>::new_inner(FinalizerBatch::Empty);
            inner
                .state
                .get_mut()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .phase = Phase::Closed;
            Ok(inner)
        })?;
        Ok(Self {
            scopes,
            batches,
            callbacks,
            limits,
            cleanup: None,
            panic_pool: None,
        })
    }
    /// Attach explicit independent cleanup backing before any scope admission.
    #[must_use]
    pub fn with_cleanup(mut self, cleanup: crate::PreparedCleanupConfig<E>) -> Self {
        self.cleanup = Some(cleanup);
        self
    }
    /// Attach a pool with room for checkout and body catch sites per scope.
    pub fn with_panic_diagnostics(
        mut self,
        pool: crate::PreparedPanicPool,
    ) -> Result<Self, MemoryError> {
        let needed = self
            .scopes
            .capacity()
            .checked_mul(2)
            .ok_or(MemoryError::ArithmeticOverflow)?;
        if pool.capacity() < needed {
            return Err(MemoryError::NoSlot);
        }
        self.panic_pool = Some(pool);
        Ok(self)
    }
    pub fn try_clone(&self) -> Result<Self, MemoryError> {
        Ok(Self {
            scopes: self.scopes.try_clone()?,
            batches: self.batches.try_clone()?,
            callbacks: self.callbacks.try_clone()?,
            panic_pool: self
                .panic_pool
                .as_ref()
                .map(crate::PreparedPanicPool::try_clone)
                .transpose()?,
            limits: self.limits,
            cleanup: self
                .cleanup
                .as_ref()
                .map(crate::PreparedCleanupConfig::try_clone)
                .transpose()?,
        })
    }
    #[must_use]
    pub fn capacity(&self) -> usize {
        self.scopes.capacity()
    }
    /// Active includes closed rows retained by observers/detached cleanup. Such
    /// retention reduces availability but never permits stale row reuse.
    #[must_use]
    pub fn active(&self) -> usize {
        self.scopes.active()
    }
    /// Close new scope admission, not already-admitted cleanup/batch capacity.
    pub fn close(&self) {
        self.scopes.close();
    }
    pub fn checkout(&self) -> Result<PreparedScopeOwner<E>, MemoryError> {
        if let Some(cleanup) = &self.cleanup {
            cleanup.validate_frames()?;
            if self.callbacks.same_pool(&cleanup.frames) {
                return Err(MemoryError::ControlPoolAlias);
            }
        }
        let batches = self.batches.try_clone()?;
        let callbacks = self.callbacks.try_clone()?;
        let root = FinalizerBatch::prepared(batches.try_checkout()?);
        let mut inner = self.scopes.try_checkout()?;
        // Unique row checkout proves every prior handle/listener/segment lease
        // for that generation has gone. Replace state without locking or running
        // destructors under a gate; preserve the prepared Event storage itself.
        let mut state = Scope::<E>::new_state(root);
        state.segment_limit = Some(self.limits.segments);
        let old = std::mem::replace(
            inner
                .state
                .get_mut()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
            state,
        );
        let owner = PreparedScopeOwner {
            scope: PreparedScope {
                inner: inner.share(),
                batches,
                callbacks,
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
            },
            settled: false,
        };
        // Owner guard is installed first: a user E destructor unwinding here
        // cannot strand the newly opened generation or its root batch.
        drop(old);
        Ok(owner)
    }
    /// Admit exactly one scope before returning its single-use Effect. Missing
    /// diagnostic storage is a pre-execution failure, so no mapper is invoked.
    pub fn admit_scoped<A, R, F>(
        &self,
        body: F,
    ) -> Result<Effect<A, E, R>, super::PreparedScopeAdmissionError>
    where
        A: Send + 'static,
        R: Send + Sync + 'static,
        F: FnOnce(PreparedScope<E>) -> Effect<A, E, R> + Send + 'static,
    {
        let pool = self
            .panic_pool
            .as_ref()
            .ok_or(super::PreparedScopeAdmissionError::Memory(
                MemoryError::UnsupportedRunContract,
            ))?;
        let panic = pool
            .try_reserve()
            .map_err(super::PreparedScopeAdmissionError::Memory)?;
        let owner = match catch_unwind(AssertUnwindSafe(|| self.checkout())) {
            Ok(Ok(owner)) => {
                drop(panic);
                owner
            }
            Ok(Err(error)) => return Err(super::PreparedScopeAdmissionError::Memory(error)),
            Err(payload) => {
                return Err(super::PreparedScopeAdmissionError::Defect(
                    crate::internal::runtime::panic_defect(Some(panic), payload),
                ));
            }
        };
        crate::effect_scope! { prepared; owner: owner, body: body, }
            .map_err(super::PreparedScopeAdmissionError::Memory)
    }
    pub fn scoped<A, R, F, M>(self, body: F, on_capacity: M) -> Effect<A, E, R>
    where
        A: Send + 'static,
        R: Send + Sync + 'static,
        F: Fn(PreparedScope<E>) -> Effect<A, E, R> + Send + Sync + 'static,
        M: Fn(MemoryError) -> E + Send + Sync + 'static,
    {
        self.try_scoped(move |scope| Ok(body(scope)), on_capacity)
    }
    /// Fallible description construction is mapped before executing business
    /// work. Ordinary Effect description allocation remains a separate domain.
    pub fn try_scoped<A, R, F, M>(self, body: F, on_capacity: M) -> Effect<A, E, R>
    where
        A: Send + 'static,
        R: Send + Sync + 'static,
        F: Fn(PreparedScope<E>) -> Result<Effect<A, E, R>, MemoryError> + Send + Sync + 'static,
        M: Fn(MemoryError) -> E + Send + Sync + 'static,
    {
        Effect::new(move |environment, context, finish| {
            let checkout_panic = match self
                .panic_pool
                .as_ref()
                .map(crate::PreparedPanicPool::try_reserve)
                .transpose()
            {
                Ok(value) => value,
                Err(error) => return failed(finish, error, &on_capacity),
            };
            let body_panic = match self
                .panic_pool
                .as_ref()
                .map(crate::PreparedPanicPool::try_reserve)
                .transpose()
            {
                Ok(value) => value,
                Err(error) => return failed(finish, error, &on_capacity),
            };
            let owner = match catch_unwind(AssertUnwindSafe(|| self.checkout())) {
                Ok(Ok(owner)) => owner,
                Ok(Err(error)) => return failed(finish, error, &on_capacity),
                Err(payload) => {
                    return runtime::finish(
                        finish,
                        Exit::Die(crate::internal::runtime::panic_defect(
                            checkout_panic,
                            payload,
                        )),
                    );
                }
            };
            let cleanup_buffer = match owner.reserve_cleanup_buffer() {
                Ok(buffer) => buffer,
                Err(error) => return failed(finish, error, &on_capacity),
            };
            let close_buffer = match owner.reserve_cleanup_buffer() {
                Ok(buffer) => buffer,
                Err(error) => return failed(finish, error, &on_capacity),
            };
            let handle = match owner.handle() {
                Ok(handle) => handle,
                Err(error) => return failed(finish, error, &on_capacity),
            };
            let closing = match owner.scope.inner.try_clone() {
                Ok(inner) => ScopeLink::Prepared(inner),
                Err(error) => return failed(finish, error, &on_capacity),
            };
            super::start::start(
                (
                    owner,
                    handle,
                    closing,
                    cleanup_buffer,
                    close_buffer,
                    body_panic,
                ),
                |scope| body(scope).map_err(|error| capacity_outcome(error, &on_capacity)),
                environment,
                context,
                finish,
            )
        })
    }
}
