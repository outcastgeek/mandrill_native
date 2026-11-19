//! Race selects at the child's business continuation, then drains BOTH real
//! provider settlements. A driver lost before that continuation also selects.
//! It reuses the same Fork, Join, Scope Closing and poll_drive machinery.
use super::super::resource::Closing;
use super::super::scope_binding::{Construction, ScopeConstruction};
use super::*;
use crate::{PreparedScopeLimits, PreparedScopePool};

#[derive(Clone, Copy)]
enum Side {
    Left,
    Right,
}
struct Selected {
    winner: Option<Side>,
    waiter: Option<Waker>,
}
struct Selection {
    value: Environment<Mutex<Selected>>,
}
pub(super) struct Mark {
    selection: Selection,
    side: Side,
}
impl Mark {
    pub(super) fn complete(self) {
        self.selection.choose(self.side);
    }
}
impl Selection {
    fn prepare<V: Send + 'static, E: Send + 'static>(
        frames: &FramePool<'static>,
        context: &PreparedRunContext<V, E>,
    ) -> Result<Self, MemoryError> {
        let (publisher, value) = Environment::reserve(frames, context)?;
        publisher.install(Mutex::new(Selected {
            winner: None,
            waiter: None,
        }));
        Ok(Self { value })
    }
    fn mark(&self, side: Side) -> Result<Mark, MemoryError> {
        Ok(Mark {
            selection: Self {
                value: self.value.try_clone()?,
            },
            side,
        })
    }
    fn choose(&self, side: Side) {
        let wake = self.value.with(|value| {
            let mut selected = value
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if selected.winner.is_none() {
                selected.winner = Some(side);
                selected.waiter.take()
            } else {
                None
            }
        });
        if let Ok(Some(wake)) = wake {
            wake.wake();
        }
    }
    fn poll(&self, cx: &mut Context<'_>) -> Result<Option<Side>, Defect> {
        let waiter = cx.waker().clone();
        let (winner, old) = self.value.with(|value| {
            let mut selected = value
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            match selected.winner {
                Some(side) => (Some(side), Some(waiter)),
                None => (None, selected.waiter.replace(waiter)),
            }
        })?;
        drop(old);
        Ok(winner)
    }
}
pub struct Race<L, R, KL = TaskProvider, KR = TaskProvider> {
    left: L,
    right: R,
    marker: PhantomData<fn() -> (KL, KR)>,
}
#[must_use]
pub fn race<L, R>(left: L, right: R) -> Race<L, R>
where
    L: Description,
    R: Description<Value = L::Value, Error = L::Error, Env = L::Env>,
    L::Env: HasPreparedProvider<L::Value, L::Error, TaskProvider>,
{
    Race {
        left,
        right,
        marker: PhantomData,
    }
}
#[must_use]
pub fn race_on<L, R, KL, KR>(left: L, right: R) -> Race<L, R, KL, KR>
where
    L: Description,
    R: Description<Value = L::Value, Error = L::Error, Env = L::Env>,
    KL: Send + 'static,
    KR: Send + 'static,
    L::Env:
        HasPreparedProvider<L::Value, L::Error, KL> + HasPreparedProvider<L::Value, L::Error, KR>,
{
    Race {
        left,
        right,
        marker: PhantomData,
    }
}
pub struct Storage<LS, RS, E: Send + 'static> {
    left: LS,
    right: RS,
    scopes: PreparedScopePool<E>,
}
struct WaitControls<
    L: Description,
    R: Description<Value = L::Value, Error = L::Error, Env = L::Env>,
    KL: Send + 'static,
    KR: Send + 'static,
    V: Send + 'static,
> where
    L::Env:
        HasPreparedProvider<L::Value, L::Error, KL> + HasPreparedProvider<L::Value, L::Error, KR>,
{
    left_join: JoinAdmission<L::Value, L::Error, Budget<L, KL>, V>,
    right_join: JoinAdmission<R::Value, R::Error, Budget<R, KR>, V>,
    left_store: Store<L::Value, L::Error>,
    right_store: Store<L::Value, L::Error>,
    left_result: Slot<Exit<L::Value, L::Error>>,
    right_result: Slot<Exit<L::Value, L::Error>>,
    left_cancel: Cancel<L::Error>,
    right_cancel: Cancel<L::Error>,
    left_task: Task<L::Value, L::Error, Budget<L, KL>>,
    right_task: Task<L::Value, L::Error, Budget<R, KR>>,
    left_context: PreparedRunContext<V, L::Error>,
    right_context: PreparedRunContext<V, L::Error>,
    left_poll: PollContext,
    right_poll: PollContext,
    selection: Selection,
    cancellation: PreparedCancellationWait<V, L::Error>,
    context: PreparedRunContext<V, L::Error>,
    cleanup_buffer: crate::CleanupFailures<L::Error>,
    panic: Option<crate::PreparedPanicReservation>,
}
pub struct Admission<
    L: Description,
    R: Description<Value = L::Value, Error = L::Error, Env = L::Env>,
    KL: Send + 'static,
    KR: Send + 'static,
    V: Send + 'static,
> where
    L::Env:
        HasPreparedProvider<L::Value, L::Error, KL> + HasPreparedProvider<L::Value, L::Error, KR>,
{
    entry: FrameReservation<'static, Start<L, R, KL, KR, V>>,
    left: ForkAdmission<L, KL, V>,
    right: ForkAdmission<R, KR, V>,
    left_permit: ScopePermit<L::Error>,
    right_permit: ScopePermit<L::Error>,
    started_left: FrameReservation<'static, StartedLeft<L, R, KL, KR, V>>,
    started_right: FrameReservation<'static, StartedRight<L, R, KL, KR, V>>,
    pending: FrameReservation<'static, Wait<L, R, KL, KR, V>>,
    controls: WaitControls<L, R, KL, KR, V>,
    closing: Closing<Race<L, R, KL, KR>, V>,
}
struct StartAdmission<
    L: Description,
    R: Description<Value = L::Value, Error = L::Error, Env = L::Env>,
    KL: Send + 'static,
    KR: Send + 'static,
    V: Send + 'static,
> where
    L::Env:
        HasPreparedProvider<L::Value, L::Error, KL> + HasPreparedProvider<L::Value, L::Error, KR>,
{
    left: ForkAdmission<L, KL, V>,
    right: ForkAdmission<R, KR, V>,
    left_permit: ScopePermit<L::Error>,
    right_permit: ScopePermit<L::Error>,
    started_left: FrameReservation<'static, StartedLeft<L, R, KL, KR, V>>,
    started_right: FrameReservation<'static, StartedRight<L, R, KL, KR, V>>,
    pending: FrameReservation<'static, Wait<L, R, KL, KR, V>>,
    controls: WaitControls<L, R, KL, KR, V>,
    closing: Closing<Race<L, R, KL, KR>, V>,
}
struct Start<
    L: Description,
    R: Description<Value = L::Value, Error = L::Error, Env = L::Env>,
    KL: Send + 'static,
    KR: Send + 'static,
    V: Send + 'static,
> where
    L::Env:
        HasPreparedProvider<L::Value, L::Error, KL> + HasPreparedProvider<L::Value, L::Error, KR>,
{
    description: Race<L, R, KL, KR>,
    admission: StartAdmission<L, R, KL, KR, V>,
    finish: Finish<L::Value, L::Error>,
}
struct StartedLeft<
    L: Description,
    R: Description<Value = L::Value, Error = L::Error, Env = L::Env>,
    KL: Send + 'static,
    KR: Send + 'static,
    V: Send + 'static,
> where
    L::Env:
        HasPreparedProvider<L::Value, L::Error, KL> + HasPreparedProvider<L::Value, L::Error, KR>,
{
    right: R,
    right_admission: ForkAdmission<R, KR, V>,
    right_permit: ScopePermit<L::Error>,
    started_right: FrameReservation<'static, StartedRight<L, R, KL, KR, V>>,
    pending: FrameReservation<'static, Wait<L, R, KL, KR, V>>,
    controls: WaitControls<L, R, KL, KR, V>,
    finish: Finish<L::Value, L::Error>,
}
struct StartedRight<
    L: Description,
    R: Description<Value = L::Value, Error = L::Error, Env = L::Env>,
    KL: Send + 'static,
    KR: Send + 'static,
    V: Send + 'static,
> where
    L::Env:
        HasPreparedProvider<L::Value, L::Error, KL> + HasPreparedProvider<L::Value, L::Error, KR>,
{
    left: PreparedFiber<L::Value, L::Error, Budget<L, KL>>,
    pending: FrameReservation<'static, Wait<L, R, KL, KR, V>>,
    controls: WaitControls<L, R, KL, KR, V>,
    finish: Finish<L::Value, L::Error>,
}
impl<L, R, KL, KR, V> Consume<(), Step> for Start<L, R, KL, KR, V>
where
    L: Description,
    R: Description<Value = L::Value, Error = L::Error, Env = L::Env>,
    KL: Send + 'static,
    KR: Send + 'static,
    V: Send + 'static,
    L::Env:
        HasPreparedProvider<L::Value, L::Error, KL> + HasPreparedProvider<L::Value, L::Error, KR>,
{
    fn consume(self, (): ()) -> Step {
        let admission = self.admission;
        let finish = admission.closing.continuation(self.finish);
        let left = Fork::<L, KL> {
            program: self.description.left,
            permit: admission.left_permit,
            marker: PhantomData,
        };
        left.dispatch(
            admission.left,
            Continuation(Once::prepared(
                admission.started_left,
                StartedLeft {
                    right: self.description.right,
                    right_admission: admission.right,
                    right_permit: admission.right_permit,
                    started_right: admission.started_right,
                    pending: admission.pending,
                    controls: admission.controls,
                    finish,
                },
            )),
        )
        .0
    }
}
impl<L, R, KL, KR, V>
    Consume<Exit<PreparedFiber<L::Value, L::Error, Budget<L, KL>>, L::Error>, Step>
    for StartedLeft<L, R, KL, KR, V>
where
    L: Description,
    R: Description<Value = L::Value, Error = L::Error, Env = L::Env>,
    KL: Send + 'static,
    KR: Send + 'static,
    V: Send + 'static,
    L::Env:
        HasPreparedProvider<L::Value, L::Error, KL> + HasPreparedProvider<L::Value, L::Error, KR>,
{
    fn consume(
        self,
        outcome: Exit<PreparedFiber<L::Value, L::Error, Budget<L, KL>>, L::Error>,
    ) -> Step {
        match outcome.into_failure() {
            Ok(left) => {
                Fork::<R, KR> {
                    program: self.right,
                    permit: self.right_permit,
                    marker: PhantomData,
                }
                .dispatch(
                    self.right_admission,
                    Continuation(Once::prepared(
                        self.started_right,
                        StartedRight {
                            left,
                            pending: self.pending,
                            controls: self.controls,
                            finish: self.finish,
                        },
                    )),
                )
                .0
            }
            Err(failure) => self.finish.call(failure).unwrap_or(Step::Done),
        }
    }
}
struct Wait<
    L: Description,
    R: Description<Value = L::Value, Error = L::Error, Env = L::Env>,
    KL: Send + 'static,
    KR: Send + 'static,
    V: Send + 'static,
> where
    L::Env:
        HasPreparedProvider<L::Value, L::Error, KL> + HasPreparedProvider<L::Value, L::Error, KR>,
{
    left: Step,
    right: Step,
    left_result: Slot<Exit<L::Value, L::Error>>,
    right_result: Slot<Exit<L::Value, L::Error>>,
    left_cancel: Cancel<L::Error>,
    right_cancel: Cancel<L::Error>,
    left_task: Task<L::Value, L::Error, Budget<L, KL>>,
    right_task: Task<L::Value, L::Error, Budget<R, KR>>,
    left_context: PreparedRunContext<V, L::Error>,
    right_context: PreparedRunContext<V, L::Error>,
    left_poll: PollContext,
    right_poll: PollContext,
    selection: Selection,
    cancellation: PreparedCancellationWait<V, L::Error>,
    context: PreparedRunContext<V, L::Error>,
    cleanup_buffer: Option<crate::CleanupFailures<L::Error>>,
    panic: Option<crate::PreparedPanicReservation>,
    finish: Option<Finish<L::Value, L::Error>>,
    left_done: bool,
    right_done: bool,
    settled: bool,
}
impl<L, R, KL, KR, V>
    Consume<Exit<PreparedFiber<R::Value, R::Error, Budget<R, KR>>, L::Error>, Step>
    for StartedRight<L, R, KL, KR, V>
where
    L: Description,
    R: Description<Value = L::Value, Error = L::Error, Env = L::Env>,
    KL: Send + 'static,
    KR: Send + 'static,
    V: Send + 'static,
    L::Env:
        HasPreparedProvider<L::Value, L::Error, KL> + HasPreparedProvider<L::Value, L::Error, KR>,
{
    fn consume(
        self,
        outcome: Exit<PreparedFiber<R::Value, R::Error, Budget<R, KR>>, L::Error>,
    ) -> Step {
        match outcome.into_failure() {
            Err(failure) => self.finish.call(failure).unwrap_or(Step::Done),
            Ok(right) => {
                let controls = self.controls;
                let left = Join::<L::Value, L::Error, Budget<L, KL>, L::Env> {
                    fiber: self.left,
                    marker: PhantomData,
                }
                .dispatch(
                    controls.left_join,
                    Continuation(Once::prepared(controls.left_store.0, controls.left_store.1)),
                )
                .0;
                let right = Join::<L::Value, L::Error, Budget<R, KR>, L::Env> {
                    fiber: right,
                    marker: PhantomData,
                }
                .dispatch(
                    controls.right_join,
                    Continuation(Once::prepared(
                        controls.right_store.0,
                        controls.right_store.1,
                    )),
                )
                .0;
                Step::Suspended(Pending::prepared(
                    self.pending,
                    Wait {
                        left,
                        right,
                        left_result: controls.left_result,
                        right_result: controls.right_result,
                        left_cancel: controls.left_cancel,
                        right_cancel: controls.right_cancel,
                        left_task: controls.left_task,
                        right_task: controls.right_task,
                        left_context: controls.left_context,
                        right_context: controls.right_context,
                        left_poll: controls.left_poll,
                        right_poll: controls.right_poll,
                        selection: controls.selection,
                        cancellation: controls.cancellation,
                        context: controls.context,
                        cleanup_buffer: Some(controls.cleanup_buffer),
                        panic: controls.panic,
                        finish: Some(self.finish),
                        left_done: false,
                        right_done: false,
                        settled: false,
                    },
                ))
            }
        }
    }
}
impl<L, R, KL, KR, V> Unpin for Wait<L, R, KL, KR, V>
where
    L: Description,
    R: Description<Value = L::Value, Error = L::Error, Env = L::Env>,
    KL: Send + 'static,
    KR: Send + 'static,
    V: Send + 'static,
    L::Env:
        HasPreparedProvider<L::Value, L::Error, KL> + HasPreparedProvider<L::Value, L::Error, KR>,
{
}
impl<L, R, KL, KR, V> Drop for Wait<L, R, KL, KR, V>
where
    L: Description,
    R: Description<Value = L::Value, Error = L::Error, Env = L::Env>,
    KL: Send + 'static,
    KR: Send + 'static,
    V: Send + 'static,
    L::Env:
        HasPreparedProvider<L::Value, L::Error, KL> + HasPreparedProvider<L::Value, L::Error, KR>,
{
    fn drop(&mut self) {
        if !self.settled {
            self.left_cancel.request();
            self.right_cancel.request();
        }
    }
}
impl<L, R, KL, KR, V> Future for Wait<L, R, KL, KR, V>
where
    L: Description,
    R: Description<Value = L::Value, Error = L::Error, Env = L::Env>,
    KL: Send + 'static,
    KR: Send + 'static,
    V: Send + 'static,
    L::Env:
        HasPreparedProvider<L::Value, L::Error, KL> + HasPreparedProvider<L::Value, L::Error, KR>,
{
    type Output = Step;
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Step> {
        let this = self.get_mut();
        if Pin::new(&mut this.cancellation).poll(cx).is_ready() {
            this.left_cancel.request();
            this.right_cancel.request();
        }
        let selected = this.selection.poll(cx);
        match selected {
            Ok(Some(Side::Left)) => this.right_cancel.request(),
            Ok(Some(Side::Right)) => this.left_cancel.request(),
            Ok(None) => {}
            Err(_) => {
                this.left_cancel.request();
                this.right_cancel.request();
            }
        }
        if !this.left_done
            && poll_drive(
                &mut this.left,
                &this.left_context,
                &mut this.left_poll,
                Publication::Publish,
                &mut || (),
                cx,
            )
            .is_ready()
        {
            this.left_done = true;
            this.selection.choose(Side::Left);
        }
        if !this.right_done
            && poll_drive(
                &mut this.right,
                &this.right_context,
                &mut this.right_poll,
                Publication::Publish,
                &mut || (),
                cx,
            )
            .is_ready()
        {
            this.right_done = true;
            this.selection.choose(Side::Right);
        }
        if !this.left_done || !this.right_done {
            this.context.acknowledge_clock();
            return Poll::Pending;
        }
        let left = this.left_result.take();
        let right = this.right_result.take();
        let outcome = match (this.selection.poll(cx), left, right) {
            (Ok(Some(side)), Ok(Some(left)), Ok(Some(right))) => {
                let (winner, loser, lost, protected) = match side {
                    Side::Left => (
                        left,
                        right,
                        this.right_task.update(|state| state.driver_lost),
                        this.left_cancel.protected(),
                    ),
                    Side::Right => (
                        right,
                        left,
                        this.left_task.update(|state| state.driver_lost),
                        this.right_cancel.protected(),
                    ),
                };
                this.context
                    .root
                    .contexts
                    .protected_failure(this.context.id, protected);
                let winner = this.context.publish(winner);
                match this.cleanup_buffer.take() {
                    Some(buffer) => {
                        let mut accumulator =
                            crate::PreparedCleanupAccumulator::from_reserved(winner, buffer);
                        let cleanup = catch_unwind(AssertUnwindSafe(|| {
                            accumulator.append_loser(loser, lost.unwrap_or(true))
                        }));
                        if let Err(payload) = cleanup {
                            accumulator.append_cleanup(Exit::Die(
                                super::super::super::panic_defect(this.panic.take(), payload),
                            ));
                        }
                        accumulator.finish()
                    }
                    None => Exit::Die(Defect::from(
                        crate::LibraryDefect::MissingInterpreterOutcome,
                    )),
                }
            }
            (Err(defect), _, _) | (_, Err(defect), _) | (_, _, Err(defect)) => Exit::Die(defect),
            (Ok(None), Ok(_), Ok(_))
            | (Ok(Some(_)), Ok(None), Ok(_))
            | (Ok(Some(_)), Ok(_), Ok(None)) => Exit::Die(Defect::from(
                crate::LibraryDefect::MissingInterpreterOutcome,
            )),
        };
        this.settled = true;
        Poll::Ready(
            this.finish
                .take()
                .and_then(|finish| finish.call(outcome))
                .unwrap_or(Step::Done),
        )
    }
}
impl<L, R, KL, KR> Description for Race<L, R, KL, KR>
where
    L: Description,
    R: Description<Value = L::Value, Error = L::Error, Env = L::Env>,
    KL: Send + 'static,
    KR: Send + 'static,
    L::Env:
        HasPreparedProvider<L::Value, L::Error, KL> + HasPreparedProvider<L::Value, L::Error, KR>,
{
}
impl<L, R, KL, KR> Spec for Race<L, R, KL, KR>
where
    L: Description,
    R: Description<Value = L::Value, Error = L::Error, Env = L::Env>,
    KL: Send + 'static,
    KR: Send + 'static,
    L::Env:
        HasPreparedProvider<L::Value, L::Error, KL> + HasPreparedProvider<L::Value, L::Error, KR>,
{
    type Value = L::Value;
    type Error = L::Error;
    type Env = L::Env;
    type Storage =
        Storage<<Fork<L, KL> as Spec>::Storage, <Fork<R, KR> as Spec>::Storage, L::Error>;
    type Admission<V: Send + 'static> = Admission<L, R, KL, KR, V>;
    fn provision<V: Send + 'static>(
        memory: &PreparedMemory<'static>,
        region: RegionId,
        capacity: usize,
        frames: &FramePool<'static>,
        cleanup: Option<&crate::PreparedCleanupConfig<Self::Error>>,
    ) -> Result<Self::Storage, MemoryError> {
        let _ = cleanup;
        let scopes = crate::effect_scope! {prepare_pool;memory:memory,metadata:region,scopes:capacity,limits:PreparedScopeLimits {finalizers:2,segments:0},callbacks:frames.try_clone()?,}?;
        let scopes = scopes.with_cleanup(
            cleanup
                .ok_or(MemoryError::UnsupportedRunContract)?
                .try_clone()?,
        );
        Ok(Storage {
            left: Fork::<L, KL>::provision::<V>(memory, region, capacity, frames, cleanup)?,
            right: Fork::<R, KR>::provision::<V>(memory, region, capacity, frames, cleanup)?,
            scopes,
        })
    }
    fn storage_bytes<V: Send + 'static>(
        capacity: usize,
        layout: Layout,
    ) -> Result<Bytes, MemoryError> {
        Fork::<L, KL>::storage_bytes::<V>(capacity, layout)?
            .checked_add(Fork::<R, KR>::storage_bytes::<V>(capacity, layout)?)?
            .checked_add(
                PreparedScopeLimits {
                    finalizers: 2,
                    segments: 0,
                }
                .pool_demand::<L::Error>(capacity)?
                .storage_payload,
            )
    }
    fn scope_finalizers() -> Result<usize, MemoryError> {
        Ok(0)
    }
    fn shape<V: Send + 'static>() -> Result<Demand, MemoryError> {
        Fork::<L, KL>::shape::<V>()?
            .add(Fork::<R, KR>::shape::<V>()?)?
            .add(Join::<L::Value, L::Error, Budget<L, KL>, L::Env>::shape::<V>()?)?
            .add(Join::<L::Value, L::Error, Budget<R, KR>, L::Env>::shape::<V>()?)?
            .add(Closing::<Self, V>::shape(2)?)?
            .add(Demand::cleanup(1))?
            .add(Demand::panic_sites(1))?
            .add(Demand::cell::<Start<L, R, KL, KR, V>>(false))?
            .add(Demand::cell::<StartedLeft<L, R, KL, KR, V>>(true))?
            .add(Demand::cell::<StartedRight<L, R, KL, KR, V>>(true))?
            .add(Demand::cell::<Wait<L, R, KL, KR, V>>(false))?
            .add(Demand::cell::<ResultStore<L::Value, L::Error>>(true))?
            .add(Demand::cell::<ResultStore<L::Value, L::Error>>(true))?
            .add(Environment::<Mutex<Option<Exit<L::Value, L::Error>>>>::demand::<V, L::Error>())?
            .add(Environment::<Mutex<Option<Exit<L::Value, L::Error>>>>::demand::<V, L::Error>())?
            .add(Environment::<Mutex<Selected>>::demand::<V, L::Error>())
    }
    fn reserve_scoped<V: Send + 'static>(
        frames: &FramePool<'static>,
        storage: &Self::Storage,
        environment: &Environment<L::Env>,
        context: &PreparedRunContext<V, L::Error>,
        _scope: &PreparedScope<L::Error>,
        _cleanup: &PreparedRunContext<V, L::Error>,
    ) -> Result<Self::Admission<V>, MemoryError> {
        Self::reserve(frames, storage, environment, context)
    }
    fn reserve<V: Send + 'static>(
        frames: &FramePool<'static>,
        storage: &Self::Storage,
        environment: &Environment<L::Env>,
        context: &PreparedRunContext<V, L::Error>,
    ) -> Result<Self::Admission<V>, MemoryError> {
        let owner = storage.scopes.checkout()?;
        let owner = match &context.root.panic_pool {
            Some(pool) => owner.with_panic_diagnostics(pool.try_clone()?)?,
            None => owner,
        };
        let scope = owner.handle()?;
        let protected = context.protected()?;
        let (left_permit, right_permit) =
            <(ScopeConstruction, ScopeConstruction) as Construction>::reserve(&scope)?;
        let selection = Selection::prepare(frames, context)?;
        let mut left = Fork::<L, KL>::reserve(frames, &storage.left, environment, context)?;
        let mut right = Fork::<R, KR>::reserve(frames, &storage.right, environment, context)?;
        left.controls.store.1.selected = Some(selection.mark(Side::Left)?);
        right.controls.store.1.selected = Some(selection.mark(Side::Right)?);
        let left_context = context.protected()?;
        let right_context = context.protected()?;
        let left_result = Slot::prepare(frames, context)?;
        let right_result = Slot::prepare(frames, context)?;
        let cleanup_buffer = owner.reserve_cleanup_buffer()?;
        let controls = WaitControls {
            cleanup_buffer,
            panic: context.reserve_panic()?,
            left_join: Join::<L::Value, L::Error, Budget<L, KL>, L::Env>::reserve(
                frames,
                &(),
                environment,
                &left_context,
            )?,
            right_join: Join::<L::Value, L::Error, Budget<R, KR>, L::Env>::reserve(
                frames,
                &(),
                environment,
                &right_context,
            )?,
            left_store: (
                frames.reserve()?,
                ResultStore {
                    result: left_result.try_clone()?,
                    selected: None,
                },
            ),
            right_store: (
                frames.reserve()?,
                ResultStore {
                    result: right_result.try_clone()?,
                    selected: None,
                },
            ),
            left_result,
            right_result,
            left_cancel: left.controls.fiber.cancel.clone_preflight()?,
            right_cancel: right.controls.fiber.cancel.clone_preflight()?,
            left_task: left.controls.task.clone_preflight()?,
            right_task: right.controls.task.clone_preflight()?,
            left_poll: left_context.poll_context(),
            right_poll: right_context.poll_context(),
            left_context,
            right_context,
            selection,
            cancellation: context.cancelled()?,
            context: context.try_clone()?,
        };
        Ok(Admission {
            entry: frames.reserve()?,
            left,
            right,
            left_permit,
            right_permit,
            started_left: frames.reserve()?,
            started_right: frames.reserve()?,
            pending: frames.reserve()?,
            controls,
            closing: Closing::reserve(owner, frames, context, &protected)?,
        })
    }
    fn dispatch<V: Send + 'static>(
        self,
        admission: Self::Admission<V>,
        finish: Continuation<L::Value, L::Error>,
    ) -> Transition {
        // Separate entry reservation from its payload, without a second lease.
        let Admission {
            entry,
            left,
            right,
            left_permit,
            right_permit,
            started_left,
            started_right,
            pending,
            controls,
            closing,
        } = admission;
        Transition(Step::More(Once::prepared(
            entry,
            Start {
                description: self,
                admission: StartAdmission {
                    left,
                    right,
                    left_permit,
                    right_permit,
                    started_left,
                    started_right,
                    pending,
                    controls,
                    closing,
                },
                finish: finish.0,
            },
        )))
    }
}
