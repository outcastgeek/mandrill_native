use super::internal::journal::JournalError;
use super::{Transaction, TransactionDomain, TransactionDomainError, TxRef, TxValue};
use crate::{Chunk, Effect, Release, Stream};
use event_listener::Event;
use futures_util::future::select;
use std::collections::VecDeque;
use std::num::{NonZeroU64, NonZeroUsize};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TxPubSubOverflow {
    Reject,
    DropOldest,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TxPubSubError {
    Full,
    SequenceExhausted,
    Lagged { first_available: NonZeroU64 },
    MissingCell,
    Cancelled,
    Disposed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TxPublishReceipt {
    pub sequence: NonZeroU64,
    pub dropped_oldest: bool,
}

struct PubSubState<T: TxValue> {
    last_sequence: u64,
    retained: VecDeque<(NonZeroU64, Arc<T>)>,
}

impl<T: TxValue> Clone for PubSubState<T> {
    fn clone(&self) -> Self {
        Self {
            last_sequence: self.last_sequence,
            retained: self.retained.clone(),
        }
    }
}

impl<T: TxValue> TxValue for PubSubState<T> {}

pub struct TxPubSub<T: TxValue> {
    cell: TxRef<PubSubState<T>>,
    capacity: NonZeroUsize,
    overflow: TxPubSubOverflow,
}

impl<T: TxValue> Clone for TxPubSub<T> {
    fn clone(&self) -> Self {
        Self {
            cell: self.cell.clone(),
            capacity: self.capacity,
            overflow: self.overflow,
        }
    }
}

impl<T: TxValue> TxPubSub<T> {
    pub fn new(
        domain: &TransactionDomain,
        capacity: NonZeroUsize,
        overflow: TxPubSubOverflow,
    ) -> Result<Self, TransactionDomainError> {
        let cell = domain.reference(PubSubState {
            last_sequence: 0,
            retained: VecDeque::new(),
        })?;
        Ok(Self {
            cell,
            capacity,
            overflow,
        })
    }

    pub fn publish(&self, value: Arc<T>) -> Transaction<TxPublishReceipt, TxPubSubError> {
        let capacity = self.capacity;
        let overflow = self.overflow;
        self.cell.update(move |current| {
            let sequence = current
                .last_sequence
                .checked_add(1)
                .and_then(NonZeroU64::new)
                .ok_or(TxPubSubError::SequenceExhausted)?;
            let mut next = current.clone();
            let dropped_oldest = if next.retained.len() >= capacity.get() {
                match overflow {
                    TxPubSubOverflow::Reject => return Err(TxPubSubError::Full),
                    TxPubSubOverflow::DropOldest => {
                        next.retained.pop_front();
                        true
                    }
                }
            } else {
                false
            };
            next.last_sequence = sequence.get();
            next.retained.push_back((sequence, Arc::clone(&value)));
            Ok((
                next,
                TxPublishReceipt {
                    sequence,
                    dropped_oldest,
                },
            ))
        })
    }

    pub fn subscribe<R: Send + Sync + 'static>(
        &self,
    ) -> Effect<TxPubSubSubscription<T>, TxPubSubError, R> {
        let pubsub = self.clone();
        Effect::try_sync(
            move || {
                let (_, state) = pubsub
                    .cell
                    .inspect_committed()
                    .map_err(|_| TxPubSubError::MissingCell)?;
                Ok(pubsub.subscription_after(state.last_sequence))
            },
            std::convert::identity,
        )
    }

    #[must_use]
    pub fn subscription_after(&self, after: u64) -> TxPubSubSubscription<T> {
        TxPubSubSubscription {
            inner: Arc::new(PubSubSubscriptionInner {
                cell: self.cell.clone(),
                cursor: Mutex::new(after),
                disposed: AtomicBool::new(false),
                closed: Event::new(),
            }),
        }
    }

    #[must_use]
    pub fn stream<R: Send + Sync + 'static>(
        &self,
    ) -> Stream<(NonZeroU64, Arc<T>), TxPubSubError, R> {
        Stream::from_pull(
            self.subscribe::<R>(),
            |resource| {
                resource
                    .access(|subscription| subscription.clone())
                    .flat_map(|subscription| {
                        subscription.next().map(|value| Chunk::Values(vec![value]))
                    })
            },
            |subscription| {
                Release::sync(move || {
                    let _ = subscription.dispose();
                })
            },
        )
    }
}

struct PubSubSubscriptionInner<T: TxValue> {
    cell: TxRef<PubSubState<T>>,
    cursor: Mutex<u64>,
    disposed: AtomicBool,
    closed: Event,
}

pub struct TxPubSubSubscription<T: TxValue> {
    inner: Arc<PubSubSubscriptionInner<T>>,
}

impl<T: TxValue> Clone for TxPubSubSubscription<T> {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

impl<T: TxValue> TxPubSubSubscription<T> {
    #[must_use]
    pub fn dispose(&self) -> bool {
        let first = !self.inner.disposed.swap(true, Ordering::AcqRel);
        if first {
            self.inner.closed.notify(usize::MAX);
        }
        first
    }

    pub fn next<R: Send + Sync + 'static>(&self) -> Effect<(NonZeroU64, Arc<T>), TxPubSubError, R> {
        let subscription = self.clone();
        Effect::try_future(
            move |cancellation| {
                let subscription = subscription.clone();
                async move {
                    loop {
                        if subscription.inner.disposed.load(Ordering::Acquire) {
                            return Err(TxPubSubError::Disposed);
                        }
                        let changed = subscription.inner.cell.changed().listen();
                        let closed = subscription.inner.closed.listen();
                        let (_, state) = subscription
                            .inner
                            .cell
                            .inspect_committed()
                            .map_err(|_error: JournalError| TxPubSubError::MissingCell)?;
                        {
                            let mut cursor = subscription
                                .inner
                                .cursor
                                .lock()
                                .unwrap_or_else(std::sync::PoisonError::into_inner);
                            if let Some((first, _)) = state.retained.front()
                                && (*cursor).saturating_add(1) < first.get()
                            {
                                *cursor = first.get() - 1;
                                return Err(TxPubSubError::Lagged {
                                    first_available: *first,
                                });
                            }
                            if let Some((sequence, value)) = state
                                .retained
                                .iter()
                                .find(|(sequence, _)| sequence.get() > *cursor)
                            {
                                *cursor = sequence.get();
                                return Ok((*sequence, Arc::clone(value)));
                            }
                        }
                        if subscription.inner.disposed.load(Ordering::Acquire) {
                            return Err(TxPubSubError::Disposed);
                        }
                        let _ = select(
                            Box::pin(changed),
                            Box::pin(select(Box::pin(closed), Box::pin(cancellation.cancelled()))),
                        )
                        .await;
                        if cancellation.is_requested() {
                            return Err(TxPubSubError::Cancelled);
                        }
                    }
                }
            },
            std::convert::identity,
        )
    }
}
