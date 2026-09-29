//! Asynchronous queue transfer and shutdown behavior.

use crate::internal::notification::notify_all;
use crate::internal::runtime::{self, Step};
use crate::internal::waiting::{Position, Waiting};
use crate::{
    Cancellation, CleanupFailure, Defect, Effect, Exit, Finalization, LibraryDefect, PrimaryOutcome,
};
use event_listener::Event;
use futures_util::future::select;
use std::collections::VecDeque;
use std::num::NonZeroUsize;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Arc, Mutex};

/// Explicit admission policy, with no default capacity or hidden worker pool.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QueueCapacity {
    /// No explicit capacity bound is imposed.
    Unbounded,
    /// The supplied positive capacity bound is enforced. Contains `NonZeroUsize`.
    Bounded(NonZeroUsize),
    /// Producer and consumer meet directly without a buffered slot.
    Rendezvous,
}

impl QueueCapacity {
    /// Adapt the source's optional nonnegative capacity. Zero means rendezvous;
    /// Rust's unsigned input excludes negative capacities before execution.
    #[must_use]
    /// # Parameters
    /// - `limit` (`Option<usize>`): The caller-supplied limit.
    ///
    /// # Returns
    /// Type: `Self`.
    pub const fn from_limit(limit: Option<usize>) -> Self {
        match limit {
            None => Self::Unbounded,
            Some(limit) => match NonZeroUsize::new(limit) {
                Some(limit) => Self::Bounded(limit),
                None => Self::Rendezvous,
            },
        }
    }

    ///
    /// # Parameters
    /// - `buffered` (`usize`): The buffered supplied to this operation.
    ///
    /// # Returns
    /// Type: `bool`.
    /// Returns whether the requested condition holds.
    const fn permits_buffer(self, buffered: usize) -> bool {
        match self {
            Self::Unbounded => true,
            Self::Bounded(limit) => buffered < limit.get(),
            Self::Rendezvous => false,
        }
    }
}

/// Expected queue failure, as in the reference implementation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QueueError {
    /// The provider is shutting down and cannot accept this work.
    Shutdown,
}

impl std::fmt::Display for QueueError {
    ///
    /// # Parameters
    /// - `formatter` (`&mut std::fmt::Formatter<'_>`): Destination formatter used by the Rust formatting trait.
    ///
    /// # Returns
    /// Type: `std::fmt::Result`.
    /// Returns `std::fmt::Result`.
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(self, formatter)
    }
}

impl std::error::Error for QueueError {}

/// Drain retains only values already committed to the buffer. Discard removes
/// those values too. Both reject every pending offer/take and future offers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QueueShutdown {
    /// Stop admission and wait for already accepted work.
    Drain,
    /// Drop the selected value under the configured overflow policy.
    Discard,
}

/// Typed waiter used by queue.
///
/// # Type parameters
/// - `B`: Transformed value type.
struct Waiter<B> {
    outcome: Mutex<Option<Exit<B, QueueError>>>,
    changed: Event,
}

impl<B> Waiter<B> {
    ///
    /// # Returns
    /// Type: `Arc<Self>`.
    /// Returns `Arc<Self>`.
    fn new() -> Arc<Self> {
        Arc::new(Self {
            outcome: Mutex::new(None),
            changed: Event::new(),
        })
    }

    ///
    /// # Type parameters
    /// - `B`: Transformed output type.
    ///
    /// # Parameters
    /// - `outcome` (`Exit<B, QueueError>`): Complete terminal outcome, including expected errors and cleanup failures.
    ///
    /// # Returns
    /// Type: `()`.
    fn complete(&self, outcome: Exit<B, QueueError>) {
        *self
            .outcome
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(outcome);
    }

    ///
    /// # Type parameters
    /// - `B`: Transformed output type.
    ///
    /// # Returns
    /// Type: `Option<Exit<B, QueueError>>`.
    /// Returns the value when present, or `None`.
    fn take(&self) -> Option<Exit<B, QueueError>> {
        self.outcome
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
    }
}

/// Typed offer used by queue.
///
/// # Type parameters
/// - `A`: Application value type.
struct Offer<A> {
    value: A,
    waiter: Arc<Waiter<()>>,
    cancellation: Cancellation,
}

/// Typed taker used by queue.
///
/// # Type parameters
/// - `A`: Application value type.
struct Taker<A> {
    waiter: Arc<Waiter<A>>,
    cancellation: Cancellation,
}

/// Typed state used by queue.
///
/// # Type parameters
/// - `A`: Application value type.
struct State<A> {
    capacity: QueueCapacity,
    buffer: VecDeque<A>,
    offers: Waiting<Offer<A>>,
    takers: Waiting<Taker<A>>,
    closed: bool,
}

/// Executor-independent FIFO queue with movable, Send-but-not-Sync payloads.
/// Operations commit under one admission gate. Waiting nodes have targeted
/// completion events; cancellation/abandonment removes only uncommitted nodes.
/// Transfers already committed to a buffer/taker are never rolled back.
///
/// # Type parameters
/// - `A`: Application value type. Bound or type: `Send + 'static`.
pub struct AsyncQueue<A: Send + 'static> {
    state: Arc<Mutex<State<A>>>,
}

impl<A: Send + 'static> Clone for AsyncQueue<A> {
    ///
    /// # Returns
    /// Type: `Self`.
    fn clone(&self) -> Self {
        Self {
            state: Arc::clone(&self.state),
        }
    }
}

/// Variants of notify used by queue.
///
/// # Type parameters
/// - `A`: Application value type.
enum Notify<A> {
    Offer(Arc<Waiter<()>>),
    Take(Arc<Waiter<A>>),
}

/// Typed changes used by queue.
///
/// # Type parameters
/// - `A`: Application value type.
struct Changes<A> {
    notify: Vec<Notify<A>>,
    discard: Vec<A>,
}

impl<A> Default for Changes<A> {
    ///
    /// # Returns
    /// Type: `Self`.
    fn default() -> Self {
        Self {
            notify: Vec::new(),
            discard: Vec::new(),
        }
    }
}

impl<A> Changes<A> {
    ///
    /// # Parameters
    /// - `waiter` (`Arc<Waiter<()>>`): The waiter supplied to this operation.
    /// - `outcome` (`Exit<(), QueueError>`): Complete terminal outcome, including expected errors and cleanup failures.
    ///
    /// # Returns
    /// Type: `()`.
    fn offered(&mut self, waiter: Arc<Waiter<()>>, outcome: Exit<(), QueueError>) {
        waiter.complete(outcome);
        self.notify.push(Notify::Offer(waiter));
    }

    ///
    /// # Type parameters
    /// - `A`: Success or application-value type.
    ///
    /// # Parameters
    /// - `waiter` (`Arc<Waiter<A>>`): The waiter supplied to this operation.
    /// - `outcome` (`Exit<A, QueueError>`): Complete terminal outcome, including expected errors and cleanup failures.
    ///
    /// # Returns
    /// Type: `()`.
    fn taken(&mut self, waiter: Arc<Waiter<A>>, outcome: Exit<A, QueueError>) {
        waiter.complete(outcome);
        self.notify.push(Notify::Take(waiter));
    }

    ///
    /// # Type parameters
    /// - `B`: Transformed success-value type.
    ///
    /// # Parameters
    /// - `outcome` (`Exit<B, QueueError>`): Complete terminal outcome, including expected errors and cleanup failures.
    ///
    /// # Returns
    /// Type: `Exit<B, QueueError>`.
    /// Returns `Exit<B, QueueError>`.
    fn finish<B>(self, outcome: Exit<B, QueueError>) -> Exit<B, QueueError> {
        let mut failures = Vec::new();
        // Complete every node before invoking callbacks. Broadcast and all
        // discarded payload destruction occur outside the admission gate.
        for notify in self.notify {
            let notified = catch_unwind(AssertUnwindSafe(|| match notify {
                Notify::Offer(waiter) => notify_all(&waiter.changed),
                Notify::Take(waiter) => notify_all(&waiter.changed),
            }));
            if let Err(panic) = notified {
                failures.push(CleanupFailure::ReleaseDefect(Defect::from_panic(panic)));
            }
        }
        for value in self.discard {
            if let Err(panic) = catch_unwind(AssertUnwindSafe(|| drop(value))) {
                failures.push(CleanupFailure::ReleaseDefect(Defect::from_panic(panic)));
            }
        }
        let mut failures = failures.into_iter();
        let Some(first) = failures.next() else {
            return outcome;
        };
        let mut rest: Vec<_> = failures.collect();
        let primary = match outcome {
            Exit::Success(value) => {
                if let Err(panic) = catch_unwind(AssertUnwindSafe(|| drop(value))) {
                    rest.push(CleanupFailure::ReleaseDefect(Defect::from_panic(panic)));
                }
                PrimaryOutcome::Succeeded
            }
            Exit::Fail(error) => PrimaryOutcome::Failed(error),
            Exit::Die(defect) => PrimaryOutcome::Defected(defect),
            Exit::Interrupted => PrimaryOutcome::WasInterrupted,
            Exit::FinalizationFailed(mut details) => {
                details.rest.push(first);
                details.rest.extend(rest);
                return Exit::FinalizationFailed(details);
            }
        };
        Exit::FinalizationFailed(Finalization {
            primary,
            first,
            rest,
        })
    }
}

/// Variants of admission used by queue.
///
/// # Type parameters
/// - `B`: Transformed value type.
enum Admission<B> {
    Ready(Exit<B, QueueError>),
    Pending(Position, Arc<Waiter<B>>),
}

/// Removes a pending queue offer or take when its registration is withdrawn.
trait Registration<A: Send + 'static>: Send + 'static {
    /// Type alias for value in queue.
    type Value: Send + 'static;

    ///
    /// # Returns
    /// Type: `&Waiter<Self::Value>`.
    fn waiter(&self) -> &Waiter<Self::Value>;
    ///
    /// # Type parameters
    /// - `A`: Success or application-value type.
    ///
    /// # Parameters
    /// - `queue` (`&AsyncQueue<A>`): Queue definition or store that owns the selected messages.
    ///
    /// # Returns
    /// Type: `Changes<A>`.
    /// Returns `Changes<A>`.
    fn withdraw(&self, queue: &AsyncQueue<A>) -> Changes<A>;
}

/// Typed offered used by queue.
struct Offered(Position, Arc<Waiter<()>>);
/// Typed taken used by queue.
///
/// # Type parameters
/// - `A`: Application value type.
struct Taken<A>(Position, Arc<Waiter<A>>);

impl<A: Send + 'static> Registration<A> for Offered {
    /// Type alias for value in queue.
    type Value = ();
    ///
    /// # Returns
    /// Type: `&Waiter<()>`.
    fn waiter(&self) -> &Waiter<()> {
        &self.1
    }
    ///
    /// # Type parameters
    /// - `A`: Success or application-value type.
    ///
    /// # Parameters
    /// - `queue` (`&AsyncQueue<A>`): Queue definition or store that owns the selected messages.
    ///
    /// # Returns
    /// Type: `Changes<A>`.
    /// Returns `Changes<A>`.
    fn withdraw(&self, queue: &AsyncQueue<A>) -> Changes<A> {
        queue.withdraw_offer(self.0, &self.1)
    }
}

impl<A: Send + 'static> Registration<A> for Taken<A> {
    /// Type alias for value in queue.
    type Value = A;
    ///
    /// # Type parameters
    /// - `A`: Success or application-value type.
    ///
    /// # Returns
    /// Type: `&Waiter<A>`.
    fn waiter(&self) -> &Waiter<A> {
        &self.1
    }
    ///
    /// # Type parameters
    /// - `A`: Success or application-value type.
    ///
    /// # Parameters
    /// - `queue` (`&AsyncQueue<A>`): Queue definition or store that owns the selected messages.
    ///
    /// # Returns
    /// Type: `Changes<A>`.
    /// Returns `Changes<A>`.
    fn withdraw(&self, queue: &AsyncQueue<A>) -> Changes<A> {
        queue.withdraw_take(self.0, &self.1)
    }
}

/// Typed registered used by queue.
///
/// # Type parameters
/// - `A`: Application value type. Bound or type: `Send + 'static`.
/// - `K`: Key type used for identity or lookup. Bound or type: `Registration<A>`.
struct Registered<A: Send + 'static, K: Registration<A>> {
    queue: AsyncQueue<A>,
    registration: K,
}

impl<A: Send + 'static, K: Registration<A>> Drop for Registered<A, K> {
    ///
    /// # Returns
    /// Type: `()`.
    fn drop(&mut self) {
        let _ = self
            .registration
            .withdraw(&self.queue)
            .finish(Exit::Success(()));
        // If a taker was abandoned after commit, it owns the transferred value;
        // discard it outside all gates, without undoing that transfer. Drop has
        // no caller to receive an awaited/fallible cleanup result.
        if let Some(outcome) = self.registration.waiter().take() {
            let _ = catch_unwind(AssertUnwindSafe(|| drop(outcome)));
        }
    }
}

impl<A: Send + 'static> AsyncQueue<A> {
    #[must_use]
    /// Construct a `AsyncQueue` from its required inputs and explicit policies.
    ///
    /// # Type parameters
    /// - `A`: Application value type. Bound: `Send + 'static`.
    ///
    /// # Parameters
    /// - `capacity` (`QueueCapacity`): The caller-supplied capacity bound.
    ///
    /// # Returns
    /// Type: `Self`.
    pub fn new(capacity: QueueCapacity) -> Self {
        Self {
            state: Arc::new(Mutex::new(State {
                capacity,
                buffer: VecDeque::new(),
                offers: Waiting::default(),
                takers: Waiting::default(),
                closed: false,
            })),
        }
    }

    /// Create a `AsyncQueue` through the declared Effect and environment capabilities.
    ///
    /// # Type parameters
    /// - `A`: Application value type. Bound: `Send + 'static`.
    /// - `R`: Environment type. Bounds: `Send + Sync + 'static`.
    ///
    /// # Parameters
    /// - `capacity` (`QueueCapacity`): The caller-supplied capacity bound.
    ///
    /// # Returns
    /// Type: `Effect<Self, QueueError, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    pub fn make<R: Send + Sync + 'static>(capacity: QueueCapacity) -> Effect<Self, QueueError, R> {
        Effect::sync(move || Self::new(capacity))
    }

    /// Move one owned candidate on the first started execution. Reuse reports
    /// QueueOfferConsumed; `offer_with` produces fresh candidates without Clone.
    /// A pre-cancelled execution leaves this candidate available.
    ///
    /// # Type parameters
    /// - `R`: Environment type. Bounds: `Send + Sync + 'static`.
    /// - `A`: Success or application-value type.
    ///
    /// # Parameters
    /// - `value` (`A`): Value read, transformed, encoded, or stored by this operation.
    ///
    /// # Returns
    /// Type: `Effect<(), QueueError, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    pub fn offer<R: Send + Sync + 'static>(&self, value: A) -> Effect<(), QueueError, R> {
        let value = Mutex::new(Some(value));
        self.offer_candidate(move || {
            value
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take()
                .ok_or_else(|| Defect::from(LibraryDefect::QueueOfferConsumed))
        })
    }

    /// Offer a value and use the caller’s callback for the queue’s completion result.
    ///
    /// # Type parameters
    /// - `R`: Environment type. Bounds: `Send + Sync + 'static`.
    /// - `A`: Success or application-value type.
    ///
    /// # Parameters
    /// - `factory` (`impl Fn() -> A + Send + Sync + 'static`): Factory invoked to create fresh data for an execution.
    ///
    /// # Returns
    /// Type: `Effect<(), QueueError, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    pub fn offer_with<R: Send + Sync + 'static>(
        &self,
        factory: impl Fn() -> A + Send + Sync + 'static,
    ) -> Effect<(), QueueError, R> {
        self.offer_candidate(move || Ok(factory()))
    }

    /// Explicit stored-payload copying for consumers who intend that ownership.
    ///
    /// # Type parameters
    /// - `R`: Environment type. Bounds: `Send + Sync + 'static`.
    /// - `A`: Success or application-value type.
    ///
    /// # Parameters
    /// - `value` (`A`): Value read, transformed, encoded, or stored by this operation.
    ///
    /// # Returns
    /// Type: `Effect<(), QueueError, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    pub fn offer_cloned<R: Send + Sync + 'static>(&self, value: A) -> Effect<(), QueueError, R>
    where
        A: Clone + Sync,
    {
        self.offer_with(move || value.clone())
    }

    ///
    /// # Type parameters
    /// - `R`: Bounds: `Send + Sync + 'static`.
    /// - `A`: Success or application-value type.
    ///
    /// # Parameters
    /// - `factory` (`impl Fn() -> Result<A, Defect> + Send + Sync + 'static`): Factory invoked to create fresh data for an execution.
    ///
    /// # Returns
    /// Type: `Effect<(), QueueError, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    fn offer_candidate<R: Send + Sync + 'static>(
        &self,
        factory: impl Fn() -> Result<A, Defect> + Send + Sync + 'static,
    ) -> Effect<(), QueueError, R> {
        let queue = self.clone();
        Effect::new(move |_, context, finish| {
            let value = match catch_unwind(AssertUnwindSafe(&factory)) {
                Ok(Ok(value)) => value,
                Ok(Err(defect)) => return runtime::finish(finish, Exit::Die(defect)),
                Err(panic) => return runtime::finish(finish, Exit::Die(Defect::from_panic(panic))),
            };
            let (admission, changes) = queue.admit_offer(value, &context.cancellation);
            match admission {
                Admission::Ready(outcome) => runtime::finish(finish, changes.finish(outcome)),
                Admission::Pending(position, waiter) => {
                    let registered = Registered {
                        queue: queue.clone(),
                        registration: Offered(position, waiter),
                    };
                    if let Err(failure) = changes.finish(Exit::Success(())).into_failure() {
                        let outcome = registered
                            .registration
                            .withdraw(&registered.queue)
                            .finish(failure);
                        return runtime::finish(finish, outcome);
                    }
                    Step::Suspended(Box::pin(async move {
                        let outcome = registered.wait(context.cancellation).await;
                        runtime::finish(finish, outcome)
                    }))
                }
            }
        })
    }

    /// Transfer the owned value out when the current state permits it.
    ///
    /// # Type parameters
    /// - `R`: Environment type. Bounds: `Send + Sync + 'static`.
    /// - `A`: Success or application-value type.
    ///
    /// # Returns
    /// Type: `Effect<A, QueueError, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    pub fn take<R: Send + Sync + 'static>(&self) -> Effect<A, QueueError, R> {
        let queue = self.clone();
        Effect::new(move |_, context, finish| {
            let (admission, changes) = queue.admit_take(&context.cancellation);
            match admission {
                Admission::Ready(outcome) => runtime::finish(finish, changes.finish(outcome)),
                Admission::Pending(position, waiter) => {
                    let registered = Registered {
                        queue: queue.clone(),
                        registration: Taken(position, waiter),
                    };
                    if let Err(failure) = changes.finish(Exit::Success(())).into_failure() {
                        let outcome = registered
                            .registration
                            .withdraw(&registered.queue)
                            .finish(failure);
                        return runtime::finish(finish, outcome);
                    }
                    Step::Suspended(Box::pin(async move {
                        let outcome = registered.wait(context.cancellation).await;
                        runtime::finish(finish, outcome)
                    }))
                }
            }
        })
    }

    /// Return the number of values currently buffered by the queue.
    ///
    /// # Type parameters
    /// - `A`: Application value type. Bound: `Send + 'static`.
    /// - `R`: Environment type. Bounds: `Send + Sync + 'static`.
    ///
    /// # Returns
    /// Type: `Effect<usize, QueueError, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    pub fn size<R: Send + Sync + 'static>(&self) -> Effect<usize, QueueError, R> {
        let queue = self.clone();
        Effect::sync(move || {
            queue
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .buffer
                .len()
        })
    }

    /// Stop accepting new work and describe the settlement of owned work.
    ///
    /// # Type parameters
    /// - `A`: Application value type. Bound: `Send + 'static`.
    /// - `R`: Environment type. Bounds: `Send + Sync + 'static`.
    ///
    /// # Parameters
    /// - `mode` (`QueueShutdown`): Selected behavior for this operation.
    ///
    /// # Returns
    /// Type: `Effect<(), QueueError, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    pub fn shutdown<R: Send + Sync + 'static>(
        &self,
        mode: QueueShutdown,
    ) -> Effect<(), QueueError, R> {
        let queue = self.clone();
        Effect::ready(move |_, _| {
            let (offers, takers, changes) = {
                let mut state = queue
                    .state
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                state.closed = true;
                let mut changes = Changes::default();
                match mode {
                    QueueShutdown::Drain => {}
                    QueueShutdown::Discard => {
                        changes.discard.extend(std::mem::take(&mut state.buffer))
                    }
                }
                while let Some(offer) = state.offers.pop_front() {
                    let outcome = if offer.cancellation.is_requested() {
                        Exit::Interrupted
                    } else {
                        Exit::Fail(QueueError::Shutdown)
                    };
                    changes.offered(offer.waiter, outcome);
                    changes.discard.push(offer.value);
                }
                while let Some(taker) = state.takers.pop_front() {
                    let outcome = if taker.cancellation.is_requested() {
                        Exit::Interrupted
                    } else {
                        Exit::Fail(QueueError::Shutdown)
                    };
                    changes.taken(taker.waiter, outcome);
                }
                (
                    std::mem::take(&mut state.offers),
                    std::mem::take(&mut state.takers),
                    changes,
                )
            };
            // These lists now contain only vacant slots. Release their high-water
            // storage outside the gate at this terminal lifecycle boundary.
            drop((offers, takers));
            changes.finish(Exit::Success(()))
        })
    }

    ///
    /// # Type parameters
    /// - `A`: Success or application-value type.
    ///
    /// # Parameters
    /// - `value` (`A`): Value read, transformed, encoded, or stored by this operation.
    /// - `cancellation` (`&Cancellation`): The cancellation token for this operation.
    ///
    /// # Returns
    /// Type: `(Admission<()>, Changes<A>)`.
    /// Returns `(Admission<()>, Changes<A>)`.
    fn admit_offer(&self, value: A, cancellation: &Cancellation) -> (Admission<()>, Changes<A>) {
        let mut changes = Changes::default();
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let admission = if cancellation.is_requested() {
            changes.discard.push(value);
            Admission::Ready(Exit::Interrupted)
        } else if state.closed {
            changes.discard.push(value);
            Admission::Ready(Exit::Fail(QueueError::Shutdown))
        } else {
            let mut taker = None;
            while let Some(waiting) = state.takers.pop_front() {
                if waiting.cancellation.is_requested() {
                    changes.taken(waiting.waiter, Exit::Interrupted);
                } else {
                    taker = Some(waiting);
                    break;
                }
            }
            match taker {
                Some(taker) => {
                    changes.taken(taker.waiter, Exit::Success(value));
                    Admission::Ready(Exit::Success(()))
                }
                None if state.capacity.permits_buffer(state.buffer.len()) => {
                    state.buffer.push_back(value);
                    Admission::Ready(Exit::Success(()))
                }
                None => {
                    let waiter = Waiter::new();
                    let position = state.offers.push_back(Offer {
                        value,
                        waiter: Arc::clone(&waiter),
                        cancellation: cancellation.clone(),
                    });
                    Admission::Pending(position, waiter)
                }
            }
        };
        (admission, changes)
    }

    ///
    /// # Type parameters
    /// - `A`: Success or application-value type.
    ///
    /// # Parameters
    /// - `cancellation` (`&Cancellation`): The cancellation token for this operation.
    ///
    /// # Returns
    /// Type: `(Admission<A>, Changes<A>)`.
    /// Returns `(Admission<A>, Changes<A>)`.
    fn admit_take(&self, cancellation: &Cancellation) -> (Admission<A>, Changes<A>) {
        let mut changes = Changes::default();
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let admission = if cancellation.is_requested() {
            Admission::Ready(Exit::Interrupted)
        } else if let Some(value) = state.buffer.pop_front() {
            if !state.closed
                && let Some(offer) = live_offer(&mut state.offers, &mut changes)
            {
                state.buffer.push_back(offer.value);
                changes.offered(offer.waiter, Exit::Success(()));
            }
            Admission::Ready(Exit::Success(value))
        } else if !state.closed {
            match live_offer(&mut state.offers, &mut changes) {
                Some(offer) => {
                    changes.offered(offer.waiter, Exit::Success(()));
                    Admission::Ready(Exit::Success(offer.value))
                }
                None => {
                    let waiter = Waiter::new();
                    let position = state.takers.push_back(Taker {
                        waiter: Arc::clone(&waiter),
                        cancellation: cancellation.clone(),
                    });
                    Admission::Pending(position, waiter)
                }
            }
        } else {
            Admission::Ready(Exit::Fail(QueueError::Shutdown))
        };
        (admission, changes)
    }

    ///
    /// # Type parameters
    /// - `A`: Success or application-value type.
    ///
    /// # Parameters
    /// - `position` (`Position`): The position supplied to this operation.
    /// - `waiter` (`&Arc<Waiter<()>>`): The waiter supplied to this operation.
    ///
    /// # Returns
    /// Type: `Changes<A>`.
    /// Returns `Changes<A>`.
    fn withdraw_offer(&self, position: Position, waiter: &Arc<Waiter<()>>) -> Changes<A> {
        let mut changes = Changes::default();
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(offer) = state
            .offers
            .remove_where(position, |offer| Arc::ptr_eq(&offer.waiter, waiter))
        {
            changes.offered(offer.waiter, Exit::Interrupted);
            changes.discard.push(offer.value);
        }
        changes
    }

    ///
    /// # Type parameters
    /// - `A`: Success or application-value type.
    ///
    /// # Parameters
    /// - `position` (`Position`): The position supplied to this operation.
    /// - `waiter` (`&Arc<Waiter<A>>`): The waiter supplied to this operation.
    ///
    /// # Returns
    /// Type: `Changes<A>`.
    /// Returns `Changes<A>`.
    fn withdraw_take(&self, position: Position, waiter: &Arc<Waiter<A>>) -> Changes<A> {
        let mut changes = Changes::default();
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(taker) = state
            .takers
            .remove_where(position, |taker| Arc::ptr_eq(&taker.waiter, waiter))
        {
            changes.taken(taker.waiter, Exit::Interrupted);
        }
        changes
    }
}

///
/// # Type parameters
/// - `A`: Successful application-value type.
///
/// # Parameters
/// - `offers` (`&mut Waiting<Offer<A>>`): The offers supplied to this operation.
/// - `changes` (`&mut Changes<A>`): The changes supplied to this operation.
///
/// # Returns
/// Type: `Option<Offer<A>>`.
/// Returns the value when present, or `None`.
fn live_offer<A>(offers: &mut Waiting<Offer<A>>, changes: &mut Changes<A>) -> Option<Offer<A>> {
    while let Some(offer) = offers.pop_front() {
        if offer.cancellation.is_requested() {
            changes.offered(offer.waiter, Exit::Interrupted);
            changes.discard.push(offer.value);
        } else {
            return Some(offer);
        }
    }
    None
}

impl<A: Send + 'static, K: Registration<A>> Registered<A, K> {
    ///
    /// # Type parameters
    /// - `K`: Type supplied by the enclosing impl or trait.
    ///
    /// # Parameters
    /// - `cancellation` (`Cancellation`): The cancellation token for this operation.
    ///
    /// # Returns
    /// Type: `Exit<K::Value, QueueError>`.
    /// Returns `Exit<K::Value, QueueError>`.
    async fn wait(self, cancellation: Cancellation) -> Exit<K::Value, QueueError> {
        loop {
            let waiter = self.registration.waiter();
            let changed = waiter.changed.listen();
            if let Some(outcome) = waiter.take() {
                return outcome;
            }
            if cancellation.is_requested() {
                let changes = self.registration.withdraw(&self.queue);
                // Committed outcomes have priority; cancellation only removes
                // this actual pending node, never a later reused slot.
                if let Some(outcome) = waiter.take() {
                    return changes.finish(outcome);
                }
            }
            let changed = std::pin::pin!(changed);
            let cancelled = std::pin::pin!(cancellation.cancelled());
            let _ = select(changed, cancelled).await;
        }
    }
}
