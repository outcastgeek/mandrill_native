use crate::internal::runtime;
use crate::pub_sub::{
    PubSub, PubSubError, PubSubOverflow, PubSubShutdown, PublishReport, Subscription,
};
use crate::{Chunk, Effect, Exit, Release, ResourceValue, Scope, Stream, StreamReader};
use std::num::NonZeroUsize;
use std::sync::{Arc, Mutex};

/// One value and a subscription registered at the same publication boundary.
/// The first `take` sees only changes committed after this snapshot. A lagged
/// subscription can obtain a fresh snapshot through `read` and resubscribe.
pub struct SubscriptionSnapshot<A: Send + Sync + 'static> {
    pub value: Arc<A>,
    pub subscription: Subscription<A>,
}

/// A current shared value with bounded change observation. It uses the same
/// atomic gate for `read`/subscription registration and publication, so no
/// change can disappear between obtaining a snapshot and subscribing.
pub struct SubscriptionRef<A: Send + Sync + 'static> {
    hub: PubSub<A>,
}

impl<A: Send + Sync + 'static> Clone for SubscriptionRef<A> {
    fn clone(&self) -> Self {
        Self {
            hub: self.hub.clone(),
        }
    }
}

impl<A: Send + Sync + 'static> SubscriptionRef<A> {
    #[must_use]
    pub fn new(initial: A, capacity: NonZeroUsize, overflow: PubSubOverflow) -> Self {
        Self {
            hub: PubSub::with_current(capacity, overflow, initial),
        }
    }

    pub fn make_with<R: Send + Sync + 'static>(
        initial: impl Fn() -> A + Send + Sync + 'static,
        capacity: NonZeroUsize,
        overflow: PubSubOverflow,
    ) -> Effect<Self, PubSubError, R> {
        Effect::sync(move || Self::new(initial(), capacity, overflow))
    }

    /// Clones only the shared handle. The stored payload is never cloned.
    pub fn read<R: Send + Sync + 'static>(&self) -> Effect<Arc<A>, PubSubError, R> {
        let reference = self.clone();
        Effect::ready(move |_, _| match reference.hub.current() {
            Ok(value) => Exit::Success(value),
            Err(error) => Exit::Fail(error),
        })
    }

    pub fn set<R: Send + Sync + 'static>(&self, value: A) -> Effect<PublishReport, PubSubError, R> {
        self.hub.publish(value)
    }

    pub(crate) fn set_with_commit<R: Send + Sync + 'static>(
        &self,
        value: A,
        commit: impl Fn(&A) + Send + Sync + 'static,
    ) -> Effect<PublishReport, PubSubError, R> {
        self.hub.publish_with_commit(value, commit)
    }

    pub fn set_with<R: Send + Sync + 'static>(
        &self,
        factory: impl Fn() -> A + Send + Sync + 'static,
    ) -> Effect<PublishReport, PubSubError, R> {
        self.hub.publish_with(factory)
    }

    /// Reading the current value and registering the change subscription is
    /// one atomic operation relative to every successful `set` commit.
    pub fn subscribe<R: Send + Sync + 'static>(
        &self,
    ) -> Effect<SubscriptionSnapshot<A>, PubSubError, R> {
        let reference = self.clone();
        Effect::ready(move |_, _| match reference.hub.subscribe_current_now() {
            Ok((value, subscription)) => Exit::Success(SubscriptionSnapshot {
                value,
                subscription,
            }),
            Err(error) => Exit::Fail(error),
        })
    }

    pub fn subscribe_scoped<R: Send + Sync + 'static>(
        &self,
        scope: &Scope<PubSubError>,
    ) -> Effect<ResourceValue<SubscriptionSnapshot<A>>, PubSubError, R> {
        scope.acquire(self.subscribe(), |snapshot| {
            Release::sync(move || {
                snapshot.subscription.close_now();
                drop(snapshot);
            })
        })
    }

    /// The first stream element is the atomic subscription snapshot. Later
    /// elements are committed changes; a bounded-buffer loss is a typed error.
    #[must_use]
    pub fn stream<R: Send + Sync + 'static>(&self) -> Stream<Arc<A>, PubSubError, R> {
        let reference = self.clone();
        Stream::new(move |scope| {
            reference.subscribe_scoped(&scope).map(|resource| {
                let initial =
                    Mutex::new(Some(resource.with(|snapshot| Arc::clone(&snapshot.value))));
                StreamReader::new(Effect::new(move |env, context, finish| {
                    if context.cancelled() {
                        return runtime::finish(finish, Exit::Interrupted);
                    }
                    if let Some(first) = initial
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .take()
                    {
                        return runtime::finish(
                            finish,
                            match first {
                                Ok(value) => Exit::Success(Chunk::Values(vec![value])),
                                Err(defect) => Exit::Die(defect),
                            },
                        );
                    }
                    match resource.with(|snapshot| snapshot.subscription.pull_chunk::<R>()) {
                        Ok(next) => next.execute(env, context, finish),
                        Err(defect) => runtime::finish(finish, Exit::Die(defect)),
                    }
                }))
            })
        })
    }

    pub fn shutdown<R: Send + Sync + 'static>(
        &self,
        mode: PubSubShutdown,
    ) -> Effect<bool, PubSubError, R> {
        self.hub.shutdown(mode)
    }
}
