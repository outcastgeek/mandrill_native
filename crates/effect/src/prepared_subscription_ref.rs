//! Prepared subscription reference backed by the same atomic publication gate.
use crate::memory::{MemoryError, PreparedMemory, RegionId};
use crate::{
    PreparedCancellation, PreparedPanicPool, PreparedPayloadView, PreparedPubSub,
    PreparedPubSubCapacity, PreparedPubSubDemand, PreparedPubSubError, PreparedPublication,
    PreparedPublish, PreparedSubscription, PubSubError, PubSubOptions, PubSubOverflow,
    PubSubShutdown, ReplayPolicy,
};
use std::num::NonZeroUsize;

/// Prepared current value and its associated bounded change hub.
pub struct PreparedSubscriptionRef<T: Send + Sync + 'static> {
    hub: PreparedPubSub<T>,
}
/// Atomic current snapshot and unique subscription close authority.
pub struct PreparedSubscriptionSnapshot<T: Send + Sync + 'static> {
    /// Current value retained independently of subsequent publications.
    pub current: PreparedPayloadView<T>,
    /// Changes registered atomically with the current observation.
    pub changes: PreparedSubscription<T>,
}
/// A preadmitted publication and its not-yet-invoked factory.
pub struct PreparedSubscriptionSet<
    T: Send + Sync + 'static,
    F: FnOnce() -> Result<T, PubSubError> + Send + 'static,
> {
    publication: PreparedPublication<T>,
    factory: F,
}
impl<T: Send + Sync + 'static> PreparedSubscriptionRef<T> {
    /// Report the physical hub demand, with separately reported startup Arc controls.
    pub fn demand(
        capacity: NonZeroUsize,
        overflow: PubSubOverflow,
        physical: PreparedPubSubCapacity,
    ) -> Result<PreparedPubSubDemand, PreparedPubSubError> {
        PreparedPubSub::<T>::demand(
            PubSubOptions {
                capacity,
                overflow,
                replay: ReplayPolicy::None,
            },
            physical,
            true,
        )
    }
    /// Prepare the entire hub and reserve its initial cell before consuming `initial`.
    /// A physical or policy refusal returns the original value.
    #[allow(clippy::result_large_err)]
    pub fn prepare(
        memory: &PreparedMemory<'static>,
        region: RegionId,
        capacity: NonZeroUsize,
        overflow: PubSubOverflow,
        physical: PreparedPubSubCapacity,
        panics: &PreparedPanicPool,
        initial: T,
    ) -> Result<Self, (PreparedPubSubError, T)> {
        let hub = PreparedPubSub::prepare_current(
            memory,
            region,
            PubSubOptions {
                capacity,
                overflow,
                replay: ReplayPolicy::None,
            },
            physical,
            panics,
            initial,
        )?;
        Ok(Self { hub })
    }
    /// Fallibly retain this same prepared reference.
    pub fn try_clone(&self) -> Result<Self, MemoryError> {
        Ok(Self {
            hub: self.hub.try_clone()?,
        })
    }
    /// Retain the current value without cloning T.
    pub fn read_now(&self) -> Result<PreparedPayloadView<T>, PreparedPubSubError> {
        self.hub.current()
    }
    /// Observe current and register changes under the same publication gate.
    pub fn subscribe_now(&self) -> Result<PreparedSubscriptionSnapshot<T>, PreparedPubSubError> {
        let (current, changes) = self.hub.subscribe_current()?;
        Ok(PreparedSubscriptionSnapshot { current, changes })
    }
    /// Reserve the physical publication and fanout before a delayed set factory.
    pub fn admit_set(&self) -> Result<PreparedPublication<T>, MemoryError> {
        self.hub.admit_publication()
    }
    /// Reserve the publication before moving a replacement into its factory.
    #[allow(clippy::result_large_err, clippy::type_complexity)]
    pub fn set(
        &self,
        value: T,
    ) -> Result<
        PreparedSubscriptionSet<T, impl FnOnce() -> Result<T, PubSubError> + Send + 'static>,
        (MemoryError, T),
    > {
        let publication = match self.hub.admit_publication() {
            Ok(value) => value,
            Err(error) => return Err((error, value)),
        };
        Ok(PreparedSubscriptionSet {
            publication,
            factory: move || Ok(value),
        })
    }
    /// Reserve publication and fanout owners before invoking the factory.
    #[allow(clippy::result_large_err, clippy::type_complexity)]
    pub fn set_with<F: FnOnce() -> T + Send + 'static>(
        &self,
        factory: F,
    ) -> Result<
        PreparedSubscriptionSet<T, impl FnOnce() -> Result<T, PubSubError> + Send + 'static>,
        (MemoryError, F),
    > {
        let publication = match self.hub.admit_publication() {
            Ok(value) => value,
            Err(error) => return Err((error, factory)),
        };
        Ok(PreparedSubscriptionSet {
            publication,
            factory: move || Ok(factory()),
        })
    }
    /// First shutdown mode wins, preserving the hub's existing lifecycle.
    pub fn shutdown_now(&self, mode: PubSubShutdown) -> Result<bool, MemoryError> {
        self.hub.shutdown_now(mode)
    }
}
impl<T: Send + Sync + 'static, F: FnOnce() -> Result<T, PubSubError> + Send + 'static>
    PreparedSubscriptionSet<T, F>
{
    /// Attach a physically admitted cancellation listener; refusal retains this set.
    #[allow(clippy::result_large_err)]
    pub fn run(
        self,
        token: &PreparedCancellation,
    ) -> Result<PreparedPublish<T, F>, (MemoryError, Self)> {
        match self.publication.run(token, self.factory) {
            Ok(future) => Ok(future),
            Err((error, publication, factory)) => Err((
                error,
                Self {
                    publication,
                    factory,
                },
            )),
        }
    }
    /// Attach an existing prepared native context.
    pub fn run_with_context<E: Send + 'static>(
        self,
        context: crate::internal::runtime::prepared_graph::FutureContext<E>,
    ) -> PreparedPublish<T, F> {
        self.publication.run_with_context(context, self.factory)
    }
    // The existing trusted contract forbids user callbacks/reentry under the gate.
    #[allow(dead_code)] // Invoked by the prepared metadata integration in a later increment.
    pub(crate) fn run_guarded_with_context<
        E: Send + 'static,
        G: Fn(&T) -> bool + Send + 'static,
    >(
        self,
        context: crate::internal::runtime::prepared_graph::FutureContext<E>,
        guard: G,
    ) -> PreparedPublish<T, F, G> {
        self.publication
            .run_guarded_with_context(context, self.factory, guard)
    }
    // Commit hooks share the same trusted, metadata-only gate contract.
    #[allow(dead_code)]
    pub(crate) fn run_commit_with_context<E: Send + 'static, G: Fn(&T) + Send + 'static>(
        self,
        context: crate::internal::runtime::prepared_graph::FutureContext<E>,
        commit: G,
    ) -> PreparedPublish<T, F, impl Fn(&T) -> bool + Send + 'static> {
        self.run_guarded_with_context(context, move |value| {
            commit(value);
            true
        })
    }
}
