//! Reusable while-loop lanes use the existing prepared Step evaluator.
use super::super::retention::{GroupLease, OriginGroup};
use super::*;
use crate::memory::SlotLease;

/// Descriptor for the existing lazy while-loop grammar.
pub struct WhileLoop<G, F, M, N> {
    guard: G,
    body: F,
    on_capacity: M,
    marker: PhantomData<fn() -> N>,
}
/// Select a reusable guard, body factory, and typed runtime capacity mapping.
pub fn while_loop<G, F, M, N>(guard: G, body: F, on_capacity: M) -> WhileLoop<G, F, M, N>
where
    N: Description<Value = ()>,
    G: Fn() -> bool + Send + Sync + 'static,
    F: Fn() -> N + Send + Sync + 'static,
    M: Fn(MemoryError) -> N::Error + Send + Sync + 'static,
{
    WhileLoop {
        guard,
        body,
        on_capacity,
        marker: PhantomData,
    }
}
impl<G, F, M, N> Description for WhileLoop<G, F, M, N>
where
    N: Description<Value = ()>,
    G: Fn() -> bool + Send + Sync + 'static,
    F: Fn() -> N + Send + Sync + 'static,
    M: Fn(MemoryError) -> N::Error + Send + Sync + 'static,
{
}
// During rearm the current advance and terminal remain live while the next
// continuation, resume, and advance are admitted: five actual cells.
const CONTROL_CELLS: usize = 5;
struct Lane<N: Description> {
    body_frames: FramePool<'static>,
    controls: FramePool<'static>,
    groups: SlotPool<'static, OriginGroup>,
    body: N::Storage,
}
pub struct LoopStorage<N: Description> {
    lanes: SlotPool<'static, Lane<N>>,
}
struct ScopedState<V: Send + 'static, E: Send + 'static> {
    scope: crate::PreparedScope<E>,
    cleanup: PreparedRunContext<V, E>,
}
type ScopedAdmission<'a, V, E> =
    Option<(&'a crate::PreparedScope<E>, &'a PreparedRunContext<V, E>)>;
struct State<
    G: Send + 'static,
    F: Send + 'static,
    M: Send + 'static,
    N: Description<Value = ()>,
    V: Send + 'static,
> {
    guard: G,
    body: F,
    on_capacity: M,
    lane: SlotLease<'static, Lane<N>>,
    parent: PreparedRunContext<V, N::Error>,
    enclosing_frames: FramePool<'static>,
    environment: Environment<N::Env>,
    scoped: Option<ScopedState<V, N::Error>>,
    finish: Finish<(), N::Error>,
    failed: FrameReservation<'static, Invoke<(), N::Error>>,
    panic: Option<crate::PreparedPanicReservation>,
}
struct Iteration<N: Description<Value = ()>, V: Send + 'static> {
    admission: N::Admission<V>,
    context: PreparedRunContext<V, N::Error>,
    group: GroupLease,
    promotion: PreparedOutputOwnerReservation<GroupLease, V, N::Error>,
}
struct Controls<
    G: Send + 'static,
    F: Send + 'static,
    M: Send + 'static,
    N: Description<Value = ()>,
    V: Send + 'static,
> {
    continuation: FrameReservation<'static, Continue<G, F, M, N, V>>,
    resume: FrameReservation<'static, Resume<G, F, M, N, V>>,
    advance: FrameReservation<'static, Advance<G, F, M, N, V>>,
}
pub struct LoopAdmission<
    G: Send + 'static,
    F: Send + 'static,
    M: Send + 'static,
    N: Description<Value = ()>,
    V: Send + 'static,
> {
    entry: FrameReservation<'static, Entry<G, F, M, N, V>>,
    initial: Initial<G, F, M, N, V>,
}
struct Initial<
    G: Send + 'static,
    F: Send + 'static,
    M: Send + 'static,
    N: Description<Value = ()>,
    V: Send + 'static,
> {
    lane: SlotLease<'static, Lane<N>>,
    parent: PreparedRunContext<V, N::Error>,
    enclosing_frames: FramePool<'static>,
    environment: Environment<N::Env>,
    scoped: Option<ScopedState<V, N::Error>>,
    iteration: Iteration<N, V>,
    controls: Controls<G, F, M, N, V>,
    failed: FrameReservation<'static, Invoke<(), N::Error>>,
    panic: Option<crate::PreparedPanicReservation>,
}
struct Entry<
    G: Send + 'static,
    F: Send + 'static,
    M: Send + 'static,
    N: Description<Value = ()>,
    V: Send + 'static,
> {
    description: WhileLoop<G, F, M, N>,
    initial: Initial<G, F, M, N, V>,
    finish: Finish<(), N::Error>,
}
struct Continue<
    G: Send + 'static,
    F: Send + 'static,
    M: Send + 'static,
    N: Description<Value = ()>,
    V: Send + 'static,
> {
    state: State<G, F, M, N, V>,
    context: PreparedRunContext<V, N::Error>,
    group: GroupLease,
    promotion: PreparedOutputOwnerReservation<GroupLease, V, N::Error>,
    resume: FrameReservation<'static, Resume<G, F, M, N, V>>,
    advance: FrameReservation<'static, Advance<G, F, M, N, V>>,
}
struct Resume<
    G: Send + 'static,
    F: Send + 'static,
    M: Send + 'static,
    N: Description<Value = ()>,
    V: Send + 'static,
> {
    state: State<G, F, M, N, V>,
    context: PreparedRunContext<V, N::Error>,
    group: GroupLease,
    promotion: PreparedOutputOwnerReservation<GroupLease, V, N::Error>,
    outcome: Exit<(), N::Error>,
    advance: FrameReservation<'static, Advance<G, F, M, N, V>>,
}
struct Advance<
    G: Send + 'static,
    F: Send + 'static,
    M: Send + 'static,
    N: Description<Value = ()>,
    V: Send + 'static,
> {
    state: State<G, F, M, N, V>,
}
fn controls<
    G: Send + 'static,
    F: Send + 'static,
    M: Send + 'static,
    N: Description<Value = ()>,
    V: Send + 'static,
>(
    frames: &FramePool<'static>,
) -> Result<Controls<G, F, M, N, V>, MemoryError> {
    Ok(Controls {
        continuation: frames.reserve()?,
        resume: frames.reserve()?,
        advance: frames.reserve()?,
    })
}
fn iteration<N: Description<Value = ()>, V: Send + 'static>(
    lane: &Lane<N>,
    parent: &PreparedRunContext<V, N::Error>,
    enclosing_frames: &FramePool<'static>,
    environment: &Environment<N::Env>,
    scoped: Option<&ScopedState<V, N::Error>>,
) -> Result<Iteration<N, V>, MemoryError> {
    let promotion = parent.reserve_output_owner::<GroupLease>(enclosing_frames)?;
    let (context, group) = OriginGroup::begin(&lane.groups, parent, environment, enclosing_frames)?;
    let admission = if let Some(scoped) = scoped {
        let cleanup = scoped.cleanup.with_origin_of(&context)?;
        N::reserve_scoped(
            &lane.body_frames,
            &lane.body,
            environment,
            &context,
            &scoped.scope,
            &cleanup,
        )?
    } else {
        N::reserve(&lane.body_frames, &lane.body, environment, &context)?
    };
    Ok(Iteration {
        admission,
        context,
        group,
        promotion,
    })
}
fn terminal<G: Send + 'static, F: Send + 'static, M: Send + 'static, N, V>(
    state: State<G, F, M, N, V>,
    outcome: Exit<(), N::Error>,
) -> Step
where
    N: Description<Value = ()>,
    V: Send + 'static,
{
    Step::More(Once::prepared(
        state.failed,
        Invoke {
            outcome,
            finish: state.finish,
        },
    ))
}
fn mapped<G: Send + 'static, F: Send + 'static, M, N, V>(
    mut state: State<G, F, M, N, V>,
    error: MemoryError,
) -> Step
where
    N: Description<Value = ()>,
    V: Send + 'static,
    M: Fn(MemoryError) -> N::Error + Send + Sync + 'static,
{
    if state.parent.is_cancelled() {
        return terminal(state, Exit::Interrupted);
    }
    let outcome = match catch_unwind(AssertUnwindSafe(|| (state.on_capacity)(error))) {
        Ok(error) => Exit::Fail(error),
        Err(panic) => Exit::Die(super::super::panic_defect(state.panic.take(), panic)),
    };
    terminal(state, outcome)
}
fn start<G, F, M, N, V>(
    mut state: State<G, F, M, N, V>,
    iteration: Iteration<N, V>,
    controls: Controls<G, F, M, N, V>,
) -> Step
where
    N: Description<Value = ()>,
    V: Send + 'static,
    G: Fn() -> bool + Send + Sync + 'static,
    F: Fn() -> N + Send + Sync + 'static,
    M: Fn(MemoryError) -> N::Error + Send + Sync + 'static,
{
    if state.parent.is_cancelled() {
        return terminal(state, Exit::Interrupted);
    }
    let guard = catch_unwind(AssertUnwindSafe(|| (state.guard)()));
    match guard {
        Err(panic) => {
            let defect = super::super::panic_defect(state.panic.take(), panic);
            terminal(state, Exit::Die(defect))
        }
        Ok(false) => terminal(state, Exit::Success(())),
        Ok(true) => {
            let description = match catch_unwind(AssertUnwindSafe(|| (state.body)())) {
                Ok(value) => value,
                Err(panic) => {
                    let defect = super::super::panic_defect(state.panic.take(), panic);
                    return terminal(state, Exit::Die(defect));
                }
            };
            let finish = Once::prepared(
                controls.continuation,
                Continue {
                    state,
                    context: iteration.context,
                    group: iteration.group,
                    promotion: iteration.promotion,
                    resume: controls.resume,
                    advance: controls.advance,
                },
            );
            description
                .dispatch(iteration.admission, Continuation(finish))
                .0
        }
    }
}
impl<G, F, M, N, V> Consume<(), Step> for Entry<G, F, M, N, V>
where
    N: Description<Value = ()>,
    V: Send + 'static,
    G: Fn() -> bool + Send + Sync + 'static,
    F: Fn() -> N + Send + Sync + 'static,
    M: Fn(MemoryError) -> N::Error + Send + Sync + 'static,
{
    fn consume(self, (): ()) -> Step {
        let Initial {
            lane,
            parent,
            enclosing_frames,
            environment,
            scoped,
            iteration,
            controls,
            failed,
            panic,
        } = self.initial;
        let state = State {
            guard: self.description.guard,
            body: self.description.body,
            on_capacity: self.description.on_capacity,
            lane,
            parent,
            enclosing_frames,
            environment,
            scoped,
            finish: self.finish,
            failed,
            panic,
        };
        start(state, iteration, controls)
    }
}
impl<G, F, M, N, V> Consume<Exit<(), N::Error>, Step> for Continue<G, F, M, N, V>
where
    N: Description<Value = ()>,
    V: Send + 'static,
    G: Fn() -> bool + Send + Sync + 'static,
    F: Fn() -> N + Send + Sync + 'static,
    M: Fn(MemoryError) -> N::Error + Send + Sync + 'static,
{
    fn consume(self, outcome: Exit<(), N::Error>) -> Step {
        Step::More(Once::prepared(
            self.resume,
            Resume {
                state: self.state,
                context: self.context,
                group: self.group,
                promotion: self.promotion,
                outcome,
                advance: self.advance,
            },
        ))
    }
}
impl<G, F, M, N, V> Consume<(), Step> for Resume<G, F, M, N, V>
where
    N: Description<Value = ()>,
    V: Send + 'static,
    G: Fn() -> bool + Send + Sync + 'static,
    F: Fn() -> N + Send + Sync + 'static,
    M: Fn(MemoryError) -> N::Error + Send + Sync + 'static,
{
    fn consume(self, (): ()) -> Step {
        match self.outcome {
            Exit::Success(()) => {
                drop(self.context);
                drop(self.group);
                drop(self.promotion);
                Step::More(Once::prepared(self.advance, Advance { state: self.state }))
            }
            outcome @ (Exit::Fail(_)
            | Exit::Die(_)
            | Exit::Interrupted
            | Exit::FinalizationFailed(_)) => {
                // The promotion owner belongs to the enclosing sink; only the
                // non-root-counting group lease is installed in its payload.
                let authority = self.promotion.install_for_invocation(self.group);
                drop(authority);
                drop(self.context);
                drop(self.advance);
                terminal(self.state, outcome)
            }
        }
    }
}
impl<G, F, M, N, V> Consume<(), Step> for Advance<G, F, M, N, V>
where
    N: Description<Value = ()>,
    V: Send + 'static,
    G: Fn() -> bool + Send + Sync + 'static,
    F: Fn() -> N + Send + Sync + 'static,
    M: Fn(MemoryError) -> N::Error + Send + Sync + 'static,
{
    fn consume(self, (): ()) -> Step {
        let state = self.state;
        if state.parent.is_cancelled() {
            return terminal(state, Exit::Interrupted);
        }
        let iteration = match iteration::<N, V>(
            &state.lane,
            &state.parent,
            &state.enclosing_frames,
            &state.environment,
            state.scoped.as_ref(),
        ) {
            Ok(value) => value,
            Err(error) => return mapped(state, error),
        };
        let controls = match controls::<G, F, M, N, V>(&state.lane.controls) {
            Ok(value) => value,
            Err(error) => {
                drop(iteration);
                return mapped(state, error);
            }
        };
        start(state, iteration, controls)
    }
}
fn private_cell<T>(continuation: bool) -> Demand {
    let mut demand = Demand::cell::<T>(continuation);
    demand.frames_per_run = 0;
    demand.continuations = 0;
    demand.descriptions = 0;
    demand
}
fn body_frame_count(demand: Demand) -> Result<usize, MemoryError> {
    let persistent = demand
        .origin_frames
        .checked_sub(demand.environment_origin_frames)
        .ok_or(MemoryError::ArithmeticOverflow)?;
    demand
        .frames_per_run
        .checked_sub(persistent)
        .ok_or(MemoryError::ArithmeticOverflow)
}
fn reserve_initial<G, F, M, N, V>(
    frames: &FramePool<'static>,
    storage: &LoopStorage<N>,
    environment: &Environment<N::Env>,
    context: &PreparedRunContext<V, N::Error>,
    scoped: ScopedAdmission<'_, V, N::Error>,
) -> Result<LoopAdmission<G, F, M, N, V>, MemoryError>
where
    N: Description<Value = ()>,
    V: Send + 'static,
    G: Send + 'static,
    F: Send + 'static,
    M: Send + 'static,
{
    let origin = output_origin_reserve::<N::Env, V, N::Error>(frames, environment, context)?;
    let lane = storage.lanes.try_checkout()?;
    let parent = context.try_clone()?;
    let enclosing_frames = frames.try_clone()?;
    let environment = environment.try_clone()?;
    let scoped = scoped
        .map(|(scope, cleanup)| {
            Ok(ScopedState {
                scope: scope.try_clone()?,
                cleanup: cleanup.try_clone()?,
            })
        })
        .transpose()?;
    let failed = lane.controls.reserve()?;
    let iteration = iteration::<N, V>(
        &lane,
        &parent,
        &enclosing_frames,
        &environment,
        scoped.as_ref(),
    )?;
    let controls = controls::<G, F, M, N, V>(&lane.controls)?;
    let panic = context.reserve_panic()?;
    let entry = frames.reserve()?;
    origin
        .attach(environment.try_clone()?)
        .map_err(|(error, _)| error)?;
    Ok(LoopAdmission {
        entry,
        initial: Initial {
            lane,
            parent,
            enclosing_frames,
            environment,
            scoped,
            iteration,
            controls,
            failed,
            panic,
        },
    })
}
impl<G, F, M, N> Spec for WhileLoop<G, F, M, N>
where
    N: Description<Value = ()>,
    G: Fn() -> bool + Send + Sync + 'static,
    F: Fn() -> N + Send + Sync + 'static,
    M: Fn(MemoryError) -> N::Error + Send + Sync + 'static,
{
    type Value = ();
    type Error = N::Error;
    type Env = N::Env;
    type Admission<V: Send + 'static> = LoopAdmission<G, F, M, N, V>;
    type Storage = LoopStorage<N>;
    fn provision<V: Send + 'static>(
        memory: &PreparedMemory<'static>,
        region: RegionId,
        capacity: usize,
        frames: &FramePool<'static>,
        cleanup: Option<&crate::PreparedCleanupConfig<Self::Error>>,
    ) -> Result<Self::Storage, MemoryError> {
        let body_demand = N::shape::<V>()?;
        let layout = frames.layout();
        if layout.align() < body_demand.frame_layout.align() {
            return Err(MemoryError::UnsupportedAlignment {
                requested: body_demand.frame_layout.align(),
                supported: layout.align(),
            });
        }
        if layout.size() < body_demand.frame_layout.size() {
            return Err(MemoryError::ValueTooLarge {
                requested: Bytes::new(body_demand.frame_layout.size()),
                capacity: Bytes::new(layout.size()),
            });
        }
        let lanes = memory.slots(region, capacity, |_| {
            let body_frames = memory.frames(region, body_frame_count(body_demand)?, layout)?;
            let controls = memory.frames(region, CONTROL_CELLS, layout)?;
            let groups = memory.slots(region, 1, |_| Ok(OriginGroup::new()))?;
            let body = N::provision::<V>(memory, region, 1, &body_frames, cleanup)?;
            Ok(Lane {
                body_frames,
                controls,
                groups,
                body,
            })
        })?;
        Ok(LoopStorage { lanes })
    }
    fn storage_bytes<V: Send + 'static>(
        capacity: usize,
        layout: Layout,
    ) -> Result<Bytes, MemoryError> {
        let body = N::shape::<V>()?;
        let lane = FramePool::storage_bytes(body_frame_count(body)?, layout)?
            .checked_add(FramePool::storage_bytes(CONTROL_CELLS, layout)?)?
            .checked_add(SlotPool::<OriginGroup>::storage_bytes(1)?)?
            .checked_add(N::storage_bytes::<V>(1, layout)?)?;
        SlotPool::<Lane<N>>::storage_bytes(capacity)?.checked_add(lane.checked_mul(capacity)?)
    }
    fn scope_finalizers() -> Result<usize, MemoryError> {
        N::scope_finalizers()
    }
    fn shape<V: Send + 'static>() -> Result<Demand, MemoryError> {
        let mut body = N::shape::<V>()?;
        // Each possible origin gets one enclosing-pool headroom cell. Exact
        // base-environment aliases can instead use the private lane; the two
        // alternatives must both be physically available before selection.
        body.frames_per_run = body.origin_frames;
        body.continuations = 0;
        body.descriptions = 0;
        let promotion = output_owner_shape::<GroupLease, V, N::Error>();
        body.add(Demand::cell::<Entry<G, F, M, N, V>>(false))?
            .add(private_cell::<Continue<G, F, M, N, V>>(true))?
            .add(private_cell::<Resume<G, F, M, N, V>>(true))?
            .add(private_cell::<Advance<G, F, M, N, V>>(false))?
            .add(private_cell::<Invoke<(), N::Error>>(true))?
            .add(promotion)?
            .add(Demand::panic_sites(1))?
            .add(output_origin_shape::<N::Env, V, N::Error>())
    }
    fn reserve_scoped<V: Send + 'static>(
        frames: &FramePool<'static>,
        storage: &Self::Storage,
        environment: &Environment<Self::Env>,
        context: &PreparedRunContext<V, Self::Error>,
        enclosing: &crate::PreparedScope<Self::Error>,
        cleanup: &PreparedRunContext<V, Self::Error>,
    ) -> Result<Self::Admission<V>, MemoryError> {
        reserve_initial(
            frames,
            storage,
            environment,
            context,
            Some((enclosing, cleanup)),
        )
    }
    fn reserve<V: Send + 'static>(
        frames: &FramePool<'static>,
        storage: &Self::Storage,
        environment: &Environment<Self::Env>,
        context: &PreparedRunContext<V, Self::Error>,
    ) -> Result<Self::Admission<V>, MemoryError> {
        reserve_initial(frames, storage, environment, context, None)
    }
    fn dispatch<V: Send + 'static>(
        self,
        admission: Self::Admission<V>,
        finish: Continuation<(), Self::Error>,
    ) -> Transition {
        Transition(Step::More(Once::prepared(
            admission.entry,
            Entry {
                description: self,
                initial: admission.initial,
                finish: finish.0,
            },
        )))
    }
}
