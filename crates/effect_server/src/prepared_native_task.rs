//! Physically admitted native task owners using the existing prepared provider.
use effect::memory::PreparedUnitFuture;
use effect::prepared_provider::PreparedCancellation;
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

#[derive(Clone)]
pub(crate) struct TokioNativeContext(pub(crate) tokio::runtime::Handle);
impl effect::prepared_provider::ProviderWorkerContext for TokioNativeContext {
    type Guard<'a> = tokio::runtime::EnterGuard<'a>;
    fn enter(&self) -> Self::Guard<'_> { self.0.enter() }
}

/// Provider-stored native unit future, polled and destroyed in its selected
/// Tokio runtime context. The runtime handle does not own the provider runtime.
pub struct PreparedNativeTaskFuture {
    future: Option<PreparedUnitFuture>,
    cancellation: PreparedCancellation,
    runtime: tokio::runtime::Handle,
}
impl Unpin for PreparedNativeTaskFuture {}
impl PreparedNativeTaskFuture {
    pub(crate) fn new(
        future: PreparedUnitFuture,
        cancellation: PreparedCancellation,
        runtime: tokio::runtime::Handle,
    ) -> Self {
        Self {
            future: Some(future),
            cancellation,
            runtime,
        }
    }
}
impl Future for PreparedNativeTaskFuture {
    type Output = ();
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        let this = self.get_mut();
        let _entered = this.runtime.enter();
        if this.cancellation.is_requested() {
            drop(this.future.take());
            return Poll::Ready(());
        }
        match this.future.as_mut() {
            Some(future) => Pin::new(future).poll(cx),
            None => Poll::Ready(()),
        }
    }
}
impl Drop for PreparedNativeTaskFuture {
    fn drop(&mut self) {
        let _entered = self.runtime.enter();
        drop(self.future.take());
    }
}

// Before provider publication, refusal/unwind still destroys the native
// future while the caller-selected reactor is entered.
struct PendingNativeFuture {
    future: Option<PreparedUnitFuture>,
    runtime: tokio::runtime::Handle,
}
impl PendingNativeFuture {
    #[allow(clippy::expect_used)] // The unpublished owner is consumed once.
    fn into_task(mut self, cancellation: PreparedCancellation) -> PreparedNativeTaskFuture {
        let runtime = self.runtime.clone();
        let future = self.future.take().expect("unpublished native future");
        PreparedNativeTaskFuture::new(future, cancellation, runtime)
    }
}
impl Drop for PendingNativeFuture {
    fn drop(&mut self) {
        let _entered = self.runtime.enter();
        drop(self.future.take());
    }
}

// Explicit pair preserves future-before-retirement order even before a
// provisional receipt record can be claimed.
struct UnpublishedOwners {
    future: Option<PendingNativeFuture>,
    retirement: Option<effect::prepared_provider::PreparedExecutionLease>,
}
impl Drop for UnpublishedOwners {
    fn drop(&mut self) {
        let retirement = std::mem::DropGuard::new(self.retirement.take(), drop);
        drop(self.future.take());
        drop(retirement);
    }
}

use crate::prepared_connection_retention::ConnectionHandle;
use effect::memory::{
    Bytes, FixedVector, FramePool, MemoryError, PreparedMemory, RegionId, SharedSlot, SlotPool,
};
use effect::prepared_provider::{
    PreparedExecutionLease, PreparedReceipt, PreparedSubmitter, ProviderBudget, ProviderRefusal,
    ProviderSettlement, ProviderWorkingSet,
};
use effect::{Defect, PreparedPanicPool, PreparedPanicReservation};
use std::sync::{Arc, Mutex};

type Receipt<D> = PreparedReceipt<PreparedNativeTaskFuture, (), D>;
struct TaskRecord<E: Send + 'static, R: Send + Sync + 'static, D: ProviderWorkingSet> {
    state: Mutex<TaskState<E, R, D>>,
}
enum TaskState<E: Send + 'static, R: Send + Sync + 'static, D: ProviderWorkingSet> {
    Vacant,
    Busy {
        connection: ConnectionHandle<E, R>,
        cancel: bool,
        notify: bool,
        abandoned: bool,
        waiter: Option<std::task::Waker>,
    },
    Pending {
        connection: ConnectionHandle<E, R>,
        receipt: Receipt<D>,
        diagnostic: PreparedPanicReservation,
        cancel: bool,
        waiter: Option<std::task::Waker>,
    },
}
type BudgetFactory<D> = dyn Fn() -> Result<ProviderBudget<D>, MemoryError> + Send + Sync;
/// Fixed region-backed native-task receipt registry using a caller-prepared provider.
/// D and its diagnostic leases must own the real native working set; this type
/// does not infer or certify Hyper's captured pointees or parser storage.
pub struct PreparedNativeTaskPool<
    E: Send + 'static,
    R: Send + Sync + 'static,
    D: ProviderWorkingSet,
> {
    records: FixedVector<'static, SharedSlot<'static, TaskRecord<E, R, D>>>,
    submitter: PreparedSubmitter<PreparedNativeTaskFuture, (), D>,
    frames: FramePool<'static>,
    panics: PreparedPanicPool,
    budget: Arc<BudgetFactory<D>>,
}
impl<E: Send + 'static, R: Send + Sync + 'static, D: ProviderWorkingSet>
    PreparedNativeTaskPool<E, R, D>
{
    /// Region bytes for receipt records and their fixed index; provider, frame,
    /// diagnostic and caller-native backing are separately provisioned.
    pub fn storage_bytes(capacity: usize) -> Result<Bytes, MemoryError> {
        let index = std::alloc::Layout::array::<SharedSlot<'static, TaskRecord<E, R, D>>>(capacity)
            .map_err(|_| MemoryError::InvalidLayout)?;
        SlotPool::<TaskRecord<E, R, D>>::storage_bytes(capacity)?
            .checked_add(Bytes::new(index.size()))
    }
    /// Retain the caller's already prepared provider, frame pool and budget factory.
    pub fn prepare(
        memory: &PreparedMemory<'static>,
        region: RegionId,
        submitter: PreparedSubmitter<PreparedNativeTaskFuture, (), D>,
        frames: FramePool<'static>,
        capacity: usize,
        panics: PreparedPanicPool,
        budget: Arc<BudgetFactory<D>>,
    ) -> Result<Self, MemoryError> {
        let _ = Self::storage_bytes(capacity)?;
        let slots = memory.slots(region, capacity, |_| {
            Ok(TaskRecord {
                state: Mutex::new(TaskState::Vacant),
            })
        })?;
        let mut records = FixedVector::prepare(memory.allocator(region)?, capacity)?;
        for _ in 0..capacity {
            records
                .push(slots.checkout()?.share())
                .map_err(|_| MemoryError::NoSlot)?;
        }
        Ok(Self {
            records,
            submitter,
            frames,
            panics,
            budget,
        })
    }
    pub(crate) fn frames(&self) -> &FramePool<'static> {
        &self.frames
    }
    #[allow(clippy::unreachable, clippy::expect_used)] // This owner holds each admitted value once.
    pub(crate) fn submit(
        &self,
        connection: ConnectionHandle<E, R>,
        future: PreparedUnitFuture,
        retirement: PreparedExecutionLease,
        runtime: tokio::runtime::Handle,
    ) -> Result<(), ProviderRefusal> {
        let owners = UnpublishedOwners {
            future: Some(PendingNativeFuture {
                future: Some(future),
                runtime,
            }),
            retirement: Some(retirement),
        };
        let mut owner = Some(connection.try_clone().map_err(ProviderRefusal::Memory)?);
        let mut selected = None;
        for record in self.records.as_slice() {
            let mut state = record
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if matches!(*state, TaskState::Vacant) {
                let Some(connection) = owner.take() else {
                    break;
                };
                *state = TaskState::Busy {
                    connection,
                    cancel: false,
                    notify: false,
                    abandoned: false,
                    waiter: None,
                };
                selected = Some(record);
                break;
            }
        }
        drop(owner);
        let record = selected.ok_or(ProviderRefusal::Memory(MemoryError::NoSlot))?;
        let mut provisional = Provisional {
            record,
            owners: Some(owners),
            armed: true,
        };
        // Claim-before-close is visible to abandonment; claim-after-close is
        // refused here before provider, budget or native factory work.
        if connection.is_closed() { return Err(ProviderRefusal::Closed); }
        connection.signal_tasks();
        let diagnostic = self.panics.try_reserve().map_err(ProviderRefusal::Memory)?;
        let submission = self.submitter.reserve()?;
        let budget = (self.budget)().map_err(ProviderRefusal::Memory)?;
        // After the last fallible prefix, the provider owns the footer before
        // the infallible ownership-only future factory can publish the task.
        let submission = submission.with_post_retirement_lease(
            provisional
                .owners
                .as_mut()
                .expect("provisional owners")
                .retirement
                .take()
                .expect("admitted retirement owner"),
        );
        let future = provisional
            .owners
            .as_mut()
            .expect("provisional owners")
            .future
            .take()
            .expect("admitted native future");
        let receipt = submission
            .submit(
                || Ok::<_, MemoryError>(budget),
                move |_, cancellation| future.into_task(cancellation),
            )
            .map_err(|error| match error {
                effect::prepared_provider::PreparedSubmitError::Refused(error) => error,
                effect::prepared_provider::PreparedSubmitError::WorkingSet(error) => {
                    ProviderRefusal::Memory(error)
                }
            })?;
        let (cancel, waiter, abandoned) = {
            let mut state = record
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            match std::mem::replace(&mut *state, TaskState::Vacant) {
                TaskState::Busy {
                    connection,
                    cancel,
                    waiter,
                    abandoned,
                    ..
                } => {
                    if abandoned {
                        (cancel, waiter, Some((receipt, diagnostic, connection)))
                    } else {
                        *state = TaskState::Pending { connection, receipt, diagnostic, cancel, waiter: None };
                        (cancel, waiter, None)
                    }
                }
                TaskState::Vacant | TaskState::Pending { .. } => {
                    unreachable!("private provisional owner")
                }
            }
        };
        provisional.armed = false;
        let notify = std::mem::DropGuard::new(waiter, |waiter| {
            if let Some(waiter) = waiter {
                waiter.wake();
            }
        });
        if let Some((receipt, diagnostic, owner)) = abandoned {
            drop(receipt); drop(diagnostic); drop(owner);
        } else if cancel {
            self.cancel_connection(&connection);
        }
        drop(notify);
        Ok(())
    }
}
struct Provisional<'a, E: Send + 'static, R: Send + Sync + 'static, D: ProviderWorkingSet> {
    record: &'a TaskRecord<E, R, D>,
    owners: Option<UnpublishedOwners>,
    armed: bool,
}
impl<E: Send + 'static, R: Send + Sync + 'static, D: ProviderWorkingSet> Drop
    for Provisional<'_, E, R, D>
{
    fn drop(&mut self) {
        if self.armed {
            let vacate = std::mem::DropGuard::new(self.record, |record| {
                let previous = {
                    let mut state = record
                        .state
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    std::mem::replace(&mut *state, TaskState::Vacant)
                };
                if let TaskState::Busy {
                    connection, waiter, ..
                } = previous
                {
                    let notify = std::mem::DropGuard::new(waiter, |waiter| {
                        if let Some(waiter) = waiter {
                            waiter.wake();
                        }
                    });
                    drop(connection);
                    drop(notify);
                } else {
                    drop(previous);
                }
            });
            drop(self.owners.take());
            drop(vacate);
        }
    }
}

// An operation owns a receipt outside the metadata gate. If its poll or wake
// unwinds, Drop restores the receipt before notifying any competing observer.
struct ReceiptOperation<'a, E: Send + 'static, R: Send + Sync + 'static, D: ProviderWorkingSet> {
    record: &'a TaskRecord<E, R, D>,
    receipt: Option<Receipt<D>>,
    diagnostic: Option<PreparedPanicReservation>,
}
impl<E: Send + 'static, R: Send + Sync + 'static, D: ProviderWorkingSet>
    ReceiptOperation<'_, E, R, D>
{
    fn take<'a>(
        record: &'a TaskRecord<E, R, D>,
        connection: &ConnectionHandle<E, R>,
    ) -> Option<ReceiptOperation<'a, E, R, D>> {
        let mut state = record
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        match &mut *state {
            TaskState::Pending {
                connection: owner, ..
            } if owner.same_connection(connection) => {}
            TaskState::Busy {
                connection: owner,
                notify,
                ..
            } if owner.same_connection(connection) => {
                *notify = true;
                return None;
            }
            TaskState::Vacant | TaskState::Busy { .. } | TaskState::Pending { .. } => return None,
        }
        let previous = std::mem::replace(&mut *state, TaskState::Vacant);
        if let TaskState::Pending {
            connection,
            receipt,
            diagnostic,
            cancel,
            waiter,
        } = previous
        {
            *state = TaskState::Busy {
                connection,
                cancel,
                notify: false,
                abandoned: false,
                waiter,
            };
            Some(ReceiptOperation {
                record,
                receipt: Some(receipt),
                diagnostic: Some(diagnostic),
            })
        } else {
            None
        }
    }
    fn retire(mut self) -> Option<Defect> {
        let receipt = self.receipt.take()?;
        let diagnostic = self.diagnostic.take()?;
        let defect = {
            let result = receipt.result();
            match result.as_ref().and_then(|value| value.settlement()) {
                Some(ProviderSettlement::Panicked(payload)) => {
                    Some(diagnostic.capture_borrowed(payload.as_ref()))
                }
                Some(ProviderSettlement::Returned(())) => None,
                None => Some(effect::LibraryDefect::PreparedConstructionUnbound.into()),
            }
        };
        drop(receipt);
        drop(self);
        defect
    }
}
// Owns the receipt while request_cancel invokes its waker outside metadata gates.
// Its Drop restores the pending record even if that waker unwinds.
struct ReceiptRestore<'a, E: Send + 'static, R: Send + Sync + 'static, D: ProviderWorkingSet> {
    record: &'a TaskRecord<E, R, D>,
    receipt: Option<Receipt<D>>,
    diagnostic: Option<PreparedPanicReservation>,
    armed: bool,
}
impl<E: Send + 'static, R: Send + Sync + 'static, D: ProviderWorkingSet> Drop
    for ReceiptRestore<'_, E, R, D>
{
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let (owner, notify) = {
            let mut state = self
                .record
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            match std::mem::replace(&mut *state, TaskState::Vacant) {
                TaskState::Busy {
                    connection,
                    cancel,
                    notify,
                    abandoned,
                    mut waiter,
                } => {
                    if !abandoned && let (Some(receipt), Some(diagnostic)) =
                        (self.receipt.take(), self.diagnostic.take())
                    {
                        *state = TaskState::Pending {
                            connection,
                            receipt,
                            diagnostic,
                            cancel,
                            waiter: if notify || cancel {
                                None
                            } else {
                                waiter.take()
                            },
                        };
                        (None, waiter)
                    } else {
                        (Some(connection), waiter)
                    }
                }
                other @ (TaskState::Vacant | TaskState::Pending { .. }) => {
                    *state = other;
                    (None, None)
                }
            }
        };
        let notify = std::mem::DropGuard::new(notify, |waiter| {
            if let Some(waiter) = waiter {
                waiter.wake();
            }
        });
        drop(owner);
        drop(notify);
    }
}
impl<E: Send + 'static, R: Send + Sync + 'static, D: ProviderWorkingSet> Drop
    for ReceiptOperation<'_, E, R, D>
{
    fn drop(&mut self) {
        let mut restore = ReceiptRestore {
            record: self.record,
            receipt: self.receipt.take(),
            diagnostic: self.diagnostic.take(),
            armed: true,
        };
        let mut requested = false;
        loop {
            let mut state = self
                .record
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if let TaskState::Busy { cancel: true, .. } = &*state
                && !requested
                && restore.receipt.is_some()
            {
                drop(state);
                if let Some(receipt) = &restore.receipt {
                    receipt.request_cancel();
                }
                requested = true;
                continue;
            }
            let previous = std::mem::replace(&mut *state, TaskState::Vacant);
            let (owner, notify) = match previous {
                TaskState::Busy {
                    connection,
                    cancel,
                    notify,
                    abandoned,
                    mut waiter,
                } => {
                    if !abandoned && let (Some(receipt), Some(diagnostic)) =
                        (restore.receipt.take(), restore.diagnostic.take())
                    {
                        *state = TaskState::Pending {
                            connection,
                            receipt,
                            diagnostic,
                            cancel: if requested { false } else { cancel },
                            waiter: if notify { None } else { waiter.take() },
                        };
                        (None, waiter)
                    } else {
                        (Some(connection), waiter)
                    }
                }
                other @ (TaskState::Vacant | TaskState::Pending { .. }) => {
                    *state = other;
                    (None, None)
                }
            };
            restore.armed = false;
            drop(state);
            let notify = std::mem::DropGuard::new(notify, |waiter| {
                if let Some(waiter) = waiter {
                    waiter.wake();
                }
            });
            drop(owner);
            drop(notify);
            break;
        }
    }
}
impl<E: Send + 'static, R: Send + Sync + 'static, D: ProviderWorkingSet>
    PreparedNativeTaskPool<E, R, D>
{
    pub(crate) fn cancel_connection(&self, connection: &ConnectionHandle<E, R>) {
        for record in self.records.as_slice() {
            let matches = {
                let mut state = record
                    .state
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                match &mut *state {
                    TaskState::Busy {
                        connection: owner,
                        cancel,
                        notify,
                        ..
                    } if owner.same_connection(connection) => {
                        *cancel = true;
                        *notify = true;
                        true
                    }
                    TaskState::Pending {
                        connection: owner,
                        cancel,
                        ..
                    } if owner.same_connection(connection) => {
                        *cancel = true;
                        true
                    }
                    TaskState::Vacant | TaskState::Busy { .. } | TaskState::Pending { .. } => false,
                }
            };
            if matches {
                drop(ReceiptOperation::take(record, connection));
            }
        }
    }
    pub(crate) fn poll_connection(
        &self,
        connection: &ConnectionHandle<E, R>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Defect>> {
        let mut pending = false;
        for record in self.records.as_slice() {
            let relevant = {
                let state = record
                    .state
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                matches!(&*state, TaskState::Busy { connection: owner, .. } | TaskState::Pending { connection: owner, .. } if owner.same_connection(connection))
            };
            if !relevant {
                continue;
            }
            // Clone outside the gate, then recheck this retained generation.
            let mut candidate = Some(cx.waker().clone());
            let previous = {
                let mut state = record
                    .state
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                match &mut *state {
                    TaskState::Busy {
                        connection: owner,
                        waiter,
                        ..
                    }
                    | TaskState::Pending {
                        connection: owner,
                        waiter,
                        ..
                    } if owner.same_connection(connection) => {
                        Some(std::mem::replace(waiter, candidate.take()))
                    }
                    TaskState::Vacant | TaskState::Busy { .. } | TaskState::Pending { .. } => None,
                }
            };
            drop(candidate);
            let Some(previous) = previous else { continue };
            drop(previous);
            if let Some(mut operation) = ReceiptOperation::take(record, connection) {
                let Some(receipt) = &mut operation.receipt else {
                    continue;
                };
                match receipt.poll_execution_settled(cx) {
                    Poll::Pending => {
                        pending = true;
                        drop(operation);
                    }
                    Poll::Ready(()) => {
                        if let Some(defect) = operation.retire() {
                            return Poll::Ready(Some(defect));
                        }
                    }
                }
            } else {
                let state = record
                    .state
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                if matches!(&*state, TaskState::Busy { connection: owner, .. } | TaskState::Pending { connection: owner, .. } if owner.same_connection(connection))
                {
                    pending = true;
                }
            }
        }
        if pending {
            Poll::Pending
        } else {
            Poll::Ready(None)
        }
    }
}

/// Private type-erased server boundary; the caller-supplied D remains held by
/// the typed pool and no future or working-set backing is erased here.
pub(crate) trait NativeTaskDispatch<E: Send + 'static, R: Send + Sync + 'static>:
    Send + Sync
{
    fn frames(&self) -> &FramePool<'static>;
    fn submit(
        &self,
        connection: ConnectionHandle<E, R>,
        future: PreparedUnitFuture,
        retirement: PreparedExecutionLease,
        runtime: tokio::runtime::Handle,
    ) -> Result<(), ProviderRefusal>;
    fn cancel_connection(&self, connection: &ConnectionHandle<E, R>);
    fn abandon_connection(&self, connection: &ConnectionHandle<E, R>);
    fn poll_connection(
        &self,
        connection: &ConnectionHandle<E, R>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Defect>>;
}
impl<E: Send + 'static, R: Send + Sync + 'static, D: ProviderWorkingSet> NativeTaskDispatch<E, R>
    for PreparedNativeTaskPool<E, R, D>
{
    fn frames(&self) -> &FramePool<'static> {
        self.frames()
    }
    fn submit(
        &self,
        connection: ConnectionHandle<E, R>,
        future: PreparedUnitFuture,
        retirement: PreparedExecutionLease,
        runtime: tokio::runtime::Handle,
    ) -> Result<(), ProviderRefusal> {
        self.submit(connection, future, retirement, runtime)
    }
    fn cancel_connection(&self, connection: &ConnectionHandle<E, R>) {
        self.cancel_connection(connection);
    }
    fn abandon_connection(&self, connection: &ConnectionHandle<E, R>) { self.abandon_connection(connection); }
    fn poll_connection(
        &self,
        connection: &ConnectionHandle<E, R>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Defect>> {
        self.poll_connection(connection, cx)
    }
}

// Advance before dropping each observer; unwinding resumes the remaining scan.
struct AbandonDrain<'a, E: Send + 'static, R: Send + Sync + 'static, D: ProviderWorkingSet> {
    pool: &'a PreparedNativeTaskPool<E, R, D>,
    connection: &'a ConnectionHandle<E, R>,
    next: usize,
    active: bool,
}
impl<E: Send + 'static, R: Send + Sync + 'static, D: ProviderWorkingSet> AbandonDrain<'_, E, R, D> {
    fn run(&mut self) {
        while let Some(record) = self.pool.records.as_slice().get(self.next) {
            self.next += 1;
            let removed = {
                let mut state = record.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                match &mut *state {
                    TaskState::Busy { connection, cancel, notify, abandoned, waiter } if connection.same_connection(self.connection) => {
                        *cancel = true; *notify = true; *abandoned = true;
                        let old = waiter.take();
                        drop(state);
                        drop(old);
                        continue;
                    }
                    TaskState::Pending { connection, .. } if connection.same_connection(self.connection) => Some(std::mem::replace(&mut *state, TaskState::Vacant)),
                    TaskState::Vacant | TaskState::Busy { .. } | TaskState::Pending { .. } => None,
                }
            };
            drop(removed);
        }
        self.active = false;
    }
}
impl<E: Send + 'static, R: Send + Sync + 'static, D: ProviderWorkingSet> Drop for AbandonDrain<'_, E, R, D> {
    fn drop(&mut self) { if self.active { self.run(); } }
}
impl<E: Send + 'static, R: Send + Sync + 'static, D: ProviderWorkingSet> PreparedNativeTaskPool<E, R, D> {
    pub(crate) fn abandon_connection(&self, connection: &ConnectionHandle<E, R>) {
        AbandonDrain { pool: self, connection, next: 0, active: true }.run();
    }
}

type NativeActorOwner<E, R> = (Arc<dyn NativeTaskDispatch<E, R>>, ConnectionHandle<E, R>);
/// The native connection future is destroyed before its unique observer owner.
pub(crate) struct NativeConnectionActor<F, E: Send + 'static, R: Send + Sync + 'static> {
    future: F,
    _guard: Option<NativeActorGuard<E, R>>,
}
struct NativeActorGuard<E: Send + 'static, R: Send + Sync + 'static> {
    tasks: Arc<dyn NativeTaskDispatch<E, R>>,
    connection: ConnectionHandle<E, R>,
}
impl<E: Send + 'static, R: Send + Sync + 'static> Drop for NativeActorGuard<E, R> {
    fn drop(&mut self) {
        self.connection.close();
        self.tasks.abandon_connection(&self.connection);
    }
}
impl<F, E: Send + 'static, R: Send + Sync + 'static> NativeConnectionActor<F, E, R> {
    pub(crate) fn new(future: F, owner: Option<NativeActorOwner<E, R>>) -> Self {
        Self { future, _guard: owner.map(|(tasks, connection)| NativeActorGuard { tasks, connection }) }
    }
}
impl<F: Future, E: Send + 'static, R: Send + Sync + 'static> Future for NativeConnectionActor<F, E, R> {
    type Output = F::Output;
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        // SAFETY: the future is structurally pinned and dropped before its guard.
        unsafe { self.map_unchecked_mut(|this| &mut this.future) }.poll(cx)
    }
}
