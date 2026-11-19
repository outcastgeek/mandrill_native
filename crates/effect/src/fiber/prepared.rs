use super::{Completion, Fiber, FiberOwner, Inner, Produced};
use crate::cancellation::PreparedCancellationBank;
use crate::execution::PreparedTaskCompletion;
use crate::internal::runtime::{
    self, Context as EffectContext, Finish, ParallelContextReservation, Step,
};
use crate::internal::runtime_storage::{Consume, Pending, ReservedOnce, ReservedOnceReservation};
use crate::memory::{
    Bytes, FramePool, FrameReservation, FrameVacancy, MemoryError, PreparedMemory,
    PreparedUnitFuture, RegionId, SharedSlot, SlotLease, SlotPool, prepared_notification,
};
use crate::scope::Registered;
use crate::{Cancellation, Defect, Exit, LibraryDefect, ScheduledTask, Scope};
use std::alloc::Layout;
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

type FiberRecords<E> = SlotPool<'static, Option<Inner<(), E>>>;

pub(crate) struct PreparedUnitForks<E: Send + 'static> {
    fibers: FiberRecords<E>,
    finalizers: FramePool<'static>,
    tasks: Option<PreparedTaskPool>,
}

struct PreparedTaskPool {
    frames: FramePool<'static>,
    cancellations: PreparedCancellationBank,
}

impl PreparedTaskPool {
    fn try_clone(&self) -> Result<Self, MemoryError> {
        Ok(Self {
            frames: self.frames.try_clone()?,
            cancellations: self.cancellations.try_clone()?,
        })
    }
}

pub(crate) struct PreparedUnitFork<E: Send + 'static> {
    fiber: SlotLease<'static, Option<Inner<(), E>>>,
    release: FrameReservation<'static, Release<E>>,
    abandon: FrameReservation<'static, Abandon<E>>,
    wait: FrameReservation<'static, CleanupWait<E>>,
    pub(super) task: Option<PreparedForkTask<E>>,
}

pub(crate) struct PreparedForkDispatch<E: Send + 'static> {
    pub(crate) fork: PreparedUnitFork<E>,
    pub(crate) observers: prepared_notification::Event,
    pub(crate) permits: (SharedSlot<'static, ()>, SharedSlot<'static, ()>),
}

impl<E: Send + 'static> PreparedUnitForks<E> {
    fn frames(count: usize) -> Result<(usize, Layout), MemoryError> {
        let layouts = [
            Layout::new::<Release<E>>(),
            Layout::new::<Abandon<E>>(),
            Layout::new::<CleanupWait<E>>(),
        ];
        let size = layouts.iter().map(Layout::size).max().unwrap_or_default();
        let align = layouts.iter().map(Layout::align).max().unwrap_or(1);
        let layout =
            Layout::from_size_align(size, align).map_err(|_| MemoryError::InvalidLayout)?;
        let count = count
            .checked_mul(layouts.len())
            .ok_or(MemoryError::ArithmeticOverflow)?;
        Ok((count, layout))
    }

    pub(crate) fn storage_bytes(count: usize) -> Result<Bytes, MemoryError> {
        let (frames, layout) = Self::frames(count)?;
        FiberRecords::<E>::storage_bytes(count)?
            .checked_add(FramePool::storage_bytes(frames, layout)?)
    }

    pub(crate) fn prepare(
        memory: &PreparedMemory<'static>,
        region: RegionId,
        count: usize,
    ) -> Result<Self, MemoryError> {
        let (frames, layout) = Self::frames(count)?;
        Ok(Self {
            fibers: memory.slots(region, count, |_| Ok(None))?,
            finalizers: memory.frames(region, frames, layout)?,
            tasks: None,
        })
    }

    pub(crate) fn try_clone(&self) -> Result<Self, MemoryError> {
        Ok(Self {
            fibers: self.fibers.try_clone()?,
            finalizers: self.finalizers.try_clone()?,
            tasks: self
                .tasks
                .as_ref()
                .map(PreparedTaskPool::try_clone)
                .transpose()?,
        })
    }

    pub(crate) fn task_storage_bytes(
        count: usize,
        cancellation_listeners: usize,
        body: Layout,
    ) -> Result<Bytes, MemoryError> {
        let (frames, layout) = PreparedForkTask::<E>::frames(count, body)?;
        FramePool::storage_bytes(frames, layout)?.checked_add(
            PreparedCancellationBank::detached_storage_bytes(count, cancellation_listeners)?,
        )
    }

    pub(crate) fn prepare_tasks(
        &mut self,
        memory: &PreparedMemory<'static>,
        region: RegionId,
        count: usize,
        cancellation_listeners: usize,
        body: Layout,
    ) -> Result<(), MemoryError> {
        let (frames, layout) = PreparedForkTask::<E>::frames(count, body)?;
        self.tasks = Some(PreparedTaskPool {
            frames: memory.frames(region, frames, layout)?,
            cancellations: PreparedCancellationBank::prepare_detached(
                memory,
                region,
                count,
                cancellation_listeners,
            )?,
        });
        Ok(())
    }

    pub(crate) fn reserve(
        &self,
        task_layout: Option<Layout>,
    ) -> Result<PreparedUnitFork<E>, MemoryError> {
        let task = if let Some(layout) = task_layout {
            Some(PreparedForkTask::reserve(
                self.tasks
                    .as_ref()
                    .ok_or(MemoryError::MissingRuntimeStorage)?,
                layout,
            )?)
        } else {
            None
        };
        Ok(PreparedUnitFork {
            fiber: self.fibers.try_checkout()?,
            release: self.finalizers.reserve()?,
            abandon: self.finalizers.reserve()?,
            wait: self.finalizers.reserve()?,
            task,
        })
    }
}

pub(super) struct PreparedForkTask<E: Send + 'static> {
    body: FrameVacancy<'static>,
    notice: ReservedOnceReservation<ForkNotice<E>, PreparedTaskCompletion, ()>,
    origin: ParallelContextReservation,
    cancellation: Cancellation,
}

impl<E: Send + 'static> PreparedForkTask<E> {
    fn frames(count: usize, body: Layout) -> Result<(usize, Layout), MemoryError> {
        let layouts = [
            body,
            ReservedOnce::<PreparedTaskCompletion, ()>::layout::<ForkNotice<E>>(),
            ParallelContextReservation::layout(),
        ];
        let size = layouts.iter().map(Layout::size).max().unwrap_or_default();
        let align = layouts.iter().map(Layout::align).max().unwrap_or(1);
        let layout =
            Layout::from_size_align(size, align).map_err(|_| MemoryError::InvalidLayout)?;
        Ok((
            count
                .checked_mul(layouts.len())
                .ok_or(MemoryError::ArithmeticOverflow)?,
            layout,
        ))
    }

    fn reserve(pool: &PreparedTaskPool, body: Layout) -> Result<Self, MemoryError> {
        let frames = &pool.frames;
        Ok(Self {
            body: frames.reserve_layout(body)?,
            notice: ReservedOnce::reserve(frames)?,
            origin: ParallelContextReservation::reserve(frames)?,
            cancellation: pool.cancellations.reserve_detached()?,
        })
    }

    pub(super) fn prepare_start(
        self,
        parent: &EffectContext,
    ) -> (EffectContext, PreparedTaskStart<E>) {
        let child = parent.child_prepared(self.cancellation, self.origin);
        (
            child,
            PreparedTaskStart {
                body: self.body,
                notice: self.notice,
            },
        )
    }
}

pub(crate) fn prepared_fork_task_layout<I, F, E>(_: fn(I) -> F) -> Layout
where
    F: Future<Output = Exit<(), E>> + Send + 'static,
    E: Send + 'static,
{
    fn layout<I, F: Future<Output = ()>>(_: fn(I) -> F) -> Layout {
        Layout::new::<F>()
    }
    layout(ForkTask::<F, E>::run)
}

pub(super) struct PreparedTaskStart<E: Send + 'static> {
    body: FrameVacancy<'static>,
    notice: ReservedOnceReservation<ForkNotice<E>, PreparedTaskCompletion, ()>,
}

impl<E: Send + 'static> PreparedTaskStart<E> {
    pub(super) fn schedule<F: Future<Output = Exit<(), E>> + Send + 'static>(
        self,
        work: F,
        context: EffectContext,
        fiber: Fiber<(), E>,
        permit: SharedSlot<'static, ()>,
    ) -> ScheduledTask {
        let notice = self
            .notice
            .initialize(ForkNotice(fiber.clone()), ForkNotice::complete);
        let future = ForkTask {
            work,
            context,
            fiber,
            _permit: permit,
        }
        .run();
        #[allow(clippy::expect_used)]
        // The exact ForkTask<F,E>::run layout was checked before publication.
        let frame = self
            .body
            .typed(&future)
            .expect("pre-admitted fork future layout");
        ScheduledTask::prepared(PreparedUnitFuture::initialize(frame, future), notice)
    }
}

struct ForkTask<F, E: Send + 'static> {
    work: F,
    context: EffectContext,
    fiber: Fiber<(), E>,
    _permit: SharedSlot<'static, ()>,
}

impl<F: Future<Output = Exit<(), E>>, E: Send + 'static> ForkTask<F, E> {
    async fn run(self) {
        self.fiber.started();
        let outcome = runtime::publish(self.work.await, &self.context);
        *self
            .fiber
            .inner
            .produced
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(Produced {
            outcome,
            protected: self.context.was_protected(),
        });
    }
}

struct ForkNotice<E: Send + 'static>(Fiber<(), E>);

impl<E: Send + 'static> ForkNotice<E> {
    fn complete(self, event: PreparedTaskCompletion) {
        let produced = self
            .0
            .inner
            .produced
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        match event {
            PreparedTaskCompletion::Ready => {
                let Produced { outcome, protected } = produced.unwrap_or_else(|| Produced {
                    outcome: Exit::Die(Defect::from(LibraryDefect::MissingInterpreterOutcome)),
                    protected: false,
                });
                self.0.publish(Completion::Ready(outcome), protected);
            }
            PreparedTaskCompletion::Lost(defect) => {
                self.0.publish(Completion::Lost(defect), false);
                drop(produced);
            }
        }
    }
}

impl<E: Send + 'static> PreparedUnitFork<E> {
    pub(super) fn initialize(
        mut self,
        scope: &Scope<E>,
        cancellation: Cancellation,
        observers: prepared_notification::Event,
        permit: SharedSlot<'static, ()>,
    ) -> (Fiber<(), E>, Registered<E>) {
        *self.fiber = Some(Fiber::initialize(cancellation, Some(observers)));
        let fiber = Fiber {
            inner: FiberOwner::Prepared(Some(self.fiber.share())),
        };
        let tag = fiber.retirement_tag();
        fiber
            .inner
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .retirement = Some(scope.unit_retirement(tag));
        let release = Release {
            fiber: fiber.clone(),
            wait: self.wait,
            permit,
        };
        let abandon = Abandon {
            fiber: fiber.clone(),
        };
        let mut finalizer =
            Registered::with_prepared_abandon(self.release, release, self.abandon, abandon);
        finalizer.set_retirement_tag(tag);
        (fiber, finalizer)
    }
}

struct Release<E: Send + 'static> {
    fiber: Fiber<(), E>,
    wait: FrameReservation<'static, CleanupWait<E>>,
    permit: SharedSlot<'static, ()>,
}

impl<E: Send + 'static> Consume<Finish<(), E>, Step> for Release<E> {
    fn consume(self, finish: Finish<(), E>) -> Step {
        let Self {
            fiber,
            wait,
            permit,
        } = self;
        let wait = Pending::prepared(
            wait,
            CleanupWait {
                fiber,
                finish: Some(finish),
                cancelled: false,
                _permit: permit,
            },
        );
        Step::Suspended(wait)
    }
}

struct Abandon<E: Send + 'static> {
    fiber: Fiber<(), E>,
}

impl<E: Send + 'static> Consume<(), ()> for Abandon<E> {
    fn consume(self, (): ()) {
        self.fiber.cancel();
    }
}

struct CleanupWait<E: Send + 'static> {
    fiber: Fiber<(), E>,
    finish: Option<Finish<(), E>>,
    cancelled: bool,
    _permit: SharedSlot<'static, ()>,
}

// Only handles and an unpinned continuation move with this future. The
// outcome remains in the Fiber's separate owned record until cleanup takes it.
impl<E: Send + 'static> Unpin for CleanupWait<E> {}

impl<E: Send + 'static> Future for CleanupWait<E> {
    type Output = Step;

    fn poll(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        if !this.cancelled {
            this.cancelled = true;
            this.fiber.cancel();
        }
        this.fiber.inner.cleanup_waker.register(context.waker());
        let Some(outcome) = this.fiber.cleanup_ready() else {
            return Poll::Pending;
        };
        drop(this.fiber.inner.cleanup_waker.take());
        Poll::Ready(
            this.finish
                .take()
                .and_then(|finish| finish.call(outcome))
                .unwrap_or(Step::Done),
        )
    }
}

impl<E: Send + 'static> Drop for CleanupWait<E> {
    fn drop(&mut self) {
        drop(self.fiber.inner.cleanup_waker.take());
    }
}
