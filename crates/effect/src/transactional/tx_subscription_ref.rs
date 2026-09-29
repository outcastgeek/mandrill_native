use super::internal::journal::JournalError;
use super::{Transaction, TransactionDomain, TransactionDomainError, TxRef, TxValue};
use crate::{Chunk, Effect, Release, Stream};
use event_listener::Event;
use futures_util::future::select;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TxSubscriptionError {
    MissingCell,
    WrongValueType,
    Cancelled,
    Disposed,
}

fn map_error(error: JournalError) -> TxSubscriptionError {
    match error {
        JournalError::WrongValueType => TxSubscriptionError::WrongValueType,
        JournalError::Conflict
        | JournalError::MixedDomain
        | JournalError::MissingCell
        | JournalError::VersionExhausted => TxSubscriptionError::MissingCell,
    }
}

pub struct TxSubscriptionRef<T: TxValue> {
    cell: TxRef<T>,
}

impl<T: TxValue> Clone for TxSubscriptionRef<T> {
    fn clone(&self) -> Self {
        Self {
            cell: self.cell.clone(),
        }
    }
}

impl<T: TxValue> TxSubscriptionRef<T> {
    pub fn new(domain: &TransactionDomain, initial: T) -> Result<Self, TransactionDomainError> {
        domain.reference(initial).map(|cell| Self { cell })
    }

    #[must_use]
    pub fn get<E: Send + 'static>(&self) -> Transaction<Arc<T>, E> {
        self.cell.read()
    }

    pub fn set<E: Send + 'static>(&self, value: Arc<T>) -> Transaction<(), E> {
        self.cell.write(value)
    }

    /// Read the latest committed value without joining an active transaction.
    pub fn read_committed<R: Send + Sync + 'static>(
        &self,
    ) -> Effect<Arc<T>, TxSubscriptionError, R> {
        let cell = self.cell.clone();
        Effect::try_sync(
            move || {
                cell.inspect_committed()
                    .map(|(_, value)| value)
                    .map_err(map_error)
            },
            std::convert::identity,
        )
    }

    /// Capture initial value and version under the same commit gate. `next`
    /// registers its waiter before checking that version again, so no commit
    /// between subscription and first wait is lost.
    pub fn subscribe<R: Send + Sync + 'static>(
        &self,
    ) -> Effect<(Arc<T>, TxSubscription<T>), TxSubscriptionError, R> {
        let cell = self.cell.clone();
        Effect::try_sync(
            move || {
                let (version, value) = cell.inspect_committed().map_err(map_error)?;
                Ok((
                    value,
                    TxSubscription {
                        inner: Arc::new(SubscriptionInner {
                            cell: cell.clone(),
                            cursor: Mutex::new(version),
                            disposed: AtomicBool::new(false),
                            closed: Event::new(),
                        }),
                    },
                ))
            },
            std::convert::identity,
        )
    }

    #[must_use]
    pub fn stream<R: Send + Sync + 'static>(&self) -> Stream<Arc<T>, TxSubscriptionError, R> {
        let acquire = self
            .subscribe::<R>()
            .map(|(initial, subscription)| (Mutex::new(Some(initial)), subscription));
        Stream::from_pull(
            acquire,
            |resource| {
                resource
                    .access(|(initial, subscription)| {
                        let first = initial
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner)
                            .take();
                        (first, subscription.clone())
                    })
                    .flat_map(|(first, subscription)| match first {
                        Some(value) => Effect::sync_once(move || Chunk::Values(vec![value])),
                        None => subscription.next().map(|value| Chunk::Values(vec![value])),
                    })
            },
            |(_, subscription)| {
                Release::sync(move || {
                    let _ = subscription.dispose();
                })
            },
        )
    }
}

struct SubscriptionInner<T: TxValue> {
    cell: TxRef<T>,
    cursor: Mutex<u64>,
    disposed: AtomicBool,
    closed: Event,
}

pub struct TxSubscription<T: TxValue> {
    inner: Arc<SubscriptionInner<T>>,
}

impl<T: TxValue> Clone for TxSubscription<T> {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

impl<T: TxValue> TxSubscription<T> {
    #[must_use]
    pub fn dispose(&self) -> bool {
        let first = !self.inner.disposed.swap(true, Ordering::AcqRel);
        if first {
            self.inner.closed.notify(usize::MAX);
        }
        first
    }

    pub fn next<R: Send + Sync + 'static>(&self) -> Effect<Arc<T>, TxSubscriptionError, R> {
        let subscription = self.clone();
        Effect::try_future(
            move |cancellation| {
                let subscription = subscription.clone();
                async move {
                    loop {
                        if subscription.inner.disposed.load(Ordering::Acquire) {
                            return Err(TxSubscriptionError::Disposed);
                        }
                        let listener = subscription.inner.cell.changed().listen();
                        let closed = subscription.inner.closed.listen();
                        let (version, value) = subscription
                            .inner
                            .cell
                            .inspect_committed()
                            .map_err(map_error)?;
                        {
                            let mut cursor = subscription
                                .inner
                                .cursor
                                .lock()
                                .unwrap_or_else(std::sync::PoisonError::into_inner);
                            if version > *cursor {
                                *cursor = version;
                                return Ok(value);
                            }
                        }
                        if subscription.inner.disposed.load(Ordering::Acquire) {
                            return Err(TxSubscriptionError::Disposed);
                        }
                        let _ = select(
                            Box::pin(listener),
                            Box::pin(select(Box::pin(closed), Box::pin(cancellation.cancelled()))),
                        )
                        .await;
                        if cancellation.is_requested() {
                            return Err(TxSubscriptionError::Cancelled);
                        }
                    }
                }
            },
            std::convert::identity,
        )
    }
}
