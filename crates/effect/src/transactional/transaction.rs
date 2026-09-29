//! Typed transactional composition, retry, and commit.

use super::HasTransactionDomain;
use super::internal::journal::{JournalError, TxJournal};
use crate::Effect;
use futures_util::future::{poll_fn, select};
use std::num::NonZeroUsize;
use std::sync::Arc;
use std::task::Poll;

/// Type alias for transaction step in transactional transaction.
///
/// # Type parameters
/// - `A`: Application value type.
/// - `E`: Domain-error type.
type TransactionStep<A, E> = dyn Fn(&mut TxJournal) -> TxStep<A, E> + Send + Sync;

///
/// # Returns
/// Type: `()`.
async fn yield_after_conflict() {
    let mut yielded = false;
    poll_fn(|context| {
        if std::mem::replace(&mut yielded, true) {
            Poll::Ready(())
        } else {
            context.waker().wake_by_ref();
            Poll::Pending
        }
    })
    .await
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// The caller’s bounds for optimistic conflict retries and waiting.
pub struct TransactionPolicy {
    /// Explicit finite conflict retries; no hidden spin or backoff constant.
    pub max_conflicts: NonZeroUsize,
}

#[derive(Debug)]
/// An expected failure, conflict, or domain failure during atomic execution.
///
/// # Type parameters
/// - `E`: Domain-error type.
pub enum TransactionError<E> {
    /// The application’s expected domain operation failed. Retains the `E` detail.
    Domain(E),
    /// The transaction references cells from different domains.
    MixedDomain,
    /// The required cell is missing.
    MissingCell,
    /// The supplied value type does not match the required or recorded value.
    WrongValueType,
    /// No further version can be allocated without exceeding its range.
    VersionExhausted,
    /// The configured conflict limit was reached.
    ConflictLimit,
    /// A transaction requested retry without observing any cell to await.
    RetryWithoutReads,
    /// Execution was cancelled.
    Cancelled,
}

impl<E> TransactionError<E> {
    /// # Type parameters
    /// - `E`: Domain-error type.
    ///
    /// # Parameters
    /// - `error` (`JournalError`): The error to propagate or record.
    ///
    /// # Returns
    /// Type: `Self`.
    pub(crate) fn from_journal(error: JournalError) -> Self {
        match error {
            JournalError::MixedDomain => Self::MixedDomain,
            JournalError::MissingCell => Self::MissingCell,
            JournalError::WrongValueType => Self::WrongValueType,
            JournalError::VersionExhausted => Self::VersionExhausted,
            JournalError::Conflict => Self::ConflictLimit,
        }
    }
}

/// Variants of tx step used by transactional transaction.
///
/// # Type parameters
/// - `A`: Application value type.
/// - `E`: Domain-error type.
pub(crate) enum TxStep<A, E> {
    Success(A),
    Failure(E),
    Retry,
    Conflict,
    Invalid(TransactionError<E>),
}

/// An inert, reusable transaction description. Its constructor is private to
/// this crate; only transactional operations compose inside this closure, so
/// an Effect/Future cannot be bound as a transaction step.
///
/// # Type parameters
/// - `A`: Application value type. Bound or type: `Send + 'static`.
/// - `E`: Domain-error type. Bound or type: `Send + 'static`.
pub struct Transaction<A: Send + 'static, E: Send + 'static> {
    evaluate: Arc<TransactionStep<A, E>>,
}

impl<A: Send + 'static, E: Send + 'static> Clone for Transaction<A, E> {
    ///
    /// # Returns
    /// Type: `Self`.
    fn clone(&self) -> Self {
        Self {
            evaluate: Arc::clone(&self.evaluate),
        }
    }
}

impl<A: Send + 'static, E: Send + 'static> Transaction<A, E> {
    /// # Type parameters
    /// - `A`: Success or application-value type.
    /// - `E`: Domain-error type.
    ///
    /// # Parameters
    /// - `evaluate` (`impl Fn(&mut TxJournal) -> TxStep<A, E> + Send + Sync + 'static`): The evaluate supplied to this operation.
    ///
    /// # Returns
    /// Type: `Self`.
    pub(crate) fn new(
        evaluate: impl Fn(&mut TxJournal) -> TxStep<A, E> + Send + Sync + 'static,
    ) -> Self {
        Self {
            evaluate: Arc::new(evaluate),
        }
    }

    /// Produce a fresh successful value when this description is executed.
    ///
    /// # Type parameters
    /// - `E`: Domain-error type. Bound: `Send + 'static`.
    /// - `A`: Success or application-value type.
    ///
    /// # Parameters
    /// - `value` (`impl Fn() -> A + Send + Sync + 'static`): Value read, transformed, encoded, or stored by this operation.
    ///
    /// # Returns
    /// Type: `Self`.
    pub fn succeed_with(value: impl Fn() -> A + Send + Sync + 'static) -> Self {
        Self::new(move |_| TxStep::Success(value()))
    }

    /// Create a fresh expected transaction error when this description runs.
    ///
    /// # Type parameters
    /// - `A`: Application value type. Bound: `Send + 'static`.
    /// - `E`: Domain-error type.
    ///
    /// # Parameters
    /// - `error` (`impl Fn() -> E + Send + Sync + 'static`): The error to propagate or record.
    ///
    /// # Returns
    /// Type: `Self`.
    pub fn fail_with(error: impl Fn() -> E + Send + Sync + 'static) -> Self {
        Self::new(move |_| TxStep::Failure(error()))
    }

    #[must_use]
    /// Describe another attempt under the supplied retry policy.
    ///
    /// # Type parameters
    /// - `A`: Application value type. Bound: `Send + 'static`.
    /// - `E`: Domain-error type. Bound: `Send + 'static`.
    ///
    /// # Returns
    /// Type: `Self`.
    pub fn retry() -> Self {
        Self::new(|_| TxStep::Retry)
    }

    /// Transform successful values while preserving other outcome channels.
    ///
    /// # Type parameters
    /// - `B`: Transformed value type. Bounds: `Send + 'static`.
    /// - `A`: Success or application-value type.
    /// - `E`: Domain-error type.
    ///
    /// # Parameters
    /// - `map` (`impl Fn(A) -> B + Send + Sync + 'static`): The map supplied to this operation.
    ///
    /// # Returns
    /// Type: `Transaction<B, E>`.
    pub fn map<B: Send + 'static>(
        self,
        map: impl Fn(A) -> B + Send + Sync + 'static,
    ) -> Transaction<B, E> {
        Transaction::new(move |journal| match (self.evaluate)(journal) {
            TxStep::Success(value) => TxStep::Success(map(value)),
            TxStep::Failure(error) => TxStep::Failure(error),
            TxStep::Retry => TxStep::Retry,
            TxStep::Conflict => TxStep::Conflict,
            TxStep::Invalid(error) => TxStep::Invalid(error),
        })
    }

    /// Build a dependent operation from each successful value.
    ///
    /// # Type parameters
    /// - `B`: Transformed value type. Bounds: `Send + 'static`.
    /// - `A`: Success or application-value type.
    /// - `E`: Domain-error type.
    ///
    /// # Parameters
    /// - `next` (`impl Fn(A) -> Transaction<B, E> + Send + Sync + 'static`): Next value or state produced by the transition.
    ///
    /// # Returns
    /// Type: `Transaction<B, E>`.
    pub fn flat_map<B: Send + 'static>(
        self,
        next: impl Fn(A) -> Transaction<B, E> + Send + Sync + 'static,
    ) -> Transaction<B, E> {
        Transaction::new(move |journal| match (self.evaluate)(journal) {
            TxStep::Success(value) => (next(value).evaluate)(journal),
            TxStep::Failure(error) => TxStep::Failure(error),
            TxStep::Retry => TxStep::Retry,
            TxStep::Conflict => TxStep::Conflict,
            TxStep::Invalid(error) => TxStep::Invalid(error),
        })
    }

    /// Try the second branch only when the first explicitly retries. Writes
    /// from the abandoned branch are rolled back; both read sets remain
    /// observed if both branches retry.
    #[must_use]
    /// # Type parameters
    /// - `A`: Application value type. Bound: `Send + 'static`.
    /// - `E`: Domain-error type. Bound: `Send + 'static`.
    ///
    /// # Parameters
    /// - `other` (`Self`): Second value used for comparison or composition.
    ///
    /// # Returns
    /// Type: `Self`.
    pub fn or_else(self, other: Self) -> Self {
        Self::new(move |journal| {
            let before = journal.branch();
            match (self.evaluate)(journal) {
                TxStep::Retry => {
                    let first_reads = journal.observed();
                    journal.rollback(before);
                    let second = (other.evaluate)(journal);
                    if matches!(&second, TxStep::Retry) {
                        journal.merge_observed(first_reads);
                    }
                    second
                }
                result @ (TxStep::Success(_)
                | TxStep::Failure(_)
                | TxStep::Conflict
                | TxStep::Invalid(_)) => result,
            }
        })
    }

    /// Validate the read set and commit all transaction writes as one state change.
    ///
    /// # Type parameters
    /// - `R`: Environment type. Additional bound: `HasTransactionDomain + Send + Sync + 'static`.
    /// - `A`: Success or application-value type.
    /// - `E`: Domain-error type.
    ///
    /// # Parameters
    /// - `policy` (`TransactionPolicy`): The caller-supplied behavior policy.
    ///
    /// # Returns
    /// Type: `Effect<A, TransactionError<E>, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    pub fn atomically<R>(&self, policy: TransactionPolicy) -> Effect<A, TransactionError<E>, R>
    where
        R: HasTransactionDomain + Send + Sync + 'static,
    {
        let transaction = self.clone();
        Effect::try_future_with_env(
            move |env: Arc<R>, cancellation| {
                let transaction = transaction.clone();
                async move {
                    let domain = env.transaction_domain();
                    let mut conflicts = 0usize;
                    loop {
                        if cancellation.is_requested() {
                            return Err(TransactionError::Cancelled);
                        }
                        let mut journal = TxJournal::new(Arc::clone(&domain.inner));
                        match (transaction.evaluate)(&mut journal) {
                            TxStep::Success(value) => {
                                if cancellation.is_requested() {
                                    return Err(TransactionError::Cancelled);
                                }
                                match journal.commit() {
                                    Ok(_) => return Ok(value),
                                    Err(JournalError::Conflict) => {}
                                    Err(error) => {
                                        return Err(TransactionError::from_journal(error));
                                    }
                                }
                            }
                            TxStep::Failure(error) => return Err(TransactionError::Domain(error)),
                            TxStep::Invalid(error) => return Err(error),
                            TxStep::Conflict => {}
                            TxStep::Retry => {
                                let observed = journal.observed();
                                if observed.is_empty() {
                                    return Err(TransactionError::RetryWithoutReads);
                                }
                                let listener = domain.inner.changed.listen();
                                let stale = {
                                    let state = domain
                                        .inner
                                        .state
                                        .lock()
                                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                                    observed.iter().any(|(id, record)| {
                                        state.cells.get(id).map(|cell| cell.version)
                                            != Some(record.version)
                                    })
                                };
                                if stale {
                                    continue;
                                }
                                let _ =
                                    select(Box::pin(listener), Box::pin(cancellation.cancelled()))
                                        .await;
                                if cancellation.is_requested() {
                                    return Err(TransactionError::Cancelled);
                                }
                                continue;
                            }
                        }
                        conflicts = conflicts
                            .checked_add(1)
                            .ok_or(TransactionError::ConflictLimit)?;
                        if conflicts > policy.max_conflicts.get() {
                            return Err(TransactionError::ConflictLimit);
                        }
                        yield_after_conflict().await;
                    }
                }
            },
            std::convert::identity,
        )
    }
}

impl<E: Send + 'static> Transaction<(), E> {
    /// The iterable is rebuilt for every optimistic retry. Each iteration
    /// shares this transaction's journal and publishes no writes until commit.
    ///
    /// # Type parameters
    /// - `Items`: Input collection or iterator type. Additional bound: `IntoIterator + 'static`.
    /// - `E`: Domain-error type.
    ///
    /// # Parameters
    /// - `items` (`impl Fn() -> Items + Send + Sync + 'static`): The items supplied to this operation.
    /// - `body` (`impl Fn(Items::Item) -> Transaction<(), E> + Send + Sync + 'static`): Application callback or effect executed inside the owning scope.
    ///
    /// # Returns
    /// Type: `Self`.
    pub fn for_each<Items>(
        items: impl Fn() -> Items + Send + Sync + 'static,
        body: impl Fn(Items::Item) -> Transaction<(), E> + Send + Sync + 'static,
    ) -> Self
    where
        Items: IntoIterator + 'static,
    {
        Transaction::new(move |journal| {
            for item in items() {
                match (body(item).evaluate)(journal) {
                    TxStep::Success(()) => {}
                    TxStep::Failure(error) => return TxStep::Failure(error),
                    TxStep::Retry => return TxStep::Retry,
                    TxStep::Conflict => return TxStep::Conflict,
                    TxStep::Invalid(error) => return TxStep::Invalid(error),
                }
            }
            TxStep::Success(())
        })
    }
}
