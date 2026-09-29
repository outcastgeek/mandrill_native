//! Typed reactive query descriptions and refresh behavior.

use super::{
    AsyncResult, InvalidationBatch, InvalidationError, InvalidationKey, InvalidationRegistry,
};
use crate::pub_sub::{Subscription, SubscriptionEvent};
use crate::{
    Chunk, Effect, Exit, Release, ResourceValue, Scope, SharedValue, Stream, StreamReader,
};
use std::num::NonZeroU64;
use std::sync::{Arc, Mutex};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// A source, dependency, or evaluation failure in a reactive query.
pub enum ReactiveQueryError {
    /// The ation failed validation. Carries `InvalidationError`.
    Invalidation(InvalidationError),
    /// Another stream pull is already in progress.
    ConcurrentPull,
    /// No further generation can be allocated without exceeding its range.
    GenerationExhausted,
    /// The subscriber’s owning source has closed.
    SubscriptionEnded,
}

#[derive(Clone, Copy)]
/// Variants of query stage used by reactivity query.
enum QueryStage {
    Initial,
    NeedEvaluation,
    Evaluating,
    EvaluationActive,
    Waiting,
}

/// Typed query state used by reactivity query.
///
/// # Type parameters
/// - `A`: Application value type. Bound or type: `Send + 'static`.
struct QueryState<A: Send + 'static> {
    stage: QueryStage,
    generation: Option<NonZeroU64>,
    previous_success: Option<SharedValue<A>>,
    seen_revision: Option<NonZeroU64>,
}

/// An inert description of a query and its stable invalidation dependencies.
/// Each stream open creates independent state and subscribes before the first
/// evaluation. The evaluation itself remains an Effect in R.
///
/// # Type parameters
/// - `A`: Application value type. Bound or type: `Send + 'static`.
/// - `E`: Domain-error type. Bound or type: `Send + 'static`.
/// - `R`: Environment type. Bound or type: `Send + Sync + 'static`.
pub struct ReactiveQuery<A: Send + 'static, E: Send + 'static, R: Send + Sync + 'static> {
    registry: InvalidationRegistry,
    dependencies: Arc<[InvalidationKey]>,
    evaluate: Arc<dyn Fn() -> Effect<A, E, R> + Send + Sync>,
}

impl<A: Send + 'static, E: Send + 'static, R: Send + Sync + 'static> Clone
    for ReactiveQuery<A, E, R>
{
    ///
    /// # Returns
    /// Type: `Self`.
    fn clone(&self) -> Self {
        Self {
            registry: self.registry.clone(),
            dependencies: Arc::clone(&self.dependencies),
            evaluate: Arc::clone(&self.evaluate),
        }
    }
}

impl<A: Send + 'static, E: Send + 'static, R: Send + Sync + 'static> ReactiveQuery<A, E, R> {
    #[must_use]
    /// Construct a `ReactiveQuery` from its required inputs and explicit policies.
    ///
    /// # Type parameters
    /// - `A`: Success or application-value type.
    /// - `E`: Domain-error type.
    /// - `R`: Environment type.
    ///
    /// # Parameters
    /// - `registry` (`InvalidationRegistry`): Registry that owns definitions and rejects identity conflicts.
    /// - `dependencies` (`impl IntoIterator<Item = InvalidationKey>`): The dependencies supplied to this operation.
    /// - `evaluate` (`impl Fn() -> Effect<A, E, R> + Send + Sync + 'static`): The evaluate supplied to this operation.
    ///
    /// # Returns
    /// Type: `Self`.
    pub fn new(
        registry: InvalidationRegistry,
        dependencies: impl IntoIterator<Item = InvalidationKey>,
        evaluate: impl Fn() -> Effect<A, E, R> + Send + Sync + 'static,
    ) -> Self {
        let ordered = dependencies
            .into_iter()
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        Self {
            registry,
            dependencies: Arc::from(ordered),
            evaluate: Arc::new(evaluate),
        }
    }

    /// The first pull publishes Initial, the second Running, and the third
    /// evaluates. Later pulls wait for relevant invalidations. A lagged bounded
    /// subscription forces resnapshot/reload rather than losing an update.
    #[must_use]
    /// # Type parameters
    /// - `A`: Success or application-value type.
    /// - `E`: Domain-error type.
    /// - `R`: Environment type.
    ///
    /// # Returns
    /// Type: `Stream<AsyncResult<A, E>, ReactiveQueryError, R>`.
    /// Returns a scoped stream with the declared item, error, and environment types.
    pub fn stream(&self) -> Stream<AsyncResult<A, E>, ReactiveQueryError, R> {
        let query = self.clone();
        Stream::new(move |scope: Scope<ReactiveQueryError>| {
            let query = query.clone();
            let subscription = query
                .registry
                .subscribe()
                .map_error(ReactiveQueryError::Invalidation);
            scope
                .acquire(subscription, |subscription| {
                    Release::sync(move || {
                        subscription.close_now();
                    })
                })
                .map(move |subscription| {
                    let state = Arc::new(Mutex::new(QueryState {
                        stage: QueryStage::Initial,
                        generation: None,
                        previous_success: None,
                        seen_revision: None,
                    }));
                    let query = query.clone();
                    StreamReader::new(query_pull(query, subscription, state))
                })
        })
    }
}

///
/// # Type parameters
/// - `A`: Successful application-value type. Additional bound: `Send + 'static`.
/// - `E`: Expected application-error type. Additional bound: `Send + 'static`.
/// - `R`: Environment type supplying required capabilities. Additional bound: `Send + Sync + 'static`.
///
/// # Parameters
/// - `query` (`ReactiveQuery<A, E, R>`): The query supplied to this operation.
/// - `subscription` (`ResourceValue<Subscription<InvalidationBatch>>`): The subscription supplied to this operation.
/// - `state` (`Arc<Mutex<QueryState<A>>>`): Current or replacement state owned by this operation.
///
/// # Returns
/// Type: `Effect<Chunk<AsyncResult<A, E>>, ReactiveQueryError, R>`.
/// Returns a lazy `Effect` with the declared success, error, and environment types.
fn query_pull<A, E, R>(
    query: ReactiveQuery<A, E, R>,
    subscription: ResourceValue<Subscription<InvalidationBatch>>,
    state: Arc<Mutex<QueryState<A>>>,
) -> Effect<Chunk<AsyncResult<A, E>>, ReactiveQueryError, R>
where
    A: Send + 'static,
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    Effect::defer(move || {
        let query = query.clone();
        let subscription = subscription.clone();
        let state = Arc::clone(&state);
        let stage = {
            let mut state = state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            match state.stage {
                QueryStage::Initial => {
                    state.stage = QueryStage::NeedEvaluation;
                    return Effect::sync(|| Chunk::Values(vec![AsyncResult::Initial]));
                }
                QueryStage::NeedEvaluation => {
                    let next = match state.generation {
                        None => Some(NonZeroU64::MIN),
                        Some(generation) => {
                            generation.get().checked_add(1).and_then(NonZeroU64::new)
                        }
                    };
                    let Some(generation) = next else {
                        return Effect::from_result_once(Err(
                            ReactiveQueryError::GenerationExhausted,
                        ));
                    };
                    state.generation = Some(generation);
                    state.stage = QueryStage::Evaluating;
                    let previous_success = state.previous_success.clone();
                    return Effect::sync_once(move || {
                        Chunk::Values(vec![AsyncResult::Running {
                            generation,
                            refreshing: previous_success.is_some(),
                            previous_success,
                        }])
                    });
                }
                QueryStage::Evaluating => {
                    state.stage = QueryStage::EvaluationActive;
                    QueryStage::Evaluating
                }
                QueryStage::EvaluationActive => QueryStage::EvaluationActive,
                QueryStage::Waiting => QueryStage::Waiting,
            }
        };
        match stage {
            QueryStage::Evaluating => evaluate_query(query, subscription, state),
            QueryStage::Waiting => wait_invalidation(query, subscription, state),
            QueryStage::Initial | QueryStage::NeedEvaluation | QueryStage::EvaluationActive => {
                Effect::from_result_once(Err(ReactiveQueryError::ConcurrentPull))
            }
        }
    })
}

///
/// # Type parameters
/// - `A`: Successful application-value type. Additional bound: `Send + 'static`.
/// - `E`: Expected application-error type. Additional bound: `Send + 'static`.
/// - `R`: Environment type supplying required capabilities. Additional bound: `Send + Sync + 'static`.
///
/// # Parameters
/// - `query` (`ReactiveQuery<A, E, R>`): The query supplied to this operation.
/// - `_subscription` (`ResourceValue<Subscription<InvalidationBatch>>`): The subscription supplied to this operation.
/// - `state` (`Arc<Mutex<QueryState<A>>>`): Current or replacement state owned by this operation.
///
/// # Returns
/// Type: `Effect<Chunk<AsyncResult<A, E>>, ReactiveQueryError, R>`.
/// Returns a lazy `Effect` with the declared success, error, and environment types.
fn evaluate_query<A, E, R>(
    query: ReactiveQuery<A, E, R>,
    _subscription: ResourceValue<Subscription<InvalidationBatch>>,
    state: Arc<Mutex<QueryState<A>>>,
) -> Effect<Chunk<AsyncResult<A, E>>, ReactiveQueryError, R>
where
    A: Send + 'static,
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    let generation = state
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .generation;
    let Some(generation) = generation else {
        return Effect::from_result_once(Err(ReactiveQueryError::ConcurrentPull));
    };
    let dependencies = Arc::clone(&query.dependencies);
    let registry = query.registry.clone();
    let evaluate = Arc::clone(&query.evaluate);
    let state_for_cleanup = Arc::clone(&state);
    registry
        .revision_of(Arc::clone(&dependencies))
        .map_error(ReactiveQueryError::Invalidation)
        .flat_map(move |before| {
            let registry = registry.clone();
            let dependencies = Arc::clone(&dependencies);
            let state = Arc::clone(&state);
            (evaluate)()
                .materialize_exit()
                .map_error(|never: std::convert::Infallible| match never {})
                .flat_map(move |outcome| {
                    let outcome = Mutex::new(Some(outcome));
                    let state = Arc::clone(&state);
                    registry
                        .revision_of(Arc::clone(&dependencies))
                        .map_error(ReactiveQueryError::Invalidation)
                        .flat_map(move |after| {
                            let outcome = outcome
                                .lock()
                                .unwrap_or_else(std::sync::PoisonError::into_inner)
                                .take();
                            let Some(outcome) = outcome else {
                                return Effect::from_result_once(Err(
                                    ReactiveQueryError::ConcurrentPull,
                                ));
                            };
                            let mut state = state
                                .lock()
                                .unwrap_or_else(std::sync::PoisonError::into_inner);
                            if state.generation != Some(generation)
                                || !matches!(state.stage, QueryStage::EvaluationActive)
                            {
                                return Effect::sync(|| Chunk::Values(Vec::new()));
                            }
                            let previous = state.previous_success.clone();
                            let result = AsyncResult::settle(generation, outcome, previous);
                            if let AsyncResult::Settled {
                                outcome: Exit::Success(value),
                                ..
                            } = &result
                            {
                                state.previous_success = Some(value.clone());
                            }
                            state.seen_revision = after;
                            state.stage = if after != before {
                                QueryStage::NeedEvaluation
                            } else {
                                QueryStage::Waiting
                            };
                            Effect::sync_once(move || Chunk::Values(vec![result]))
                        })
                })
        })
        .ensuring(Effect::sync(move || {
            let mut state = state_for_cleanup
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if state.generation == Some(generation)
                && matches!(state.stage, QueryStage::EvaluationActive)
            {
                state.stage = QueryStage::NeedEvaluation;
            }
        }))
}

///
/// # Type parameters
/// - `A`: Successful application-value type. Additional bound: `Send + 'static`.
/// - `E`: Expected application-error type. Additional bound: `Send + 'static`.
/// - `R`: Environment type supplying required capabilities. Additional bound: `Send + Sync + 'static`.
///
/// # Parameters
/// - `query` (`ReactiveQuery<A, E, R>`): The query supplied to this operation.
/// - `subscription` (`ResourceValue<Subscription<InvalidationBatch>>`): The subscription supplied to this operation.
/// - `state` (`Arc<Mutex<QueryState<A>>>`): Current or replacement state owned by this operation.
///
/// # Returns
/// Type: `Effect<Chunk<AsyncResult<A, E>>, ReactiveQueryError, R>`.
/// Returns a lazy `Effect` with the declared success, error, and environment types.
fn wait_invalidation<A, E, R>(
    query: ReactiveQuery<A, E, R>,
    subscription: ResourceValue<Subscription<InvalidationBatch>>,
    state: Arc<Mutex<QueryState<A>>>,
) -> Effect<Chunk<AsyncResult<A, E>>, ReactiveQueryError, R>
where
    A: Send + 'static,
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    let next = match subscription.with(|value| value.take::<R>()) {
        Ok(next) => next
            .map_error(|error| ReactiveQueryError::Invalidation(InvalidationError::PubSub(error))),
        Err(defect) => return Effect::die(defect),
    };
    next.flat_map(move |event| {
        let relevant = match event {
            SubscriptionEvent::Item(batch) => {
                let seen = state
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .seen_revision;
                Some(batch.revision) > seen
                    && batch
                        .keys
                        .iter()
                        .any(|key| query.dependencies.binary_search(key).is_ok())
            }
            SubscriptionEvent::Lagged(_) => true,
            SubscriptionEvent::End => {
                return Effect::from_result_once(Err(ReactiveQueryError::SubscriptionEnded));
            }
        };
        if relevant {
            state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .stage = QueryStage::NeedEvaluation;
        }
        query_pull(query.clone(), subscription.clone(), Arc::clone(&state))
    })
}
