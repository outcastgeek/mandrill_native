//! Atomic snapshot-and-subscribe state observation.

use crate::internal::runtime;
use crate::pub_sub::{
    PubSub, PubSubError, PubSubOverflow, PubSubShutdown, PublishReport, Subscription,
};
use crate::{Chunk, Effect, Exit, ResourceValue, Scope, Stream, StreamReader};
use crate::{effect_map, effect_scope_acquire, effect_sync};
use std::num::NonZeroUsize;
use std::sync::{Arc, Mutex};

/// One value and a subscription registered at the same publication boundary.
/// The first `take` sees only changes committed after this snapshot. A lagged
/// subscription can obtain a fresh snapshot through `read` and resubscribe.
///
/// # Type parameters
/// - `A`: Application value type. Bound or type: `Send + Sync + 'static`.
pub struct SubscriptionSnapshot<A: Send + Sync + 'static> {
    /// The value carried by this record. Type: `Arc<A>`.
    pub value: Arc<A>,
    /// Owned subscription used to observe future changes. Type: `Subscription<A>`.
    pub subscription: Subscription<A>,
}

/// A current shared value with bounded change observation. It uses the same
/// atomic gate for `read`/subscription registration and publication, so no
/// change can disappear between obtaining a snapshot and subscribing.
///
/// # Type parameters
/// - `A`: Application value type. Bound or type: `Send + Sync + 'static`.
pub struct SubscriptionRef<A: Send + Sync + 'static> {
    hub: PubSub<A>,
}

impl<A: Send + Sync + 'static> Clone for SubscriptionRef<A> {
    ///
    /// # Returns
    /// Type: `Self`.
    fn clone(&self) -> Self {
        Self {
            hub: self.hub.clone(),
        }
    }
}

impl<A: Send + Sync + 'static> SubscriptionRef<A> {
    #[must_use]
    /// Construct a `SubscriptionRef` from its required inputs and explicit policies.
    ///
    /// # Type parameters
    /// - `A`: Success or application-value type.
    ///
    /// # Parameters
    /// - `initial` (`A`): Initial value used before any state transition.
    /// - `capacity` (`NonZeroUsize`): The caller-supplied capacity bound.
    /// - `overflow` (`PubSubOverflow`): Behavior when the configured buffer capacity is full.
    ///
    /// # Returns
    /// Type: `Self`.
    pub fn new(initial: A, capacity: NonZeroUsize, overflow: PubSubOverflow) -> Self {
        Self {
            hub: PubSub::with_current(capacity, overflow, initial),
        }
    }

    /// Create a `SubscriptionRef` using the supplied factory and capability environment.
    ///
    /// # Type parameters
    /// - `R`: Environment type. Bounds: `Send + Sync + 'static`.
    /// - `A`: Success or application-value type.
    ///
    /// # Parameters
    /// - `initial` (`impl Fn() -> A + Send + Sync + 'static`): Initial value used before any state transition.
    /// - `capacity` (`NonZeroUsize`): The caller-supplied capacity bound.
    /// - `overflow` (`PubSubOverflow`): Behavior when the configured buffer capacity is full.
    ///
    /// # Returns
    /// Type: `Effect<Self, PubSubError, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    pub fn make_with<R: Send + Sync + 'static>(
        initial: impl Fn() -> A + Send + Sync + 'static,
        capacity: NonZeroUsize,
        overflow: PubSubOverflow,
    ) -> Effect<Self, PubSubError, R> {
        effect_sync! { body: move || Self::new(initial(), capacity, overflow), }
    }

    /// Clones only the shared handle. The stored payload is never cloned.
    ///
    /// # Type parameters
    /// - `R`: Environment type. Bounds: `Send + Sync + 'static`.
    /// - `A`: Success or application-value type.
    ///
    /// # Returns
    /// Type: `Effect<Arc<A>, PubSubError, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    pub fn read<R: Send + Sync + 'static>(&self) -> Effect<Arc<A>, PubSubError, R> {
        let reference = self.clone();
        Effect::ready(move |_, _| match reference.hub.current() {
            Ok(value) => Exit::Success(value),
            Err(error) => Exit::Fail(error),
        })
    }

    /// Replace the current value through the declared state capability.
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
    pub fn set<R: Send + Sync + 'static>(&self, value: A) -> Effect<PublishReport, PubSubError, R> {
        self.hub.publish(value)
    }

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
    pub(crate) fn set_with_commit<R: Send + Sync + 'static>(
        &self,
        value: A,
        commit: impl Fn(&A) + Send + Sync + 'static,
    ) -> Effect<PublishReport, PubSubError, R> {
        self.hub.publish_with_commit(value, commit)
    }

    pub(crate) fn set_with_guard<R: Send + Sync + 'static>(
        &self,
        value: A,
        guard: impl Fn(&A) -> bool + Send + Sync + 'static,
    ) -> Effect<PublishReport, PubSubError, R> {
        self.hub.publish_with_guard(value, guard)
    }
    /// Compute and publish a replacement value for this subscription reference.
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
    pub fn set_with<R: Send + Sync + 'static>(
        &self,
        factory: impl Fn() -> A + Send + Sync + 'static,
    ) -> Effect<PublishReport, PubSubError, R> {
        self.hub.publish_with(factory)
    }

    /// Reading the current value and registering the change subscription is
    /// one atomic operation relative to every successful `set` commit.
    ///
    /// # Type parameters
    /// - `R`: Environment type. Bounds: `Send + Sync + 'static`.
    /// - `A`: Success or application-value type.
    ///
    /// # Returns
    /// Type: `Effect<SubscriptionSnapshot<A>, PubSubError, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
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
    /// Type: `Effect<ResourceValue<SubscriptionSnapshot<A>>, PubSubError, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    pub fn subscribe_scoped<R: Send + Sync + 'static>(
        &self,
        scope: &Scope<PubSubError>,
    ) -> Effect<ResourceValue<SubscriptionSnapshot<A>>, PubSubError, R> {
        effect_scope_acquire! {
        scope: scope, acquire: self.subscribe(), release: |snapshot| {
                    crate::Release::sync(move || {
                        snapshot.subscription.close_now();
                        drop(snapshot);
                    })
                },
        }
    }

    /// The first stream element is the atomic subscription snapshot. Later
    /// elements are committed changes; a bounded-buffer loss is a typed error.
    #[must_use]
    /// # Type parameters
    /// - `R`: Environment type. Bounds: `Send + Sync + 'static`.
    /// - `A`: Success or application-value type.
    ///
    /// # Returns
    /// Type: `Stream<Arc<A>, PubSubError, R>`.
    /// Returns a scoped stream with the declared item, error, and environment types.
    pub fn stream<R: Send + Sync + 'static>(&self) -> Stream<Arc<A>, PubSubError, R> {
        let reference = self.clone();
        Stream::new(move |scope| {
            effect_map! { program: reference.subscribe_scoped(&scope), map: |resource| {
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
            }, }
        })
    }

    /// Stop accepting new work and describe the settlement of owned work.
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
        self.hub.shutdown(mode)
    }
}
