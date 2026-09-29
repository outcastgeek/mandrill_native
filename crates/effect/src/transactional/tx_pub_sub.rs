//! Publish-subscribe delivery after transaction commit.

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
/// The policy applied when a transactional subscriber’s buffer is full.
pub enum TxPubSubOverflow {
    /// Reject the conflicting input without applying its proposed state change.
    Reject,
    /// Evict the oldest buffered value to admit the new value.
    DropOldest,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// A publication, capacity, or lifecycle failure in transactional PubSub.
pub enum TxPubSubError {
    /// The configured capacity has been reached.
    Full,
    /// No further sequence can be allocated without exceeding its range.
    SequenceExhausted,
    /// A transactional subscriber missed entries beyond retention.
    Lagged {
        /// First retained sequence available for resumption. Type: `NonZeroU64`.
        first_available: NonZeroU64,
    },
    /// The required cell is missing.
    MissingCell,
    /// Execution was cancelled.
    Cancelled,
    /// The resource has already been disposed.
    Disposed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// The committed subscriber-delivery accounting for one publication.
pub struct TxPublishReceipt {
    /// The ordered sequence position. Type: `NonZeroU64`.
    pub sequence: NonZeroU64,
    /// Whether overflow evicted the oldest buffered publication. Type: `bool`.
    pub dropped_oldest: bool,
}

/// Typed pub sub state used by transactional tx pub sub.
///
/// # Type parameters
/// - `T`: Value or item type. Bound or type: `TxValue`.
struct PubSubState<T: TxValue> {
    last_sequence: u64,
    retained: VecDeque<(NonZeroU64, Arc<T>)>,
}

impl<T: TxValue> Clone for PubSubState<T> {
    ///
    /// # Returns
    /// Type: `Self`.
    fn clone(&self) -> Self {
        Self {
            last_sequence: self.last_sequence,
            retained: self.retained.clone(),
        }
    }
}

impl<T: TxValue> TxValue for PubSubState<T> {}

/// A publication channel whose messages become visible only after commit.
///
/// # Type parameters
/// - `T`: Value or item type. Bound or type: `TxValue`.
pub struct TxPubSub<T: TxValue> {
    cell: TxRef<PubSubState<T>>,
    capacity: NonZeroUsize,
    overflow: TxPubSubOverflow,
}

impl<T: TxValue> Clone for TxPubSub<T> {
    ///
    /// # Returns
    /// Type: `Self`.
    fn clone(&self) -> Self {
        Self {
            cell: self.cell.clone(),
            capacity: self.capacity,
            overflow: self.overflow,
        }
    }
}

impl<T: TxValue> TxPubSub<T> {
    /// Construct a `TxPubSub` from its required inputs and explicit policies.
    ///
    /// # Type parameters
    /// - `T`: Value or item type. Bound: `TxValue`.
    ///
    /// # Parameters
    /// - `domain` (`&TransactionDomain`): Stable identity or provider of the operation’s transaction domain.
    /// - `capacity` (`NonZeroUsize`): The caller-supplied capacity bound.
    /// - `overflow` (`TxPubSubOverflow`): Behavior when the configured buffer capacity is full.
    ///
    /// # Returns
    /// Type: `Result<Self, TransactionDomainError>`.
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

    /// Add a publication to the journal so subscribers see it only after commit.
    ///
    /// # Type parameters
    /// - `T`: Value or item type.
    ///
    /// # Parameters
    /// - `value` (`Arc<T>`): Value read, transformed, encoded, or stored by this operation.
    ///
    /// # Returns
    /// Type: `Transaction<TxPublishReceipt, TxPubSubError>`.
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

    /// Register an observer for future committed changes.
    ///
    /// # Type parameters
    /// - `R`: Environment type. Bounds: `Send + Sync + 'static`.
    /// - `T`: Value or item type.
    ///
    /// # Returns
    /// Type: `Effect<TxPubSubSubscription<T>, TxPubSubError, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
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
    /// Register a subscriber beginning after the supplied committed position.
    ///
    /// # Type parameters
    /// - `T`: Value or item type.
    ///
    /// # Parameters
    /// - `after` (`u64`): Cursor or instant after which results are selected.
    ///
    /// # Returns
    /// Type: `TxPubSubSubscription<T>`.
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
    /// Return the stream associated with this value.
    ///
    /// # Type parameters
    /// - `R`: Environment type. Bounds: `Send + Sync + 'static`.
    /// - `T`: Value or item type.
    ///
    /// # Returns
    /// Type: `Stream<(NonZeroU64, Arc<T>), TxPubSubError, R>`.
    /// Returns a scoped stream with the declared item, error, and environment types.
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

/// Typed pub sub subscription inner used by transactional tx pub sub.
///
/// # Type parameters
/// - `T`: Value or item type. Bound or type: `TxValue`.
struct PubSubSubscriptionInner<T: TxValue> {
    cell: TxRef<PubSubState<T>>,
    cursor: Mutex<u64>,
    disposed: AtomicBool,
    closed: Event,
}

/// A subscriber that observes messages committed by its transaction domain.
///
/// # Type parameters
/// - `T`: Value or item type. Bound or type: `TxValue`.
pub struct TxPubSubSubscription<T: TxValue> {
    inner: Arc<PubSubSubscriptionInner<T>>,
}

impl<T: TxValue> Clone for TxPubSubSubscription<T> {
    ///
    /// # Returns
    /// Type: `Self`.
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

impl<T: TxValue> TxPubSubSubscription<T> {
    #[must_use]
    /// Release owned native state and await its disposal where required.
    ///
    /// # Type parameters
    /// - `T`: Value or item type. Bound: `TxValue`.
    ///
    /// # Returns
    /// Type: `bool`.
    /// Returns whether the requested condition holds.
    pub fn dispose(&self) -> bool {
        let first = !self.inner.disposed.swap(true, Ordering::AcqRel);
        if first {
            self.inner.closed.notify(usize::MAX);
        }
        first
    }

    /// Advance to the next value or recorded step when one exists.
    ///
    /// # Type parameters
    /// - `R`: Environment type. Bounds: `Send + Sync + 'static`.
    /// - `T`: Value or item type.
    ///
    /// # Returns
    /// Type: `Effect<(NonZeroU64, Arc<T>), TxPubSubError, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
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
