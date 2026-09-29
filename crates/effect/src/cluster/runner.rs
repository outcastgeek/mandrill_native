//! Claiming and executing accepted Cluster messages.

use super::{
    ClaimOptions, ClaimToken, ClusterExecutor, ClusterMessageStore, ClusterRequestId,
    CompletedReply, DurableClusterMessageStore, HasClusterExecutor, HasClusterMessageStore,
    MessageClaim, StoreCompletion,
};
use crate::{Effect, HasTaskSpawner, HasWallClock, UtcMillis, WallClock};
use std::num::NonZeroUsize;
use std::sync::{Arc, Mutex};

/// Type alias for cluster run result in cluster runner.
///
/// # Type parameters
/// - `Store`: Backing store provider type.
/// - `Execution`: Execution provider or mode type.
/// - `Clock`: Clock provider type.
pub type ClusterRunResult<Store, Execution, Clock> =
    Result<ClusterRunOutcome, ClusterRunnerError<Store, Execution, Clock>>;
/// Type alias for cluster run effect in cluster runner.
///
/// # Type parameters
/// - `Store`: Backing store provider type.
/// - `Execution`: Execution provider or mode type.
/// - `Clock`: Clock provider type.
/// - `R`: Environment type.
pub type ClusterRunEffect<Store, Execution, Clock, R> = Effect<
    Vec<ClusterRunResult<Store, Execution, Clock>>,
    ClusterRunnerError<Store, Execution, Clock>,
    R,
>;

#[derive(Debug)]
/// A failure while claiming, dispatching, or settling accepted cluster messages.
///
/// # Type parameters
/// - `Store`: Backing store provider type.
/// - `Execution`: Execution provider or mode type.
/// - `Clock`: Clock provider type.
pub enum ClusterRunnerError<Store, Execution, Clock> {
    /// The claim may have committed; ownership could not be confirmed. Retains the `Store` detail.
    UncertainClaim(Store),
    /// Execution failed after a claim; the handler may have performed an
    /// external effect. Inspect the request before deciding to run it again.
    Execution {
        /// Fencing token used to reject stale owners or claims. Type: `ClaimToken`.
        token: ClaimToken,
        /// The underlying failure or provider cause. Type: `Execution`.
        source: Execution,
    },
    /// The handler returned a reply, but the completion timestamp could not
    /// be obtained; its external effects may already have occurred.
    UncertainCompletionClock {
        /// Fencing token used to reject stale owners or claims. Type: `ClaimToken`.
        token: ClaimToken,
        /// The underlying failure or provider cause. Type: `Clock`.
        source: Clock,
    },
    /// Handler side effects may have occurred before completion was stored.
    /// A later claim may run the body again; inspect by request identity.
    UncertainCompletion {
        /// Fencing token used to reject stale owners or claims. Type: `ClaimToken`.
        token: ClaimToken,
        /// The underlying failure or provider cause. Type: `Store`.
        source: Store,
    },
    /// An owned input was already consumed.
    InputConsumed,
}

/// The result of one runner attempt, including whether work was claimed or settled.
pub struct ClusterRunOutcome {
    /// The request being handled. Type: `ClusterRequestId`.
    pub request: ClusterRequestId,
    /// Store decision after attempting to settle the claimed message. Type: `StoreCompletion`.
    pub completion: StoreCompletion,
}

/// One bounded local delivery pass. Store acceptance, execution and recorded
/// completion remain separate phases. Scheduling comes only from R, and each
/// claim's failure is reported independently without concealing other work.
pub struct ClusterRunner;

impl ClusterRunner {
    /// Run a delivery pass only with a message store that warrants recovery
    /// of accepted and unacknowledged work after its restart boundary.
    ///
    /// # Type parameters
    /// - `R`: Environment type. Additional bound: `HasClusterMessageStore + HasClusterExecutor + HasTaskSpawner + HasWallClock + Send + Sync + 'static`. Additional bound: `:Store: DurableClusterMessageStore<Error = StoreError>`. Additional bound: `:Executor: ClusterExecutor<R, Error = ExecutionError>`. Additional bound: `:Clock: WallClock<Error = ClockError>`.
    /// - `StoreError`: Error returned by the backing store. Additional bound: `Send + 'static`.
    /// - `ExecutionError`: Error produced during execution. Additional bound: `Send + 'static`.
    /// - `ClockError`: Error returned by the clock capability. Additional bound: `Send + 'static`.
    ///
    /// # Parameters
    /// - `options` (`ClaimOptions`): Caller-selected bounds and behavior for this operation.
    /// - `parallelism` (`NonZeroUsize`): The parallelism supplied to this operation.
    ///
    /// # Returns
    /// Type: `ClusterRunEffect<StoreError, ExecutionError, ClockError, R>`.
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

    /// Claim and process one bounded batch of accepted cluster messages.
    ///
    /// # Type parameters
    /// - `R`: Environment type. Additional bound: `HasClusterMessageStore + HasClusterExecutor + HasTaskSpawner + HasWallClock + Send + Sync + 'static`. Additional bound: `:Store: ClusterMessageStore<Error = StoreError>`. Additional bound: `:Executor: ClusterExecutor<R, Error = ExecutionError>`. Additional bound: `:Clock: WallClock<Error = ClockError>`.
    /// - `StoreError`: Error returned by the backing store. Additional bound: `Send + 'static`.
    /// - `ExecutionError`: Error produced during execution. Additional bound: `Send + 'static`.
    /// - `ClockError`: Error returned by the clock capability. Additional bound: `Send + 'static`.
    ///
    /// # Parameters
    /// - `options` (`ClaimOptions`): Caller-selected bounds and behavior for this operation.
    /// - `parallelism` (`NonZeroUsize`): The parallelism supplied to this operation.
    ///
    /// # Returns
    /// Type: `ClusterRunEffect<StoreError, ExecutionError, ClockError, R>`.
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

///
/// # Type parameters
/// - `R`: Environment type supplying required capabilities. Additional bound: `HasClusterMessageStore + HasClusterExecutor + HasWallClock + Send + Sync + 'static`. Additional bound: `:Store: ClusterMessageStore<Error = StoreError>`. Additional bound: `:Executor: ClusterExecutor<R, Error = ExecutionError>`. Additional bound: `:Clock: WallClock<Error = ClockError>`.
/// - `StoreError`: Error returned by the backing store. Additional bound: `Send + 'static`.
/// - `ExecutionError`: Error produced during execution. Additional bound: `Send + 'static`.
/// - `ClockError`: Error returned by the clock capability. Additional bound: `Send + 'static`.
///
/// # Parameters
/// - `claim` (`MessageClaim`): Fenced ownership token for the selected message.
///
/// # Returns
/// Type: `Effect<ClusterRunOutcome, ClusterRunnerError<StoreError, ExecutionError, ClockError>, R>`.
/// Returns a lazy `Effect` with the declared success, error, and environment types.
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
