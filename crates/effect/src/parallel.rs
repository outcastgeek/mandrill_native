use crate::fiber::JoinedOrigin;
use crate::internal::notification::notify_all;
use crate::internal::outcome;
use crate::internal::runtime::{self, Context, Finish, Step};
use crate::{
    Cancellation, CleanupFailure, Defect, Effect, Exit, Fiber, HasTaskSpawner, LibraryDefect,
    PrimaryOutcome, Scope,
};
use event_listener::Event;
use futures_util::future::{Either, join, select};
use futures_util::stream::{FuturesUnordered, StreamExt};
use std::iter::{Enumerate, RepeatN};
use std::num::NonZeroUsize;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Arc, Mutex};

#[derive(Clone, Copy)]
enum Side {
    Left,
    Right,
}

#[derive(Default)]
struct Selection {
    winner: Mutex<Option<Side>>,
    changed: Event,
}

impl Selection {
    fn choose(&self, side: Side) {
        let first = {
            let mut winner = self
                .winner
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if winner.is_some() {
                false
            } else {
                *winner = Some(side);
                true
            }
        };
        if first {
            notify_all(&self.changed);
        }
    }

    async fn winner(&self) -> Side {
        loop {
            let changed = self.changed.listen();
            let winner = *self
                .winner
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if let Some(winner) = winner {
                return winner;
            }
            changed.await;
        }
    }
}

fn contender<A, E, R>(
    program: Effect<A, E, R>,
    selection: Arc<Selection>,
    side: Side,
) -> Effect<A, E, R>
where
    A: Send + 'static,
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    Effect::new(move |env, context, finish| {
        let selection = Arc::clone(&selection);
        program.clone().execute(
            env,
            context.clone(),
            runtime::continuation(move |outcome| {
                let outcome = runtime::publish(outcome, &context);
                // Commit selection at completion, rather than selecting whichever
                // completed future the coordinator happens to poll first.
                selection.choose(side);
                runtime::finish(finish, outcome)
            }),
        )
    })
}

impl<A, E, R> Effect<A, E, R>
where
    A: Send + 'static,
    E: Send + 'static,
    R: HasTaskSpawner + Send + Sync + 'static,
{
    /// Select the first terminal outcome, then cancel and join the loser.
    /// Like the reference, retain loser cleanup failures, not its ordinary
    /// primary failure. Driver loss is additionally retained as an ownership
    /// defect. Submission errors abort admission and still join admitted work.
    pub fn race(self, right: Self) -> Self {
        Scope::scoped(move |scope| {
            let left = self.clone();
            let right = right.clone();
            Effect::new(move |env, context, finish| {
                let selection = Arc::new(Selection::default());
                let left = contender(left.clone(), Arc::clone(&selection), Side::Left);
                let right = contender(right.clone(), Arc::clone(&selection), Side::Right);
                let admitted = scope.clone();
                let left_cancellation = context.cancellation.child();
                let child = context.parallel(left_cancellation);
                let admission_env = Arc::clone(&env);
                scope.start_parallel(
                    left,
                    &admission_env,
                    child,
                    runtime::continuation(move |left: Exit<Fiber<A, E>, E>| {
                        match left.into_failure() {
                            Ok(left) => {
                                let right_cancellation = context.cancellation.child();
                                let child = context.parallel(right_cancellation);
                                let admission_env = Arc::clone(&env);
                                admitted.start_parallel(
                                    right,
                                    &admission_env,
                                    child,
                                    runtime::continuation(move |right: Exit<Fiber<A, E>, E>| {
                                        match right.into_failure() {
                                            Ok(right) => Step::Suspended(Box::pin(async move {
                                                runtime::finish(
                                                    finish,
                                                    settle_race(left, right, selection, context)
                                                        .await,
                                                )
                                            })),
                                            Err(failure) => runtime::finish(finish, failure),
                                        }
                                    }),
                                )
                            }
                            Err(failure) => runtime::finish(finish, failure),
                        }
                    }),
                )
            })
        })
    }

    /// Run at most the explicit positive number of independently scheduled
    /// workers. Results retain input order and move without A/E Clone bounds.
    /// A terminal failure stops reservations and cancels/joins all workers;
    /// subsequent child cleanup failures are retained in completion order.
    pub fn all_concurrent(concurrency: NonZeroUsize, programs: Vec<Self>) -> Effect<Vec<A>, E, R> {
        let programs: Arc<[Self]> = programs.into();
        Self::all_concurrent_shared(concurrency, programs)
    }

    /// Reuse an already shared immutable description collection without copying
    /// its elements or converting another Vec into shared storage.
    pub fn all_concurrent_shared(
        concurrency: NonZeroUsize,
        programs: Arc<[Self]>,
    ) -> Effect<Vec<A>, E, R> {
        concurrent(concurrency, move || Programs(Arc::clone(&programs)))
    }

    /// Borrow immutable inputs while creating each selected operation. Inputs
    /// are shared once; neither input nor result/error payloads are cloned.
    /// The callback runs lazily on a worker outside bookkeeping locks. Any data
    /// captured by its returned reusable Effect must have its own valid ownership.
    pub fn for_each_concurrent<T, F>(
        concurrency: NonZeroUsize,
        values: Vec<T>,
        transform: F,
    ) -> Effect<Vec<A>, E, R>
    where
        T: Send + Sync + 'static,
        F: Fn(&T) -> Self + Send + Sync + 'static,
    {
        let values: Arc<[T]> = values.into();
        Self::for_each_concurrent_shared(concurrency, values, transform)
    }

    /// Borrow inputs that are already shared, retaining their existing allocation.
    /// This also avoids the initial owned Vec-to-Arc collection conversion.
    pub fn for_each_concurrent_shared<T, F>(
        concurrency: NonZeroUsize,
        values: Arc<[T]>,
        transform: F,
    ) -> Effect<Vec<A>, E, R>
    where
        T: Send + Sync + 'static,
        F: Fn(&T) -> Self + Send + Sync + 'static,
    {
        let transform = Arc::new(transform);
        concurrent(concurrency, move || Values {
            values: Arc::clone(&values),
            transform: Arc::clone(&transform),
        })
    }
}

async fn settle_race<A: Send + 'static, E: Send + 'static>(
    left: Fiber<A, E>,
    right: Fiber<A, E>,
    selection: Arc<Selection>,
    context: Context,
) -> Exit<A, E> {
    // A lost driver never reaches contender's completion continuation. Guarded
    // terminal status also wakes selection, so loss cannot strand the race.
    let left_status = std::pin::pin!(left.settled());
    let right_status = std::pin::pin!(right.settled());
    let statuses = std::pin::pin!(select(left_status, right_status));
    let selected = std::pin::pin!(selection.winner());
    let winner = match select(selected, statuses).await {
        Either::Left((winner, _)) => winner,
        Either::Right((Either::Left(_), _)) => {
            selection.choose(Side::Left);
            selection.winner().await
        }
        Either::Right((Either::Right(_), _)) => {
            selection.choose(Side::Right);
            selection.winner().await
        }
    };
    match winner {
        Side::Left => right.cancel(),
        Side::Right => left.cancel(),
    }
    let (left, right) = join(left.take_owned(), right.take_owned()).await;
    let (winner, loser) = match winner {
        Side::Left => (left, right),
        Side::Right => (right, left),
    };
    context.inherit_protection(winner.protected);
    let winner = runtime::publish(winner.outcome, &context);
    let cleanup = catch_unwind(AssertUnwindSafe(|| match loser.origin {
        JoinedOrigin::Program => outcome::failures(loser.outcome),
        JoinedOrigin::DriverLost | JoinedOrigin::ObservationFailed => {
            outcome::cleanup_failures(loser.outcome)
        }
    }));
    match cleanup {
        Ok(cleanup) => outcome::append_failures(winner, cleanup),
        Err(panic) => outcome::append_cleanup(winner, Exit::Die(Defect::from_panic(panic))),
    }
}

trait Source<A: Send + 'static, E: Send + 'static, R: Send + Sync + 'static>:
    Send + Sync + 'static
{
    fn count(&self) -> usize;
    fn is_empty(&self) -> bool;
    fn job(&self, index: usize) -> Option<Effect<A, E, R>>;
}

struct Programs<A: Send + 'static, E: Send + 'static, R: Send + Sync + 'static>(
    Arc<[Effect<A, E, R>]>,
);
impl<A: Send + 'static, E: Send + 'static, R: Send + Sync + 'static> Source<A, E, R>
    for Programs<A, E, R>
{
    fn count(&self) -> usize {
        self.0.len()
    }
    fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
    fn job(&self, index: usize) -> Option<Effect<A, E, R>> {
        self.0.get(index).cloned()
    }
}

struct Values<T, F> {
    values: Arc<[T]>,
    transform: Arc<F>,
}
impl<A, E, R, T, F> Source<A, E, R> for Values<T, F>
where
    A: Send + 'static,
    E: Send + 'static,
    R: Send + Sync + 'static,
    T: Send + Sync + 'static,
    F: Fn(&T) -> Effect<A, E, R> + Send + Sync + 'static,
{
    fn count(&self) -> usize {
        self.values.len()
    }
    fn is_empty(&self) -> bool {
        self.values.is_empty()
    }
    fn job(&self, index: usize) -> Option<Effect<A, E, R>> {
        self.values.get(index).map(|value| (self.transform)(value))
    }
}

// enumerate supplies identities; the finite take length derives from input.
// There is no numeric sentinel, scheduler quantum, or allocation cutoff.
type Reservations = Enumerate<RepeatN<()>>;
type Workers = RepeatN<()>;

struct Collection<A, E> {
    reservations: Reservations,
    values: Vec<Option<A>>,
    terminal: Option<Exit<Vec<A>, E>>,
    cleanup: Vec<CleanupFailure<E>>,
}

enum Recorded<A, E> {
    First,
    AdditionalPrimary(PrimaryOutcome<E>),
    AdditionalOutcome(Exit<Vec<A>, E>),
}

struct Batch<A: Send + 'static, E: Send + 'static, R: Send + Sync + 'static, S> {
    source: S,
    cancellation: Cancellation,
    state: Mutex<Collection<A, E>>,
    required_environment: std::marker::PhantomData<fn() -> R>,
}

impl<A, E, R, S> Batch<A, E, R, S>
where
    A: Send + 'static,
    E: Send + 'static,
    R: Send + Sync + 'static,
    S: Source<A, E, R>,
{
    fn reserve(&self) -> Option<usize> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.terminal.is_some() || self.cancellation.is_requested() {
            None
        } else {
            state.reservations.next().map(|(index, ())| index)
        }
    }

    fn record(&self, index: usize, outcome: Exit<A, E>) {
        match outcome.into_failure() {
            Ok(value) => {
                let unplaced = {
                    let mut state = self
                        .state
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    if let Some(slot) = state.values.get_mut(index).filter(|slot| slot.is_none()) {
                        *slot = Some(value);
                        None
                    } else {
                        Some(value)
                    }
                };
                if let Some(unplaced) = unplaced {
                    drop(unplaced);
                    self.record_failure(Exit::Die(Defect::from(
                        LibraryDefect::ParallelResultMissing,
                    )));
                }
            }
            Err(failure) => self.record_failure(failure),
        }
    }

    fn record_failure(&self, failure: Exit<Vec<A>, E>) {
        let recorded = {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if state.terminal.is_none() {
                state.terminal = Some(failure);
                Recorded::First
            } else {
                match failure {
                    Exit::FinalizationFailed(details) => {
                        // Commit the whole cleanup group under this gate in
                        // completion order. Only its discarded primary is
                        // destroyed afterward, outside the bookkeeping gate.
                        state.cleanup.push(details.first);
                        state.cleanup.extend(details.rest);
                        Recorded::AdditionalPrimary(details.primary)
                    }
                    other @ (Exit::Success(_)
                    | Exit::Fail(_)
                    | Exit::Die(_)
                    | Exit::Interrupted) => Recorded::AdditionalOutcome(other),
                }
            }
        };
        match recorded {
            Recorded::First => {
                let _ = self.cancellation.request();
            }
            Recorded::AdditionalOutcome(failure) => {
                // Source semantics ignore a later ordinary primary failure.
                drop(failure);
            }
            Recorded::AdditionalPrimary(primary) => drop(primary),
        }
    }

    fn record_worker_failure(&self, failure: Exit<Vec<A>, E>) {
        let secondary = {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if state.terminal.is_none() {
                state.terminal = Some(failure);
                None
            } else {
                Some(failure)
            }
        };
        match secondary {
            None => {
                let _ = self.cancellation.request();
            }
            Some(failure) => {
                // Lost worker ownership is additional infrastructure failure,
                // not an ordinary losing application error that can be ignored.
                let failures = outcome::cleanup_failures(failure);
                self.state
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .cleanup
                    .extend(failures);
            }
        }
    }

    fn finish(&self) -> Exit<Vec<A>, E> {
        let (terminal, values, cleanup) = {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            (
                state.terminal.take(),
                std::mem::take(&mut state.values),
                std::mem::take(&mut state.cleanup),
            )
        };
        let outcome = match terminal {
            Some(terminal) => discard_partial(terminal, values),
            None if self.cancellation.is_requested() => discard_partial(Exit::Interrupted, values),
            None => match catch_unwind(AssertUnwindSafe(|| {
                values.into_iter().collect::<Option<Vec<_>>>()
            })) {
                Ok(Some(values)) => Exit::Success(values),
                Ok(None) => Exit::Die(Defect::from(LibraryDefect::ParallelResultMissing)),
                Err(panic) => Exit::Die(Defect::from_panic(panic)),
            },
        };
        outcome::append_failures(outcome, cleanup)
    }
}

fn discard_partial<A, E>(terminal: Exit<Vec<A>, E>, values: Vec<Option<A>>) -> Exit<Vec<A>, E> {
    match catch_unwind(AssertUnwindSafe(|| drop(values))) {
        Ok(()) => terminal,
        Err(panic) => outcome::append_cleanup(terminal, Exit::Die(Defect::from_panic(panic))),
    }
}

fn concurrent<A, E, R, S>(
    concurrency: NonZeroUsize,
    factory: impl Fn() -> S + Send + Sync + 'static,
) -> Effect<Vec<A>, E, R>
where
    A: Send + 'static,
    E: Send + 'static,
    R: HasTaskSpawner + Send + Sync + 'static,
    S: Source<A, E, R>,
{
    let factory = Arc::new(factory);
    Scope::scoped(move |scope| {
        let factory = Arc::clone(&factory);
        Effect::new(move |env, context, finish| {
            let source = match catch_unwind(AssertUnwindSafe(|| factory())) {
                Ok(source) => source,
                Err(panic) => return runtime::finish(finish, Exit::Die(Defect::from_panic(panic))),
            };
            let count = source.count();
            let workers = concurrency.get().min(count);
            if source.is_empty() {
                return runtime::finish(finish, Exit::Success(Vec::new()));
            }
            let batch = Arc::new(Batch {
                source,
                cancellation: context.cancellation.child(),
                state: Mutex::new(Collection {
                    reservations: std::iter::repeat_n((), count).enumerate(),
                    values: std::iter::repeat_with(|| None).take(count).collect(),
                    terminal: None,
                    cleanup: Vec::new(),
                }),
                required_environment: std::marker::PhantomData,
            });
            submit_workers(
                scope.clone(),
                batch,
                std::iter::repeat_n((), workers),
                Vec::with_capacity(workers),
                env,
                context,
                finish,
            )
        })
    })
}

fn worker<A, E, R, S>(batch: Arc<Batch<A, E, R, S>>) -> Effect<(), E, R>
where
    A: Send + 'static,
    E: Send + 'static,
    R: Send + Sync + 'static,
    S: Source<A, E, R>,
{
    Effect::new(move |env, context, finish| {
        let batch = Arc::clone(&batch);
        Step::Suspended(Box::pin(async move {
            while let Some(index) = batch.reserve() {
                let outcome = match catch_unwind(AssertUnwindSafe(|| batch.source.job(index))) {
                    Ok(Some(program)) => {
                        let item = context.parallel(context.cancellation.clone());
                        runtime::evaluate_context(program, Arc::clone(&env), &item).await
                    }
                    Ok(None) => Exit::Die(Defect::from(LibraryDefect::ParallelResultMissing)),
                    Err(panic) => Exit::Die(Defect::from_panic(panic)),
                };
                batch.record(index, outcome);
            }
            runtime::finish(finish, Exit::Success(()))
        }))
    })
}

fn submit_workers<A, E, R, S>(
    scope: Scope<E>,
    batch: Arc<Batch<A, E, R, S>>,
    mut remaining: Workers,
    mut fibers: Vec<Fiber<(), E>>,
    env: Arc<R>,
    context: Context,
    finish: Finish<Vec<A>, E>,
) -> Step
where
    A: Send + 'static,
    E: Send + 'static,
    R: HasTaskSpawner + Send + Sync + 'static,
    S: Source<A, E, R>,
{
    match remaining.next() {
        Some(()) => {
            let admitted = scope.clone();
            let pending = Arc::clone(&batch);
            let child = context.parallel(batch.cancellation.clone());
            let admission_env = Arc::clone(&env);
            scope.start_parallel(
                worker(Arc::clone(&batch)),
                &admission_env,
                child,
                runtime::continuation(move |fiber| match fiber.into_failure() {
                    Ok(fiber) => {
                        fibers.push(fiber);
                        submit_workers(admitted, pending, remaining, fibers, env, context, finish)
                    }
                    Err(failure) => runtime::finish(finish, failure),
                }),
            )
        }
        None => Step::Suspended(Box::pin(async move {
            let mut joined = FuturesUnordered::new();
            for fiber in fibers {
                joined.push(async move { fiber.take_owned().await });
            }
            while let Some(joined) = joined.next().await {
                match joined.outcome.into_failure() {
                    Ok(()) | Err(Exit::Interrupted) => {}
                    Err(failure @ (Exit::Fail(_) | Exit::Die(_) | Exit::FinalizationFailed(_))) => {
                        batch.record_worker_failure(failure);
                    }
                    Err(Exit::Success(_)) => {
                        batch.record_worker_failure(Exit::Die(Defect::from(
                            LibraryDefect::ParallelResultMissing,
                        )));
                    }
                }
            }
            runtime::finish(finish, batch.finish())
        })),
    }
}
