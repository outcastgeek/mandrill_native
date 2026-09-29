use crate::internal::notification::notify_all;
use crate::internal::runtime::{self, Step};
use crate::{
    Cancellation, Chunk, Defect, Effect, Exit, Release, ResourceValue, Scope, Stream, StreamReader,
};
use event_listener::Event;
use futures_util::{FutureExt, future::select};
use std::collections::{BTreeMap, VecDeque};
use std::num::{NonZeroU64, NonZeroUsize};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Arc, Mutex};

/// A bounded subscriber's response to a full buffer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PubSubOverflow {
    Backpressure,
    DropNewest,
    DropOldest,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReplayPolicy {
    None,
    Last(NonZeroUsize),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PubSubOptions {
    pub capacity: NonZeroUsize,
    pub overflow: PubSubOverflow,
    pub replay: ReplayPolicy,
}

impl PubSubOptions {
    /// This pure policy check also runs during compilation in a const context.
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
pub enum PubSubError {
    Shutdown,
    InvalidReplay,
    SubscriptionIdExhausted,
    PublishValueConsumed,
    LagCountOverflow,
    Lagged { dropped: NonZeroUsize },
}

impl std::fmt::Display for PubSubError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(self, formatter)
    }
}

impl std::error::Error for PubSubError {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PublishReport {
    pub delivered: usize,
    pub dropped: usize,
}

struct Subscriber<A> {
    buffered: VecDeque<Arc<A>>,
    dropped: usize,
    changed: Arc<Event>,
}

struct State<A> {
    closed: bool,
    next_id: Option<u64>,
    subscribers: BTreeMap<u64, Subscriber<A>>,
    history: VecDeque<Arc<A>>,
    current: Option<Arc<A>>,
}

struct Inner<A> {
    state: Mutex<State<A>>,
    space_changed: Event,
    options: PubSubOptions,
    retain_current: bool,
}

type CommitHook<A> = Arc<dyn Fn(&A) + Send + Sync>;

/// A bounded, executor-independent broadcast hub. Payloads are shared through
/// Arc; publishing never requires `A: Clone`. Backpressure checks all current
/// subscribers before committing one publication. Drop policies report loss to
/// both the publisher and each affected subscriber.
pub struct PubSub<A: Send + Sync + 'static> {
    inner: Arc<Inner<A>>,
}

impl<A: Send + Sync + 'static> Clone for PubSub<A> {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

impl<A: Send + Sync + 'static> PubSub<A> {
    pub fn new(options: PubSubOptions) -> Result<Self, PubSubError> {
        Ok(Self::with_state(options.validate()?, None, false))
    }

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

    pub fn make<R: Send + Sync + 'static>(options: PubSubOptions) -> Effect<Self, PubSubError, R> {
        Effect::try_sync(move || Self::new(options), |error| error)
    }

    /// A fresh subscription is registered on each execution. Replay history
    /// and registration are read under the same gate as publication.
    pub fn subscribe<R: Send + Sync + 'static>(&self) -> Effect<Subscription<A>, PubSubError, R> {
        let hub = self.clone();
        Effect::ready(move |_, _| match hub.subscribe_now() {
            Ok(subscription) => Exit::Success(subscription),
            Err(error) => Exit::Fail(error),
        })
    }

    pub fn subscribe_scoped<R: Send + Sync + 'static>(
        &self,
        scope: &Scope<PubSubError>,
    ) -> Effect<ResourceValue<Subscription<A>>, PubSubError, R> {
        scope.acquire(self.subscribe(), |subscription| {
            Release::sync(move || {
                subscription.close_now();
                drop(subscription);
            })
        })
    }

    /// Convert one scoped subscription into a stream. A loss notification is
    /// a typed stream failure rather than a silently skipped element.
    #[must_use]
    pub fn stream<R: Send + Sync + 'static>(&self) -> Stream<Arc<A>, PubSubError, R> {
        let hub = self.clone();
        Stream::new(move |scope| {
            hub.subscribe_scoped(&scope).map(|resource| {
                StreamReader::new(Effect::defer(move || {
                    match resource.with(|subscription| subscription.pull_chunk::<R>()) {
                        Ok(next) => next,
                        Err(defect) => Effect::die(defect),
                    }
                }))
            })
        })
    }

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
    pub fn publish_with<R: Send + Sync + 'static>(
        &self,
        factory: impl Fn() -> A + Send + Sync + 'static,
    ) -> Effect<PublishReport, PubSubError, R> {
        self.publish_candidate(move || Ok(factory()), None)
    }

    /// Commit related state while publication is admitted and before any
    /// subscriber can observe the value. The callback must not reenter this hub.
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
            Some(Arc::new(commit)),
        )
    }

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
pub enum PubSubShutdown {
    Drain,
    Discard,
}

enum PublishAttempt<A> {
    Wait,
    Complete(PublishReport, Vec<Arc<A>>, Vec<Arc<Event>>),
    Failed(PubSubError),
}

impl<A: Send + Sync + 'static> Inner<A> {
    async fn publish(
        &self,
        value: Arc<A>,
        cancellation: Cancellation,
        commit: Option<CommitHook<A>>,
    ) -> Exit<PublishReport, PubSubError> {
        loop {
            let changed = self.space_changed.listen();
            let attempt = 'attempt: {
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
                let full = state
                    .subscribers
                    .values()
                    .filter(|subscriber| subscriber.buffered.len() == self.options.capacity.get())
                    .count();
                if full > usize::default()
                    && matches!(self.options.overflow, PubSubOverflow::Backpressure)
                {
                    PublishAttempt::Wait
                } else if state.subscribers.values().any(|subscriber| {
                    subscriber.buffered.len() == self.options.capacity.get()
                        && subscriber.dropped == usize::MAX
                }) {
                    PublishAttempt::Failed(PubSubError::LagCountOverflow)
                } else {
                    if let Some(commit) = &commit {
                        commit(&value);
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
                                    break 'attempt PublishAttempt::Wait;
                                }
                            }
                        }
                        subscriber.buffered.push_back(Arc::clone(&value));
                        notifications.push(Arc::clone(&subscriber.changed));
                        delivered += NonZeroUsize::MIN.get();
                    }
                    if let ReplayPolicy::Last(limit) = self.options.replay {
                        if state.history.len() == limit.get() {
                            discarded.extend(state.history.pop_front());
                        }
                        state.history.push_back(Arc::clone(&value));
                    }
                    if self.retain_current {
                        discarded.extend(state.current.replace(Arc::clone(&value)));
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
            };
            match attempt {
                PublishAttempt::Wait => {
                    let changed = std::pin::pin!(changed);
                    let cancelled = std::pin::pin!(cancellation.cancelled());
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

struct SubscriptionInner<A> {
    hub: Arc<Inner<A>>,
    id: u64,
    changed: Arc<Event>,
}

/// Unique subscription ownership. Dropping the handle unregisters it and
/// wakes pending takers and backpressured publishers; no payload is copied.
pub struct Subscription<A: Send + Sync + 'static> {
    inner: Arc<SubscriptionInner<A>>,
}

pub enum SubscriptionEvent<A> {
    Item(Arc<A>),
    Lagged(NonZeroUsize),
    End,
}

impl<A: Send + Sync + 'static> Subscription<A> {
    pub(crate) fn close_now(&self) -> bool {
        self.inner.close()
    }

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

    pub fn pull_chunk<R: Send + Sync + 'static>(&self) -> Effect<Chunk<Arc<A>>, PubSubError, R> {
        self.take().flat_map(|event| match event {
            SubscriptionEvent::Item(value) => Effect::sync_once(move || Chunk::Values(vec![value])),
            SubscriptionEvent::Lagged(dropped) => {
                Effect::fail_with(move || PubSubError::Lagged { dropped })
            }
            SubscriptionEvent::End => Effect::sync(|| Chunk::End),
        })
    }

    pub fn unsubscribe<R: Send + Sync + 'static>(&self) -> Effect<bool, PubSubError, R> {
        let subscription = Arc::clone(&self.inner);
        Effect::sync(move || subscription.close())
    }
}

impl<A: Send + Sync + 'static> Drop for Subscription<A> {
    fn drop(&mut self) {
        let _ = catch_unwind(AssertUnwindSafe(|| self.inner.close()));
    }
}

impl<A: Send + Sync + 'static> SubscriptionInner<A> {
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
            let changed = std::pin::pin!(changed);
            let cancelled = std::pin::pin!(cancellation.cancelled());
            let _ = select(changed, cancelled).await;
        }
    }
}
