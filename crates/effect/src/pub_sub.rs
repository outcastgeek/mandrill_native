//! Bounded publish-subscribe streams and overload policies.

use crate::internal::notification::notify_all;
use crate::internal::runtime::{self, Step};
use crate::{
    Cancellation, Chunk, Defect, Effect, Exit, ResourceValue, Scope, Stream, StreamReader,
};
use crate::{effect, effect_die, effect_map, effect_scope_acquire, effect_sync, effect_try_sync};
use event_listener::Event;
use futures_util::{FutureExt, future::select};
use std::collections::{BTreeMap, VecDeque};
use std::num::{NonZeroU64, NonZeroUsize};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::pin::pin;
use std::sync::{Arc, Mutex};

/// A bounded subscriber's response to a full buffer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PubSubOverflow {
    /// Wait for consumer capacity before admitting another value.
    Backpressure,
    /// Discard the incoming value when the buffer is full.
    DropNewest,
    /// Evict the oldest buffered value to admit the new value.
    DropOldest,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// The amount of earlier publication history offered to a new subscriber.
pub enum ReplayPolicy {
    /// No value is present.
    None,
    /// Select the last matching candidate. Contains `NonZeroUsize`.
    Last(NonZeroUsize),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// Capacity, replay, and overflow behavior for a publication channel.
pub struct PubSubOptions {
    /// The maximum admitted capacity. Type: `NonZeroUsize`.
    pub capacity: NonZeroUsize,
    /// Policy applied when the subscriber buffer is full. Type: `PubSubOverflow`.
    pub overflow: PubSubOverflow,
    /// History offered to a newly registered subscriber. Type: `ReplayPolicy`.
    pub replay: ReplayPolicy,
}

impl PubSubOptions {
    /// This pure policy check also runs during compilation in a const context.
    ///
    /// # Returns
    /// Type: `Result<Self, PubSubError>`.
    pub const fn validate(self) -> Result<Self, PubSubError> {
        match self.replay {
            ReplayPolicy::Last(count) if count.get() > self.capacity.get() => {
                Err(PubSubError::InvalidReplay)
            }
            ReplayPolicy::None | ReplayPolicy::Last(_) => Ok(self),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// A typed publication or subscription lifecycle failure.
pub enum PubSubError {
    /// The provider is shutting down and cannot accept this work.
    Shutdown,
    /// A synchronous callback attempt was rejected because the channel gate was held.
    Contended,
    /// The replay failed validation.
    InvalidReplay,
    /// No further subscription id can be allocated without exceeding its range.
    SubscriptionIdExhausted,
    /// The publication payload was already consumed by an earlier run.
    PublishValueConsumed,
    /// Computing lag count exceeded its supported range.
    LagCountOverflow,
    /// A subscriber fell behind the bounded replay buffer.
    Lagged {
        /// Number of values no longer available. Type: `NonZeroUsize`.
        dropped: NonZeroUsize,
    },
}

impl std::fmt::Display for PubSubError {
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

impl std::error::Error for PubSubError {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// Counts of subscribers reached, lagged, or otherwise affected by a publication.
pub struct PublishReport {
    /// Number of subscribers that received this publication. Type: `usize`.
    pub delivered: usize,
    /// Number of values discarded under the overflow policy. Type: `usize`.
    pub dropped: usize,
}

/// Typed subscriber used by pub sub.
///
/// # Type parameters
/// - `A`: Application value type.
struct Subscriber<A> {
    buffered: VecDeque<Arc<A>>,
    dropped: usize,
    changed: Arc<Event>,
}

/// Typed state used by pub sub.
///
/// # Type parameters
/// - `A`: Application value type.
struct State<A> {
    closed: bool,
    next_id: Option<u64>,
    subscribers: BTreeMap<u64, Subscriber<A>>,
    history: VecDeque<Arc<A>>,
    current: Option<Arc<A>>,
}

/// Typed inner used by pub sub.
///
/// # Type parameters
/// - `A`: Application value type.
struct Inner<A> {
    state: Mutex<State<A>>,
    space_changed: Event,
    options: PubSubOptions,
    retain_current: bool,
}

/// Type alias for commit hook in pub sub.
///
/// # Type parameters
/// - `A`: Application value type.
type CommitHook<A> = Arc<dyn Fn(&A) -> bool + Send + Sync>;

/// A bounded, executor-independent broadcast hub. Payloads are shared through
/// Arc; publishing never requires `A: Clone`. Backpressure checks all current
/// subscribers before committing one publication. Drop policies report loss to
/// both the publisher and each affected subscriber.
///
/// # Type parameters
/// - `A`: Application value type. Bound or type: `Send + Sync + 'static`.
pub struct PubSub<A: Send + Sync + 'static> {
    inner: Arc<Inner<A>>,
}

impl<A: Send + Sync + 'static> Clone for PubSub<A> {
    ///
    /// # Returns
    /// Type: `Self`.
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

impl<A: Send + Sync + 'static> PubSub<A> {
    /// Construct a `PubSub` from its required inputs and explicit policies.
    ///
    /// # Type parameters
    /// - `A`: Application value type. Bound: `Send + Sync + 'static`.
    ///
    /// # Parameters
    /// - `options` (`PubSubOptions`): Caller-selected bounds and behavior for this operation.
    ///
    /// # Returns
    /// Type: `Result<Self, PubSubError>`.
    pub fn new(options: PubSubOptions) -> Result<Self, PubSubError> {
        Ok(Self::with_state(options.validate()?, None, false))
    }

    /// # Type parameters
    /// - `A`: Success or application-value type.
    ///
    /// # Parameters
    /// - `capacity` (`NonZeroUsize`): The caller-supplied capacity bound.
    /// - `overflow` (`PubSubOverflow`): Behavior when the configured buffer capacity is full.
    /// - `initial` (`A`): Initial value used before any state transition.
    ///
    /// # Returns
    /// Type: `Self`.
    pub(crate) fn with_current(
        capacity: NonZeroUsize,
        overflow: PubSubOverflow,
        initial: A,
    ) -> Self {
        Self::with_state(
            PubSubOptions {
                capacity,
                overflow,
                replay: ReplayPolicy::None,
            },
            Some(Arc::new(initial)),
            true,
        )
    }

    ///
    /// # Type parameters
    /// - `A`: Success or application-value type.
    ///
    /// # Parameters
    /// - `options` (`PubSubOptions`): Caller-selected bounds and behavior for this operation.
    /// - `current` (`Option<Arc<A>>`): The current supplied to this operation.
    /// - `retain_current` (`bool`): The retain current supplied to this operation.
    ///
    /// # Returns
    /// Type: `Self`.
    fn with_state(options: PubSubOptions, current: Option<Arc<A>>, retain_current: bool) -> Self {
        Self {
            inner: Arc::new(Inner {
                state: Mutex::new(State {
                    closed: false,
                    next_id: Some(u64::default()),
                    subscribers: BTreeMap::new(),
                    history: VecDeque::new(),
                    current,
                }),
                space_changed: Event::new(),
                options,
                retain_current,
            }),
        }
    }

    /// Create a `PubSub` through the declared Effect and environment capabilities.
    ///
    /// # Type parameters
    /// - `A`: Application value type. Bound: `Send + Sync + 'static`.
    /// - `R`: Environment type. Bounds: `Send + Sync + 'static`.
    ///
    /// # Parameters
    /// - `options` (`PubSubOptions`): Caller-selected bounds and behavior for this operation.
    ///
    /// # Returns
    /// Type: `Effect<Self, PubSubError, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    pub fn make<R: Send + Sync + 'static>(options: PubSubOptions) -> Effect<Self, PubSubError, R> {
        effect_try_sync! { body: move || Self::new(options), on_error: |error| error, }
    }

    /// A fresh subscription is registered on each execution. Replay history
    /// and registration are read under the same gate as publication.
    ///
    /// # Type parameters
    /// - `R`: Environment type. Bounds: `Send + Sync + 'static`.
    /// - `A`: Success or application-value type.
    ///
    /// # Returns
    /// Type: `Effect<Subscription<A>, PubSubError, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    pub fn subscribe<R: Send + Sync + 'static>(&self) -> Effect<Subscription<A>, PubSubError, R> {
        let hub = self.clone();
        Effect::ready(move |_, _| match hub.subscribe_now() {
            Ok(subscription) => Exit::Success(subscription),
            Err(error) => Exit::Fail(error),
        })
    }

    /// Register a subscriber owned by the supplied scope.
    ///
    /// # Type parameters
    /// - `R`: Environment type. Bounds: `Send + Sync + 'static`.
    /// - `A`: Success or application-value type.
    ///
    /// # Parameters
    /// - `scope` (`&Scope<PubSubError>`): Owner that joins child work and releases resources at close.
    ///
    /// # Returns
    /// Type: `Effect<ResourceValue<Subscription<A>>, PubSubError, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    pub fn subscribe_scoped<R: Send + Sync + 'static>(
        &self,
        scope: &Scope<PubSubError>,
    ) -> Effect<ResourceValue<Subscription<A>>, PubSubError, R> {
        effect_scope_acquire! {
        scope: scope, acquire: self.subscribe(), release: |subscription| {
                    crate::Release::sync(move || {
                        subscription.close_now();
                        drop(subscription);
                    })
                },
        }
    }

    /// Convert one scoped subscription into a stream. A loss notification is
    /// a typed stream failure rather than a silently skipped element.
    #[must_use]
    /// # Type parameters
    /// - `R`: Environment type. Bounds: `Send + Sync + 'static`.
    /// - `A`: Success or application-value type.
    ///
    /// # Returns
    /// Type: `Stream<Arc<A>, PubSubError, R>`.
    /// Returns a scoped stream with the declared item, error, and environment types.
    pub fn stream<R: Send + Sync + 'static>(&self) -> Stream<Arc<A>, PubSubError, R> {
        let hub = self.clone();
        Stream::new(move |scope| {
            effect_map! { program: hub.subscribe_scoped(&scope), map: |resource| {
                StreamReader::new({ let __dsl_factory = move || {
                    match resource.with(|subscription| subscription.pull_chunk::<R>()) {
                        Ok(next) => next,
                        Err(defect) => effect_die! { defect: defect, },
                    }
                }; effect! { __dsl_factory()? } })
            }, }
        })
    }

    /// # Type parameters
    /// - `A`: Success or application-value type.
    ///
    /// # Returns
    /// Type: `Result<Subscription<A>, PubSubError>`.
    pub(crate) fn subscribe_now(&self) -> Result<Subscription<A>, PubSubError> {
        let mut state = self
            .inner
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.closed {
            return Err(PubSubError::Shutdown);
        }
        let id = state.next_id.ok_or(PubSubError::SubscriptionIdExhausted)?;
        let buffered = state.history.iter().cloned().collect();
        let changed = Arc::new(Event::new());
        state.next_id = id.checked_add(NonZeroU64::MIN.get());
        state.subscribers.insert(
            id,
            Subscriber {
                buffered,
                dropped: usize::default(),
                changed: Arc::clone(&changed),
            },
        );
        Ok(Subscription {
            inner: Arc::new(SubscriptionInner {
                hub: Arc::clone(&self.inner),
                id,
                changed,
            }),
        })
    }

    /// # Type parameters
    /// - `A`: Success or application-value type.
    ///
    /// # Returns
    /// Type: `Result<(Arc<A>, Subscription<A>), PubSubError>`.
    pub(crate) fn subscribe_current_now(&self) -> Result<(Arc<A>, Subscription<A>), PubSubError> {
        let mut state = self
            .inner
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.closed {
            return Err(PubSubError::Shutdown);
        }
        let current = state
            .current
            .as_ref()
            .map(Arc::clone)
            .ok_or(PubSubError::Shutdown)?;
        let id = state.next_id.ok_or(PubSubError::SubscriptionIdExhausted)?;
        let changed = Arc::new(Event::new());
        state.next_id = id.checked_add(NonZeroU64::MIN.get());
        state.subscribers.insert(
            id,
            Subscriber {
                buffered: VecDeque::new(),
                dropped: usize::default(),
                changed: Arc::clone(&changed),
            },
        );
        Ok((
            current,
            Subscription {
                inner: Arc::new(SubscriptionInner {
                    hub: Arc::clone(&self.inner),
                    id,
                    changed,
                }),
            },
        ))
    }

    /// # Type parameters
    /// - `A`: Success or application-value type.
    ///
    /// # Returns
    /// Type: `Result<Arc<A>, PubSubError>`.
    pub(crate) fn current(&self) -> Result<Arc<A>, PubSubError> {
        let state = self
            .inner
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.closed {
            return Err(PubSubError::Shutdown);
        }
        state
            .current
            .as_ref()
            .map(Arc::clone)
            .ok_or(PubSubError::Shutdown)
    }

    /// Consume one owned value on the first started execution. Re-running the
    /// recipe reports a typed consumed-value error instead of cloning payloads.
    ///
    /// # Type parameters
    /// - `R`: Environment type. Bounds: `Send + Sync + 'static`.
    /// - `A`: Success or application-value type.
    ///
    /// # Parameters
    /// - `value` (`A`): Value read, transformed, encoded, or stored by this operation.
    ///
    /// # Returns
    /// Type: `Effect<PublishReport, PubSubError, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    pub fn publish<R: Send + Sync + 'static>(
        &self,
        value: A,
    ) -> Effect<PublishReport, PubSubError, R> {
        let value = Mutex::new(Some(value));
        self.publish_candidate(
            move || {
                value
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .take()
                    .ok_or(PubSubError::PublishValueConsumed)
            },
            None,
        )
    }

    /// Create one fresh owned value per execution, including after a prior run
    /// was cancelled or rejected. The factory runs once per started execution.
    ///
    /// # Type parameters
    /// - `R`: Environment type. Bounds: `Send + Sync + 'static`.
    /// - `A`: Success or application-value type.
    ///
    /// # Parameters
    /// - `factory` (`impl Fn() -> A + Send + Sync + 'static`): Factory invoked to create fresh data for an execution.
    ///
    /// # Returns
    /// Type: `Effect<PublishReport, PubSubError, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    pub fn publish_with<R: Send + Sync + 'static>(
        &self,
        factory: impl Fn() -> A + Send + Sync + 'static,
    ) -> Effect<PublishReport, PubSubError, R> {
        self.publish_candidate(move || Ok(factory()), None)
    }

    /// Commit related state while publication is admitted and before any
    /// subscriber can observe the value. The callback must not reenter this hub.
    ///
    /// # Type parameters
    /// - `R`: Environment type. Bounds: `Send + Sync + 'static`.
    /// - `A`: Success or application-value type.
    ///
    /// # Parameters
    /// - `value` (`A`): Value read, transformed, encoded, or stored by this operation.
    /// - `commit` (`impl Fn(&A) + Send + Sync + 'static`): The commit supplied to this operation.
    ///
    /// # Returns
    /// Type: `Effect<PublishReport, PubSubError, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    pub(crate) fn publish_with_commit<R: Send + Sync + 'static>(
        &self,
        value: A,
        commit: impl Fn(&A) + Send + Sync + 'static,
    ) -> Effect<PublishReport, PubSubError, R> {
        let value = Mutex::new(Some(value));
        self.publish_candidate(
            move || {
                value
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .take()
                    .ok_or(PubSubError::PublishValueConsumed)
            },
            Some(Arc::new(move |value| {
                commit(value);
                true
            })),
        )
    }

    /// Use a trusted metadata-only guard after deciding capacity and before publication.
    ///
    /// False rejects without appending or replacing current state. The guard runs under
    /// the channel gate and must not invoke application callbacks, await or reenter the
    /// channel; notification and discarded-value destruction happen after unlocking.
    pub(crate) fn publish_with_guard<R: Send + Sync + 'static>(
        &self,
        value: A,
        guard: impl Fn(&A) -> bool + Send + Sync + 'static,
    ) -> Effect<PublishReport, PubSubError, R> {
        let value = Mutex::new(Some(value));
        self.publish_candidate(
            move || {
                value
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .take()
                    .ok_or(PubSubError::PublishValueConsumed)
            },
            Some(Arc::new(guard)),
        )
    }
    ///
    /// # Type parameters
    /// - `R`: Bounds: `Send + Sync + 'static`.
    /// - `A`: Success or application-value type.
    ///
    /// # Parameters
    /// - `factory` (`impl Fn() -> Result<A, PubSubError> + Send + Sync + 'static`): Factory invoked to create fresh data for an execution.
    /// - `commit` (`Option<CommitHook<A>>`): The commit supplied to this operation.
    ///
    /// # Returns
    /// Type: `Effect<PublishReport, PubSubError, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    fn publish_candidate<R: Send + Sync + 'static>(
        &self,
        factory: impl Fn() -> Result<A, PubSubError> + Send + Sync + 'static,
        commit: Option<CommitHook<A>>,
    ) -> Effect<PublishReport, PubSubError, R> {
        let hub = self.clone();
        Effect::new(move |_, context, finish| {
            let value = match catch_unwind(AssertUnwindSafe(&factory)) {
                Ok(Ok(value)) => Arc::new(value),
                Ok(Err(error)) => return runtime::finish(finish, Exit::Fail(error)),
                Err(panic) => return runtime::finish(finish, Exit::Die(Defect::from_panic(panic))),
            };
            let hub = hub.clone();
            let commit = commit.clone();
            Step::Suspended(Box::pin(async move {
                let outcome =
                    AssertUnwindSafe(hub.inner.publish(value, context.cancellation, commit))
                        .catch_unwind()
                        .await
                        .unwrap_or_else(|panic| Exit::Die(Defect::from_panic(panic)));
                runtime::finish(finish, outcome)
            }))
        })
    }

    /// Drain allows existing subscriber buffers to be read; discard removes
    /// them. Both reject new publications/subscriptions and wake pending work.
    /// The first shutdown policy wins; later calls return false.
    ///
    /// # Type parameters
    /// - `A`: Application value type. Bound: `Send + Sync + 'static`.
    /// - `R`: Environment type. Bounds: `Send + Sync + 'static`.
    ///
    /// # Parameters
    /// - `mode` (`PubSubShutdown`): Selected behavior for this operation.
    ///
    /// # Returns
    /// Type: `Effect<bool, PubSubError, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    pub fn shutdown<R: Send + Sync + 'static>(
        &self,
        mode: PubSubShutdown,
    ) -> Effect<bool, PubSubError, R> {
        let hub = self.clone();
        Effect::ready(move |_, _| {
            let (discarded, notifications) = {
                let mut state = hub
                    .inner
                    .state
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                if state.closed {
                    return Exit::Success(false);
                }
                state.closed = true;
                let mut discarded = Vec::new();
                discarded.extend(std::mem::take(&mut state.history));
                discarded.extend(state.current.take());
                if matches!(mode, PubSubShutdown::Discard) {
                    for subscriber in state.subscribers.values_mut() {
                        discarded.extend(std::mem::take(&mut subscriber.buffered));
                    }
                }
                let notifications = state
                    .subscribers
                    .values()
                    .map(|subscriber| Arc::clone(&subscriber.changed))
                    .collect::<Vec<_>>();
                (discarded, notifications)
            };
            notify_all(&hub.inner.space_changed);
            for changed in notifications {
                notify_all(&changed);
            }
            drop(discarded);
            Exit::Success(true)
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// The state reported when publication closes and subscribers settle.
pub enum PubSubShutdown {
    /// Stop admission and wait for already accepted work.
    Drain,
    /// Drop the selected value under the configured overflow policy.
    Discard,
}

/// Variants of publish attempt used by pub sub.
///
/// # Type parameters
/// - `A`: Application value type.
enum PublishAttempt<A> {
    Wait,
    Complete(PublishReport, Vec<Arc<A>>, Vec<Arc<Event>>),
    Failed(PubSubError),
}

impl<A: Send + Sync + 'static> Inner<A> {
    ///
    /// # Type parameters
    /// - `A`: Success or application-value type.
    ///
    /// # Parameters
    /// - `value` (`Arc<A>`): Value read, transformed, encoded, or stored by this operation.
    /// - `cancellation` (`Cancellation`): The cancellation token for this operation.
    /// - `commit` (`Option<CommitHook<A>>`): The commit supplied to this operation.
    ///
    /// # Returns
    /// Type: `Exit<PublishReport, PubSubError>`.
    /// Returns `Exit<PublishReport, PubSubError>`.
    fn publish_locked(
        &self,
        state: &mut State<A>,
        value: &Arc<A>,
        commit: Option<&CommitHook<A>>,
    ) -> PublishAttempt<A> {
        let full = state
            .subscribers
            .values()
            .filter(|subscriber| subscriber.buffered.len() == self.options.capacity.get())
            .count();
        if full > usize::default() && matches!(self.options.overflow, PubSubOverflow::Backpressure)
        {
            PublishAttempt::Wait
        } else if state.subscribers.values().any(|subscriber| {
            subscriber.buffered.len() == self.options.capacity.get()
                && subscriber.dropped == usize::MAX
        }) {
            PublishAttempt::Failed(PubSubError::LagCountOverflow)
        } else {
            if commit.is_some_and(|commit| !commit(value)) {
                return PublishAttempt::Complete(
                    PublishReport {
                        delivered: usize::default(),
                        dropped: usize::default(),
                    },
                    Vec::new(),
                    Vec::new(),
                );
            }
            let mut discarded = Vec::new();
            let mut notifications = Vec::with_capacity(state.subscribers.len());
            let mut delivered = usize::default();
            for subscriber in state.subscribers.values_mut() {
                let full = subscriber.buffered.len() == self.options.capacity.get();
                if full {
                    subscriber.dropped += NonZeroUsize::MIN.get();
                    match self.options.overflow {
                        PubSubOverflow::DropNewest => {
                            notifications.push(Arc::clone(&subscriber.changed));
                            continue;
                        }
                        PubSubOverflow::DropOldest => {
                            discarded.extend(subscriber.buffered.pop_front());
                        }
                        PubSubOverflow::Backpressure => {
                            return PublishAttempt::Wait;
                        }
                    }
                }
                subscriber.buffered.push_back(Arc::clone(value));
                notifications.push(Arc::clone(&subscriber.changed));
                delivered += NonZeroUsize::MIN.get();
            }
            if let ReplayPolicy::Last(limit) = self.options.replay {
                if state.history.len() == limit.get() {
                    discarded.extend(state.history.pop_front());
                }
                state.history.push_back(Arc::clone(value));
            }
            if self.retain_current {
                discarded.extend(state.current.replace(Arc::clone(value)));
            }
            PublishAttempt::Complete(
                PublishReport {
                    delivered,
                    dropped: full,
                },
                discarded,
                notifications,
            )
        }
    }

    async fn publish(
        &self,
        value: Arc<A>,
        cancellation: Cancellation,
        commit: Option<CommitHook<A>>,
    ) -> Exit<PublishReport, PubSubError> {
        loop {
            let changed = self.space_changed.listen();
            let attempt = {
                let mut state = self
                    .state
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                if cancellation.is_requested() {
                    return Exit::Interrupted;
                }
                if state.closed {
                    return Exit::Fail(PubSubError::Shutdown);
                }
                self.publish_locked(&mut state, &value, commit.as_ref())
            };
            match attempt {
                PublishAttempt::Wait => {
                    let changed = pin!(changed);
                    let cancelled = pin!(cancellation.cancelled());
                    let _ = select(changed, cancelled).await;
                }
                PublishAttempt::Complete(report, discarded, notifications) => {
                    for changed in notifications {
                        notify_all(&changed);
                    }
                    drop(discarded);
                    return Exit::Success(report);
                }
                PublishAttempt::Failed(error) => return Exit::Fail(error),
            }
        }
    }
}

/// Typed subscription inner used by pub sub.
///
/// # Type parameters
/// - `A`: Application value type.
struct SubscriptionInner<A> {
    hub: Arc<Inner<A>>,
    id: u64,
    changed: Arc<Event>,
}

/// Unique subscription ownership. Dropping the handle unregisters it and
/// wakes pending takers and backpressured publishers; no payload is copied.
///
/// # Type parameters
/// - `A`: Application value type. Bound or type: `Send + Sync + 'static`.
pub struct Subscription<A: Send + Sync + 'static> {
    inner: Arc<SubscriptionInner<A>>,
}

/// A delivered value or lifecycle event observed by a subscriber.
///
/// # Type parameters
/// - `A`: Application value type.
pub enum SubscriptionEvent<A> {
    /// Deliver one application item. Contains `Arc<A>`.
    Item(Arc<A>),
    /// The subscriber missed values and must resnapshot. Contains `NonZeroUsize`.
    Lagged(NonZeroUsize),
    /// The stream has no further items.
    End,
}

impl<A: Send + Sync + 'static> Subscription<A> {
    /// # Type parameters
    /// - `A`: Application value type. Bound: `Send + Sync + 'static`.
    ///
    /// # Returns
    /// Type: `bool`.
    /// Returns whether the requested condition holds.
    pub(crate) fn close_now(&self) -> bool {
        self.inner.close()
    }

    /// Transfer the owned value out when the current state permits it.
    ///
    /// # Type parameters
    /// - `R`: Environment type. Bounds: `Send + Sync + 'static`.
    /// - `A`: Success or application-value type.
    ///
    /// # Returns
    /// Type: `Effect<SubscriptionEvent<A>, PubSubError, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    pub fn take<R: Send + Sync + 'static>(&self) -> Effect<SubscriptionEvent<A>, PubSubError, R> {
        let subscription = Arc::clone(&self.inner);
        Effect::new(move |_, context, finish| {
            let subscription = Arc::clone(&subscription);
            Step::Suspended(Box::pin(async move {
                let outcome = AssertUnwindSafe(subscription.take(context.cancellation))
                    .catch_unwind()
                    .await
                    .unwrap_or_else(|panic| Exit::Die(Defect::from_panic(panic)));
                runtime::finish(finish, outcome)
            }))
        })
    }

    /// Receive the next bounded batch of published values.
    ///
    /// # Type parameters
    /// - `R`: Environment type. Bounds: `Send + Sync + 'static`.
    /// - `A`: Success or application-value type.
    ///
    /// # Returns
    /// Type: `Effect<Chunk<Arc<A>>, PubSubError, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    pub fn pull_chunk<R: Send + Sync + 'static>(&self) -> Effect<Chunk<Arc<A>>, PubSubError, R> {
        {
            let __dsl_program = self.take();
            effect! { let event = __dsl_program.clone()?; match event {
                SubscriptionEvent::Item(value) => effect_sync! { once; body: move || Chunk::Values(vec![value]), },
                SubscriptionEvent::Lagged(dropped) => {
                    effect_try_sync! { body: { let __dsl_error = move || PubSubError::Lagged { dropped }; move || Err(__dsl_error()) }, on_error: ::core::convert::identity, }
                }
                SubscriptionEvent::End => effect_sync! { body: || Chunk::End, },
            }? }
        }
    }

    /// Remove this subscriber and release its retained delivery state.
    ///
    /// # Type parameters
    /// - `A`: Application value type. Bound: `Send + Sync + 'static`.
    /// - `R`: Environment type. Bounds: `Send + Sync + 'static`.
    ///
    /// # Returns
    /// Type: `Effect<bool, PubSubError, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    pub fn unsubscribe<R: Send + Sync + 'static>(&self) -> Effect<bool, PubSubError, R> {
        let subscription = Arc::clone(&self.inner);
        effect_sync! { body: move || subscription.close(), }
    }
}

impl<A: Send + Sync + 'static> Drop for Subscription<A> {
    ///
    /// # Returns
    /// Type: `()`.
    fn drop(&mut self) {
        let _ = catch_unwind(AssertUnwindSafe(|| self.inner.close()));
    }
}

impl<A: Send + Sync + 'static> SubscriptionInner<A> {
    ///
    /// # Returns
    /// Type: `bool`.
    /// Returns whether the requested condition holds.
    fn close(&self) -> bool {
        let removed = self
            .hub
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .subscribers
            .remove(&self.id);
        let first = removed.is_some();
        if first {
            notify_all(&self.changed);
            notify_all(&self.hub.space_changed);
        }
        drop(removed);
        first
    }

    ///
    /// # Type parameters
    /// - `A`: Success or application-value type.
    ///
    /// # Parameters
    /// - `cancellation` (`Cancellation`): The cancellation token for this operation.
    ///
    /// # Returns
    /// Type: `Exit<SubscriptionEvent<A>, PubSubError>`.
    /// Returns `Exit<SubscriptionEvent<A>, PubSubError>`.
    async fn take(&self, cancellation: Cancellation) -> Exit<SubscriptionEvent<A>, PubSubError> {
        loop {
            let changed = self.changed.listen();
            let (event, freed) = {
                let mut state = self
                    .hub
                    .state
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                if cancellation.is_requested() {
                    return Exit::Interrupted;
                }
                let closed = state.closed;
                match state.subscribers.get_mut(&self.id) {
                    None => (Some(SubscriptionEvent::End), false),
                    Some(subscriber) => {
                        if let Some(dropped) =
                            NonZeroUsize::new(std::mem::take(&mut subscriber.dropped))
                        {
                            (Some(SubscriptionEvent::Lagged(dropped)), false)
                        } else if let Some(value) = subscriber.buffered.pop_front() {
                            (Some(SubscriptionEvent::Item(value)), true)
                        } else if closed {
                            (Some(SubscriptionEvent::End), false)
                        } else {
                            (None, false)
                        }
                    }
                }
            };
            if let Some(event) = event {
                if freed {
                    notify_all(&self.hub.space_changed);
                }
                return Exit::Success(event);
            }
            let changed = pin!(changed);
            let cancelled = pin!(cancellation.cancelled());
            let _ = select(changed, cancelled).await;
        }
    }
}

/// Full-buffer policy for synchronous callback publication; backpressure is deliberately unavailable.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CallbackOverflow {
    /// Discard the incoming publication when subscriber capacity is full.
    DropNewest,
    /// Evict the oldest buffered publication to admit the incoming one.
    DropOldest,
}
/// How synchronous publication responds when another operation owns the channel gate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CallbackContention {
    /// Discard the incoming value if the channel gate is already held.
    DropNewest,
    /// Return PubSubError::Contended when the gate cannot be acquired immediately.
    Reject,
}
/// Explicit capacity, history, overflow and gate-contention settings for [`CallbackPubSub`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CallbackPubSubOptions {
    /// Maximum retained publication capacity; must also accommodate requested replay history.
    pub capacity: NonZeroUsize,
    /// Policy when publication buffers are full; neither choice waits for consumers.
    pub overflow: CallbackOverflow,
    /// Bounded publication history supplied to new subscriptions.
    pub replay: ReplayPolicy,
    /// Immediate drop or typed rejection when another operation owns the gate.
    pub contention: CallbackContention,
}
/// Why a synchronous callback publication discarded its incoming value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CallbackDrop {
    /// The buffer policy dropped the incoming value; retains the publication counts.
    Full(PublishReport),
    /// The callback publication could not acquire the channel gate.
    Contended,
}
/// Immediate callback publication result, distinct from typed channel errors.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CallbackPublish {
    /// Publication committed according to the buffer policy; inspect delivery/drop counts in the report.
    Accepted(PublishReport),
    /// The incoming value was discarded due to full buffers or gate contention.
    Dropped(CallbackDrop),
    /// The channel has shut down; no value was published.
    Closed,
}
/// A synchronous publication facade over the scoped [`PubSub`] engine.
///
/// [`Self::try_publish`] uses try_lock and never waits for backpressure or the gate.
/// It may allocate and notify observers; it is not a hard real-time or wait-free
/// primitive. Clones share the channel, while subscriptions and streams retain
/// normal scope-owned cancellation and cleanup.
pub struct CallbackPubSub<A: Send + Sync + 'static> {
    hub: PubSub<A>,
    contention: CallbackContention,
}
impl<A: Send + Sync + 'static> Clone for CallbackPubSub<A> {
    fn clone(&self) -> Self {
        Self {
            hub: self.hub.clone(),
            contention: self.contention,
        }
    }
}
impl<A: Send + Sync + 'static> CallbackPubSub<A> {
    /// Create a shared callback channel; replay larger than capacity returns InvalidReplay.
    pub fn new(options: CallbackPubSubOptions) -> Result<Self, PubSubError> {
        Ok(Self {
            hub: PubSub::new(PubSubOptions {
                capacity: options.capacity,
                overflow: match options.overflow {
                    CallbackOverflow::DropNewest => PubSubOverflow::DropNewest,
                    CallbackOverflow::DropOldest => PubSubOverflow::DropOldest,
                },
                replay: options.replay,
            })?,
            contention: options.contention,
        })
    }
    /// Consume one value and attempt publication immediately, without waiting for capacity.
    ///
    /// Gate contention follows the configured drop/reject policy. A closed channel
    /// returns Closed; a full-buffer drop returns its report. Waking subscribers and
    /// destroying discarded values occur outside the channel gate.
    pub fn try_publish(&self, value: A) -> Result<CallbackPublish, PubSubError> {
        let value = Arc::new(value);
        let attempt = {
            let mut state = match self.hub.inner.state.try_lock() {
                Ok(state) => state,
                Err(std::sync::TryLockError::Poisoned(error)) => error.into_inner(),
                Err(std::sync::TryLockError::WouldBlock) => {
                    return match self.contention {
                        CallbackContention::DropNewest => {
                            Ok(CallbackPublish::Dropped(CallbackDrop::Contended))
                        }
                        CallbackContention::Reject => Err(PubSubError::Contended),
                    };
                }
            };
            if state.closed {
                return Ok(CallbackPublish::Closed);
            }
            self.hub.inner.publish_locked(&mut state, &value, None)
        };
        match attempt {
            PublishAttempt::Complete(report, discarded, notifications) => {
                for changed in notifications {
                    notify_all(&changed);
                }
                drop(discarded);
                Ok(if report.delivered == 0 && report.dropped > 0 {
                    CallbackPublish::Dropped(CallbackDrop::Full(report))
                } else {
                    CallbackPublish::Accepted(report)
                })
            }
            PublishAttempt::Failed(error) => Err(error),
            PublishAttempt::Wait => Err(PubSubError::Contended),
        }
    }
    /// Lazily register a subscription whose unsubscription is owned by scope, using the same channel history.
    pub fn subscribe_scoped<R: Send + Sync + 'static>(
        &self,
        scope: &Scope<PubSubError>,
    ) -> Effect<ResourceValue<Subscription<A>>, PubSubError, R> {
        self.hub.subscribe_scoped(scope)
    }
    /// Create a reusable scoped stream of shared publications; each opening registers its own subscription.
    #[must_use]
    pub fn stream<R: Send + Sync + 'static>(&self) -> Stream<Arc<A>, PubSubError, R> {
        self.hub.stream()
    }
    /// Lazily close the channel with the supplied drain/discard policy and settle subscriber notifications.
    pub fn shutdown<R: Send + Sync + 'static>(
        &self,
        mode: PubSubShutdown,
    ) -> Effect<bool, PubSubError, R> {
        self.hub.shutdown(mode)
    }
}
