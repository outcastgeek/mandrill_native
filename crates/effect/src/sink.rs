//! Typed consumers for effectful stream elements.

use crate::internal::runtime;
use crate::internal::workflow::{self, bind, success};
use crate::{Chunk, Defect, Effect, Exit, LibraryDefect, Stream, StreamReader};
use crate::{effect_die, effect_scope, effect_sync, sink};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;

/// Shared consumer callback dispatched through a Rust trait-object vtable.
/// Input, result, error, and environment types are still compile-time types.
///
/// # Type parameters
/// - `A`: Application value type.
/// - `B`: Transformed value type.
/// - `E`: Domain-error type.
/// - `R`: Environment type.
type Consume<A, B, E, R> = Arc<dyn Fn(StreamReader<A, E, R>) -> Effect<B, E, R> + Send + Sync>;

/// A typed, scoped consumer of a Stream. Its result and domain-error types
/// remain attached to the source throughout execution and finalization.
///
/// # Type parameters
/// - `A`: Application value type. Bound or type: `Send + 'static`.
/// - `B`: Transformed value type. Bound or type: `Send + 'static`.
/// - `E`: Domain-error type. Bound or type: `Send + 'static`.
/// - `R`: Environment type. Bound or type: `Send + Sync + 'static`.
pub struct Sink<A: Send + 'static, B: Send + 'static, E: Send + 'static, R: Send + Sync + 'static> {
    consume: Consume<A, B, E, R>,
}

impl<A: Send + 'static, B: Send + 'static, E: Send + 'static, R: Send + Sync + 'static> Clone
    for Sink<A, B, E, R>
{
    ///
    /// # Returns
    /// Type: `Self`.
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
    /// Construct a `Sink` from its required inputs and explicit policies.
    ///
    /// # Type parameters
    /// - `A`: Success or application-value type.
    /// - `B`: Transformed output type.
    /// - `E`: Domain-error type.
    /// - `R`: Environment type.
    ///
    /// # Parameters
    /// - `consume` (`impl Fn(StreamReader<A, E, R>) -> Effect<B, E, R> + Send + Sync + 'static`): The consume supplied to this operation.
    ///
    /// # Returns
    /// Type: `Self`.
    pub fn new(
        consume: impl Fn(StreamReader<A, E, R>) -> Effect<B, E, R> + Send + Sync + 'static,
    ) -> Self {
        Self {
            consume: Arc::new(consume),
        }
    }

    /// Execute or describe one run using the supplied environment and inputs.
    ///
    /// # Type parameters
    /// - `A`: Success or application-value type.
    /// - `B`: Transformed output type.
    /// - `E`: Domain-error type.
    /// - `R`: Environment type.
    ///
    /// # Parameters
    /// - `stream` (`Stream<A, E, R>`): Scoped source of ordered chunks or events.
    ///
    /// # Returns
    /// Type: `Effect<B, E, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    pub fn run(&self, stream: Stream<A, E, R>) -> Effect<B, E, R> {
        let sink = self.clone();
        effect_scope! {
            body |scope| {
            let sink = sink.clone();
            let stream = stream.clone();
            let reader = stream.open(scope)?;
            (sink.consume)(reader)?
            },
        }
    }

    #[must_use]
    /// Consume stream items into an accumulator under the sink’s ownership.
    ///
    /// # Type parameters
    /// - `A`: Success or application-value type.
    /// - `B`: Transformed output type.
    /// - `E`: Domain-error type.
    /// - `R`: Environment type.
    ///
    /// # Parameters
    /// - `initial` (`impl Fn() -> B + Send + Sync + 'static`): Initial value used before any state transition.
    /// - `step` (`impl Fn(B, A) -> Effect<B, E, R> + Send + Sync + 'static`): The step supplied to this operation.
    ///
    /// # Returns
    /// Type: `Self`.
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

///
/// # Type parameters
/// - `A`: Successful application-value type. Additional bound: `Send + 'static`.
/// - `S`: State or store value type. Additional bound: `Send + 'static`.
/// - `E`: Expected application-error type. Additional bound: `Send + 'static`.
/// - `R`: Environment type supplying required capabilities. Additional bound: `Send + Sync + 'static`.
/// - `F`: Callback or field-definition type. Additional bound: `Fn(S, A) -> Effect<S, E, R> + Send + Sync + 'static`.
///
/// # Parameters
/// - `reader` (`StreamReader<A, E, R>`): Scoped handle used to pull further items.
/// - `state` (`S`): Current or replacement state owned by this operation.
/// - `step` (`Arc<F>`): The step supplied to this operation.
///
/// # Returns
/// Type: `workflow::Workflow<S, E, R>`.
/// Returns `workflow::Workflow<S, E, R>`.
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

///
/// # Type parameters
/// - `A`: Successful application-value type. Additional bound: `Send + 'static`.
/// - `S`: State or store value type. Additional bound: `Send + 'static`.
/// - `E`: Expected application-error type. Additional bound: `Send + 'static`.
/// - `R`: Environment type supplying required capabilities. Additional bound: `Send + Sync + 'static`.
/// - `F`: Callback or field-definition type. Additional bound: `Fn(S, A) -> Effect<S, E, R> + Send + Sync + 'static`.
///
/// # Parameters
/// - `reader` (`StreamReader<A, E, R>`): Scoped handle used to pull further items.
/// - `state` (`S`): Current or replacement state owned by this operation.
/// - `values` (`std::vec::IntoIter<A>`): Owned values processed in their declared order.
/// - `step` (`Arc<F>`): The step supplied to this operation.
///
/// # Returns
/// Type: `workflow::Workflow<S, E, R>`.
/// Returns `workflow::Workflow<S, E, R>`.
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
    /// Collect consumed stream items under the declared collection bound.
    ///
    /// # Type parameters
    /// - `A`: Application value type. Bound: `Send + 'static`.
    /// - `E`: Domain-error type. Bound: `Send + 'static`.
    /// - `R`: Environment type. Bound: `Send + Sync + 'static`.
    ///
    /// # Parameters
    /// - `limit` (`Option<usize>`): The caller-supplied limit.
    ///
    /// # Returns
    /// Type: `Self`.
    pub fn collect(limit: Option<usize>) -> Self {
        sink! {
            fold_effect;
            initial: Vec::new,
            step: move |mut values, value| {
            if limit.is_some_and(|limit| values.len() >= limit) {
                effect_die! { defect: Defect::from(LibraryDefect::StreamCollectionLimitExceeded), }
            } else {
                values.push(value);
                effect_sync! { once; body: move || values, }
            }
            },
        }
    }
}

impl<A: Send + 'static, E: Send + 'static, R: Send + Sync + 'static> Sink<A, (), E, R> {
    #[must_use]
    /// Apply the supplied callback to each consumed item in order.
    ///
    /// # Type parameters
    /// - `A`: Success or application-value type.
    /// - `E`: Domain-error type.
    /// - `R`: Environment type.
    ///
    /// # Parameters
    /// - `consume` (`impl Fn(A) -> Effect<(), E, R> + Send + Sync + 'static`): The consume supplied to this operation.
    ///
    /// # Returns
    /// Type: `Self`.
    pub fn for_each(consume: impl Fn(A) -> Effect<(), E, R> + Send + Sync + 'static) -> Self {
        Self::fold(|| (), move |(), value| consume(value))
    }
}
