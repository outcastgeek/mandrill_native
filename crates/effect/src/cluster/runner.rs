use super::{
    ClaimOptions, ClaimToken, ClusterExecutor, ClusterMessageStore, ClusterRequestId,
    CompletedReply, DurableClusterMessageStore, HasClusterExecutor, HasClusterMessageStore,
    MessageClaim, StoreCompletion,
};
use crate::{Effect, HasTaskSpawner, HasWallClock, UtcMillis, WallClock};
use std::num::NonZeroUsize;
use std::sync::{Arc, Mutex};

pub type ClusterRunResult<Store, Execution, Clock> =
    Result<ClusterRunOutcome, ClusterRunnerError<Store, Execution, Clock>>;
pub type ClusterRunEffect<Store, Execution, Clock, R> = Effect<
    Vec<ClusterRunResult<Store, Execution, Clock>>,
    ClusterRunnerError<Store, Execution, Clock>,
    R,
>;

#[derive(Debug)]
pub enum ClusterRunnerError<Store, Execution, Clock> {
    UncertainClaim(Store),
    /// Execution failed after a claim; the handler may have performed an
    /// external effect. Inspect the request before deciding to run it again.
    Execution {
        token: ClaimToken,
        source: Execution,
    },
    /// The handler returned a reply, but the completion timestamp could not
    /// be obtained; its external effects may already have occurred.
    UncertainCompletionClock {
        token: ClaimToken,
        source: Clock,
    },
    /// Handler side effects may have occurred before completion was stored.
    /// A later claim may run the body again; inspect by request identity.
    UncertainCompletion {
        token: ClaimToken,
        source: Store,
    },
    InputConsumed,
}

pub struct ClusterRunOutcome {
    pub request: ClusterRequestId,
    pub completion: StoreCompletion,
}

/// One bounded local delivery pass. Store acceptance, execution and recorded
/// completion remain separate phases. Scheduling comes only from R, and each
/// claim's failure is reported independently without concealing other work.
pub struct ClusterRunner;

impl ClusterRunner {
    /// Run a delivery pass only with a message store that warrants recovery
    /// of accepted and unacknowledged work after its restart boundary.
    pub fn run_once_durable<R, StoreError, ExecutionError, ClockError>(
        options: ClaimOptions,
        parallelism: NonZeroUsize,
    ) -> ClusterRunEffect<StoreError, ExecutionError, ClockError, R>
    where
        R: HasClusterMessageStore
            + HasClusterExecutor
            + HasTaskSpawner
            + HasWallClock
            + Send
            + Sync
            + 'static,
        R::Store: DurableClusterMessageStore<Error = StoreError>,
        R::Executor: ClusterExecutor<R, Error = ExecutionError>,
        R::Clock: WallClock<Error = ClockError>,
        StoreError: Send + 'static,
        ExecutionError: Send + 'static,
        ClockError: Send + 'static,
    {
        Self::run_once(options, parallelism)
    }

    pub fn run_once<R, StoreError, ExecutionError, ClockError>(
        options: ClaimOptions,
        parallelism: NonZeroUsize,
    ) -> ClusterRunEffect<StoreError, ExecutionError, ClockError, R>
    where
        R: HasClusterMessageStore
            + HasClusterExecutor
            + HasTaskSpawner
            + HasWallClock
            + Send
            + Sync
            + 'static,
        R::Store: ClusterMessageStore<Error = StoreError>,
        R::Executor: ClusterExecutor<R, Error = ExecutionError>,
        R::Clock: WallClock<Error = ClockError>,
        StoreError: Send + 'static,
        ExecutionError: Send + 'static,
        ClockError: Send + 'static,
    {
        let options = Mutex::new(Some(options));
        Effect::service(|env: &R| env.cluster_message_store()).flat_map(move |store| {
            let options = options
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take();
            let Some(options) = options else {
                return Effect::from_result_once(Err(ClusterRunnerError::InputConsumed));
            };
            store
                .claim(options)
                .map_error(ClusterRunnerError::UncertainClaim)
                .provide_some(|_: &R| ())
                .flat_map(move |claims| {
                    let programs = claims
                        .into_iter()
                        .map(|claim| {
                            run_claim::<R, StoreError, ExecutionError, ClockError>(claim).either()
                        })
                        .collect();
                    Effect::all_concurrent(parallelism, programs)
                })
        })
    }
}

fn run_claim<R, StoreError, ExecutionError, ClockError>(
    claim: MessageClaim,
) -> Effect<ClusterRunOutcome, ClusterRunnerError<StoreError, ExecutionError, ClockError>, R>
where
    R: HasClusterMessageStore + HasClusterExecutor + HasWallClock + Send + Sync + 'static,
    R::Store: ClusterMessageStore<Error = StoreError>,
    R::Executor: ClusterExecutor<R, Error = ExecutionError>,
    R::Clock: WallClock<Error = ClockError>,
    StoreError: Send + 'static,
    ExecutionError: Send + 'static,
    ClockError: Send + 'static,
{
    let token = claim.token;
    let payload = claim.payload;
    Effect::service(|env: &R| (env.cluster_executor(), env.cluster_message_store())).flat_map(
        move |(executor, store)| {
            let token = token.clone();
            let payload = Arc::clone(&payload);
            executor
                .execute(token.address.clone(), payload)
                .map_error({
                    let error_token = token.clone();
                    move |source| ClusterRunnerError::Execution {
                        token: error_token.clone(),
                        source,
                    }
                })
                .flat_map(move |reply| {
                    let reply = Mutex::new(Some(reply));
                    let token = token.clone();
                    let store = Arc::clone(&store);
                    Effect::wall_now()
                        .map_error({
                            let error_token = token.clone();
                            move |source| ClusterRunnerError::UncertainCompletionClock {
                                token: error_token.clone(),
                                source,
                            }
                        })
                        .flat_map(move |completed_at: UtcMillis| {
                            let reply = reply
                                .lock()
                                .unwrap_or_else(std::sync::PoisonError::into_inner)
                                .take();
                            let Some(reply) = reply else {
                                return Effect::from_result_once(Err(
                                    ClusterRunnerError::InputConsumed,
                                ));
                            };
                            let completion = CompletedReply {
                                token: token.clone(),
                                reply,
                                completed_at,
                            };
                            let error_token = token.clone();
                            let request = token.request.clone();
                            store
                                .complete(completion)
                                .map_error(move |source| ClusterRunnerError::UncertainCompletion {
                                    token: error_token.clone(),
                                    source,
                                })
                                .provide_some(|_: &R| ())
                                .map(move |completion| ClusterRunOutcome {
                                    request: request.clone(),
                                    completion,
                                })
                        })
                        .uninterruptible()
                })
        },
    )
}
