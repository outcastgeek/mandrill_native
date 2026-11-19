//! Typed lowering target for the existing computation grammar. Descriptions stay
//! lazy; their concrete capture/continuation cells are admitted before invocation.
//! All transitions are the existing Step/Once evaluator, never a second runtime.
#![doc = include_str!("graph/README.md")]
mod control;
mod environment;
mod future;
mod looping;
mod outcome;
pub use outcome::{CaptureTransform, EitherTransform, ErrorTransform, Outcome, capture, either, map_error};
pub use future::{FutureContext, NativeFuture, native_future};
pub(crate) use future::{PreparedCancellationControl, PreparedCancellationProbe};
mod import;
pub mod layer;
pub use import::{Import, ImportEnvironment, import, import_with_environment};
mod scheduled;
pub use scheduled::{Scheduled, scheduled};
mod scoped;
pub use scoped::{Scoped, ScopedProgram, scope, scope_construct};
mod fork;
pub use fork::{Fork, Join, PreparedFiber, Race, race, race_on};
mod provider_result;
pub use provider_result::{ProviderResultDescription, provider_result};
mod resource;
pub mod scope_binding;
pub use environment::{Environment, PreparedEnvironmentPublisher, Provided, provide_some};
pub use resource::{Access, Resource, ResourceValue, Using, resource};
mod late_input;
mod run;
use super::{
    Domain, DynamicRunContract, Kind, Loss, PreparedInvocation, PreparedOutput,
    PreparedOutputOwnerReservation, PreparedPollInstrumentation, PreparedRunContext, Root, Store,
};
use crate::internal::runtime::{Finish, Step};
use crate::internal::runtime_storage::{Consume, Once};
use crate::memory::{
    Bytes, FramePool, FrameReservation, MemoryError, MemoryPolicy, PreparedMemory, RegionId,
    SharedSlot, SlotPool,
};
use crate::{Defect, Exit, PreparedTraceContext, TraceContext};
pub use control::{Branch, Defer, branch_left, branch_right, defer};
pub use late_input::{InputPublisher, InputRecipe, RootRecipe};
pub use looping::{WhileLoop, while_loop};
pub use run::{
    ConstructedGraph, ConstructedGraphPlan, DeferredGraphAdmission, InputConstructor,
    OwnedConstructor, OwnedGraphAdmission, OwnedGraphInvocation, PendingGraphInvocation,
    PendingInputSharedGraph, PendingOwnedInputGraph, PendingOwnedInputSharedGraph,
    PendingOwnedSharedGraph, PendingSharedGraphInvocation, PreparedGraph, PreparedGraphPlan,
};
use std::alloc::Layout;
use std::marker::PhantomData;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

/// Compiler-derived storage, not callback pointer sizes or a transitive native
/// dependency promise. Every cell in this shape has an actual typed reservation.
#[derive(Clone, Copy, Debug)]
pub struct Demand {
    pub frame_layout: Layout,
    pub frames_per_run: usize,
    pub continuations: usize,
    pub descriptions: usize,
    /// Output-origin frames that may have to survive an iteration.
    pub origin_frames: usize,
    /// Origin-frame subset for environment handles selected at admission.
    pub environment_origin_frames: usize,
    pub record_payload: Bytes,
    /// Simultaneously reserved independent cleanup accumulators per run.
    pub cleanup_sites: usize,
    /// Simultaneously admitted panic diagnostics per run.
    pub panic_sites: usize,
}
impl Demand {
    /// Add one concrete reserved callback cell per admitted run.
    #[doc(hidden)]
    pub fn with_reserved_callback_layout(self, layout: Layout) -> Result<Self, MemoryError> {
        let mut callback = Self::cell::<()>(false);
        callback.frame_layout = layout;
        callback.descriptions = 0;
        self.add(callback)
    }
    fn cell<T>(continuation: bool) -> Self {
        Self {
            frame_layout: Layout::new::<T>(),
            frames_per_run: 1,
            continuations: usize::from(continuation),
            descriptions: usize::from(!continuation),
            origin_frames: 0,
            environment_origin_frames: 0,
            record_payload: Bytes::new(0),
            cleanup_sites: 0,
            panic_sites: 0,
        }
    }
    fn cleanup(sites: usize) -> Self {
        Self {
            frame_layout: Layout::new::<()>(),
            frames_per_run: 0,
            continuations: 0,
            descriptions: 0,
            origin_frames: 0,
            environment_origin_frames: 0,
            record_payload: Bytes::new(0),
            cleanup_sites: sites,
            panic_sites: 0,
        }
    }
    fn panic_sites(sites: usize) -> Self {
        Self {
            frame_layout: Layout::new::<()>(),
            frames_per_run: 0,
            continuations: 0,
            descriptions: 0,
            origin_frames: 0,
            environment_origin_frames: 0,
            record_payload: Bytes::new(0),
            cleanup_sites: 0,
            panic_sites: sites,
        }
    }
    fn add(self, other: Self) -> Result<Self, MemoryError> {
        let layout = Layout::from_size_align(
            self.frame_layout.size().max(other.frame_layout.size()),
            self.frame_layout.align().max(other.frame_layout.align()),
        )
        .map_err(|_| MemoryError::InvalidLayout)?
        .pad_to_align();
        Ok(Self {
            frame_layout: layout,
            frames_per_run: self
                .frames_per_run
                .checked_add(other.frames_per_run)
                .ok_or(MemoryError::ArithmeticOverflow)?,
            continuations: self
                .continuations
                .checked_add(other.continuations)
                .ok_or(MemoryError::ArithmeticOverflow)?,
            descriptions: self
                .descriptions
                .checked_add(other.descriptions)
                .ok_or(MemoryError::ArithmeticOverflow)?,
            origin_frames: self
                .origin_frames
                .checked_add(other.origin_frames)
                .ok_or(MemoryError::ArithmeticOverflow)?,
            environment_origin_frames: self
                .environment_origin_frames
                .checked_add(other.environment_origin_frames)
                .ok_or(MemoryError::ArithmeticOverflow)?,
            record_payload: self.record_payload.checked_add(other.record_payload)?,
            cleanup_sites: self
                .cleanup_sites
                .checked_add(other.cleanup_sites)
                .ok_or(MemoryError::ArithmeticOverflow)?,
            panic_sites: self
                .panic_sites
                .checked_add(other.panic_sites)
                .ok_or(MemoryError::ArithmeticOverflow)?,
        })
    }
}
fn native_origins_shape<D: Send + 'static, V: Send + 'static, E: Send + 'static>()
-> Result<Demand, MemoryError> {
    use super::{NativeGraphOrigins, OriginSnapshot, OriginSnapshotReservation};
    Demand {
        frame_layout: OriginSnapshotReservation::<V, E>::layout(),
        frames_per_run: 1,
        continuations: 0,
        descriptions: 0,
        origin_frames: 1,
        environment_origin_frames: 0,
        record_payload: Bytes::new(0),
        cleanup_sites: 0,
        panic_sites: 0,
    }
    .add(Demand {
        frame_layout: NativeGraphOrigins::<D>::layout(),
        frames_per_run: 1,
        continuations: 0,
        descriptions: 0,
        origin_frames: 1,
        environment_origin_frames: 0,
        record_payload: Bytes::new(0),
        cleanup_sites: 0,
        panic_sites: 0,
    })?
    .add(output_owner_shape::<OriginSnapshot, V, E>())
}
fn output_origin_shape<R: Send + Sync + 'static, V: Send + 'static, E: Send + 'static>() -> Demand {
    let mut demand = output_owner_shape::<Environment<R>, V, E>();
    demand.environment_origin_frames = 1;
    demand
}
fn output_owner_shape<T: Send + 'static, V: Send + 'static, E: Send + 'static>() -> Demand {
    Demand {
        frame_layout: PreparedOutputOwnerReservation::<T, V, E>::layout(),
        frames_per_run: 1,
        continuations: 0,
        descriptions: 0,
        origin_frames: 1,
        environment_origin_frames: 0,
        record_payload: Bytes::new(0),
        cleanup_sites: 0,
        panic_sites: 0,
    }
}
fn output_origin_reserve<R: Send + Sync + 'static, V: Send + 'static, E: Send + 'static>(
    frames: &FramePool<'static>,
    environment: &Environment<R>,
    context: &PreparedRunContext<V, E>,
) -> Result<PreparedOutputOwnerReservation<Environment<R>, V, E>, MemoryError> {
    context.reserve_environment_owner(frames, environment.origin_key())
}

/// Opaque authority wrappers keep private Step/Finish out of expansion APIs.
/// Only the common runtime constructs or consumes their private fields.
pub struct Continuation<A: Send + 'static, E: Send + 'static>(pub(crate) Finish<A, E>);
pub struct Transition(pub(crate) Step);
// Sealed implementation contract: ordinary Effect/Layer objects remain intact.
// Expansion returns inferred descriptors; opaque Arc recipes are not inspected.
mod sealed {
    use super::*;
    pub trait Spec: Send + Sized + 'static {
        type Value: Send + 'static;
        type Error: Send + 'static;
        type Env: Send + Sync + 'static;
        type Admission<RootValue: Send + 'static>: Send + 'static;
        type Storage: Send + Sync + 'static;
        fn provision<V: Send + 'static>(
            memory: &PreparedMemory<'static>,
            region: RegionId,
            capacity: usize,
            frames: &FramePool<'static>,
            cleanup: Option<&crate::PreparedCleanupConfig<Self::Error>>,
        ) -> Result<Self::Storage, MemoryError>;
        fn storage_bytes<V: Send + 'static>(
            _capacity: usize,
            _layout: Layout,
        ) -> Result<Bytes, MemoryError> {
            Ok(Bytes::new(0))
        }
        // Mandatory per-description enclosing-scope demand: no composite may
        // silently inherit a default zero when it contains scoped operations.
        fn scope_finalizers() -> Result<usize, MemoryError>;
        fn reserve_scoped<V: Send + 'static>(
            frames: &FramePool<'static>,
            storage: &Self::Storage,
            environment: &Environment<Self::Env>,
            context: &PreparedRunContext<V, Self::Error>,
            enclosing: &crate::PreparedScope<Self::Error>,
            enclosing_cleanup: &PreparedRunContext<V, Self::Error>,
        ) -> Result<Self::Admission<V>, MemoryError>;
        fn shape<RootValue: Send + 'static>() -> Result<Demand, MemoryError>;
        fn reserve<RootValue: Send + 'static>(
            frames: &FramePool<'static>,
            storage: &Self::Storage,
            environment: &Environment<Self::Env>,
            context: &PreparedRunContext<RootValue, Self::Error>,
        ) -> Result<Self::Admission<RootValue>, MemoryError>;
        fn dispatch<RootValue: Send + 'static>(
            self,
            admission: Self::Admission<RootValue>,
            finish: Continuation<Self::Value, Self::Error>,
        ) -> Transition;
    }
}
use sealed::Spec;
/// Sealed concrete descriptor emitted beneath the existing macro grammar.
pub trait Description: Spec {}

struct Invoke<A: Send + 'static, E: Send + 'static> {
    outcome: Exit<A, E>,
    finish: Finish<A, E>,
}
impl<A: Send + 'static, E: Send + 'static> Consume<(), Step> for Invoke<A, E> {
    fn consume(self, (): ()) -> Step {
        self.finish.call(self.outcome).unwrap_or(Step::Done)
    }
}
trait Leaf: Description {
    fn check_cancellation() -> bool {
        true
    }
    fn begins_operation() -> bool {
        true
    }
    fn invoke(self, environment: &Self::Env) -> Exit<Self::Value, Self::Error>;
}
struct LeafTerminal<D: Description, V: Send + 'static> {
    outcome: Exit<D::Value, D::Error>,
    finish: Finish<D::Value, D::Error>,
    _context: PreparedRunContext<V, D::Error>,
}
impl<D: Description, V: Send + 'static> Consume<(), Step> for LeafTerminal<D, V> {
    fn consume(self, (): ()) -> Step {
        self.finish.call(self.outcome).unwrap_or(Step::Done)
    }
}
pub struct LeafAdmission<D: Description, RootValue: Send + 'static> {
    operation: FrameReservation<'static, LeafOperation<D, RootValue>>,
    terminal: FrameReservation<'static, LeafTerminal<D, RootValue>>,
    environment: Environment<D::Env>,
    context: PreparedRunContext<RootValue, D::Error>,
    panic: Option<crate::PreparedPanicReservation>,
}
struct LeafOperation<D: Description, RootValue: Send + 'static> {
    description: D,
    environment: Environment<D::Env>,
    terminal: FrameReservation<'static, LeafTerminal<D, RootValue>>,
    finish: Finish<D::Value, D::Error>,
    context: PreparedRunContext<RootValue, D::Error>,
    panic: Option<crate::PreparedPanicReservation>,
}
impl<D: Leaf, RootValue: Send + 'static> Consume<(), Step> for LeafOperation<D, RootValue> {
    fn consume(self, (): ()) -> Step {
        if D::begins_operation() {
            self.context
                .root
                .contexts
                .protected_failure(self.context.id, false);
        }
        let outcome = if D::check_cancellation() && self.context.is_cancelled() {
            Exit::Interrupted
        } else {
            // OPEN diagnostic ownership finding, shared with existing prepared
            // future adapter; no bounded-panic/accounting completion is asserted.
            catch_unwind(AssertUnwindSafe(|| {
                self.environment
                    .with(|environment| self.description.invoke(environment))
            }))
            .unwrap_or_else(|panic| Err(super::panic_defect(self.panic, panic)))
            .unwrap_or_else(Exit::Die)
        };
        Step::More(Once::prepared(
            self.terminal,
            LeafTerminal::<D, RootValue> {
                outcome,
                finish: self.finish,
                _context: self.context,
            },
        ))
    }
}
fn leaf_shape<D: Leaf, RootValue: Send + 'static>() -> Result<Demand, MemoryError> {
    Demand::cell::<LeafOperation<D, RootValue>>(false)
        .add(Demand::cell::<LeafTerminal<D, RootValue>>(true))?
        .add(Demand::panic_sites(1))
}
fn leaf_reserve<D: Leaf, RootValue: Send + 'static>(
    frames: &FramePool<'static>,
    environment: &Environment<D::Env>,
    context: &PreparedRunContext<RootValue, D::Error>,
) -> Result<LeafAdmission<D, RootValue>, MemoryError> {
    Ok(LeafAdmission {
        operation: frames.reserve()?,
        terminal: frames.reserve()?,
        environment: environment.try_clone()?,
        context: context.try_clone()?,
        panic: context.reserve_panic()?,
    })
}
fn leaf_dispatch<D: Leaf, RootValue: Send + 'static>(
    description: D,
    admission: LeafAdmission<D, RootValue>,
    finish: Finish<D::Value, D::Error>,
) -> Step {
    Step::More(Once::prepared(
        admission.operation,
        LeafOperation {
            description,
            environment: admission.environment,
            terminal: admission.terminal,
            finish,
            context: admission.context,
            panic: admission.panic,
        },
    ))
}
/// Owned success, including non-Clone payloads, inside one admitted description.
pub struct Success<A, E, R> {
    value: A,
    marker: PhantomData<fn() -> (E, R)>,
}
pub fn success<A: Send + 'static, E: Send + 'static, R: Send + Sync + 'static>(
    value: A,
) -> Success<A, E, R> {
    Success {
        value,
        marker: PhantomData,
    }
}
impl<A: Send + 'static, E: Send + 'static, R: Send + Sync + 'static> Description
    for Success<A, E, R>
{
}
impl<A: Send + 'static, E: Send + 'static, R: Send + Sync + 'static> Leaf for Success<A, E, R> {
    // Workflow::from_exit carries pure values without a cancellation precheck
    // or beginning a fresh native operation; final publication is unchanged.
    fn check_cancellation() -> bool {
        false
    }
    fn begins_operation() -> bool {
        false
    }
    fn invoke(self, _environment: &R) -> Exit<A, E> {
        Exit::Success(self.value)
    }
}
impl<A: Send + 'static, E: Send + 'static, R: Send + Sync + 'static> Spec for Success<A, E, R> {
    type Value = A;
    type Error = E;
    type Env = R;
    type Admission<V: Send + 'static> = LeafAdmission<Self, V>;
    type Storage = ();
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
    fn shape<V: Send + 'static>() -> Result<Demand, MemoryError> {
        let demand: Result<Demand, MemoryError> = leaf_shape::<Self, V>();
        demand?.add(output_origin_shape::<Self::Env, V, Self::Error>())
    }
    fn scope_finalizers() -> Result<usize, MemoryError> {
        Ok(0)
    }
    fn reserve_scoped<V: Send + 'static>(
        frames: &FramePool<'static>,
        storage: &Self::Storage,
        environment: &Environment<Self::Env>,
        context: &PreparedRunContext<V, Self::Error>,
        enclosing: &crate::PreparedScope<Self::Error>,
        enclosing_cleanup: &PreparedRunContext<V, Self::Error>,
    ) -> Result<Self::Admission<V>, MemoryError> {
        let _ = (enclosing, enclosing_cleanup);
        Self::reserve(frames, storage, environment, context)
    }
    fn reserve<V: Send + 'static>(
        frames: &FramePool<'static>,
        _storage: &Self::Storage,
        environment: &Environment<R>,
        context: &PreparedRunContext<V, E>,
    ) -> Result<Self::Admission<V>, MemoryError> {
        let origin =
            output_origin_reserve::<Self::Env, V, Self::Error>(frames, environment, context)?;
        let admitted = leaf_reserve(frames, environment, context);
        match admitted {
            Ok(admitted) => {
                origin
                    .attach(environment.try_clone()?)
                    .map_err(|(error, _)| error)?;
                Ok(admitted)
            }
            Err(error) => Err(error),
        }
    }
    fn dispatch<V: Send + 'static>(
        self,
        admission: Self::Admission<V>,
        finish: Continuation<A, E>,
    ) -> Transition {
        Transition(leaf_dispatch(self, admission, finish.0))
    }
}
type ReadyTypes<A, E, R> = fn() -> (A, E, R);
pub struct Ready<F, A, E, R> {
    callback: F,
    marker: PhantomData<ReadyTypes<A, E, R>>,
}
pub fn ready<F, A: Send + 'static, E: Send + 'static, R: Send + Sync + 'static>(
    callback: F,
) -> Ready<F, A, E, R>
where
    F: FnOnce(&R) -> Exit<A, E> + Send + 'static,
{
    Ready {
        callback,
        marker: PhantomData,
    }
}
impl<F, A: Send + 'static, E: Send + 'static, R: Send + Sync + 'static> Description
    for Ready<F, A, E, R>
where
    F: FnOnce(&R) -> Exit<A, E> + Send + 'static,
{
}
impl<F, A: Send + 'static, E: Send + 'static, R: Send + Sync + 'static> Leaf for Ready<F, A, E, R>
where
    F: FnOnce(&R) -> Exit<A, E> + Send + 'static,
{
    fn invoke(self, environment: &R) -> Exit<A, E> {
        (self.callback)(environment)
    }
}
impl<F, A: Send + 'static, E: Send + 'static, R: Send + Sync + 'static> Spec for Ready<F, A, E, R>
where
    F: FnOnce(&R) -> Exit<A, E> + Send + 'static,
{
    type Value = A;
    type Error = E;
    type Env = R;
    type Admission<V: Send + 'static> = LeafAdmission<Self, V>;
    type Storage = ();
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
    fn shape<V: Send + 'static>() -> Result<Demand, MemoryError> {
        let demand: Result<Demand, MemoryError> = leaf_shape::<Self, V>();
        demand?.add(output_origin_shape::<Self::Env, V, Self::Error>())
    }
    fn scope_finalizers() -> Result<usize, MemoryError> {
        Ok(0)
    }
    fn reserve_scoped<V: Send + 'static>(
        frames: &FramePool<'static>,
        storage: &Self::Storage,
        environment: &Environment<Self::Env>,
        context: &PreparedRunContext<V, Self::Error>,
        enclosing: &crate::PreparedScope<Self::Error>,
        enclosing_cleanup: &PreparedRunContext<V, Self::Error>,
    ) -> Result<Self::Admission<V>, MemoryError> {
        let _ = (enclosing, enclosing_cleanup);
        Self::reserve(frames, storage, environment, context)
    }
    fn reserve<V: Send + 'static>(
        frames: &FramePool<'static>,
        _storage: &Self::Storage,
        environment: &Environment<R>,
        context: &PreparedRunContext<V, E>,
    ) -> Result<Self::Admission<V>, MemoryError> {
        let origin =
            output_origin_reserve::<Self::Env, V, Self::Error>(frames, environment, context)?;
        let admitted = leaf_reserve(frames, environment, context);
        match admitted {
            Ok(admitted) => {
                origin
                    .attach(environment.try_clone()?)
                    .map_err(|(error, _)| error)?;
                Ok(admitted)
            }
            Err(error) => Err(error),
        }
    }
    fn dispatch<V: Send + 'static>(
        self,
        admission: Self::Admission<V>,
        finish: Continuation<A, E>,
    ) -> Transition {
        Transition(leaf_dispatch(self, admission, finish.0))
    }
}

/// Existing native sync callback grammar lowers here, preserving its owning
/// Rust language and fresh per-invocation values without a Clone requirement.
pub fn sync<F, A: Send + 'static, E: Send + 'static, R: Send + Sync + 'static>(
    callback: F,
) -> impl Description<Value = A, Error = E, Env = R>
where
    F: FnOnce() -> A + Send + 'static,
{
    Ready {
        callback: move |_: &R| Exit::Success(callback()),
        marker: PhantomData,
    }
}
pub fn try_sync<F, M, A: Send + 'static, X, E: Send + 'static, R: Send + Sync + 'static>(
    callback: F,
    mapper: M,
) -> impl Description<Value = A, Error = E, Env = R>
where
    F: FnOnce() -> Result<A, X> + Send + 'static,
    M: FnOnce(X) -> E + Send + 'static,
{
    Ready {
        callback: move |_: &R| match callback() {
            Ok(value) => Exit::Success(value),
            Err(error) => Exit::Fail(mapper(error)),
        },
        marker: PhantomData,
    }
}

/// Existing service env projection, with the environment owner retained by the
/// admitted leaf through the callback and its capture destruction.
pub fn service<F, A: Send + 'static, E: Send + 'static, R: Send + Sync + 'static>(
    select: F,
) -> impl Description<Value = A, Error = E, Env = R>
where
    F: FnOnce(&R) -> A + Send + 'static,
{
    Ready {
        callback: move |environment: &R| Exit::Success(select(environment)),
        marker: PhantomData,
    }
}

/// The next description's concrete type is a checked dynamic capture contract:
/// reserve its WHOLE tree before invoking the producer with the previous value.
pub struct Bind<L, F, N> {
    left: L,
    next: F,
    marker: PhantomData<fn() -> N>,
}
mod binding_sealed {
    pub trait Sealed {}
    impl<D: super::Description> Sealed for D {}
}
/// Sealed existing grammar dispatch: computation vs lexical resource binding.
pub trait Bindable: binding_sealed::Sealed + Send + Sized + 'static {
    type Value: Send + 'static;
    type Error: Send + 'static;
    type Env: Send + Sync + 'static;
    type Bound<F, N>: Description<Value = N::Value, Error = Self::Error, Env = Self::Env>
    where
        N: Description<Error = Self::Error, Env = Self::Env>,
        F: FnOnce(Self::Value) -> N + Send + 'static;
    fn bind<F, N>(self, next: F) -> Self::Bound<F, N>
    where
        N: Description<Error = Self::Error, Env = Self::Env>,
        F: FnOnce(Self::Value) -> N + Send + 'static;
}
impl<D: Description> Bindable for D {
    type Value = D::Value;
    type Error = D::Error;
    type Env = D::Env;
    type Bound<F, N>
        = Bind<D, F, N>
    where
        N: Description<Error = D::Error, Env = D::Env>,
        F: FnOnce(D::Value) -> N + Send + 'static;
    fn bind<F, N>(self, next: F) -> Self::Bound<F, N>
    where
        N: Description<Error = D::Error, Env = D::Env>,
        F: FnOnce(D::Value) -> N + Send + 'static,
    {
        Bind {
            left: self,
            next,
            marker: PhantomData,
        }
    }
}
pub fn bind<P, F, N>(left: P, next: F) -> P::Bound<F, N>
where
    P: Bindable,
    N: Description<Error = P::Error, Env = P::Env>,
    F: FnOnce(P::Value) -> N + Send + 'static,
{
    left.bind(next)
}
impl<L, F, N> Description for Bind<L, F, N>
where
    L: Description,
    N: Description<Error = L::Error, Env = L::Env>,
    F: FnOnce(L::Value) -> N + Send + 'static,
{
}
pub struct BindAdmission<
    L: Description,
    F,
    N: Description<Error = L::Error, Env = L::Env>,
    V: Send + 'static,
> where
    F: FnOnce(L::Value) -> N + Send + 'static,
{
    entry: FrameReservation<'static, BindEntry<L, F, N, V>>,
    controls: BindControls<L, F, N, V>,
}
struct BindControls<
    L: Description,
    F,
    N: Description<Error = L::Error, Env = L::Env>,
    V: Send + 'static,
> where
    F: FnOnce(L::Value) -> N + Send + 'static,
{
    continuation: FrameReservation<'static, BindContinuation<L, F, N, V>>,
    resume: FrameReservation<'static, BindResume<L, F, N, V>>,
    failed: FrameReservation<'static, Invoke<N::Value, N::Error>>,
    left: L::Admission<V>,
    next: N::Admission<V>,
    context: PreparedRunContext<V, L::Error>,
    panic: Option<crate::PreparedPanicReservation>,
}
struct BindEntry<
    L: Description,
    F,
    N: Description<Error = L::Error, Env = L::Env>,
    V: Send + 'static,
> where
    F: FnOnce(L::Value) -> N + Send + 'static,
{
    description: Bind<L, F, N>,
    finish: Finish<N::Value, N::Error>,
    controls: BindControls<L, F, N, V>,
}
struct BindContinuation<
    L: Description,
    F,
    N: Description<Error = L::Error, Env = L::Env>,
    V: Send + 'static,
> where
    F: FnOnce(L::Value) -> N + Send + 'static,
{
    resume: FrameReservation<'static, BindResume<L, F, N, V>>,
    controls: BindNext<L, F, N, V>,
}
struct BindResume<
    L: Description,
    F,
    N: Description<Error = L::Error, Env = L::Env>,
    V: Send + 'static,
> where
    F: FnOnce(L::Value) -> N + Send + 'static,
{
    outcome: Exit<L::Value, L::Error>,
    controls: BindNext<L, F, N, V>,
}
struct BindNext<
    L: Description,
    F,
    N: Description<Error = L::Error, Env = L::Env>,
    V: Send + 'static,
> where
    F: FnOnce(L::Value) -> N + Send + 'static,
{
    callback: F,
    next: N::Admission<V>,
    failed: FrameReservation<'static, Invoke<N::Value, N::Error>>,
    finish: Finish<N::Value, N::Error>,
    context: PreparedRunContext<V, L::Error>,
    panic: Option<crate::PreparedPanicReservation>,
    marker: PhantomData<fn() -> L>,
}
impl<L, F, N, V> Consume<(), Step> for BindEntry<L, F, N, V>
where
    L: Description,
    N: Description<Error = L::Error, Env = L::Env>,
    F: FnOnce(L::Value) -> N + Send + 'static,
    V: Send + 'static,
{
    fn consume(self, (): ()) -> Step {
        let admission = self.controls;
        if admission.context.is_cancelled() {
            return Step::More(Once::prepared(
                admission.failed,
                Invoke {
                    outcome: Exit::Interrupted,
                    finish: self.finish,
                },
            ));
        }
        let finish = Once::prepared(
            admission.continuation,
            BindContinuation::<L, F, N, V> {
                resume: admission.resume,
                controls: BindNext {
                    callback: self.description.next,
                    next: admission.next,
                    failed: admission.failed,
                    finish: self.finish,
                    context: admission.context,
                    panic: admission.panic,
                    marker: PhantomData,
                },
            },
        );
        self.description
            .left
            .dispatch(admission.left, Continuation(finish))
            .0
    }
}
impl<L, F, N, V> Consume<Exit<L::Value, L::Error>, Step> for BindContinuation<L, F, N, V>
where
    L: Description,
    N: Description<Error = L::Error, Env = L::Env>,
    F: FnOnce(L::Value) -> N + Send + 'static,
    V: Send + 'static,
{
    fn consume(self, outcome: Exit<L::Value, L::Error>) -> Step {
        Step::More(Once::prepared(
            self.resume,
            BindResume {
                outcome,
                controls: self.controls,
            },
        ))
    }
}
impl<L, F, N, V> Consume<(), Step> for BindResume<L, F, N, V>
where
    L: Description,
    N: Description<Error = L::Error, Env = L::Env>,
    F: FnOnce(L::Value) -> N + Send + 'static,
    V: Send + 'static,
{
    fn consume(self, (): ()) -> Step {
        let controls = self.controls;
        let outcome = self.outcome;
        let next = match outcome {
            Exit::Success(value) => {
                if controls.context.is_cancelled() {
                    Err(Exit::Interrupted)
                } else {
                    catch_unwind(AssertUnwindSafe(|| (controls.callback)(value)))
                        .map_err(|panic| Exit::Die(super::panic_defect(controls.panic, panic)))
                }
            }
            Exit::Fail(error) => Err(Exit::Fail(error)),
            Exit::Die(defect) => Err(Exit::Die(defect)),
            Exit::Interrupted => Err(Exit::Interrupted),
            Exit::FinalizationFailed(failure) => Err(Exit::FinalizationFailed(failure)),
        };
        match next {
            Ok(description) => {
                description
                    .dispatch(controls.next, Continuation(controls.finish))
                    .0
            }
            Err(outcome) => Step::More(Once::prepared(
                controls.failed,
                Invoke {
                    outcome,
                    finish: controls.finish,
                },
            )),
        }
    }
}
impl<L, F, N> Spec for Bind<L, F, N>
where
    L: Description,
    N: Description<Error = L::Error, Env = L::Env>,
    F: FnOnce(L::Value) -> N + Send + 'static,
{
    type Value = N::Value;
    type Error = L::Error;
    type Env = L::Env;
    type Admission<V: Send + 'static> = BindAdmission<L, F, N, V>;
    type Storage = (L::Storage, N::Storage);
    fn provision<V: Send + 'static>(
        memory: &PreparedMemory<'static>,
        region: RegionId,
        capacity: usize,
        frames: &FramePool<'static>,
        cleanup: Option<&crate::PreparedCleanupConfig<Self::Error>>,
    ) -> Result<Self::Storage, MemoryError> {
        let _ = cleanup;
        Ok((
            L::provision::<V>(memory, region, capacity, frames, cleanup)?,
            N::provision::<V>(memory, region, capacity, frames, cleanup)?,
        ))
    }
    fn storage_bytes<V: Send + 'static>(
        capacity: usize,
        layout: Layout,
    ) -> Result<Bytes, MemoryError> {
        L::storage_bytes::<V>(capacity, layout)?
            .checked_add(N::storage_bytes::<V>(capacity, layout)?)
    }
    fn shape<V: Send + 'static>() -> Result<Demand, MemoryError> {
        let demand: Result<Demand, MemoryError> = (|| {
            L::shape::<V>()?
                .add(N::shape::<V>()?)?
                .add(Demand::cell::<BindEntry<L, F, N, V>>(false))?
                .add(Demand::cell::<BindContinuation<L, F, N, V>>(true))?
                .add(Demand::cell::<BindResume<L, F, N, V>>(true))?
                .add(Demand::cell::<Invoke<N::Value, N::Error>>(true))?
                .add(Demand::panic_sites(1))
        })();
        demand?.add(output_origin_shape::<Self::Env, V, Self::Error>())
    }
    fn scope_finalizers() -> Result<usize, MemoryError> {
        L::scope_finalizers()?
            .checked_add(N::scope_finalizers()?)
            .ok_or(MemoryError::ArithmeticOverflow)
    }
    fn reserve_scoped<V: Send + 'static>(
        frames: &FramePool<'static>,
        storage: &Self::Storage,
        environment: &Environment<Self::Env>,
        context: &PreparedRunContext<V, Self::Error>,
        enclosing: &crate::PreparedScope<Self::Error>,
        enclosing_cleanup: &PreparedRunContext<V, Self::Error>,
    ) -> Result<Self::Admission<V>, MemoryError> {
        let origin =
            output_origin_reserve::<Self::Env, V, Self::Error>(frames, environment, context)?;
        let admitted = (|| {
            Ok(BindAdmission {
                entry: frames.reserve()?,
                controls: BindControls {
                    continuation: frames.reserve()?,
                    resume: frames.reserve()?,
                    failed: frames.reserve()?,
                    left: L::reserve_scoped(
                        frames,
                        &storage.0,
                        environment,
                        context,
                        enclosing,
                        enclosing_cleanup,
                    )?,
                    next: N::reserve_scoped(
                        frames,
                        &storage.1,
                        environment,
                        context,
                        enclosing,
                        enclosing_cleanup,
                    )?,
                    context: context.try_clone()?,
                    panic: context.reserve_panic()?,
                },
            })
        })();
        match admitted {
            Ok(admitted) => {
                origin
                    .attach(environment.try_clone()?)
                    .map_err(|(error, _)| error)?;
                Ok(admitted)
            }
            Err(error) => Err(error),
        }
    }
    fn reserve<V: Send + 'static>(
        frames: &FramePool<'static>,
        storage: &Self::Storage,
        environment: &Environment<Self::Env>,
        context: &PreparedRunContext<V, Self::Error>,
    ) -> Result<Self::Admission<V>, MemoryError> {
        let origin =
            output_origin_reserve::<Self::Env, V, Self::Error>(frames, environment, context)?;
        let admitted = (|| {
            Ok(BindAdmission {
                entry: frames.reserve()?,
                controls: BindControls {
                    continuation: frames.reserve()?,
                    resume: frames.reserve()?,
                    failed: frames.reserve()?,
                    left: L::reserve(frames, &storage.0, environment, context)?,
                    next: N::reserve(frames, &storage.1, environment, context)?,
                    context: context.try_clone()?,
                    panic: context.reserve_panic()?,
                },
            })
        })();
        match admitted {
            Ok(admitted) => {
                origin
                    .attach(environment.try_clone()?)
                    .map_err(|(error, _)| error)?;
                Ok(admitted)
            }
            Err(error) => Err(error),
        }
    }
    fn dispatch<V: Send + 'static>(
        self,
        admission: Self::Admission<V>,
        finish: Continuation<Self::Value, Self::Error>,
    ) -> Transition {
        let BindAdmission { entry, controls } = admission;
        Transition(Step::More(Once::prepared(
            entry,
            BindEntry {
                description: self,
                controls,
                finish: finish.0,
            },
        )))
    }
}
