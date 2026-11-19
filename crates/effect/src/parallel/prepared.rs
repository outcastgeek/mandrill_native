//! Prepared unit-result cohorts for bounded indexed operations.

use crate::cancellation::PreparedCancellationBank;
use crate::execution::{PreparedTaskCompletion, TaskSpawner};
use crate::internal::runtime::{
    self, Context, Finish, ParallelContextReservation, PreparedNestedReservation, Step,
};
use crate::internal::runtime_storage::{ReservedOnce, ReservedOnceReservation};
use crate::memory::{
    Bytes, FixedVector, FramePool, FrameVacancy, MemoryError, PreparedMemory, PreparedUnitFuture,
    RegionId, SharedRuntimeFrame, SharedSlot, SlotPool, prepared_notification,
};
use crate::{
    Cancellation, Defect, Effect, Exit, Finalization, HasTaskSpawner, LibraryDefect,
    PreparedCleanupAccumulator, PreparedCleanupConfig, PreparedCleanupPool, PrimaryOutcome,
    ScheduledTask,
};
use futures_util::future::{Either, select};
use std::alloc::Layout;
use std::future::{Future, poll_fn};
use std::num::NonZeroUsize;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::pin::pin;
use std::sync::{Arc, Mutex, MutexGuard};

struct BuildInput<R, F> {
    env: Arc<R>,
    job: Arc<F>,
}

struct Build<E: Send + 'static> {
    input: SharedRuntimeFrame,
    start: fn(&SharedRuntimeFrame, usize, &Context, Finish<(), E>) -> Step,
}

impl<E: Send + 'static> Build<E> {
    fn reserve<R, F>(
        frames: &FramePool<'static>,
        env: Arc<R>,
        job: Arc<F>,
    ) -> Result<Self, MemoryError>
    where
        R: Send + Sync + 'static,
        F: Fn(usize) -> Effect<(), E, R> + Send + Sync + 'static,
    {
        fn start<E, R, F>(
            input: &SharedRuntimeFrame,
            index: usize,
            context: &Context,
            finish: Finish<(), E>,
        ) -> Step
        where
            E: Send + 'static,
            R: Send + Sync + 'static,
            F: Fn(usize) -> Effect<(), E, R> + Send + Sync + 'static,
        {
            // SAFETY: reserve pairs this shim with the same typed BuildInput<R,F>.
            // The shared frame remains alive and initialized throughout the borrow.
            let input = unsafe { input.pointer().cast::<BuildInput<R, F>>().as_ref() };
            (input.job)(index).execute(input.env.clone(), context.clone(), finish)
        }
        Ok(Self {
            input: frames
                .reserve::<BuildInput<R, F>>()?
                .share(BuildInput { env, job }),
            start: start::<E, R, F>,
        })
    }
}

impl<E: Send + 'static> Clone for Build<E> {
    #[allow(clippy::expect_used)] // Only the bounded cohort can share this owner.
    fn clone(&self) -> Self {
        Self {
            input: self
                .input
                .try_clone()
                .expect("prepared cohort callback owner limit"),
            start: self.start,
        }
    }
}

enum Admission {
    Staged,
    Submitting(Option<PreparedTaskCompletion>),
    Accepted,
    Settled,
}
struct WorkerControl {
    cancellation: Cancellation,
    admission: Admission,
}
struct Failure<E> {
    outcome: Exit<(), E>,
    infrastructure: bool,
}

struct State<E: Send + 'static> {
    stopped: bool,
    next: usize,
    active: usize,
    workers: FixedVector<'static, WorkerControl>,
    jobs: FixedVector<'static, PreparedNestedReservation<(), E>>,
    failures: FixedVector<'static, Failure<E>>,
}

struct Cohort<E: Send + 'static> {
    state: Mutex<State<E>>,
    changed: prepared_notification::Event,
}
struct CohortLoan<E: Send + 'static>(Option<SharedSlot<'static, Cohort<E>>>);
impl<E: Send + 'static> Clone for CohortLoan<E> {
    #[allow(clippy::expect_used)] // A live handle always retains its slot, with Arc's owner-count bound.
    fn clone(&self) -> Self {
        Self(Some(
            self.0
                .as_ref()
                .expect("live prepared cohort")
                .try_clone()
                .expect("live cohort owner limit"),
        ))
    }
}
impl<E: Send + 'static> Drop for CohortLoan<E> {
    fn drop(&mut self) {
        if let Some(mut slot) = self.0.take().and_then(SharedSlot::into_last) {
            let state = slot
                .state
                .get_mut()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            state.workers.clear();
            state.jobs.clear();
            state.failures.clear();
        }
    }
}
impl<E: Send + 'static> CohortLoan<E> {
    #[allow(clippy::expect_used)] // Last retirement consumes this handle.
    fn slot(&self) -> &Cohort<E> {
        self.0.as_ref().expect("live prepared cohort")
    }

    fn state(&self) -> MutexGuard<'_, State<E>> {
        self.slot()
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn next_job(&self) -> Option<(usize, PreparedNestedReservation<(), E>)> {
        let mut state = self.state();
        if state.stopped {
            return None;
        }
        let reservation = state.jobs.pop()?;
        let index = state.next;
        state.next += 1;
        Some((index, reservation))
    }

    fn cancel(&self) {
        let workers = {
            let mut state = self.state();
            state.stopped = true;
            state.workers.len()
        };
        for index in 0..workers {
            let cancellation = self
                .state()
                .workers
                .as_slice()
                .get(index)
                .map(|worker| worker.cancellation.clone());
            if let Some(cancellation) = cancellation {
                let _ = cancellation.request();
            }
        }
    }

    fn job_failed(&self, failure: Exit<(), E>) {
        let cancel = {
            let mut state = self.state();
            let cancel = !state.stopped;
            state.stopped = true;
            // A worker records at most one job failure and one provider loss.
            state.failures.push_reserved(Failure {
                outcome: failure,
                infrastructure: false,
            });
            cancel
        };
        if cancel {
            self.cancel();
        }
    }

    fn begin_submission(&self, index: usize) -> Option<Cancellation> {
        let mut state = self.state();
        if state.stopped {
            return None;
        }
        let worker = state.workers.as_mut_slice().get_mut(index)?;
        worker.admission = Admission::Submitting(None);
        let cancellation = worker.cancellation.clone();
        state.active += 1;
        Some(cancellation)
    }

    fn complete(&self, index: usize, completion: PreparedTaskCompletion) {
        let publish = {
            let mut state = self.state();
            let Some(worker) = state.workers.as_mut_slice().get_mut(index) else {
                return;
            };
            match &mut worker.admission {
                Admission::Submitting(provisional) => {
                    *provisional = Some(completion);
                    None
                }
                Admission::Accepted => {
                    worker.admission = Admission::Settled;
                    Some(Self::settle_locked(&mut state, completion))
                }
                Admission::Staged | Admission::Settled => None,
            }
        };
        if let Some(cancel) = publish {
            self.notify_settled(cancel);
        }
    }

    fn finish_submission(&self, index: usize, rejection: Option<Defect>) {
        let publish = {
            let mut state = self.state();
            let Some(worker) = state.workers.as_mut_slice().get_mut(index) else {
                return;
            };
            let Admission::Submitting(provisional) = &mut worker.admission else {
                return;
            };
            let completion = rejection
                .map(PreparedTaskCompletion::Lost)
                .or_else(|| provisional.take());
            if let Some(completion) = completion {
                worker.admission = Admission::Settled;
                Some(Self::settle_locked(&mut state, completion))
            } else {
                worker.admission = Admission::Accepted;
                None
            }
        };
        if let Some(cancel) = publish {
            self.notify_settled(cancel);
        }
    }

    fn settle_locked(state: &mut State<E>, result: PreparedTaskCompletion) -> bool {
        state.active -= 1;
        if let PreparedTaskCompletion::Lost(defect) = result {
            let cancel = !state.stopped;
            state.stopped = true;
            state.failures.push_reserved(Failure {
                outcome: Exit::Die(defect),
                infrastructure: true,
            });
            cancel
        } else {
            false
        }
    }

    fn notify_settled(&self, cancel: bool) {
        let _notify = std::mem::DropGuard::new(
            &self.slot().changed,
            prepared_notification::Event::notify_all,
        );
        if cancel {
            self.cancel();
        }
    }
}

struct Notice<E: Send + 'static> {
    cohort: CohortLoan<E>,
    index: usize,
}

impl<E: Send + 'static> Notice<E> {
    fn complete(self, completion: PreparedTaskCompletion) {
        self.cohort.complete(self.index, completion);
    }
}

struct Worker<E: Send + 'static> {
    cohort: CohortLoan<E>,
    context: Context,
    build: Build<E>,
}

impl<E: Send + 'static> Worker<E> {
    async fn run(self) {
        while let Some((index, reservation)) = self.cohort.next_job() {
            if self.context.cancelled() {
                return;
            }
            let result = runtime::evaluate_context_reserved(&self.context, reservation, |finish| {
                (self.build.start)(&self.build.input, index, &self.context, finish)
            });
            let result = std::panic::AssertUnwindSafe(result);
            let outcome = futures_util::FutureExt::catch_unwind(result)
                .await
                .unwrap_or_else(|panic| Exit::Die(Defect::from_panic(panic)));
            if !matches!(outcome, Exit::Success(())) {
                self.cohort.job_failed(outcome);
                return;
            }
        }
    }

    fn layout() -> Layout {
        fn layout<I, F: Future<Output = ()>>(_: fn(I) -> F) -> Layout {
            Layout::new::<F>()
        }
        layout(Self::run)
    }
}

struct Launch<E: Send + 'static> {
    body: FrameVacancy<'static>,
    notice: ReservedOnceReservation<Notice<E>, PreparedTaskCompletion, ()>,
    origin: ParallelContextReservation,
}

type Launches<E> = FixedVector<'static, Launch<E>>;

/// Independent prepared headroom for bounded concurrent unit operations.
///
/// Successes are unit values. Indexed callers retain their own outputs in
/// declaration order. This coordinator owns and joins every accepted worker.
pub struct PreparedUnitCohorts<E: Send + 'static> {
    states: SlotPool<'static, Cohort<E>>,
    launches: SlotPool<'static, Launches<E>>,
    frames: FramePool<'static>,
    cancellation: PreparedCancellationBank,
    cleanup: PreparedCleanupConfig<E>,
    items: NonZeroUsize,
    workers: NonZeroUsize,
}

fn table_bytes<T>(count: usize, width: usize) -> Result<Bytes, MemoryError> {
    let bytes = Layout::array::<T>(width)
        .map_err(|_| MemoryError::InvalidLayout)?
        .size();
    Bytes::new(bytes).checked_mul(count)
}

impl<E: Send + 'static> PreparedUnitCohorts<E> {
    fn frame_demand(
        calls: usize,
        items: usize,
        workers: usize,
    ) -> Result<(usize, Layout), MemoryError> {
        let (value, finish) = PreparedNestedReservation::<(), E>::layouts();
        let layouts = [
            value,
            finish,
            Worker::<E>::layout(),
            ReservedOnce::<PreparedTaskCompletion, ()>::layout::<Notice<E>>(),
            ParallelContextReservation::layout(),
            Layout::new::<BuildInput<(), ()>>(),
        ];
        let size = layouts.iter().map(Layout::size).max().unwrap_or_default();
        let align = layouts.iter().map(Layout::align).max().unwrap_or(1);
        let count = items
            .checked_mul(2)
            .and_then(|items| {
                workers
                    .checked_mul(3)
                    .and_then(|workers| workers.checked_add(items))
            })
            .and_then(|each| each.checked_add(1))
            .and_then(|each| each.checked_mul(calls))
            .ok_or(MemoryError::ArithmeticOverflow)?;
        Ok((
            count,
            Layout::from_size_align(size, align).map_err(|_| MemoryError::InvalidLayout)?,
        ))
    }

    /// Checked region layout for complete active cohorts and workers.
    pub fn storage_bytes(
        calls: NonZeroUsize,
        items: NonZeroUsize,
        workers: NonZeroUsize,
    ) -> Result<Bytes, MemoryError> {
        let (frames, layout) = Self::frame_demand(calls.get(), items.get(), workers.get())?;
        let nodes = calls
            .get()
            .checked_mul(workers.get())
            .ok_or(MemoryError::ArithmeticOverflow)?;
        let failures = workers
            .get()
            .checked_mul(2)
            .ok_or(MemoryError::ArithmeticOverflow)?;
        SlotPool::<Cohort<E>>::storage_bytes(calls.get())?
            .checked_add(SlotPool::<Launches<E>>::storage_bytes(calls.get())?)?
            .checked_add(table_bytes::<Launch<E>>(calls.get(), workers.get())?)?
            .checked_add(table_bytes::<WorkerControl>(calls.get(), workers.get())?)?
            .checked_add(table_bytes::<PreparedNestedReservation<(), E>>(
                calls.get(),
                items.get(),
            )?)?
            .checked_add(table_bytes::<Failure<E>>(calls.get(), failures)?)?
            .checked_add(prepared_notification::Event::storage_bytes(1)?.checked_mul(calls.get())?)?
            .checked_add(FramePool::storage_bytes(frames, layout)?)?
            .checked_add(PreparedCancellationBank::detached_storage_bytes(nodes, 1)?)?
            .checked_add(PreparedCleanupPool::<E>::storage_bytes(
                calls.get(),
                failures,
            )?)?
            .checked_add(FramePool::storage_bytes(
                calls.get(),
                PreparedCleanupPool::<E>::frame_layout(),
            )?)
    }

    /// Prepare cohorts, worker tasks, terminal notifications and ordered cleanup.
    pub fn prepare(
        memory: &PreparedMemory<'static>,
        region: RegionId,
        calls: NonZeroUsize,
        items: NonZeroUsize,
        workers: NonZeroUsize,
    ) -> Result<Self, MemoryError> {
        Self::storage_bytes(calls, items, workers)?;
        let (frames, layout) = Self::frame_demand(calls.get(), items.get(), workers.get())?;
        let nodes = calls
            .get()
            .checked_mul(workers.get())
            .ok_or(MemoryError::ArithmeticOverflow)?;
        let failures = workers
            .get()
            .checked_mul(2)
            .ok_or(MemoryError::ArithmeticOverflow)?;
        Ok(Self {
            states: memory.slots(region, calls.get(), |_| {
                Ok(Cohort {
                    state: Mutex::new(State {
                        stopped: false,
                        next: 0,
                        active: 0,
                        workers: FixedVector::prepare(memory.allocator(region)?, workers.get())?,
                        jobs: FixedVector::prepare(memory.allocator(region)?, items.get())?,
                        failures: FixedVector::prepare(memory.allocator(region)?, failures)?,
                    }),
                    changed: prepared_notification::Event::prepare(memory, region, 1)?,
                })
            })?,
            launches: memory.slots(region, calls.get(), |_| {
                FixedVector::prepare(memory.allocator(region)?, workers.get())
            })?,
            frames: memory.frames(region, frames, layout)?,
            cancellation: PreparedCancellationBank::prepare_detached(memory, region, nodes, 1)?,
            cleanup: PreparedCleanupConfig::new(
                PreparedCleanupPool::prepare(memory, region, calls.get(), failures)?,
                memory.frames(
                    region,
                    calls.get(),
                    PreparedCleanupPool::<E>::frame_layout(),
                )?,
            ),
            items,
            workers,
        })
    }

    /// Run lazily constructed unit operations using a caller-selected worker bound.
    /// All physical owners are admitted before the first worker or callback starts.
    pub fn run<R: HasTaskSpawner + Send + Sync + 'static>(
        self: &Arc<Self>,
        count: usize,
        parallelism: NonZeroUsize,
        on_capacity: impl Fn(MemoryError) -> E + Send + Sync + 'static,
        job: impl Fn(usize) -> Effect<(), E, R> + Send + Sync + 'static,
    ) -> Effect<(), E, R> {
        let pool = self.clone();
        let on_capacity = Arc::new(on_capacity);
        let job = Arc::new(job);
        crate::effect_future! {
            exit_with_context; body: move |env: Arc<R>, context| {
                let pool = pool.clone();
                let on_capacity = on_capacity.clone();
                let job = job.clone();
                async move {
                    pool.coordinate(count, parallelism, &env, &context.context, &*on_capacity, job).await
                }
            },
        }
    }

    async fn coordinate<R: HasTaskSpawner + Send + Sync + 'static>(
        &self,
        count: usize,
        parallelism: NonZeroUsize,
        env: &Arc<R>,
        context: &Context,
        on_capacity: &impl Fn(MemoryError) -> E,
        job: Arc<impl Fn(usize) -> Effect<(), E, R> + Send + Sync + 'static>,
    ) -> Exit<(), E> {
        let workers = count.min(parallelism.get());
        if count == 0 {
            return Exit::Success(());
        }
        if count > self.items.get() || workers > self.workers.get() {
            return Exit::Fail(on_capacity(MemoryError::NoSlot));
        }
        let admitted = (|| {
            let cohort = CohortLoan(Some(self.states.try_checkout()?.share()));
            let listener = cohort.slot().changed.listen()?;
            let launches = self.launches.try_checkout()?;
            let mut launches = std::mem::DropGuard::new(launches, |mut launches| launches.clear());
            let cleanup = self.cleanup.reserve_failures()?;
            let cancelled = context.cancellation.try_cancelled()?;
            {
                let mut state = cohort.state();
                state.stopped = false;
                state.next = 0;
                state.active = 0;
                for _ in 0..count {
                    state
                        .jobs
                        .push_reserved(PreparedNestedReservation::reserve(&self.frames)?);
                }
                for _ in 0..workers {
                    let cancellation = self.cancellation.reserve_detached()?;
                    launches.push_reserved(Launch {
                        body: self.frames.reserve_layout(Worker::<E>::layout())?,
                        notice: ReservedOnce::reserve(&self.frames)?,
                        origin: ParallelContextReservation::reserve(&self.frames)?,
                    });
                    state.workers.push_reserved(WorkerControl {
                        cancellation,
                        admission: Admission::Staged,
                    });
                }
            }
            let build = Build::reserve(&self.frames, env.clone(), job)?;
            Ok((cohort, listener, launches, cleanup, cancelled, build))
        })();
        let (cohort, listener, mut launches, cleanup, cancelled, build) = match admitted {
            Ok(admitted) => admitted,
            Err(error) => return Exit::Fail(on_capacity(error)),
        };
        // A dropped provider future requests cancellation while the workers'
        // own handles retain the physical cohort until scheduler settlement.
        let coordinator = std::mem::DropGuard::new(cohort, |cohort| cohort.cancel());
        for (index, launch) in launches.drain().enumerate() {
            if context.cancelled() {
                coordinator.cancel();
                break;
            }
            let Some(cancellation) = coordinator.begin_submission(index) else {
                break;
            };
            let work = Worker {
                cohort: (*coordinator).clone(),
                context: context.parallel_prepared(cancellation, launch.origin),
                build: build.clone(),
            }
            .run();
            let rejection = match launch.body.typed(&work) {
                Ok(body) => {
                    let notice = Notice {
                        cohort: (*coordinator).clone(),
                        index,
                    };
                    let task = ScheduledTask::prepared(
                        PreparedUnitFuture::initialize(body, work),
                        launch.notice.initialize(notice, Notice::complete),
                    );
                    catch_unwind(AssertUnwindSafe(|| env.task_spawner().spawn(task)))
                        .map_or_else(|panic| Some(Defect::from_panic(panic)), Result::err)
                }
                Err(_) => {
                    drop(work);
                    Some(Defect::from(LibraryDefect::PreparedConstructionUnbound))
                }
            };
            coordinator.finish_submission(index, rejection);
        }
        drop(launches);
        let mut cancelled = pin!(cancelled);
        let mut interrupted = context.cancelled();
        if interrupted {
            coordinator.cancel();
        }
        loop {
            listener.reset();
            if coordinator.state().active == 0 {
                break;
            }
            let change = poll_fn(|cx| listener.poll(cx));
            if interrupted {
                change.await;
            } else if matches!(
                select(pin!(change), cancelled.as_mut()).await,
                Either::Right(_)
            ) {
                interrupted = true;
                coordinator.cancel();
            }
        }
        let pop_failure = || coordinator.state().failures.remove_first();
        let primary = pop_failure().map_or_else(
            || {
                if interrupted || context.cancelled() {
                    Exit::Interrupted
                } else {
                    Exit::Success(())
                }
            },
            |failure| failure.outcome,
        );
        let mut cleanup = PreparedCleanupAccumulator::from_reserved(primary, cleanup);
        while let Some(failure) = pop_failure() {
            if failure.infrastructure {
                cleanup.append_cleanup(failure.outcome);
            } else if let Exit::FinalizationFailed(details) = failure.outcome {
                cleanup.append_cleanup(Exit::FinalizationFailed(Finalization {
                    primary: PrimaryOutcome::Succeeded,
                    first: details.first,
                    rest: details.rest,
                }));
                drop(details.primary);
            }
        }
        drop(listener);
        let _ = std::mem::DropGuard::dismiss(coordinator);
        cleanup.finish()
    }
}
