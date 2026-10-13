//! Safe provider-neutral SQL completion observation selected through R.

use crate::effect_future;
use effect::{Clock, Defect, Effect, Exit, HasClock};
use std::{sync::Arc, time::Duration};

/// SQL categories carry no raw SQL, parameter data or connection strings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SqlOperation {
    /// A native statement returns affected-row metadata.
    Execute,
    /// Buffered native row decoding.
    Query,
    /// Optional first-row decoding.
    First,
    /// Required native scalar decoding.
    Scalar,
    /// Incremental scoped row fold.
    Fold,
    /// Ordered nontransactional native batch.
    Batch,
    /// Operation-owned transaction and cleanup.
    Transaction,
    /// Nested native savepoint and cleanup.
    Savepoint,
    /// Locked schema/history operation.
    Migration,
}
/// Redacted terminal metadata for one observed operation, including cleanup classification.
pub struct SqlCompletion {
    /// Caller-selected semantic operation category; no SQL text is included.
    pub operation: SqlOperation,
    /// Monotonic duration in Duration units supplied by R's clock provider.
    pub duration: Duration,
    /// Whether the business result completed successfully.
    pub succeeded: bool,
    /// Whether a defect rather than an expected SQL error occurred.
    pub defected: bool,
    /// Whether cooperative interruption ended the business operation.
    pub interrupted: bool,
    /// Whether ordered cleanup failed.
    pub cleanup_failed: bool,
    /// Optional explicit count projected from a successful result, never a payload serialization.
    pub result_count: Option<u64>,
    /// Original Effect trace for native OTel/subscriber bridges; local IDs are not OTel IDs.
    pub trace: effect::TraceContext,
}
/// Provider-neutral observer selected from the same application environment R.
pub trait SqlObservation: Send + Sync {
    /// Record safe completion metadata; best-effort failures cannot replace a business exit.
    fn completed(&self, completion: SqlCompletion);
    /// Record an observer defect through the selected diagnostic provider.
    fn diagnostic(&self, defect: Defect);
}
/// Observe one SQL Effect using R's monotonic clock and selected service; no exporter is mandatory.
/// Result-count projection runs only on success and must not expose values or identifiers.
pub fn observe<A, E, R>(
    program: Effect<A, E, R>,
    operation: SqlOperation,
    select: impl Fn(&R) -> Arc<dyn SqlObservation> + Send + Sync + 'static,
    count: impl Fn(&A) -> Option<u64> + Send + Sync + 'static,
) -> Effect<A, E, R>
where
    A: Send + 'static,
    E: Send + 'static,
    R: HasClock + Send + Sync + 'static,
{
    let select = Arc::new(select);
    let count = Arc::new(count);
    effect_future! {
        exit_with_context; body: move |env, context| {
        let program = program.clone();
        let select = select.clone();
        let count = count.clone();
        async move {
            let observer =
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| select(&env))).ok();
            let started =
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| env.clock().now()));
            let outcome = context.run(program, env.clone()).await;
            let Some(observer) = observer else {
                return outcome;
            };
            let started = match started {
                Ok(started) => started,
                Err(panic) => {
                    report_diagnostic(&observer, Defect::from_panic(panic));
                    return outcome;
                }
            };
            let completed = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                env.clock().now()
            })) {
                Ok(completed) => completed,
                Err(panic) => {
                    report_diagnostic(&observer, Defect::from_panic(panic));
                    return outcome;
                }
            };
            let result_count = match &outcome {
                Exit::Success(value) => {
                    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| count(value))) {
                        Ok(count) => count,
                        Err(panic) => {
                            report_diagnostic(&observer, Defect::from_panic(panic));
                            None
                        }
                    }
                }
                Exit::Fail(_) | Exit::Die(_) | Exit::Interrupted | Exit::FinalizationFailed(_) => {
                    None
                }
            };
            let completion = SqlCompletion {
                operation,
                duration: completed.saturating_sub(started),
                succeeded: matches!(outcome, Exit::Success(_)),
                defected: matches!(outcome.primary(), effect::PrimaryOutcomeRef::Defected(_)),
                interrupted: matches!(outcome.primary(), effect::PrimaryOutcomeRef::WasInterrupted),
                cleanup_failed: matches!(outcome, Exit::FinalizationFailed(_)),
                result_count,
                trace: context.trace().clone(),
            };
            if let Err(panic) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                observer.completed(completion)
            })) {
                report_diagnostic(&observer, Defect::from_panic(panic));
            }

            outcome
        }
    }, }
}

/// Isolate observer diagnostic failures as best effort; neither channel may replace the business exit.
fn report_diagnostic(observer: &Arc<dyn SqlObservation>, defect: Defect) {
    let _result =
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| observer.diagnostic(defect)));
}
