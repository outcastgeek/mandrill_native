//! Committed snapshots and subscriptions.

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
/// A domain, publication, or lifecycle failure in a transactional subscription.
pub enum TxSubscriptionError {
    /// The required cell is missing.
    MissingCell,
    /// The supplied value type does not match the required or recorded value.
    WrongValueType,
    /// Execution was cancelled.
    Cancelled,
    /// The resource has already been disposed.
    Disposed,
}

///
/// # Parameters
/// - `error` (`JournalError`): Expected or provider error preserved in the returned outcome.
///
/// # Returns
/// Type: `TxSubscriptionError`.
/// Returns `TxSubscriptionError`.
fn map_error(error: JournalError) -> TxSubscriptionError {
    match error {
        JournalError::WrongValueType => TxSubscriptionError::WrongValueType,
        JournalError::Conflict
        | JournalError::MixedDomain
        | JournalError::MissingCell
        | JournalError::VersionExhausted => TxSubscriptionError::MissingCell,
    }
}

/// A transactional reference that publishes only committed value changes.
///
/// # Type parameters
/// - `T`: Value or item type. Bound or type: `TxValue`.
pub struct TxSubscriptionRef<T: TxValue> {
    cell: TxRef<T>,
}

impl<T: TxValue> Clone for TxSubscriptionRef<T> {
    ///
    /// # Returns
    /// Type: `Self`.
    fn clone(&self) -> Self {
        Self {
            cell: self.cell.clone(),
        }
    }
}

impl<T: TxValue> TxSubscriptionRef<T> {
    /// Construct a `TxSubscriptionRef` from its required inputs and explicit policies.
    ///
    /// # Type parameters
    /// - `T`: Value or item type.
    ///
    /// # Parameters
    /// - `domain` (`&TransactionDomain`): Stable identity or provider of the operation’s transaction domain.
    /// - `initial` (`T`): Initial value used before any state transition.
    ///
    /// # Returns
    /// Type: `Result<Self, TransactionDomainError>`.
    pub fn new(domain: &TransactionDomain, initial: T) -> Result<Self, TransactionDomainError> {
        domain.reference(initial).map(|cell| Self { cell })
    }

    #[must_use]
    /// Read the current value through the declared storage or state capability.
    ///
    /// # Type parameters
    /// - `E`: Domain-error type. Bounds: `Send + 'static`.
    /// - `T`: Value or item type.
    ///
    /// # Returns
    /// Type: `Transaction<Arc<T>, E>`.
    pub fn get<E: Send + 'static>(&self) -> Transaction<Arc<T>, E> {
        self.cell.read()
    }

    /// Replace the current value through the declared state capability.
    ///
    /// # Type parameters
    /// - `E`: Domain-error type. Bounds: `Send + 'static`.
    /// - `T`: Value or item type.
    ///
    /// # Parameters
    /// - `value` (`Arc<T>`): Value read, transformed, encoded, or stored by this operation.
    ///
    /// # Returns
    /// Type: `Transaction<(), E>`.
    pub fn set<E: Send + 'static>(&self, value: Arc<T>) -> Transaction<(), E> {
        self.cell.write(value)
    }

    /// Read the latest committed value without joining an active transaction.
    ///
    /// # Type parameters
    /// - `R`: Environment type. Bounds: `Send + Sync + 'static`.
    /// - `T`: Value or item type.
    ///
    /// # Returns
    /// Type: `Effect<Arc<T>, TxSubscriptionError, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
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
    ///
    /// # Type parameters
    /// - `R`: Environment type. Bounds: `Send + Sync + 'static`.
    /// - `T`: Value or item type.
    ///
    /// # Returns
    /// Type: `Effect<(Arc<T>, TxSubscription<T>), TxSubscriptionError, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
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
    /// Return the stream associated with this value.
    ///
    /// # Type parameters
    /// - `R`: Environment type. Bounds: `Send + Sync + 'static`.
    /// - `T`: Value or item type.
    ///
    /// # Returns
    /// Type: `Stream<Arc<T>, TxSubscriptionError, R>`.
    /// Returns a scoped stream with the declared item, error, and environment types.
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

/// Typed subscription inner used by transactional tx subscription ref.
///
/// # Type parameters
/// - `T`: Value or item type. Bound or type: `TxValue`.
struct SubscriptionInner<T: TxValue> {
    cell: TxRef<T>,
    cursor: Mutex<u64>,
    disposed: AtomicBool,
    closed: Event,
}

/// A subscription that begins with an atomic version/value snapshot.
///
/// # Type parameters
/// - `T`: Value or item type. Bound or type: `TxValue`.
pub struct TxSubscription<T: TxValue> {
    inner: Arc<SubscriptionInner<T>>,
}

impl<T: TxValue> Clone for TxSubscription<T> {
    ///
    /// # Returns
    /// Type: `Self`.
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

impl<T: TxValue> TxSubscription<T> {
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
    /// Type: `Effect<Arc<T>, TxSubscriptionError, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
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
