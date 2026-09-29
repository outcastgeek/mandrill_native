use crate::internal::notification::notify_all;
use crate::internal::runtime::{self, Step};
use crate::{
    Cancellation, Chunk, Defect, Effect, Exit, Fiber, HasTaskSpawner, LibraryDefect, OutcomeStatus,
    Scope, Stream, StreamReader,
};
use event_listener::{Event, EventListener};
use futures_util::FutureExt;
use futures_util::future::{Either, select};
use futures_util::lock::Mutex as AsyncMutex;
use std::collections::VecDeque;
use std::num::NonZeroUsize;
use std::panic::AssertUnwindSafe;
use std::sync::{Arc, Mutex};

/// Explicit element capacity and maximum number of values returned per pull.
/// An upstream chunk already produced remains owned by its producer until
/// admitted, so the capacity bounds the added queue, not upstream chunk size.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StreamBufferOptions {
    pub capacity: NonZeroUsize,
    pub pull_batch: NonZeroUsize,
}

struct Buffered<A> {
    inner: Arc<Inner<A>>,
}

impl<A> Clone for Buffered<A> {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

struct Inner<A> {
    capacity: NonZeroUsize,
    state: Mutex<State<A>>,
    changed: Event,
    pull_gate: AsyncMutex<()>,
}

struct State<A> {
    values: VecDeque<A>,
    terminal_observed: bool,
}

impl<A: Send + 'static> Buffered<A> {
    fn new(capacity: NonZeroUsize) -> Self {
        Self {
            inner: Arc::new(Inner {
                capacity,
                state: Mutex::new(State {
                    values: VecDeque::new(),
                    terminal_observed: false,
                }),
                changed: Event::new(),
                pull_gate: AsyncMutex::new(()),
            }),
        }
    }

    async fn offer(&self, value: A, cancellation: &Cancellation) -> Result<bool, LibraryDefect> {
        let mut value = Some(value);
        loop {
            let changed = self.inner.changed.listen();
            if cancellation.is_requested() {
                return Ok(false);
            }
            {
                let mut state = self
                    .inner
                    .state
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                if state.values.len() < self.inner.capacity.get() {
                    let was_empty = state.values.is_empty();
                    let admitted = value
                        .take()
                        .ok_or(LibraryDefect::StreamBufferValueMissing)?;
                    state.values.push_back(admitted);
                    drop(state);
                    if was_empty {
                        notify_all(&self.inner.changed);
                    }
                    return Ok(true);
                }
            }
            let cancelled = std::pin::pin!(cancellation.cancelled());
            let _ = select(changed, cancelled).await;
        }
    }

    fn take(&self, batch: NonZeroUsize) -> Option<Chunk<A>> {
        let mut state = self
            .inner
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.terminal_observed {
            return Some(Chunk::End);
        }
        if state.values.is_empty() {
            return None;
        }
        let was_full = state.values.len() == self.inner.capacity.get();
        let mut values = Vec::with_capacity(batch.get().min(state.values.len()));
        for _ in 0..batch.get() {
            let Some(value) = state.values.pop_front() else {
                break;
            };
            values.push(value);
        }
        drop(state);
        if was_full {
            notify_all(&self.inner.changed);
        }
        Some(Chunk::Values(values))
    }

    fn mark_terminal(&self) {
        self.inner
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .terminal_observed = true;
    }

    fn producer<E: Send + 'static, R: Send + Sync + 'static>(
        &self,
        reader: StreamReader<A, E, R>,
    ) -> Effect<(), E, R> {
        let buffer = self.clone();
        Effect::new(move |env, context, finish| {
            let buffer = buffer.clone();
            let reader = reader.clone();
            Step::Suspended(Box::pin(async move {
                let work = async {
                    loop {
                        let pulled =
                            runtime::evaluate_context(reader.pull(), Arc::clone(&env), &context)
                                .await;
                        match pulled.into_failure::<()>() {
                            Ok(Chunk::End) => return Exit::Success(()),
                            Ok(Chunk::Values(values)) => {
                                for value in values {
                                    match buffer.offer(value, &context.cancellation).await {
                                        Ok(true) => {}
                                        Ok(false) => return Exit::Interrupted,
                                        Err(defect) => return Exit::Die(defect.into()),
                                    }
                                }
                            }
                            Err(failure) => return failure,
                        }
                    }
                };
                let outcome = AssertUnwindSafe(work)
                    .catch_unwind()
                    .await
                    .unwrap_or_else(|panic| Exit::Die(Defect::from_panic(panic)));
                runtime::finish(finish, outcome)
            }))
        })
    }

    fn reader<E: Send + 'static, R: Send + Sync + 'static>(
        &self,
        producers: Vec<Fiber<(), E>>,
        batch: NonZeroUsize,
    ) -> StreamReader<A, E, R> {
        let buffer = self.clone();
        let active = Arc::new(Mutex::new(
            producers.into_iter().map(Some).collect::<Vec<_>>(),
        ));
        StreamReader::new(Effect::new(move |env, context, finish| {
            let buffer = buffer.clone();
            let active = Arc::clone(&active);
            Step::Suspended(Box::pin(async move {
                let work = async {
                    // Cloned readers share one pull gate, so only one can
                    // transfer a producer's non-Clone terminal error.
                    let gate = std::pin::pin!(buffer.inner.pull_gate.lock());
                    let cancelled = std::pin::pin!(context.cancellation.cancelled());
                    let _pull = match select(gate, cancelled).await {
                        Either::Left((guard, _)) => guard,
                        Either::Right(_) => return Exit::Interrupted,
                    };
                    loop {
                        let changed = buffer.inner.changed.listen();
                        if context.cancelled() {
                            return Exit::Interrupted;
                        }
                        let (left, right) = {
                            let active = active
                                .lock()
                                .unwrap_or_else(std::sync::PoisonError::into_inner);
                            (
                                active.first().cloned().flatten(),
                                active.get(1).cloned().flatten(),
                            )
                        };
                        let ready = left
                            .as_ref()
                            .and_then(|fiber| {
                                fiber.settled().now_or_never().map(|status| (0, status))
                            })
                            .or_else(|| {
                                right.as_ref().and_then(|fiber| {
                                    fiber.settled().now_or_never().map(|status| (1, status))
                                })
                            });
                        if let Some((index, status)) = ready
                            && status != OutcomeStatus::Succeeded
                        {
                            let peer = if index == 0 { &right } else { &left };
                            if let Some(peer) = peer {
                                peer.cancel();
                            }
                        }
                        if let Some(chunk) = buffer.take(batch) {
                            return Exit::Success(chunk);
                        }
                        if let Some((index, _)) = ready {
                            let Some(producer) = (if index == 0 { left } else { right }) else {
                                return Exit::Die(Defect::from(
                                    LibraryDefect::StreamProducerSlotMissing,
                                ));
                            };
                            let protected = context.protected();
                            let observed = runtime::evaluate_context(
                                producer.await_exit::<R>(),
                                Arc::clone(&env),
                                &protected,
                            )
                            .await;
                            let remaining = {
                                let mut active = active
                                    .lock()
                                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                                let Some(slot) = active.get_mut(index) else {
                                    return Exit::Die(Defect::from(
                                        LibraryDefect::StreamProducerSlotMissing,
                                    ));
                                };
                                *slot = None;
                                active.iter().filter_map(Clone::clone).collect::<Vec<_>>()
                            };
                            let failure = match observed.into_failure::<Chunk<A>>() {
                                Ok(producer_outcome) => {
                                    match producer_outcome.into_failure::<Chunk<A>>() {
                                        Ok(()) => continue,
                                        Err(failure) => failure,
                                    }
                                }
                                Err(failure) => failure,
                            };
                            for producer in remaining {
                                producer.cancel();
                            }
                            buffer.mark_terminal();
                            return failure;
                        }
                        if left.is_none() && right.is_none() {
                            buffer.mark_terminal();
                            return Exit::Success(Chunk::End);
                        }
                        match wait_for_change(changed, &context.cancellation, left, right).await {
                            Wake::Data | Wake::Finished => {}
                            Wake::Cancelled => return Exit::Interrupted,
                        }
                    }
                };
                let outcome = AssertUnwindSafe(work)
                    .catch_unwind()
                    .await
                    .unwrap_or_else(|panic| Exit::Die(Defect::from_panic(panic)));
                runtime::finish(finish, outcome)
            }))
        }))
    }
}

enum Wake {
    Data,
    Finished,
    Cancelled,
}

async fn wait_for_change<E: Send + 'static>(
    changed: EventListener,
    cancellation: &Cancellation,
    left: Option<Fiber<(), E>>,
    right: Option<Fiber<(), E>>,
) -> Wake {
    match (left, right) {
        (Some(left), Some(right)) => {
            let left_wait = std::pin::pin!(left.settled());
            let right_wait = std::pin::pin!(right.settled());
            let finished = std::pin::pin!(select(left_wait, right_wait));
            let wake = std::pin::pin!(select(changed, finished));
            let cancelled = std::pin::pin!(cancellation.cancelled());
            match select(wake, cancelled).await {
                Either::Left((Either::Left(_), _)) => Wake::Data,
                Either::Left((Either::Right(_), _)) => Wake::Finished,
                Either::Right(_) => Wake::Cancelled,
            }
        }
        (Some(fiber), None) | (None, Some(fiber)) => {
            let settled = std::pin::pin!(fiber.settled());
            let wake = std::pin::pin!(select(changed, settled));
            let cancelled = std::pin::pin!(cancellation.cancelled());
            match select(wake, cancelled).await {
                Either::Left((Either::Left(_), _)) => Wake::Data,
                Either::Left((Either::Right(_), _)) => Wake::Finished,
                Either::Right(_) => Wake::Cancelled,
            }
        }
        (None, None) => Wake::Data,
    }
}

pub(crate) fn buffered<A, E, R>(
    upstream: Stream<A, E, R>,
    options: StreamBufferOptions,
) -> Stream<A, E, R>
where
    A: Send + 'static,
    E: Send + 'static,
    R: HasTaskSpawner + Send + Sync + 'static,
{
    Stream::new(move |scope: Scope<E>| {
        let buffer = Buffered::new(options.capacity);
        let fork_scope = scope.clone();
        upstream.open(scope).flat_map(move |reader| {
            let producer = buffer.producer(reader);
            let buffer = buffer.clone();
            fork_scope
                .fork(producer)
                .map(move |fiber| buffer.reader(vec![fiber], options.pull_batch))
        })
    })
}

pub(crate) fn merged<A, E, R>(
    left: Stream<A, E, R>,
    right: Stream<A, E, R>,
    options: StreamBufferOptions,
) -> Stream<A, E, R>
where
    A: Send + 'static,
    E: Send + 'static,
    R: HasTaskSpawner + Send + Sync + 'static,
{
    Stream::new(move |scope: Scope<E>| {
        let buffer = Buffered::new(options.capacity);
        let right_open = right.open(scope.clone());
        let fork_scope = scope.clone();
        left.open(scope).flat_map(move |left_reader| {
            let buffer = buffer.clone();
            let fork_scope = fork_scope.clone();
            right_open.clone().flat_map(move |right_reader| {
                let producers = Effect::all(vec![
                    fork_scope.fork(buffer.producer(left_reader.clone())),
                    fork_scope.fork(buffer.producer(right_reader)),
                ]);
                let buffer = buffer.clone();
                producers.map(move |fibers| buffer.reader(fibers, options.pull_batch))
            })
        })
    })
}
