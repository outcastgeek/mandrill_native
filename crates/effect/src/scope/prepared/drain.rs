//! Prepared storage adapter for canonical ScopeLink::begin_close/DrainGuard.
//! All transitions, retained settlement, close waiter and terminal are physically
//! acquired before body entry. Phase/release order/outcomes use the SAME engine.
use super::{PreparedCloseWait, PreparedScopeOwner, Scope, ScopeCloseStatus, ScopeLink};
use crate::internal::runtime::prepared_graph::{Continuation, Transition};
use crate::internal::runtime::{Finish, Step};
use crate::internal::runtime_storage::{Consume, FutureReservation, Once};
use crate::memory::{FramePool, FrameReservation, MemoryError, RuntimeFrame};
use crate::scope::DrainGuard;
use crate::{CleanupFailures, Exit, PreparedCleanupAccumulator, PreparedRunContext};
use std::alloc::Layout;
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

#[derive(Clone, Copy, Debug)]
pub struct PreparedScopeCloseDemand {
    pub frame_layout: Layout,
    pub frames: usize,
    /// Independent buffers: finalizer accumulation and terminal merge.
    pub cleanup_sites: usize,
    /// One diagnostic for the concrete close waiter factory or poll.
    pub panic_sites: usize,
    /// Required layout in the separate cleanup frame pool.
    pub cleanup_frame_layout: Layout,
}
pub struct PreparedScopeClose<A: Send + 'static, E: Send + 'static, V: Send + 'static> {
    controls: Option<RuntimeFrame>,
    waiter: PreparedCloseWait<E>,
    waiting: FutureReservation<WaitOutcome<E>, ScopeCloseStatus, E>,
    joined: FrameReservation<'static, Join<A, E, V>>,
    closing: ScopeLink<E>,
    done: Done<A, E, V>,
    cleanup: PreparedCleanupAccumulator<(), E>,
}
struct Done<A: Send + 'static, E: Send + 'static, V: Send + 'static> {
    terminal: FrameReservation<'static, Terminal<A, E, V>>,
    owner: PreparedScopeOwner<E>,
    context: PreparedRunContext<V, E>,
    terminal_buffer: CleanupFailures<E>,
}
struct Terminal<A: Send + 'static, E: Send + 'static, V: Send + 'static> {
    outcome: Exit<A, E>,
    finish: Finish<A, E>,
    _context: PreparedRunContext<V, E>,
}
impl<A: Send + 'static, E: Send + 'static, V: Send + 'static> Consume<(), Step>
    for Terminal<A, E, V>
{
    fn consume(self, (): ()) -> Step {
        self.finish.call(self.outcome).unwrap_or(Step::Done)
    }
}
struct Control<A: Send + 'static, E: Send + 'static, V: Send + 'static> {
    more: FrameReservation<'static, Drain<A, E, V>>,
    resume: FrameReservation<'static, Resume<A, E, V>>,
    next: Option<RuntimeFrame>,
}
struct Drain<A: Send + 'static, E: Send + 'static, V: Send + 'static> {
    primary: Exit<A, E>,
    cleanup: PreparedCleanupAccumulator<(), E>,
    resume: FrameReservation<'static, Resume<A, E, V>>,
    next: Option<RuntimeFrame>,
    finish: Finish<A, E>,
    guard: DrainGuard<E>,
    done: Done<A, E, V>,
}
struct Resume<A: Send + 'static, E: Send + 'static, V: Send + 'static> {
    primary: Exit<A, E>,
    cleanup: PreparedCleanupAccumulator<(), E>,
    next: Option<RuntimeFrame>,
    finish: Finish<A, E>,
    guard: DrainGuard<E>,
    done: Done<A, E, V>,
    _retained: Option<RuntimeFrame>,
}
struct Join<A: Send + 'static, E: Send + 'static, V: Send + 'static> {
    primary: Exit<A, E>,
    finish: Finish<A, E>,
    done: Done<A, E, V>,
}
struct WaitOutcome<E: Send + 'static> {
    waiter: PreparedCloseWait<E>,
}
impl<E: Send + 'static> Future for WaitOutcome<E> {
    type Output = Exit<ScopeCloseStatus, E>;
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        Pin::new(&mut self.get_mut().waiter)
            .poll(cx)
            .map(Exit::Success)
    }
}
fn terminal<A: Send + 'static, E: Send + 'static, V: Send + 'static>(
    done: Done<A, E, V>,
    primary: Exit<A, E>,
    cleanup: Exit<(), E>,
    finish: Finish<A, E>,
) -> Step {
    let mut accumulator = PreparedCleanupAccumulator::from_reserved(
        done.context.publish(primary),
        done.terminal_buffer,
    );
    accumulator.append_cleanup(cleanup);
    let outcome = accumulator.finish();
    Step::More(Once::prepared(
        done.terminal,
        Terminal {
            outcome,
            finish,
            _context: done.context,
        },
    ))
}
fn settled<A: Send + 'static, E: Send + 'static, V: Send + 'static>(
    mut done: Done<A, E, V>,
    primary: Exit<A, E>,
    finish: Finish<A, E>,
) -> Step {
    let cleanup = Scope::<E>::take_outcome_inner(&done.owner.scope.inner, false);
    done.owner.settled = true;
    terminal(done, primary, cleanup, finish)
}
fn finish_drain<A: Send + 'static, E: Send + 'static, V: Send + 'static>(
    mut guard: DrainGuard<E>,
    done: Done<A, E, V>,
    primary: Exit<A, E>,
    cleanup: PreparedCleanupAccumulator<(), E>,
    finish: Finish<A, E>,
) -> Step {
    guard.scope.settle(cleanup.finish());
    guard.settled = true;
    drop(guard);
    settled(done, primary, finish)
}
fn next<A: Send + 'static, E: Send + 'static, V: Send + 'static>(
    mut control: RuntimeFrame,
    guard: DrainGuard<E>,
    done: Done<A, E, V>,
    primary: Exit<A, E>,
    cleanup: PreparedCleanupAccumulator<(), E>,
    finish: Finish<A, E>,
) -> Step {
    // SAFETY: only reserve_close creates this metadata chain with exact Control;
    // it is never pinned/projected. Empty-cell retirement remains intrusive.
    let Control { more, resume, next } = unsafe { control.take::<Control<A, E, V>>() };
    drop(control);
    Step::More(Once::prepared(
        more,
        Drain {
            primary,
            cleanup,
            resume,
            next,
            finish,
            guard,
            done,
        },
    ))
}
impl<A: Send + 'static, E: Send + 'static, V: Send + 'static> Consume<(), Step> for Drain<A, E, V> {
    fn consume(mut self, (): ()) -> Step {
        match self.guard.work.pop() {
            Some(finalizer) => finalizer.execute_prepared_with(move |frame| {
                Once::prepared(
                    self.resume,
                    Resume {
                        primary: self.primary,
                        cleanup: self.cleanup,
                        next: self.next,
                        finish: self.finish,
                        guard: self.guard,
                        done: self.done,
                        _retained: frame,
                    },
                )
            }),
            None => finish_drain(
                self.guard,
                self.done,
                self.primary,
                self.cleanup,
                self.finish,
            ),
        }
    }
}
impl<A: Send + 'static, E: Send + 'static, V: Send + 'static> Consume<Exit<(), E>, Step>
    for Resume<A, E, V>
{
    fn consume(self, result: Exit<(), E>) -> Step {
        let mut cleanup = self.cleanup;
        cleanup.append_cleanup(result);
        match self.next {
            Some(control) => next(
                control,
                self.guard,
                self.done,
                self.primary,
                cleanup,
                self.finish,
            ),
            // reserve_close derives capacity from this finite prepared batch and
            // builds capacity+1 controls. No caller-supplied counter can reduce it.
            // The final empty-work transition is therefore always pre-held.
            None => finish_drain(self.guard, self.done, self.primary, cleanup, self.finish),
        }
    }
}
impl<A: Send + 'static, E: Send + 'static, V: Send + 'static>
    Consume<Exit<ScopeCloseStatus, E>, Step> for Join<A, E, V>
{
    fn consume(self, result: Exit<ScopeCloseStatus, E>) -> Step {
        match result {
            Exit::Success(_) => settled(self.done, self.primary, self.finish),
            Exit::Fail(error) => terminal(self.done, self.primary, Exit::Fail(error), self.finish),
            Exit::Die(defect) => terminal(self.done, self.primary, Exit::Die(defect), self.finish),
            Exit::Interrupted => terminal(self.done, self.primary, Exit::Interrupted, self.finish),
            Exit::FinalizationFailed(failure) => terminal(
                self.done,
                self.primary,
                Exit::FinalizationFailed(failure),
                self.finish,
            ),
        }
    }
}
impl<A: Send + 'static, E: Send + 'static, V: Send + 'static> PreparedScopeClose<A, E, V> {
    pub fn demand(finalizers: usize) -> Result<PreparedScopeCloseDemand, MemoryError> {
        let (pending, complete) =
            FutureReservation::<WaitOutcome<E>, ScopeCloseStatus, E>::layouts();
        let layouts = [
            Layout::new::<Control<A, E, V>>(),
            Layout::new::<Drain<A, E, V>>(),
            Layout::new::<Resume<A, E, V>>(),
            Layout::new::<Terminal<A, E, V>>(),
            Layout::new::<Join<A, E, V>>(),
            PreparedCloseWait::<E>::layout::<V>(),
            pending,
            complete,
        ];
        let size = layouts.iter().map(Layout::size).max().unwrap_or(0);
        let align = layouts.iter().map(Layout::align).max().unwrap_or(1);
        let frame_layout = Layout::from_size_align(size, align)
            .map_err(|_| MemoryError::InvalidLayout)?
            .pad_to_align();
        let frames = finalizers
            .checked_add(1)
            .and_then(|count| count.checked_mul(3))
            .and_then(|count| count.checked_add(5))
            .ok_or(MemoryError::ArithmeticOverflow)?;
        Ok(PreparedScopeCloseDemand {
            frame_layout,
            frames,
            cleanup_sites: 2,
            panic_sites: 1,
            cleanup_frame_layout: crate::PreparedCleanupPool::<E>::frame_layout(),
        })
    }
    /// Opaque continuation authority is created only by existing runtime/macro
    /// lowering. No public raw Step/Finish constructor is exposed by this bridge.
    pub fn dispatch(self, primary: Exit<A, E>, continuation: Continuation<A, E>) -> Transition {
        let primary = self.done.context.publish(primary);
        let finish = continuation.0;
        let step = match self.closing.begin_close() {
            Ok(guard) => match self.controls {
                Some(control) => next(control, guard, self.done, primary, self.cleanup, finish),
                // A zero-capacity batch still owns its one empty transition.
                None => finish_drain(guard, self.done, primary, self.cleanup, finish),
            },
            Err(_scope) => self.waiting.start(
                move || WaitOutcome {
                    waiter: self.waiter,
                },
                Once::prepared(
                    self.joined,
                    Join {
                        primary,
                        finish,
                        done: self.done,
                    },
                ),
            ),
        };
        Transition(step)
    }
}
impl<E: Send + 'static> PreparedScopeOwner<E> {
    /// Consume canonical ownership into a full prepared close vector before
    /// body/acquisition work. Failure drops this owner through normal abandon.
    pub fn reserve_close<A: Send + 'static, V: Send + 'static>(
        self,
        frames: &FramePool<'static>,
        context: &PreparedRunContext<V, E>,
    ) -> Result<PreparedScopeClose<A, E, V>, MemoryError> {
        let cleanup_config = self
            .scope
            .cleanup
            .as_ref()
            .ok_or(MemoryError::UnsupportedRunContract)?;
        let cleanup = cleanup_config
            .reserve(Exit::Success(()))
            .map_err(|(error, _)| error)?;
        let terminal_buffer = cleanup_config.pool.reserve(&cleanup_config.frames)?;
        let capacity = self
            .scope
            .inner
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .finalizers
            .capacity();
        let count = capacity
            .checked_add(1)
            .ok_or(MemoryError::ArithmeticOverflow)?;
        let mut controls = None;
        for _ in 0..count {
            let record = frames.reserve::<Control<A, E, V>>()?;
            let more = frames.reserve::<Drain<A, E, V>>()?;
            let resume = frames.reserve::<Resume<A, E, V>>()?;
            controls = Some(record.erase(Control {
                more,
                resume,
                next: controls,
            }));
        }
        let closing = ScopeLink::Prepared(self.scope.inner.try_clone()?);
        let waiter = PreparedCloseWait::prepare(
            ScopeLink::Prepared(self.scope.inner.try_clone()?),
            frames,
            context,
        )?;
        let waiting = FutureReservation::<WaitOutcome<E>, ScopeCloseStatus, E>::reserve(frames)?
            .with_panic(context.reserve_panic()?);
        let joined = frames.reserve::<Join<A, E, V>>()?;
        let terminal = frames.reserve::<Terminal<A, E, V>>()?;
        let context = context.try_clone()?;
        Ok(PreparedScopeClose {
            controls,
            waiter,
            waiting,
            joined,
            closing,
            done: Done {
                terminal,
                owner: self,
                context,
                terminal_buffer,
            },
            cleanup,
        })
    }
}
