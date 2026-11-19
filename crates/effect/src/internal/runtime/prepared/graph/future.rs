//! Concrete native futures under the existing Step/Pending evaluator.
//! All pending, callback, terminal, context and listener storage is reserved
//! recursively before the graph factory is invoked.
use super::super::{DriveContext, ListenerId};
use super::*;
use crate::internal::runtime_storage::Pending;
use crate::memory::{RuntimeFrame, SharedRuntimeFrame};
use std::future::Future;
use std::pin::Pin;
use std::task::{Context as TaskContext, Poll};

struct ContextState<V: Send + 'static, E: Send + 'static> {
    listener: Option<ListenerId>,
    context: SharedRuntimeFrame,
    marker: PhantomData<fn() -> (V, E)>,
}
impl<V: Send + 'static, E: Send + 'static> Drop for ContextState<V, E> {
    fn drop(&mut self) {
        if let Some(listener) = self.listener.take() {
            typed_context::<V, E>(&self.context)
                .root
                .contexts
                .unlink(listener);
        }
    }
}
/// Owned native cancellation authority with an already reserved listener.
/// This is not an ordinary System-backed Cancellation/NativeContext. No type
/// erasure loses the actual originating node or permits a late allocation.
/// The owner is deliberately not Clone: native code can move it into its future.
pub struct FutureContext<E: Send + 'static> {
    state: RuntimeFrame,
    requested: fn(&RuntimeFrame) -> bool,
    request: fn(&RuntimeFrame) -> bool,
    trace: fn(&RuntimeFrame) -> &TraceContext,
    prepared_trace: fn(&RuntimeFrame) -> Option<&PreparedTraceContext>,
    wait: fn(
        &mut RuntimeFrame,
        &mut TaskContext<'_>,
    ) -> Poll<super::super::PreparedCancellationObservation>,
    probe: fn(&RuntimeFrame) -> Result<PreparedCancellationProbe, MemoryError>,
    marker: PhantomData<fn() -> E>,
}
/// Immutable, independently retained observation of the originating node.
pub(crate) struct PreparedCancellationProbe {
    context: SharedRuntimeFrame,
    requested: fn(&SharedRuntimeFrame) -> bool,
}
impl PreparedCancellationProbe {
    pub(crate) fn is_requested(&self) -> bool {
        (self.requested)(&self.context)
    }
}
/// The already-admitted affine cancellation listener, independent of E.
pub(crate) struct PreparedCancellationControl {
    state: RuntimeFrame,
    requested: fn(&RuntimeFrame) -> bool,
    wait: fn(
        &mut RuntimeFrame,
        &mut TaskContext<'_>,
    ) -> Poll<super::super::PreparedCancellationObservation>,
}
impl PreparedCancellationControl {
    pub(crate) fn is_requested(&self) -> bool {
        (self.requested)(&self.state)
    }
    pub(crate) fn poll_cancelled(
        &mut self,
        cx: &mut TaskContext<'_>,
    ) -> Poll<super::super::PreparedCancellationObservation> {
        (self.wait)(&mut self.state, cx)
    }
}
impl<E: Send + 'static> FutureContext<E> {
    pub(crate) fn try_probe(&self) -> Result<PreparedCancellationProbe, MemoryError> {
        (self.probe)(&self.state)
    }
    pub(crate) fn into_cancellation_control(self) -> PreparedCancellationControl {
        PreparedCancellationControl {
            state: self.state,
            requested: self.requested,
            wait: self.wait,
        }
    }
    #[must_use]
    pub fn is_requested(&self) -> bool {
        (self.requested)(&self.state)
    }
    #[must_use]
    pub fn request(&self) -> bool {
        (self.request)(&self.state)
    }
    #[must_use]
    pub fn trace(&self) -> &TraceContext {
        (self.trace)(&self.state)
    }
    /// Borrow the actual prepared trace retained by this callback's context.
    #[must_use]
    pub fn prepared_trace(&self) -> Option<&PreparedTraceContext> {
        (self.prepared_trace)(&self.state)
    }
    /// Reuses this operation's one pre-admitted notification registration.
    pub async fn cancelled(&mut self) -> super::super::PreparedCancellationObservation {
        std::future::poll_fn(|cx| (self.wait)(&mut self.state, cx)).await
    }
}
fn typed_context<V: Send + 'static, E: Send + 'static>(
    frame: &SharedRuntimeFrame,
) -> &PreparedRunContext<V, E> {
    // SAFETY: this projection is paired only with the exact typed share below;
    // the shared frame retains the immutable context for the returned borrow.
    unsafe { frame.pointer().cast::<PreparedRunContext<V, E>>().as_ref() }
}
fn probe_requested<V: Send + 'static, E: Send + 'static>(frame: &SharedRuntimeFrame) -> bool {
    typed_context::<V, E>(frame).is_cancelled()
}
fn probe<V: Send + 'static, E: Send + 'static>(
    frame: &RuntimeFrame,
) -> Result<PreparedCancellationProbe, MemoryError> {
    // SAFETY: frame and vtable use the same exact ContextState<V,E> constructor.
    let state = unsafe { frame.pointer().cast::<ContextState<V, E>>().as_ref() };
    Ok(PreparedCancellationProbe {
        context: state.context.try_clone()?,
        requested: probe_requested::<V, E>,
    })
}
fn requested<V: Send + 'static, E: Send + 'static>(frame: &RuntimeFrame) -> bool {
    // SAFETY: paired only with the exact ContextState<V,E> reservation below;
    // state is initialized and the immutable reference cannot escape this call.
    let state = unsafe { frame.pointer().cast::<ContextState<V, E>>().as_ref() };
    typed_context::<V, E>(&state.context).is_cancelled()
}
fn request<V: Send + 'static, E: Send + 'static>(frame: &RuntimeFrame) -> bool {
    // SAFETY: same exact private frame/vtable pairing, retained node ownership.
    let state = unsafe { frame.pointer().cast::<ContextState<V, E>>().as_ref() };
    typed_context::<V, E>(&state.context).request_cancellation()
}
fn trace<V: Send + 'static, E: Send + 'static>(frame: &RuntimeFrame) -> &TraceContext {
    // SAFETY: exact ContextState constructor/vtable pairing, immutable borrow
    // tied to the retained native context; never an unowned trace pointer.
    let state = unsafe { frame.pointer().cast::<ContextState<V, E>>().as_ref() };
    typed_context::<V, E>(&state.context).trace()
}
fn prepared_trace<V: Send + 'static, E: Send + 'static>(
    frame: &RuntimeFrame,
) -> Option<&PreparedTraceContext> {
    // SAFETY: this projection is paired with the initialized ContextState<V,E>
    // below. The returned borrow is tied to the frame that retains the context.
    let state = unsafe { frame.pointer().cast::<ContextState<V, E>>().as_ref() };
    typed_context::<V, E>(&state.context).prepared_trace()
}
fn wait<V: Send + 'static, E: Send + 'static>(
    frame: &mut RuntimeFrame,
    cx: &mut TaskContext<'_>,
) -> Poll<super::super::PreparedCancellationObservation> {
    // SAFETY: unique FutureContext owns this initialized, never-pinned control
    // record. No reference to its mutable listener escapes this synchronous call.
    let state = unsafe { frame.pointer().cast::<ContextState<V, E>>().as_mut() };
    let Some(listener) = state.listener else {
        return Poll::Pending;
    };
    let context = typed_context::<V, E>(&state.context);
    match context
        .root
        .contexts
        .observe(context.id, listener, cx.waker())
    {
        Some(observation) => Poll::Ready(observation),
        None => Poll::Pending,
    }
}

pub(super) fn context_shape<V: Send + 'static, E: Send + 'static>() -> Result<Demand, MemoryError> {
    let mut demand = Demand::cell::<ContextState<V, E>>(false)
        .add(Demand::cell::<PreparedRunContext<V, E>>(false))?;
    demand.origin_frames = 2;
    Ok(demand)
}
pub(super) fn reserve_context<V: Send + 'static, E: Send + 'static>(
    frames: &FramePool<'static>,
    context: &PreparedRunContext<V, E>,
) -> Result<FutureContext<E>, MemoryError> {
    let selection = context.persistent_selection()?;
    let (context, frames) = selection
        .as_ref()
        .map_or((context, frames), |(context, frames)| (context, frames));
    let capture = frames.reserve::<PreparedRunContext<V, E>>()?;
    let state = frames.reserve::<ContextState<V, E>>()?;
    let context = context.try_clone()?;
    let listener = context.root.contexts.listener(context.id)?;
    Ok(FutureContext {
        state: state.erase(ContextState {
            listener: Some(listener),
            context: capture.share(context),
            marker: PhantomData,
        }),
        requested: requested::<V, E>,
        request: request::<V, E>,
        trace: trace::<V, E>,
        prepared_trace: prepared_trace::<V, E>,
        wait: wait::<V, E>,
        probe: probe::<V, E>,
        marker: PhantomData,
    })
}
type NativeTypes<P, A, E, R> = fn() -> (P, A, E, R);
pub struct NativeFuture<F, P, A, E, R> {
    factory: F,
    marker: PhantomData<NativeTypes<P, A, E, R>>,
}
pub fn native_future<F, P, A, E, R>(factory: F) -> NativeFuture<F, P, A, E, R>
where
    A: Send + 'static,
    E: Send + 'static,
    R: Send + Sync + 'static,
    P: Future<Output = Exit<A, E>> + Send + 'static,
    F: FnOnce(Environment<R>, FutureContext<E>) -> P + Send + 'static,
{
    NativeFuture {
        factory,
        marker: PhantomData,
    }
}
struct Terminal<A: Send + 'static, E: Send + 'static, R: Send + Sync + 'static, V: Send + 'static> {
    outcome: Exit<A, E>,
    finish: Finish<A, E>,
    _environment: Environment<R>,
    context: PreparedRunContext<V, E>,
}
impl<A: Send + 'static, E: Send + 'static, R: Send + Sync + 'static, V: Send + 'static>
    Consume<(), Step> for Terminal<A, E, R, V>
{
    fn consume(self, (): ()) -> Step {
        let outcome = self.context.publish(self.outcome);
        self.finish.call(outcome).unwrap_or(Step::Done)
    }
}
struct Operation<
    F: Send + 'static,
    P: Send + 'static,
    A: Send + 'static,
    E: Send + 'static,
    R: Send + Sync + 'static,
    V: Send + 'static,
> {
    description: NativeFuture<F, P, A, E, R>,
    pending: FrameReservation<'static, Running<P, A, E, R, V>>,
    terminal: FrameReservation<'static, Terminal<A, E, R, V>>,
    environment: Environment<R>,
    native: FutureContext<E>,
    retained_environment: Environment<R>,
    finish: Finish<A, E>,
    context: PreparedRunContext<V, E>,
    panic: Option<crate::PreparedPanicReservation>,
}
struct Running<P, A: Send + 'static, E: Send + 'static, R: Send + Sync + 'static, V: Send + 'static>
{
    pending: P,
    terminal: Option<FrameReservation<'static, Terminal<A, E, R, V>>>,
    finish: Option<Finish<A, E>>,
    environment: Option<Environment<R>>,
    context: Option<PreparedRunContext<V, E>>,
    panic: Option<crate::PreparedPanicReservation>,
}
impl<P, A, E, R, V> Future for Running<P, A, E, R, V>
where
    P: Future<Output = Exit<A, E>> + Send + 'static,
    A: Send + 'static,
    E: Send + 'static,
    R: Send + Sync + 'static,
    V: Send + 'static,
{
    type Output = Step;
    fn poll(self: Pin<&mut Self>, cx: &mut TaskContext<'_>) -> Poll<Step> {
        // SAFETY: Pending places Running once into stable region storage. Only
        // pending is structurally pinned; the Option control fields are Unpin.
        let this = unsafe { self.get_unchecked_mut() };
        let result = catch_unwind(AssertUnwindSafe(|| {
            // SAFETY: same stable field, exclusive poll; never moved afterwards.
            unsafe { Pin::new_unchecked(&mut this.pending) }.poll(cx)
        }));
        let outcome = match result {
            Ok(Poll::Pending) => {
                if let Some(context) = &this.context {
                    context.acknowledge_clock();
                }
                return Poll::Pending;
            }
            Ok(Poll::Ready(outcome)) => outcome,
            // Existing diagnostic contract, not a W05-18 accounting claim.
            Err(panic) => Exit::Die(super::super::panic_defect(this.panic.take(), panic)),
        };
        match (
            this.terminal.take(),
            this.finish.take(),
            this.context.take(),
            this.environment.take(),
        ) {
            (Some(terminal), Some(finish), Some(context), Some(environment)) => {
                context.acknowledge_clock();
                Poll::Ready(Step::More(Once::prepared(
                    terminal,
                    Terminal {
                        outcome,
                        finish,
                        _environment: environment,
                        context,
                    },
                )))
            }
            _ => Poll::Ready(Step::Done),
        }
    }
}
impl<F, P, A, E, R, V> Consume<(), Step> for Operation<F, P, A, E, R, V>
where
    A: Send + 'static,
    E: Send + 'static,
    R: Send + Sync + 'static,
    V: Send + 'static,
    P: Future<Output = Exit<A, E>> + Send + 'static,
    F: FnOnce(Environment<R>, FutureContext<E>) -> P + Send + 'static,
{
    fn consume(self, (): ()) -> Step {
        self.context
            .root
            .contexts
            .protected_failure(self.context.id, false);
        if self.context.is_cancelled() {
            return Step::More(Once::prepared(
                self.terminal,
                Terminal {
                    outcome: Exit::Interrupted,
                    finish: self.finish,
                    _environment: self.retained_environment,
                    context: self.context,
                },
            ));
        }
        let result = catch_unwind(AssertUnwindSafe(|| {
            (self.description.factory)(self.environment, self.native)
        }));
        match result {
            Ok(pending) => Step::Suspended(Pending::prepared(
                self.pending,
                Running {
                    pending,
                    terminal: Some(self.terminal),
                    finish: Some(self.finish),
                    environment: Some(self.retained_environment),
                    context: Some(self.context),
                    panic: self.panic,
                },
            )),
            Err(panic) => Step::More(Once::prepared(
                self.terminal,
                Terminal {
                    outcome: Exit::Die(super::super::panic_defect(self.panic, panic)),
                    finish: self.finish,
                    _environment: self.retained_environment,
                    context: self.context,
                },
            )),
        }
    }
}

pub struct FutureAdmission<
    F: Send + 'static,
    P: Send + 'static,
    A: Send + 'static,
    E: Send + 'static,
    R: Send + Sync + 'static,
    V: Send + 'static,
> {
    operation: FrameReservation<'static, Operation<F, P, A, E, R, V>>,
    pending: FrameReservation<'static, Running<P, A, E, R, V>>,
    terminal: FrameReservation<'static, Terminal<A, E, R, V>>,
    native: FutureContext<E>,
    environment: Environment<R>,
    retained_environment: Environment<R>,
    context: PreparedRunContext<V, E>,
    panic: Option<crate::PreparedPanicReservation>,
}
impl<F, P, A, E, R> Description for NativeFuture<F, P, A, E, R>
where
    A: Send + 'static,
    E: Send + 'static,
    R: Send + Sync + 'static,
    P: Future<Output = Exit<A, E>> + Send + 'static,
    F: FnOnce(Environment<R>, FutureContext<E>) -> P + Send + 'static,
{
}
impl<F, P, A, E, R> Spec for NativeFuture<F, P, A, E, R>
where
    A: Send + 'static,
    E: Send + 'static,
    R: Send + Sync + 'static,
    P: Future<Output = Exit<A, E>> + Send + 'static,
    F: FnOnce(Environment<R>, FutureContext<E>) -> P + Send + 'static,
{
    type Value = A;
    type Error = E;
    type Env = R;
    type Admission<V: Send + 'static> = FutureAdmission<F, P, A, E, R, V>;
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
    fn scope_finalizers() -> Result<usize, MemoryError> {
        Ok(0)
    }
    fn shape<V: Send + 'static>() -> Result<Demand, MemoryError> {
        let demand: Result<Demand, MemoryError> = (|| {
            Demand::cell::<Operation<F, P, A, E, R, V>>(false)
                .add(Demand::cell::<Running<P, A, E, R, V>>(false))?
                .add(Demand::cell::<Terminal<A, E, R, V>>(true))?
                .add(context_shape::<V, E>()?)
        })();
        demand?
            .add(Demand::panic_sites(1))?
            .add(output_origin_shape::<Self::Env, V, Self::Error>())
    }
    fn reserve_scoped<V: Send + 'static>(
        frames: &FramePool<'static>,
        storage: &(),
        environment: &Environment<R>,
        context: &PreparedRunContext<V, E>,
        _enclosing: &crate::PreparedScope<E>,
        _cleanup: &PreparedRunContext<V, E>,
    ) -> Result<Self::Admission<V>, MemoryError> {
        Self::reserve(frames, storage, environment, context)
    }
    fn reserve<V: Send + 'static>(
        frames: &FramePool<'static>,
        _storage: &(),
        environment: &Environment<R>,
        context: &PreparedRunContext<V, E>,
    ) -> Result<Self::Admission<V>, MemoryError> {
        let origin =
            output_origin_reserve::<Self::Env, V, Self::Error>(frames, environment, context)?;
        let admitted = (|| {
            let panic = context.reserve_panic()?;
            let operation = frames.reserve()?;
            let pending = frames.reserve()?;
            let terminal = frames.reserve()?;
            let environment_copy = environment.try_clone()?;
            let retained_environment = environment.try_clone()?;
            let retained_context = context.try_clone()?;
            let native = reserve_context(frames, context)?;
            Ok(FutureAdmission {
                operation,
                pending,
                terminal,
                environment: environment_copy,
                retained_environment,
                context: retained_context,
                panic,
                native,
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
        finish: Continuation<A, E>,
    ) -> Transition {
        Transition(Step::More(Once::prepared(
            admission.operation,
            Operation {
                description: self,
                pending: admission.pending,
                terminal: admission.terminal,
                environment: admission.environment,
                retained_environment: admission.retained_environment,
                native: admission.native,
                finish: finish.0,
                context: admission.context,
                panic: admission.panic,
            },
        )))
    }
}
