use crate::internal::workflow::{self, bind, success};
use crate::{Defect, Effect, Exit, LibraryDefect, Release, Scope};
use futures_util::future::{Either, select};
use std::sync::{Arc, Mutex};

mod internal;
pub use internal::StreamBufferOptions;

pub enum Chunk<A> {
    Values(Vec<A>),
    End,
}

pub struct StreamReader<A: Send + 'static, E: Send + 'static, R: Send + Sync + 'static> {
    pull: Effect<Chunk<A>, E, R>,
}
impl<A: Send + 'static, E: Send + 'static, R: Send + Sync + 'static> Clone
    for StreamReader<A, E, R>
{
    fn clone(&self) -> Self {
        Self {
            pull: self.pull.clone(),
        }
    }
}
impl<A: Send + 'static, E: Send + 'static, R: Send + Sync + 'static> StreamReader<A, E, R> {
    #[must_use]
    pub const fn new(pull: Effect<Chunk<A>, E, R>) -> Self {
        Self { pull }
    }
    pub fn pull(&self) -> Effect<Chunk<A>, E, R> {
        self.pull.clone()
    }
}
type Open<A, E, R> = Arc<dyn Fn(Scope<E>) -> Effect<StreamReader<A, E, R>, E, R> + Send + Sync>;
pub struct Stream<A: Send + 'static, E: Send + 'static, R: Send + Sync + 'static> {
    open: Open<A, E, R>,
}
impl<A: Send + 'static, E: Send + 'static, R: Send + Sync + 'static> Clone for Stream<A, E, R> {
    fn clone(&self) -> Self {
        Self {
            open: Arc::clone(&self.open),
        }
    }
}
impl<A: Send + 'static, E: Send + 'static, R: Send + Sync + 'static> Stream<A, E, R> {
    #[must_use]
    pub fn new(
        open: impl Fn(Scope<E>) -> Effect<StreamReader<A, E, R>, E, R> + Send + Sync + 'static,
    ) -> Self {
        Self {
            open: Arc::new(open),
        }
    }
    pub fn open(&self, scope: Scope<E>) -> Effect<StreamReader<A, E, R>, E, R> {
        let open = Arc::clone(&self.open);
        Effect::defer(move || open(scope.clone()))
    }
    #[must_use]
    pub fn from_pull<T: Send + Sync + 'static>(
        acquire: Effect<T, E, R>,
        pull: impl Fn(crate::ResourceValue<T>) -> Effect<Chunk<A>, E, R> + Send + Sync + 'static,
        release: impl Fn(T) -> Release<E, R> + Send + Sync + 'static,
    ) -> Self {
        let pull = Arc::new(pull);
        let release = Arc::new(release);
        Self::new(move |scope| {
            let release = Arc::clone(&release);
            let pull = Arc::clone(&pull);
            scope
                .acquire(acquire.clone(), move |value| release(value))
                .map(move |resource| StreamReader::new(pull(resource)))
        })
    }
    #[must_use]
    pub fn from_iterable<I: Iterator<Item = A> + Send + 'static>(
        factory: impl Fn() -> I + Send + Sync + 'static,
    ) -> Self {
        Self::from_pull(
            Effect::sync(move || Mutex::new(factory())),
            |resource| {
                Effect::ready(move |_, _| {
                    match resource.with(|iterator| {
                        iterator
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner)
                            .next()
                    }) {
                        Ok(Some(value)) => Exit::Success(Chunk::Values(vec![value])),
                        Ok(None) => Exit::Success(Chunk::End),
                        Err(defect) => Exit::Die(defect),
                    }
                })
            },
            |iterator| Release::sync(move || drop(iterator)),
        )
    }
    #[must_use]
    pub fn map_effect<B: Send + 'static>(
        self,
        transform: impl Fn(A) -> Effect<B, E, R> + Send + Sync + 'static,
    ) -> Stream<B, E, R> {
        let transform = Arc::new(transform);
        Stream::new(move |scope| {
            let transform = Arc::clone(&transform);
            self.open(scope).map(move |reader| {
                let transform = Arc::clone(&transform);
                StreamReader::new(reader.pull().flat_map(move |chunk| {
                    match chunk {
                        Chunk::End => Effect::sync(|| Chunk::End),
                        Chunk::Values(values) => {
                            Effect::all(values.into_iter().map(|value| transform(value)).collect())
                                .map(Chunk::Values)
                        }
                    }
                }))
            })
        })
    }
    pub fn run_collect(&self, limit: Option<usize>) -> Effect<Vec<A>, E, R> {
        let stream = self.clone();
        Scope::scoped(move |scope| {
            let open = stream.open(scope);
            workflow::reusable(move || {
                bind(open.clone(), move |reader| {
                    collect(reader, Vec::new(), limit)
                })
            })
        })
    }
    pub fn run_for_each(
        &self,
        consume: impl Fn(A) -> Effect<(), E, R> + Send + Sync + 'static,
    ) -> Effect<(), E, R> {
        let stream = self.clone();
        let consume = Arc::new(consume);
        Scope::scoped(move |scope| {
            let open = stream.open(scope);
            let consume = Arc::clone(&consume);
            workflow::reusable(move || {
                let consume = Arc::clone(&consume);
                bind(open.clone(), move |reader| each(reader, consume))
            })
        })
    }
}

impl<A: Send + 'static, E: Send + 'static, R: crate::HasTaskSpawner + Send + Sync + 'static>
    Stream<A, E, R>
{
    /// Prefetch through a child owned by the opening scope. `capacity` bounds
    /// queued values, while `pull_batch` controls each downstream chunk.
    /// Values drain before the source's terminal outcome; scope cancellation
    /// interrupts the child and discards unread buffered values.
    #[must_use]
    pub fn buffer(self, options: StreamBufferOptions) -> Self {
        internal::buffered(self, options)
    }

    /// Merge two scoped sources. Arrival order across sources is deliberately
    /// unspecified; each source retains its own order. Queued values drain
    /// before a terminal failure, which cancels the other producer. Both
    /// sources must end successfully before the merged reader ends.
    #[must_use]
    pub fn merge(self, other: Self, options: StreamBufferOptions) -> Self {
        internal::merged(self, other, options)
    }
}
fn collect<A: Send + 'static, E: Send + 'static, R: Send + Sync + 'static>(
    reader: StreamReader<A, E, R>,
    mut values: Vec<A>,
    limit: Option<usize>,
) -> workflow::Workflow<Vec<A>, E, R> {
    bind(reader.pull(), move |chunk| match chunk {
        Chunk::End => success(values),
        Chunk::Values(mut chunk) => {
            if limit.is_some_and(|limit| chunk.len() > limit.saturating_sub(values.len())) {
                return workflow::Workflow::from_exit(Exit::Die(Defect::from(
                    LibraryDefect::StreamCollectionLimitExceeded,
                )));
            }
            values.append(&mut chunk);
            collect(reader, values, limit)
        }
    })
}
fn each<A: Send + 'static, E: Send + 'static, R: Send + Sync + 'static>(
    reader: StreamReader<A, E, R>,
    consume: Arc<dyn Fn(A) -> Effect<(), E, R> + Send + Sync>,
) -> workflow::Workflow<(), E, R> {
    bind(reader.pull(), move |chunk| match chunk {
        Chunk::End => success(()),
        Chunk::Values(chunk) => drain_chunk(reader, chunk.into_iter(), consume),
    })
}
fn drain_chunk<A: Send + 'static, E: Send + 'static, R: Send + Sync + 'static>(
    reader: StreamReader<A, E, R>,
    mut chunk: std::vec::IntoIter<A>,
    consume: Arc<dyn Fn(A) -> Effect<(), E, R> + Send + Sync>,
) -> workflow::Workflow<(), E, R> {
    match chunk.next() {
        Some(value) => bind(consume(value), move |()| {
            drain_chunk(reader, chunk, consume)
        }),
        None => each(reader, consume),
    }
}

impl<A: Send + 'static, E: Send + 'static, R: Send + Sync + 'static> Stream<A, E, R> {
    #[must_use]
    /// A cancellation request interrupts a blocked lock or next-item wait and
    /// releases the scoped source. Whether a partially polled item can be
    /// replayed depends on the supplied async stream's own contract.
    pub fn from_async_iterable<
        X: Send + 'static,
        S: futures_util::stream::Stream<Item = Result<A, X>> + Unpin + Send + 'static,
    >(
        factory: impl Fn() -> S + Send + Sync + 'static,
        on_error: impl Fn(X) -> E + Send + Sync + 'static,
        dispose: impl Fn(S) -> Release<E, R> + Send + Sync + 'static,
    ) -> Self {
        let on_error = Arc::new(on_error);
        let dispose = Arc::new(dispose);
        Self::from_pull(
            Effect::sync(move || Arc::new(futures_util::lock::Mutex::new(factory()))),
            move |resource| {
                let on_error = Arc::clone(&on_error);
                Effect::new(move |env, context, finish| {
                    let stream = match resource.with(Arc::clone) {
                        Ok(stream) => stream,
                        Err(defect) => {
                            return crate::internal::runtime::finish(finish, Exit::Die(defect));
                        }
                    };
                    let on_error = Arc::clone(&on_error);
                    crate::internal::runtime::Step::Suspended(Box::pin(async move {
                        let work = async {
                            let lock = std::pin::pin!(stream.lock());
                            let cancelled = std::pin::pin!(context.cancellation.cancelled());
                            let mut stream = match select(lock, cancelled).await {
                                Either::Left((stream, _)) => stream,
                                Either::Right(_) => return Exit::Interrupted,
                            };
                            use futures_util::StreamExt;
                            let next = std::pin::pin!(stream.next());
                            let cancelled = std::pin::pin!(context.cancellation.cancelled());
                            match select(next, cancelled).await {
                                Either::Left((Some(Ok(value)), _)) => {
                                    Exit::Success(Chunk::Values(vec![value]))
                                }
                                Either::Left((Some(Err(error)), _)) => Exit::Fail(on_error(error)),
                                Either::Left((None, _)) => Exit::Success(Chunk::End),
                                Either::Right(_) => Exit::Interrupted,
                            }
                        };
                        let result = futures_util::FutureExt::catch_unwind(
                            std::panic::AssertUnwindSafe(work),
                        )
                        .await
                        .unwrap_or_else(|panic| Exit::Die(Defect::from_panic(panic)));
                        drop(env);
                        crate::internal::runtime::finish(
                            finish,
                            crate::internal::runtime::publish(result, &context),
                        )
                    }))
                })
            },
            move |stream| match Arc::try_unwrap(stream) {
                Ok(stream) => dispose(stream.into_inner()),
                Err(_) => Release::from_effect(Effect::die(Defect::from(
                    LibraryDefect::StreamReaderStillOwned,
                ))),
            },
        )
    }
}
