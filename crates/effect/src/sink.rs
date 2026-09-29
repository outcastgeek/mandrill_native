use crate::internal::runtime;
use crate::internal::workflow::{self, bind, success};
use crate::{Chunk, Defect, Effect, Exit, LibraryDefect, Scope, Stream, StreamReader};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;

type Consume<A, B, E, R> = Arc<dyn Fn(StreamReader<A, E, R>) -> Effect<B, E, R> + Send + Sync>;

/// A typed, scoped consumer of a Stream. Its result and domain-error types
/// remain attached to the source throughout execution and finalization.
pub struct Sink<A: Send + 'static, B: Send + 'static, E: Send + 'static, R: Send + Sync + 'static> {
    consume: Consume<A, B, E, R>,
}

impl<A: Send + 'static, B: Send + 'static, E: Send + 'static, R: Send + Sync + 'static> Clone
    for Sink<A, B, E, R>
{
    fn clone(&self) -> Self {
        Self {
            consume: Arc::clone(&self.consume),
        }
    }
}

impl<A: Send + 'static, B: Send + 'static, E: Send + 'static, R: Send + Sync + 'static>
    Sink<A, B, E, R>
{
    #[must_use]
    pub fn new(
        consume: impl Fn(StreamReader<A, E, R>) -> Effect<B, E, R> + Send + Sync + 'static,
    ) -> Self {
        Self {
            consume: Arc::new(consume),
        }
    }

    pub fn run(&self, stream: Stream<A, E, R>) -> Effect<B, E, R> {
        let sink = self.clone();
        Scope::scoped(move |scope| {
            let sink = sink.clone();
            stream
                .open(scope)
                .flat_map(move |reader| (sink.consume)(reader))
        })
    }

    #[must_use]
    pub fn fold(
        initial: impl Fn() -> B + Send + Sync + 'static,
        step: impl Fn(B, A) -> Effect<B, E, R> + Send + Sync + 'static,
    ) -> Self {
        let initial = Arc::new(initial);
        let step = Arc::new(step);
        Sink::new(move |reader| {
            let initial = Arc::clone(&initial);
            let step = Arc::clone(&step);
            Effect::new(move |env, context, finish| {
                let state = match catch_unwind(AssertUnwindSafe(|| initial())) {
                    Ok(state) => state,
                    Err(panic) => {
                        return runtime::finish(finish, Exit::Die(Defect::from_panic(panic)));
                    }
                };
                fold_reader(reader.clone(), state, step.clone()).execute(env, context, finish)
            })
        })
    }
}

fn fold_reader<A, S, E, R, F>(
    reader: StreamReader<A, E, R>,
    state: S,
    step: Arc<F>,
) -> workflow::Workflow<S, E, R>
where
    A: Send + 'static,
    S: Send + 'static,
    E: Send + 'static,
    R: Send + Sync + 'static,
    F: Fn(S, A) -> Effect<S, E, R> + Send + Sync + 'static,
{
    bind(reader.pull(), move |chunk| match chunk {
        Chunk::End => success(state),
        Chunk::Values(values) => fold_chunk(reader, state, values.into_iter(), step),
    })
}

fn fold_chunk<A, S, E, R, F>(
    reader: StreamReader<A, E, R>,
    state: S,
    mut values: std::vec::IntoIter<A>,
    step: Arc<F>,
) -> workflow::Workflow<S, E, R>
where
    A: Send + 'static,
    S: Send + 'static,
    E: Send + 'static,
    R: Send + Sync + 'static,
    F: Fn(S, A) -> Effect<S, E, R> + Send + Sync + 'static,
{
    match values.next() {
        Some(value) => bind(step(state, value), move |state| {
            fold_chunk(reader, state, values, step)
        }),
        None => fold_reader(reader, state, step),
    }
}

impl<A: Send + 'static, E: Send + 'static, R: Send + Sync + 'static> Sink<A, Vec<A>, E, R> {
    #[must_use]
    pub fn collect(limit: Option<usize>) -> Self {
        Self::fold(Vec::new, move |mut values, value| {
            if limit.is_some_and(|limit| values.len() >= limit) {
                Effect::die(Defect::from(LibraryDefect::StreamCollectionLimitExceeded))
            } else {
                values.push(value);
                Effect::sync_once(move || values)
            }
        })
    }
}

impl<A: Send + 'static, E: Send + 'static, R: Send + Sync + 'static> Sink<A, (), E, R> {
    #[must_use]
    pub fn for_each(consume: impl Fn(A) -> Effect<(), E, R> + Send + Sync + 'static) -> Self {
        Self::fold(|| (), move |(), value| consume(value))
    }
}
