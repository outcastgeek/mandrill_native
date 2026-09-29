use super::HasTransactionDomain;
use super::internal::journal::{JournalError, TxJournal};
use crate::Effect;
use futures_util::future::{poll_fn, select};
use std::num::NonZeroUsize;
use std::sync::Arc;
use std::task::Poll;

type TransactionStep<A, E> = dyn Fn(&mut TxJournal) -> TxStep<A, E> + Send + Sync;

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
pub struct TransactionPolicy {
    /// Explicit finite conflict retries; no hidden spin or backoff constant.
    pub max_conflicts: NonZeroUsize,
}

#[derive(Debug)]
pub enum TransactionError<E> {
    Domain(E),
    MixedDomain,
    MissingCell,
    WrongValueType,
    VersionExhausted,
    ConflictLimit,
    RetryWithoutReads,
    Cancelled,
}

impl<E> TransactionError<E> {
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
pub struct Transaction<A: Send + 'static, E: Send + 'static> {
    evaluate: Arc<TransactionStep<A, E>>,
}

impl<A: Send + 'static, E: Send + 'static> Clone for Transaction<A, E> {
    fn clone(&self) -> Self {
        Self {
            evaluate: Arc::clone(&self.evaluate),
        }
    }
}

impl<A: Send + 'static, E: Send + 'static> Transaction<A, E> {
    pub(crate) fn new(
        evaluate: impl Fn(&mut TxJournal) -> TxStep<A, E> + Send + Sync + 'static,
    ) -> Self {
        Self {
            evaluate: Arc::new(evaluate),
        }
    }

    pub fn succeed_with(value: impl Fn() -> A + Send + Sync + 'static) -> Self {
        Self::new(move |_| TxStep::Success(value()))
    }

    pub fn fail_with(error: impl Fn() -> E + Send + Sync + 'static) -> Self {
        Self::new(move |_| TxStep::Failure(error()))
    }

    #[must_use]
    pub fn retry() -> Self {
        Self::new(|_| TxStep::Retry)
    }

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
