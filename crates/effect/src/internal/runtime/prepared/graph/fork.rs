//! Provider-backed child graphs with canonical Scope ownership. All promise,
//! child/close, provider, observer and result-transfer storage precedes factories.
use super::resource::Slot;
use super::scope_binding::ScopePermit;
use super::scoped::{ScopedProgram, scope_program};
use super::*;
use crate::internal::runtime::{DriveContext, PollContext, Publication, poll_drive};
use crate::internal::runtime_storage::{Pending, Settlement};
use crate::prepared_provider::{
    HasPreparedProvider, PreparedCancellation, PreparedProviderConstruction,
    PreparedProviderOutputLease, PreparedReceipt, PreparedTaskFuture, ProviderRefusal,
    ProviderWorkingSet, TaskProvider,
};
use crate::{PreparedCancellationWait, PreparedScope, scope::Registered};
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll, Waker};

type Job<A, E> = PreparedTaskFuture<A, E>;
type Receipt<A, E, D> = PreparedReceipt<Job<A, E>, Exit<A, E>, D>;
type Lease<A, E, D> = PreparedProviderOutputLease<Job<A, E>, Exit<A, E>, D>;
type Budget<N, K> = <<N as Spec>::Env as HasPreparedProvider<
    <N as Spec>::Value,
    <N as Spec>::Error,
    K,
>>::WorkingSet;
type Preparation<N, K> =
    PreparedProviderConstruction<<N as Spec>::Value, <N as Spec>::Error, Budget<N, K>>;
type PreparationValue<N, K> = super::super::construction::ConstructionValue<Preparation<N, K>>;
type Store<A, E> = (
    FrameReservation<'static, ResultStore<A, E>>,
    ResultStore<A, E>,
);
type ChildFiber<N, K> = PreparedFiber<<N as Spec>::Value, <N as Spec>::Error, Budget<N, K>>;
#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Prepared,
    Submitting,
    Running,
    Joining,
    Cleaning,
    Consumed,
}
struct State<A: Send + 'static, E: Send + 'static, D: ProviderWorkingSet> {
    phase: Phase,
    driver_lost: bool,
    waiters: waiter::Waiters,
    // Waker/descendant destruction precedes the provider entry/D if this state
    // is abandoned. Normal transitions take it and wake outside metadata gates.
    cleanup_waiter: Option<Waker>,
    receipt: Option<Receipt<A, E, D>>,
    // Private, non-observing actual entry owner; last, after all native waiter
    // descendants. A concurrent join/close cannot retire D during our wake.
    retention: Option<Lease<A, E, D>>,
}
struct Task<A: Send + 'static, E: Send + 'static, D: ProviderWorkingSet> {
    value: Environment<Mutex<State<A, E, D>>>,
}
struct NotifyOnDrop<'a, A: Send + 'static, E: Send + 'static, D: ProviderWorkingSet>(
    &'a Task<A, E, D>,
);
impl<A: Send, E: Send, D: ProviderWorkingSet> Drop for NotifyOnDrop<'_, A, E, D> {
    fn drop(&mut self) {
        self.0.notify();
    }
}
impl<A: Send, E: Send, D: ProviderWorkingSet> Task<A, E, D> {
    fn notify(&self) {
        // The task record is privately published once before any view escapes;
        // this held view excludes an unpublished/retired value during notify.
        if let Ok(mut notifications) = waiter::Notifications::take(self) {
            notifications.run();
        }
    }
    fn clone_preflight(&self) -> Result<Self, MemoryError> {
        Ok(Self {
            value: self.value.try_clone()?,
        })
    }
    fn update<T>(&self, body: impl FnOnce(&mut State<A, E, D>) -> T) -> Result<T, Defect> {
        self.value.with(|value| {
            body(
                &mut value
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner),
            )
        })
    }
    fn prepare<V: Send + 'static>(
        frames: &FramePool<'static>,
        context: &PreparedRunContext<V, E>,
    ) -> Result<Self, MemoryError> {
        let (publisher, value) = Environment::reserve(frames, context)?;
        publisher.install(Mutex::new(State {
            phase: Phase::Prepared,
            driver_lost: false,
            waiters: waiter::Waiters::new(),
            cleanup_waiter: None,
            receipt: None,
            retention: None,
        }));
        Ok(Self { value })
    }
}
struct Cancel<E: Send + 'static> {
    owner: crate::memory::SharedRuntimeFrame,
    request: fn(&crate::memory::SharedRuntimeFrame) -> bool,
    protected: fn(&crate::memory::SharedRuntimeFrame) -> bool,
    marker: PhantomData<fn() -> E>,
}
fn request<V: Send + 'static, E: Send + 'static>(
    owner: &crate::memory::SharedRuntimeFrame,
) -> bool {
    // SAFETY: prepare pairs this exact context type and retained shared frame.
    unsafe { owner.pointer().cast::<PreparedRunContext<V, E>>().as_ref() }.request_cancellation()
}
fn protected<V: Send + 'static, E: Send + 'static>(
    owner: &crate::memory::SharedRuntimeFrame,
) -> bool {
    // SAFETY: same exact-type constructor/vtable pairing as request.
    let context = unsafe { owner.pointer().cast::<PreparedRunContext<V, E>>().as_ref() };
    context.root.contexts.flags(context.id).1
}
impl<E: Send> Cancel<E> {
    fn prepare<V: Send + 'static>(
        frames: &FramePool<'static>,
        context: &PreparedRunContext<V, E>,
    ) -> Result<Self, MemoryError> {
        let selection = context.persistent_selection()?;
        let (context, frames) = selection
            .as_ref()
            .map_or((context, frames), |(context, frames)| (context, frames));
        let owner = frames
            .reserve::<PreparedRunContext<V, E>>()?
            .share(context.try_clone()?);
        Ok(Self {
            owner,
            request: request::<V, E>,
            protected: protected::<V, E>,
            marker: PhantomData,
        })
    }
    fn clone_preflight(&self) -> Result<Self, MemoryError> {
        Ok(Self {
            owner: self.owner.try_clone()?,
            request: self.request,
            protected: self.protected,
            marker: PhantomData,
        })
    }
    fn request(&self) {
        let _ = (self.request)(&self.owner);
    }
    fn protected(&self) -> bool {
        (self.protected)(&self.owner)
    }
}
/// Counted child observer. Native cancel is synchronous; consuming join is lazy
/// and transfers the original non-Clone outcome only once through the engine.
pub struct PreparedFiber<A: Send + 'static, E: Send + 'static, D: ProviderWorkingSet> {
    task: Task<A, E, D>,
    cancel: Cancel<E>,
}
impl<A: Send, E: Send, D: ProviderWorkingSet> PreparedFiber<A, E, D> {
    pub fn try_clone(&self) -> Result<Self, MemoryError> {
        Ok(Self {
            task: self.task.clone_preflight()?,
            cancel: self.cancel.clone_preflight()?,
        })
    }
    pub fn cancel(&self) {
        self.cancel.request();
    }
    #[must_use]
    pub fn join<R: Send + Sync + 'static>(self) -> Join<A, E, D, R> {
        Join {
            fiber: self,
            marker: PhantomData,
        }
    }
}
pub struct Fork<N: Description, K> {
    program: N,
    permit: ScopePermit<N::Error>,
    marker: PhantomData<fn() -> K>,
}
impl<E: Send + 'static> ScopePermit<E> {
    #[must_use]
    pub fn fork<N: Description<Error = E>>(self, program: N) -> Fork<N, TaskProvider> {
        Fork {
            program,
            permit: self,
            marker: PhantomData,
        }
    }
    #[must_use]
    pub fn fork_on<N: Description<Error = E>, K>(self, program: N) -> Fork<N, K> {
        Fork {
            program,
            permit: self,
            marker: PhantomData,
        }
    }
}
mod race;
mod waiter;
pub use race::{Race, race, race_on};
struct ResultStore<A: Send + 'static, E: Send + 'static> {
    result: Slot<Exit<A, E>>,
    selected: Option<race::Mark>,
}
impl<A: Send, E: Send> Consume<Exit<A, E>, Step> for ResultStore<A, E> {
    fn consume(self, exit: Exit<A, E>) -> Step {
        drop(self.result.put(exit));
        if let Some(mark) = self.selected {
            mark.complete();
        }
        Step::Done
    }
}
struct Child<N: Description, V: Send + 'static> {
    description: Option<ScopedProgram<N>>,
    admission: Option<<ScopedProgram<N> as Spec>::Admission<V>>,
    result: Slot<Exit<N::Value, N::Error>>,
    store: Option<Store<N::Value, N::Error>>,
    step: Step,
    poll_context: PollContext,
    wait: PreparedCancellationWait<V, N::Error>,
    provider_cancel: PreparedCancellation,
    context: PreparedRunContext<V, N::Error>,
}
impl<N: Description, V: Send> Future for Child<N, V> {
    type Output = Exit<N::Value, N::Error>;
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        // SAFETY: no inline future is projected. Step owns separately pinned
        // frames; description/control Options are consumed before dispatch.
        let this = unsafe { self.get_unchecked_mut() };
        if this.provider_cancel.is_requested() {
            let _ = this.context.request_cancellation();
        }
        let _ = Pin::new(&mut this.wait).poll(cx);
        if let Some(description) = this.description.take() {
            let (Some(admission), Some(store)) = (this.admission.take(), this.store.take()) else {
                return Poll::Ready(Exit::Die(Defect::from(
                    crate::LibraryDefect::MissingInterpreterOutcome,
                )));
            };
            this.step = description
                .dispatch(admission, Continuation(Once::prepared(store.0, store.1)))
                .0;
        }
        if poll_drive(
            &mut this.step,
            &this.context,
            &mut this.poll_context,
            Publication::Publish,
            &mut || (),
            cx,
        )
        .is_pending()
        {
            return Poll::Pending;
        }
        Poll::Ready(match this.result.take() {
            Ok(Some(exit)) => this.context.publish(exit),
            Ok(None) | Err(_) => Exit::Die(Defect::from(
                crate::LibraryDefect::MissingInterpreterOutcome,
            )),
        })
    }
}
struct Abandon<E: Send + 'static> {
    cancel: Cancel<E>,
}
impl<E: Send> Consume<(), ()> for Abandon<E> {
    fn consume(self, (): ()) {
        self.cancel.request();
    }
}
struct Cleanup<A: Send + 'static, E: Send + 'static, D: ProviderWorkingSet, V: Send + 'static> {
    task: Task<A, E, D>,
    cancel: Cancel<E>,
    pending: FrameReservation<'static, CleanupWait<A, E, D, V>>,
    ownership: PreparedOutputOwnerReservation<Lease<A, E, D>, V, E>,
    forward: FrameReservation<'static, CleanupForward<A, E>>,
    panic: Option<crate::PreparedPanicReservation>,
    provider_panic: Option<crate::PreparedPanicReservation>,
    context: PreparedRunContext<V, E>,
}
struct CleanupForward<A: Send + 'static, E: Send + 'static> {
    panic: Option<crate::PreparedPanicReservation>,
    finish: Finish<(), E>,
    marker: PhantomData<fn() -> A>,
}
impl<A: Send, E: Send> Consume<Exit<A, E>, Step> for CleanupForward<A, E> {
    fn consume(self, exit: Exit<A, E>) -> Step {
        let outcome = match exit {
            Exit::Success(value) => match catch_unwind(AssertUnwindSafe(|| drop(value))) {
                Ok(()) => Exit::Success(()),
                Err(payload) => Exit::Die(super::super::panic_defect(self.panic, payload)),
            },
            Exit::Interrupted => Exit::Success(()),
            Exit::Fail(error) => Exit::Fail(error),
            Exit::Die(defect) => Exit::Die(defect),
            Exit::FinalizationFailed(failed) => Exit::FinalizationFailed(failed),
        };
        self.finish.call(outcome).unwrap_or(Step::Done)
    }
}
impl<A: Send, E: Send, D: ProviderWorkingSet, V: Send> Consume<Finish<(), E>, Step>
    for Cleanup<A, E, D, V>
{
    fn consume(self, finish: Finish<(), E>) -> Step {
        self.cancel.request();
        Step::Suspended(Pending::prepared(
            self.pending,
            CleanupWait {
                task: self.task,
                receipt: None,
                ownership: Some(self.ownership),
                forward: Some(self.forward),
                panic: self.panic,
                provider_panic: self.provider_panic,
                finish: Some(finish),
                context: self.context,
            },
        ))
    }
}
struct CleanupWait<A: Send + 'static, E: Send + 'static, D: ProviderWorkingSet, V: Send + 'static> {
    forward: Option<FrameReservation<'static, CleanupForward<A, E>>>,
    panic: Option<crate::PreparedPanicReservation>,
    provider_panic: Option<crate::PreparedPanicReservation>,
    finish: Option<Finish<(), E>>,
    ownership: Option<PreparedOutputOwnerReservation<Lease<A, E, D>, V, E>>,
    context: PreparedRunContext<V, E>,
    receipt: Option<Receipt<A, E, D>>,
    // Last private entry guard: waiter/capture descendants precede D retirement.
    task: Task<A, E, D>,
}
/// Restore only an unobserved, privately owned receipt. Its actor already took
/// the state slot, so replacement cannot destroy a prior receipt under the gate.
fn restore_receipt<A: Send, E: Send, D: ProviderWorkingSet>(
    task: &Task<A, E, D>,
    receipt: &mut Option<Receipt<A, E, D>>,
) -> bool {
    if let Some(receipt) = receipt.take() {
        drop(task.update(|state| {
            state.receipt = Some(receipt);
            state.phase = Phase::Running;
        }));
        true
    } else {
        false
    }
}
/// Stack-only unwind rollback while native observer unlink/destruction runs.
struct ReceiptRollback<'a, A: Send + 'static, E: Send + 'static, D: ProviderWorkingSet> {
    task: &'a Task<A, E, D>,
    receipt: &'a mut Option<Receipt<A, E, D>>,
    armed: bool,
}
impl<A: Send, E: Send, D: ProviderWorkingSet> Drop for ReceiptRollback<'_, A, E, D> {
    fn drop(&mut self) {
        if self.armed && restore_receipt(self.task, self.receipt) {
            self.task.notify();
        }
    }
}
impl<A: Send, E: Send, D: ProviderWorkingSet, V: Send> Drop for CleanupWait<A, E, D, V> {
    fn drop(&mut self) {
        // A cancelled close driver is not observation or settlement. Keep the
        // actual receipt/entry/D available to escaped observers or another owner.
        if restore_receipt(&self.task, &mut self.receipt) {
            self.task.notify();
        }
    }
}
impl<A: Send, E: Send, D: ProviderWorkingSet, V: Send> Unpin for CleanupWait<A, E, D, V> {}
impl<A: Send, E: Send, D: ProviderWorkingSet, V: Send> Future for CleanupWait<A, E, D, V> {
    type Output = Step;
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Step> {
        if self.receipt.is_none() {
            let ready = self.task.update(|state| match state.phase {
                Phase::Prepared | Phase::Consumed => Some(None),
                Phase::Running => {
                    state.phase = Phase::Cleaning;
                    Some(state.receipt.take())
                }
                Phase::Submitting | Phase::Joining | Phase::Cleaning => None,
            });
            match ready {
                Ok(Some(None)) => {
                    return Poll::Ready(
                        self.finish
                            .take()
                            .and_then(|f| f.call(Exit::Success(())))
                            .unwrap_or(Step::Done),
                    );
                }
                Ok(Some(Some(receipt))) => self.receipt = Some(receipt),
                Err(defect) => {
                    return Poll::Ready(
                        self.finish
                            .take()
                            .and_then(|f| f.call(Exit::Die(defect)))
                            .unwrap_or(Step::Done),
                    );
                }
                Ok(None) => {
                    let waiter = cx.waker().clone();
                    let installed = self.task.update(|state| {
                        if matches!(
                            state.phase,
                            Phase::Submitting | Phase::Joining | Phase::Cleaning
                        ) {
                            Ok(state.cleanup_waiter.replace(waiter))
                        } else {
                            Err(waiter)
                        }
                    });
                    match installed {
                        Ok(Ok(old)) => {
                            drop(old);
                            return Poll::Pending;
                        }
                        Ok(Err(waiter)) => {
                            drop(waiter);
                            cx.waker().wake_by_ref();
                            return Poll::Pending;
                        }
                        Err(defect) => {
                            return Poll::Ready(
                                self.finish
                                    .take()
                                    .and_then(|f| f.call(Exit::Die(defect)))
                                    .unwrap_or(Step::Done),
                            );
                        }
                    }
                }
            }
        }
        let Some(receipt) = self.receipt.as_mut() else {
            return Poll::Pending;
        };
        receipt.request_cancel();
        if receipt.poll_final(cx).is_pending() {
            self.context.acknowledge_clock();
            return Poll::Pending;
        }
        let (Some(receipt), Some(ownership), Some(forward), Some(finish)) = (
            self.receipt.take(),
            self.ownership.take(),
            self.forward.take(),
            self.finish.take(),
        ) else {
            return Poll::Ready(Step::Done);
        };
        drop(self.task.update(|state| state.phase = Phase::Consumed));
        // SAFETY: only this private task cell owned its receipt, with no public
        // result aliases; poll_final excludes further worker result writes.
        let output = unsafe { receipt.into_owned_for_graph() };
        let panic = self.panic.take();
        let provider_panic = self.provider_panic.take();
        let _notify = NotifyOnDrop(&self.task);
        let step = output.handoff_exit(
            ownership,
            &self.context,
            provider_panic,
            Once::prepared(
                forward,
                CleanupForward {
                    panic,
                    finish,
                    marker: PhantomData,
                },
            ),
        );
        Poll::Ready(step)
    }
}

pub struct ForkAdmission<N: Description, K: Send + 'static, V: Send + 'static>
where
    N::Env: HasPreparedProvider<N::Value, N::Error, K>,
{
    entry: FrameReservation<'static, ForkEntry<N, K, V>>,
    controls: ForkControls<N, K, V>,
}
struct ForkControls<N: Description, K: Send + 'static, V: Send + 'static>
where
    N::Env: HasPreparedProvider<N::Value, N::Error, K>,
{
    driver: FrameReservation<'static, Child<N, V>>,
    child: <ScopedProgram<N> as Spec>::Admission<V>,
    result: Slot<Exit<N::Value, N::Error>>,
    store: Store<N::Value, N::Error>,
    wait: PreparedCancellationWait<V, N::Error>,
    poll_context: PollContext,
    worker: PreparedRunContext<V, N::Error>,
    preparation: PreparationValue<N, K>,
    snapshot: super::super::OriginSnapshotReservation<V, N::Error>,
    origins: super::super::NativeGraphOrigins<Budget<N, K>>,
    task: Task<N::Value, N::Error, Budget<N, K>>,
    fiber: PreparedFiber<N::Value, N::Error, Budget<N, K>>,
    finalizer: Registered<N::Error>,
    context: PreparedRunContext<V, N::Error>,
}
struct ForkEntry<N: Description, K: Send + 'static, V: Send + 'static>
where
    N::Env: HasPreparedProvider<N::Value, N::Error, K>,
{
    description: Fork<N, K>,
    controls: ForkControls<N, K, V>,
    finish: Finish<ChildFiber<N, K>, N::Error>,
}
fn refusal(error: ProviderRefusal) -> MemoryError {
    match error {
        ProviderRefusal::Memory(error) => error,
        ProviderRefusal::Closed => MemoryError::Closed,
        ProviderRefusal::InvalidEnvelope | ProviderRefusal::InsufficientWorkers => {
            MemoryError::UnsupportedRunContract
        }
    }
}
impl<N, K, V> Consume<(), Step> for ForkEntry<N, K, V>
where
    N: Description,
    K: Send + 'static,
    V: Send + 'static,
    N::Env: HasPreparedProvider<N::Value, N::Error, K>,
{
    fn consume(self, (): ()) -> Step {
        let ForkControls {
            driver,
            child,
            result,
            store,
            wait,
            poll_context,
            worker,
            preparation,
            snapshot,
            origins,
            task,
            fiber,
            finalizer,
            context,
        } = self.controls;
        // These captures are later locals than the pre-held controls, so their
        // payload/descendants retire before D on cancellation/closed-target exit.
        let Fork {
            program, permit, ..
        } = self.description;
        let PreparedProviderConstruction {
            submission,
            diagnostics,
            retention,
        } = match preparation.take() {
            Ok(value) => value,
            Err(_) => {
                return self
                    .finish
                    .call(Exit::Die(Defect::from(
                        crate::LibraryDefect::PreparedConstructionUnbound,
                    )))
                    .unwrap_or(Step::Done);
            }
        };
        snapshot.bind();
        submission.install_graph_origins(origins);
        if let Err(defect) = task.update(|state| {
            state.retention = Some(retention);
        }) {
            return self.finish.call(Exit::Die(defect)).unwrap_or(Step::Done);
        }
        if context.is_cancelled() {
            return self.finish.call(Exit::Interrupted).unwrap_or(Step::Done);
        }
        drop(task.update(|state| state.phase = Phase::Submitting));
        // Install the armed (Submitting) record BEFORE submitting. A concurrent
        // close cancels the child and waits for publication, never mistakes a
        // queued/running job for an unstarted empty record.
        if let Err(finalizer) = permit.into_entry().commit_prepared(finalizer) {
            drop(task.update(|state| state.phase = Phase::Consumed));
            drop(finalizer);
            return self
                .finish
                .call(Exit::Die(Defect::from(
                    crate::LibraryDefect::ScopeRegistrationClosed,
                )))
                .unwrap_or(Step::Done);
        }
        let description = scope_program(program);
        let receipt = submission.submit_graph(diagnostics, move |_, provider_cancel| {
            PreparedTaskFuture::place(
                driver,
                Child {
                    description: Some(description),
                    admission: Some(child),
                    result,
                    store: Some(store),
                    step: Step::Done,
                    poll_context,
                    wait,
                    provider_cancel,
                    context: worker,
                },
            )
        });
        let (outcome, receipt) = match receipt {
            Ok(receipt) => (Exit::Success(fiber), Some(receipt)),
            Err(_) => (
                Exit::Die(Defect::from(
                    crate::LibraryDefect::PreparedProviderHandoffRefused,
                )),
                None,
            ),
        };
        drop(task.update(|state| {
            state.phase = if receipt.is_some() {
                Phase::Running
            } else {
                Phase::Consumed
            };
            state.receipt = receipt;
        }));
        task.notify();
        self.finish.call(outcome).unwrap_or(Step::Done)
    }
}
impl<N, K> Description for Fork<N, K>
where
    N: Description,
    K: Send + 'static,
    N::Env: HasPreparedProvider<N::Value, N::Error, K>,
{
}
impl<N, K> Spec for Fork<N, K>
where
    N: Description,
    K: Send + 'static,
    N::Env: HasPreparedProvider<N::Value, N::Error, K>,
{
    type Value = PreparedFiber<N::Value, N::Error, Budget<N, K>>;
    type Error = N::Error;
    type Env = N::Env;
    type Storage = <ScopedProgram<N> as Spec>::Storage;
    type Admission<V: Send + 'static> = ForkAdmission<N, K, V>;
    fn provision<V: Send + 'static>(
        memory: &PreparedMemory<'static>,
        region: RegionId,
        capacity: usize,
        frames: &FramePool<'static>,
        cleanup: Option<&crate::PreparedCleanupConfig<Self::Error>>,
    ) -> Result<Self::Storage, MemoryError> {
        let _ = cleanup;
        ScopedProgram::<N>::provision::<V>(memory, region, capacity, frames, cleanup)
    }
    fn storage_bytes<V: Send + 'static>(
        capacity: usize,
        layout: Layout,
    ) -> Result<Bytes, MemoryError> {
        ScopedProgram::<N>::storage_bytes::<V>(capacity, layout)
    }
    fn scope_finalizers() -> Result<usize, MemoryError> {
        Ok(0)
    }
    fn shape<V: Send + 'static>() -> Result<Demand, MemoryError> {
        let demand =
            ScopedProgram::<N>::shape::<V>()?
                .add(Demand::panic_sites(2))?
                .add(Demand::cell::<ForkEntry<N, K, V>>(false))?
                .add(Demand::cell::<Child<N, V>>(false))?
                .add(Demand::cell::<ResultStore<N::Value, N::Error>>(true))?
                .add(
                    Environment::<Mutex<Option<Exit<N::Value, N::Error>>>>::demand::<V, N::Error>(),
                )?
                .add(
                    Environment::<Mutex<State<N::Value, N::Error, Budget<N, K>>>>::demand::<
                        V,
                        N::Error,
                    >(),
                )?
                .add({
                    let mut demand = Demand::cell::<PreparedRunContext<V, N::Error>>(false);
                    demand.origin_frames = 1;
                    demand
                })?
                .add(Demand::cell::<
                    Settlement<Cleanup<N::Value, N::Error, Budget<N, K>, V>, ()>,
                >(true))?
                .add(Demand::cell::<Abandon<N::Error>>(false))?
                .add(Demand::cell::<
                    CleanupWait<N::Value, N::Error, Budget<N, K>, V>,
                >(false))?
                .add(Demand::cell::<CleanupForward<N::Value, N::Error>>(true))?
                .add(output_owner_shape::<
                    Lease<N::Value, N::Error, Budget<N, K>>,
                    V,
                    N::Error,
                >())?
                .add(output_origin_shape::<N::Env, V, N::Error>())?
                .add(native_origins_shape::<Budget<N, K>, V, N::Error>()?)?;
        if <N::Env as HasPreparedProvider<N::Value, N::Error, K>>::CONSTRUCTION_KEY.is_none() {
            demand.add(Demand {
                frame_layout: PreparationValue::<N, K>::layout::<V, N::Error>(),
                frames_per_run: 1,
                continuations: 0,
                descriptions: 0,
                origin_frames: 0,
                environment_origin_frames: 0,
                record_payload: Bytes::new(0),
                cleanup_sites: 0,
                panic_sites: 0,
            })
        } else {
            Ok(demand)
        }
    }
    fn reserve_scoped<V: Send + 'static>(
        frames: &FramePool<'static>,
        storage: &Self::Storage,
        environment: &Environment<N::Env>,
        context: &PreparedRunContext<V, N::Error>,
        _scope: &PreparedScope<N::Error>,
        _cleanup: &PreparedRunContext<V, N::Error>,
    ) -> Result<Self::Admission<V>, MemoryError> {
        Self::reserve(frames, storage, environment, context)
    }
    fn reserve<V: Send + 'static>(
        frames: &FramePool<'static>,
        storage: &Self::Storage,
        environment: &Environment<N::Env>,
        context: &PreparedRunContext<V, N::Error>,
    ) -> Result<Self::Admission<V>, MemoryError> {
        let origin = output_origin_reserve::<N::Env, V, N::Error>(frames, environment, context)?;
        let preparation = if let Some(key) =
            <N::Env as HasPreparedProvider<N::Value, N::Error, K>>::CONSTRUCTION_KEY
        {
            context.construction_value::<Preparation<N, K>>(key)?
        } else {
            PreparationValue::<N, K>::native(frames, context, || {
                environment
                    .with(|env| {
                        let submission = env.prepared_provider().reserve().map_err(refusal)?;
                        let retention = submission.retain_graph_origin_preflight()?;
                        let budget = env.reserve_provider_budget()?;
                        let diagnostics = submission.install_graph_budget(budget);
                        Ok::<_, MemoryError>(PreparedProviderConstruction {
                            submission,
                            diagnostics,
                            retention,
                        })
                    })
                    .map_err(|_| MemoryError::UnsupportedRunContract)?
            })?
        };
        let worker = context.child()?;
        let (snapshot, origins) =
            super::super::OriginSnapshotReservation::reserve(frames, &worker)?;
        let protected = context.protected()?;
        let task = Task::prepare(frames, &worker)?;
        let cancel = Cancel::prepare(frames, &worker)?;
        let result = Slot::prepare(frames, &worker)?;
        let cleanup = Cleanup {
            task: task.clone_preflight()?,
            cancel: cancel.clone_preflight()?,
            pending: frames.reserve()?,
            ownership: worker.reserve_output_owner(frames)?,
            forward: frames.reserve()?,
            panic: protected.reserve_panic()?,
            provider_panic: protected.reserve_panic()?,
            context: protected,
        };
        let finalizer = Registered::from_prepared_settlement(
            frames.reserve()?,
            Settlement {
                operation: cleanup,
                guard: (),
            },
            frames.reserve()?,
            Abandon {
                cancel: cancel.clone_preflight()?,
            },
        );
        let fiber = PreparedFiber {
            task: task.clone_preflight()?,
            cancel,
        };
        let admission = ForkAdmission {
            entry: frames.reserve()?,
            controls: ForkControls {
                driver: frames.reserve()?,
                child: ScopedProgram::<N>::reserve(frames, storage, environment, &worker)?,
                store: (
                    frames.reserve()?,
                    ResultStore {
                        result: result.try_clone()?,
                        selected: None,
                    },
                ),
                result,
                wait: worker.cancelled()?,
                poll_context: worker.poll_context(),
                worker,
                preparation,
                snapshot,
                origins,
                task,
                fiber,
                finalizer,
                context: context.try_clone()?,
            },
        };
        origin
            .attach(environment.try_clone()?)
            .map_err(|(error, _)| error)?;
        Ok(admission)
    }
    fn dispatch<V: Send + 'static>(
        self,
        admission: Self::Admission<V>,
        finish: Continuation<Self::Value, N::Error>,
    ) -> Transition {
        Transition(Step::More(Once::prepared(
            admission.entry,
            ForkEntry {
                description: self,
                controls: admission.controls,
                finish: finish.0,
            },
        )))
    }
}

pub struct Join<A: Send + 'static, E: Send + 'static, D: ProviderWorkingSet, R> {
    fiber: PreparedFiber<A, E, D>,
    marker: PhantomData<fn() -> R>,
}
pub struct JoinAdmission<
    A: Send + 'static,
    E: Send + 'static,
    D: ProviderWorkingSet,
    V: Send + 'static,
> {
    entry: FrameReservation<'static, JoinEntry<A, E, D, V>>,
    pending: FrameReservation<'static, JoinWait<A, E, D, V>>,
    cancellation: PreparedCancellationWait<V, E>,
    waiter: waiter::Waiter,
    ownership: PreparedOutputOwnerReservation<Lease<A, E, D>, V, E>,
    metadata: PreparedOutputOwnerReservation<Task<A, E, D>, V, E>,
    context: PreparedRunContext<V, E>,
    panic: Option<crate::PreparedPanicReservation>,
}
struct JoinEntry<A: Send + 'static, E: Send + 'static, D: ProviderWorkingSet, V: Send + 'static> {
    fiber: PreparedFiber<A, E, D>,
    pending: FrameReservation<'static, JoinWait<A, E, D, V>>,
    cancellation: PreparedCancellationWait<V, E>,
    waiter: waiter::Waiter,
    ownership: PreparedOutputOwnerReservation<Lease<A, E, D>, V, E>,
    metadata: PreparedOutputOwnerReservation<Task<A, E, D>, V, E>,
    context: PreparedRunContext<V, E>,
    panic: Option<crate::PreparedPanicReservation>,
    finish: Finish<A, E>,
}
struct JoinWait<A: Send + 'static, E: Send + 'static, D: ProviderWorkingSet, V: Send + 'static> {
    waiter: waiter::Waiter,
    cancellation: PreparedCancellationWait<V, E>,
    finish: Option<Finish<A, E>>,
    cancel: Cancel<E>,
    ownership: Option<PreparedOutputOwnerReservation<Lease<A, E, D>, V, E>>,
    metadata: Option<PreparedOutputOwnerReservation<Task<A, E, D>, V, E>>,
    context: PreparedRunContext<V, E>,
    panic: Option<crate::PreparedPanicReservation>,
    receipt: Option<Receipt<A, E, D>>,
    // Drop puts an unobserved receipt back before waking cleanup; this last
    // guard then protects D through every preceding native waiter/capture Drop.
    task: Option<Task<A, E, D>>,
}
impl<A: Send, E: Send, D: ProviderWorkingSet, V: Send> Consume<(), Step> for JoinEntry<A, E, D, V> {
    fn consume(self, (): ()) -> Step {
        Step::Suspended(Pending::prepared(
            self.pending,
            JoinWait {
                task: Some(self.fiber.task),
                cancel: self.fiber.cancel,
                cancellation: self.cancellation,
                waiter: self.waiter,
                receipt: None,
                ownership: Some(self.ownership),
                metadata: Some(self.metadata),
                finish: Some(self.finish),
                context: self.context,
                panic: self.panic,
            },
        ))
    }
}
impl<A: Send, E: Send, D: ProviderWorkingSet, V: Send> Unpin for JoinWait<A, E, D, V> {}
impl<A: Send, E: Send, D: ProviderWorkingSet, V: Send> JoinWait<A, E, D, V> {
    fn unlink_waiter(&mut self) {
        if let Some(task) = &self.task {
            // Successful observation keeps its receipt. If native waker Drop
            // unwinds, publish it back even if the host retains this future.
            let mut rollback = ReceiptRollback {
                task,
                receipt: &mut self.receipt,
                armed: true,
            };
            self.waiter.unlink(task);
            rollback.armed = false;
        }
    }
    fn abandon_observation(&mut self) {
        if let Some(task) = &self.task {
            // Publish BEFORE unlink can destroy a native waker. Notification is
            // guarded through that destruction, and task/D stay retained here.
            let returned = restore_receipt(task, &mut self.receipt);
            let _notify = if returned {
                Some(NotifyOnDrop(task))
            } else {
                None
            };
            self.waiter.unlink(task);
        }
    }
}
impl<A: Send, E: Send, D: ProviderWorkingSet, V: Send> Drop for JoinWait<A, E, D, V> {
    fn drop(&mut self) {
        self.abandon_observation();
    }
}
impl<A: Send, E: Send, D: ProviderWorkingSet, V: Send> Future for JoinWait<A, E, D, V> {
    type Output = Step;
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Step> {
        if Pin::new(&mut self.cancellation).poll(cx).is_ready() {
            self.abandon_observation();
            return Poll::Ready(
                self.finish
                    .take()
                    .and_then(|finish| finish.call(Exit::Interrupted))
                    .unwrap_or(Step::Done),
            );
        }
        if self.receipt.is_none() {
            let Some(task) = &self.task else {
                return Poll::Pending;
            };
            let receipt = task.update(|state| {
                if state.phase == Phase::Running {
                    state.phase = Phase::Joining;
                    (Phase::Running, state.receipt.take())
                } else {
                    (state.phase, None)
                }
            });
            match receipt {
                Ok((_, Some(receipt))) => self.receipt = Some(receipt),
                Ok((Phase::Consumed, None)) => {
                    return Poll::Ready(
                        self.finish
                            .take()
                            .and_then(|finish| {
                                finish.call(Exit::Die(Defect::from(
                                    crate::LibraryDefect::FiberOutcomeConsumed,
                                )))
                            })
                            .unwrap_or(Step::Done),
                    );
                }
                Ok((
                    Phase::Prepared
                    | Phase::Submitting
                    | Phase::Running
                    | Phase::Joining
                    | Phase::Cleaning,
                    None,
                )) => {
                    let this = self.as_mut().get_mut();
                    let waiting = match &this.task {
                        Some(task) => this.waiter.listen(task, cx),
                        None => return Poll::Pending,
                    };
                    match waiting {
                        Ok(ready) => {
                            if ready {
                                cx.waker().wake_by_ref();
                            }
                            this.context.acknowledge_clock();
                            return Poll::Pending;
                        }
                        Err(defect) => {
                            return Poll::Ready(
                                this.finish
                                    .take()
                                    .and_then(|finish| finish.call(Exit::Die(defect)))
                                    .unwrap_or(Step::Done),
                            );
                        }
                    }
                }
                Err(defect) => {
                    return Poll::Ready(
                        self.finish
                            .take()
                            .and_then(|finish| finish.call(Exit::Die(defect)))
                            .unwrap_or(Step::Done),
                    );
                }
            }
        }
        let Some(receipt) = self.receipt.as_mut() else {
            return Poll::Pending;
        };
        if receipt.poll_final(cx).is_pending() {
            self.context.acknowledge_clock();
            return Poll::Pending;
        }
        if self.context.is_cancelled() {
            self.abandon_observation();
            return Poll::Ready(
                self.finish
                    .take()
                    .and_then(|finish| finish.call(Exit::Interrupted))
                    .unwrap_or(Step::Done),
            );
        }
        self.unlink_waiter();
        let (Some(task), Some(receipt), Some(ownership), Some(metadata), Some(finish)) = (
            self.task.take(),
            self.receipt.take(),
            self.ownership.take(),
            self.metadata.take(),
            self.finish.take(),
        ) else {
            return Poll::Ready(Step::Done);
        };
        // SAFETY: affine private receipt, no public result aliases and actual
        // final settlement observed, just as for awaited scheduled leaves.
        let output = unsafe { receipt.into_owned_for_graph() };
        let lost = output.graph_driver_lost();
        drop(task.update(|state| {
            state.driver_lost = lost;
            state.phase = Phase::Consumed;
        }));
        self.context
            .root
            .contexts
            .protected_failure(self.context.id, self.cancel.protected());
        let panic = self.panic.take();
        let step = {
            let _notify = NotifyOnDrop(&task);
            output.handoff_exit(ownership, &self.context, panic, finish)
        };
        // Attach the actual originating child metadata AFTER the provider owner,
        // before publishing the next Step. No consumer can release it meanwhile:
        // metadata itself is a pre-held output consumer/attachment authority.
        drop(metadata.install_for_invocation(task));
        Poll::Ready(step)
    }
}
impl<A: Send, E: Send, D: ProviderWorkingSet, R: Send + Sync + 'static> Description
    for Join<A, E, D, R>
{
}
impl<A: Send, E: Send, D: ProviderWorkingSet, R: Send + Sync + 'static> Spec for Join<A, E, D, R> {
    type Value = A;
    type Error = E;
    type Env = R;
    type Storage = ();
    type Admission<V: Send + 'static> = JoinAdmission<A, E, D, V>;
    fn provision<V: Send + 'static>(
        _memory: &PreparedMemory<'static>,
        _region: RegionId,
        _capacity: usize,
        _frames: &FramePool<'static>,
        cleanup: Option<&crate::PreparedCleanupConfig<Self::Error>>,
    ) -> Result<(), MemoryError> {
        let _ = cleanup;
        Ok(())
    }
    fn scope_finalizers() -> Result<usize, MemoryError> {
        Ok(0)
    }
    fn shape<V: Send + 'static>() -> Result<Demand, MemoryError> {
        Demand::cell::<JoinEntry<A, E, D, V>>(false)
            .add(Demand::panic_sites(1))?
            .add(Demand::cell::<JoinWait<A, E, D, V>>(false))?
            .add(output_owner_shape::<Lease<A, E, D>, V, E>())?
            .add(output_owner_shape::<Task<A, E, D>, V, E>())?
            .add(Demand {
                frame_layout: waiter::Waiter::layout::<V, E>(),
                frames_per_run: 1,
                continuations: 0,
                descriptions: 0,
                origin_frames: 0,
                environment_origin_frames: 0,
                record_payload: Bytes::new(0),
                cleanup_sites: 0,
                panic_sites: 0,
            })
    }
    fn reserve_scoped<V: Send + 'static>(
        frames: &FramePool<'static>,
        storage: &(),
        environment: &Environment<R>,
        context: &PreparedRunContext<V, E>,
        _scope: &PreparedScope<E>,
        _cleanup: &PreparedRunContext<V, E>,
    ) -> Result<Self::Admission<V>, MemoryError> {
        Self::reserve(frames, storage, environment, context)
    }
    fn reserve<V: Send + 'static>(
        frames: &FramePool<'static>,
        _storage: &(),
        _environment: &Environment<R>,
        context: &PreparedRunContext<V, E>,
    ) -> Result<Self::Admission<V>, MemoryError> {
        Ok(JoinAdmission {
            entry: frames.reserve()?,
            pending: frames.reserve()?,
            cancellation: context.cancelled()?,
            waiter: waiter::Waiter::prepare(frames, context)?,
            ownership: context.reserve_output_owner(frames)?,
            metadata: context.reserve_output_owner(frames)?,
            context: context.try_clone()?,
            panic: context.reserve_panic()?,
        })
    }
    fn dispatch<V: Send + 'static>(
        self,
        admission: Self::Admission<V>,
        finish: Continuation<A, E>,
    ) -> Transition {
        Transition(Step::More(Once::prepared(
            admission.entry,
            JoinEntry {
                fiber: self.fiber,
                pending: admission.pending,
                cancellation: admission.cancellation,
                waiter: admission.waiter,
                ownership: admission.ownership,
                metadata: admission.metadata,
                context: admission.context,
                panic: admission.panic,
                finish: finish.0,
            },
        )))
    }
}
