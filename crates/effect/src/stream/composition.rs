//! Pull-driven native body suspension using one emission slot and the existing scope/runtime.

use super::{Chunk, Stream, StreamReader};
use crate::internal::macro_future::{AsyncBindable, MacroContext};
use crate::internal::runtime::{self, Step};
use crate::{Defect, Effect, Exit, LibraryDefect, Release, ResourceValue, Scope};
use crate::{effect_finalizer, effect_uninterruptible};
use futures_util::FutureExt;
use std::future::{Future, poll_fn};
use std::panic::AssertUnwindSafe;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};

/// Suspended body owns its locals across pulls; items and errors remain statically typed.
type Body<E> = Pin<Box<dyn Future<Output = Result<(), Exit<(), E>>> + Send>>;
/// Per-open body state. A reader supports one outstanding pull; there is no prefetch queue.
struct Machine<E> {
    /// Suspended native body, absent before the first pull or after terminal completion.
    body: Option<Body<E>>,
    /// Whether a pull currently owns the body outside the mutex.
    busy: bool,
    /// Terminal completion prevents restarting the body on a later pull.
    finished: bool,
}
/// Temporary exclusive pull owner. Drop restores suspended state when a driver stops polling;
/// closing the outer scope still owns resources and drops the restored body.
struct PullOwner<E> {
    /// Checked scope-owned state, inaccessible once its release consumes it.
    state: ResourceValue<Mutex<Machine<E>>>,
    /// Body temporarily removed from the state so no synchronous lock spans await.
    body: Option<Body<E>>,
    /// Completion discards suspended state rather than replaying it.
    finished: bool,
}
impl<E> Drop for PullOwner<E> {
    /// Return the body to its scope-owned state if that owner is still live.
    fn drop(&mut self) {
        let body = self.body.take();
        let finished = self.finished;
        let _ = self.state.with(move |state| {
            let mut state = state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            state.body = body;
            state.busy = false;
            state.finished = finished;
        });
    }
}
/// Generated stream context; the active scope holds acquired resources across yield suspension.
#[doc(hidden)]
pub struct StreamContext<A: Send + 'static, E: Send + 'static, R: Send + Sync + 'static> {
    /// Existing runtime/trace context and externally owned stream scope.
    context: MacroContext<E, R>,
    /// One suspended yield value, moved to the consumer without a payload copy.
    slot: Arc<Mutex<Option<A>>>,
}
/// The yield suspension future. Its first poll offers one value and remains pending;
/// the next consumer pull resumes the body after taking that value.
struct Emit<A> {
    /// Value owned until it is offered to the active pull.
    value: Option<A>,
    /// Shared one-value rendezvous slot, never an unbounded buffer.
    slot: Arc<Mutex<Option<A>>>,
    /// Whether the value was offered by this suspension.
    offered: bool,
}
/// Emission has no self-references: pinning protects no address-dependent invariant.
impl<A> Unpin for Emit<A> {}
impl<A> Future for Emit<A> {
    /// Yield completes only when the consumer requests another pull after taking the value.
    type Output = ();
    /// Transfer once and suspend without spawning or waking a background producer.
    fn poll(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<()> {
        let this = self.get_mut();
        if this.offered {
            return Poll::Ready(());
        }
        let value = this.value.take();
        *this
            .slot
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = value;
        this.offered = true;
        Poll::Pending
    }
}
impl<A: Send + 'static, E: Send + 'static, R: Send + Sync + 'static> StreamContext<A, E, R> {
    /// Bind an ordinary Effect or resource lazily during pulling, retaining the stream's owner.
    pub async fn bind<P: AsyncBindable<E, R>>(&self, program: P) -> Result<P::Value, Exit<(), E>> {
        self.context.bind(program).await
    }
    /// Suspend after transferring one item; the value itself needs no Clone or Sync bound.
    pub fn emit(&self, value: A) -> impl Future<Output = ()> + Send + 'static {
        Emit {
            value: Some(value),
            slot: Arc::clone(&self.slot),
            offered: false,
        }
    }
    /// Append a compatible stream under the same outer owner, pulling only on consumer demand.
    pub fn append(
        &self,
        source: Stream<A, E, R>,
    ) -> impl Future<Output = Result<(), Exit<(), E>>> + Send + 'static {
        let bridge = Self {
            context: self.context.share(),
            slot: Arc::clone(&self.slot),
        };
        async move {
            let child = Scope::new();
            let close = effect_uninterruptible! { program: child.close_owned(), };
            bridge
                .context
                .bind(effect_finalizer! {
                scope: bridge.context.scope(), body: close.clone(),
                })
                .await?;
            let result = async {
                let reader = bridge.context.bind(source.open(child)).await?;
                loop {
                    match bridge.context.bind(reader.pull()).await? {
                        Chunk::Values(values) => {
                            for value in values {
                                bridge.emit(value).await;
                            }
                        }
                        Chunk::End => return Ok(()),
                    }
                }
            }
            .await;
            let cleanup = bridge.context.bind(close).await;
            match (result, cleanup) {
                (Ok(()), Ok(())) => Ok(()),
                (Ok(()), Err(failure)) => Err(failure),
                (Err(failure), Ok(())) => Err(failure),
                (Err(failure), Err(cleanup)) => {
                    Err(crate::internal::outcome::append_cleanup(failure, cleanup))
                }
            }
        }
    }
    /// Mark resources owned by a lexical stream fragment before its first bind/yield.
    #[must_use]
    pub fn checkpoint(&self) -> usize {
        self.context.checkpoint()
    }
    /// Release one completed fragment while retaining outer stream acquisitions across later yields.
    pub async fn release(&self, checkpoint: usize) -> Result<(), Exit<(), E>> {
        self.context.release(checkpoint).await
    }
    /// Observe interruption between producer loop iterations under the existing runtime.
    pub async fn cooperate(&self) -> Result<(), Exit<(), E>> {
        self.context.cooperate().await
    }
}
impl<A: Send + 'static, E: Send + 'static, R: Send + Sync + 'static> Stream<A, E, R> {
    /// Construct a native pull body. The body runs on the first pull and owns suspended locals;
    /// opening or constructing never emits items. No task, channel or whole-stream collection
    /// is introduced. Readers permit one outstanding pull; a concurrent pull is a defect.
    #[doc(hidden)]
    pub fn from_body<F>(
        factory: impl Fn(StreamContext<A, E, R>) -> F + Send + Sync + 'static,
    ) -> Self
    where
        F: Future<Output = Result<(), Exit<(), E>>> + Send + 'static,
    {
        let factory = Arc::new(factory);
        Self::new(move |scope| {
            let factory = Arc::clone(&factory);
            let stream_scope = scope.clone();
            scope
                .acquire(
                    Effect::sync(|| {
                        Mutex::new(Machine {
                            body: None,
                            busy: false,
                            finished: false,
                        })
                    }),
                    |state| Release::sync(move || drop(state)),
                )
                .map(move |state| {
                    let factory = Arc::clone(&factory);
                    let scope = stream_scope.clone();
                    let slot = Arc::new(Mutex::new(None));
                    StreamReader::new(Effect::new(move |env, context, finish| {
                        let state = state.clone();
                        let slot = Arc::clone(&slot);
                        let factory = Arc::clone(&factory);
                        let scope = scope.clone();
                        let taken = state.with(|state| {
                            let mut state = state
                                .lock()
                                .unwrap_or_else(std::sync::PoisonError::into_inner);
                            if state.busy {
                                return Err(Defect::from(LibraryDefect::StreamReaderStillOwned));
                            }
                            state.busy = true;
                            Ok((state.body.take(), state.finished))
                        });
                        let (body, finished) = match taken {
                            Ok(Ok(state)) => state,
                            Ok(Err(defect)) | Err(defect) => {
                                return runtime::finish(finish, Exit::Die(defect));
                            }
                        };
                        let mut owner = PullOwner {
                            state,
                            body,
                            finished,
                        };
                        if finished {
                            return runtime::finish(finish, Exit::Success(Chunk::End));
                        }
                        if owner.body.is_none() {
                            let bridge = StreamContext {
                                context: MacroContext::with_scope(env, context, scope),
                                slot: Arc::clone(&slot),
                            };
                            owner.body = Some(Box::pin(async move {
                                AssertUnwindSafe(async { factory(bridge).await })
                                    .catch_unwind()
                                    .await
                                    .unwrap_or_else(|panic| {
                                        Err(Exit::Die(Defect::from_panic(panic)))
                                    })
                            }));
                        }
                        Step::Composed(Box::pin(async move {
                            let result = poll_fn(|cx| {
                                let Some(body) = owner.body.as_mut() else {
                                    return Poll::Ready(Exit::Die(Defect::from(
                                        LibraryDefect::StreamProducerSlotMissing,
                                    )));
                                };
                                let result = body.as_mut().poll(cx);
                                let emitted = slot
                                    .lock()
                                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                                    .take();
                                if let Some(value) = emitted {
                                    return Poll::Ready(Exit::Success(Chunk::Values(vec![value])));
                                }
                                match result {
                                    Poll::Pending => Poll::Pending,
                                    Poll::Ready(result) => {
                                        owner.finished = true;
                                        owner.body = None;
                                        Poll::Ready(match result {
                                            Ok(()) => Exit::Success(Chunk::End),
                                            Err(failure) => match failure.into_failure() {
                                                Ok(()) => Exit::Success(Chunk::End),
                                                Err(failure) => failure,
                                            },
                                        })
                                    }
                                }
                            })
                            .await;
                            drop(owner);
                            runtime::finish(finish, result)
                        }))
                    }))
                })
        })
    }
    /// Pull each mapped stream in source order under the same owning scope. Values move
    /// into the mapper; no synchronous Iterator implementation or payload copying is required.
    ///
    /// ```
    /// use effect::{Effect, Stream, effect, stream};
    /// fn increment(value: i32) -> Effect<i32, (), ()> {
    ///     Effect::sync_once(move || value + 1)
    /// }
    /// let source: Stream<i32, (), ()> = stream! {
    ///     for value in 0..3 { yield increment(value)?; }
    /// };
    /// let nested = source.flat_map(|value| stream! { yield value; });
    /// let program = effect! { nested.run_collect(Some(3))? };
    /// # let _ = program;
    /// ```
    pub fn flat_map<B: Send + 'static>(
        self,
        map: impl Fn(A) -> Stream<B, E, R> + Send + Sync + 'static,
    ) -> Stream<B, E, R> {
        let map = Arc::new(map);
        Stream::from_body(move |context| {
            let source = self.clone();
            let map = Arc::clone(&map);
            async move {
                let reader = context.bind(source.open(context.context.scope())).await?;
                loop {
                    match context.bind(reader.pull()).await? {
                        Chunk::End => return Ok(()),
                        Chunk::Values(values) => {
                            for value in values {
                                context.append(map(value)).await?;
                            }
                        }
                    }
                }
            }
        })
    }
}
