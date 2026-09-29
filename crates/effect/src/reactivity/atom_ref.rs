use super::{Atom, AtomError, AtomId, AtomKind, AtomRegistry, AtomRegistryError};
use crate::{
    Defect, Effect, Exit, Finalization, PubSubError, PubSubOverflow, PubSubShutdown, PublishReport,
    Semaphore, SemaphoreError, SharedValue, SubscriptionRef,
};
use std::collections::BTreeSet;
use std::num::NonZeroUsize;
use std::sync::{Arc, Mutex};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AtomRefError {
    Registry(AtomRegistryError),
    Publication(PubSubError),
    /// The source mutation committed, but dependents may not have received its
    /// invalidation. The caller must reconcile through authoritative state.
    CommittedWithoutInvalidation(AtomRegistryError),
    ForeignRegistry,
    Admission(SemaphoreError),
    RevisionExhausted,
    InputConsumed,
}

pub enum AtomDependencyError<E: Send + 'static> {
    Read(AtomError<E>),
    ForeignRegistry,
    Unavailable,
    Failed(SharedValue<AtomError<E>>),
    Defect(Defect),
    Interrupted,
    FinalizationFailed(Finalization<SharedValue<AtomError<E>>>),
}

/// A typed source atom. Value snapshots share immutable Arc storage, so
/// consumers never need A: Clone. Every instance belongs to exactly one graph.
pub struct AtomRef<A: Send + Sync + 'static> {
    pub(super) id: AtomId,
    pub(super) registry: AtomRegistry,
    pub(super) value: SubscriptionRef<A>,
    pub(super) revision: Arc<Mutex<std::num::NonZeroU64>>,
    pub(super) mutation: Semaphore,
}

impl<A: Send + Sync + 'static> Clone for AtomRef<A> {
    fn clone(&self) -> Self {
        Self {
            id: self.id.clone(),
            registry: self.registry.clone(),
            value: self.value.clone(),
            revision: Arc::clone(&self.revision),
            mutation: self.mutation.clone(),
        }
    }
}

impl<A: Send + Sync + 'static> AtomRef<A> {
    pub fn new(
        registry: AtomRegistry,
        id: AtomId,
        initial: A,
        capacity: NonZeroUsize,
        overflow: PubSubOverflow,
    ) -> Result<Self, AtomRegistryError> {
        registry.register(id.clone(), AtomKind::Source)?;
        Ok(Self {
            id,
            registry,
            value: SubscriptionRef::new(initial, capacity, overflow),
            revision: Arc::new(Mutex::new(std::num::NonZeroU64::MIN)),
            mutation: Semaphore::full(NonZeroUsize::MIN),
        })
    }

    #[must_use]
    pub fn id(&self) -> &AtomId {
        &self.id
    }

    pub fn read<R: Send + Sync + 'static>(&self) -> Effect<Arc<A>, AtomRefError, R> {
        self.value.read().map_error(AtomRefError::Publication)
    }

    pub fn revision<R: Send + Sync + 'static>(
        &self,
    ) -> Effect<std::num::NonZeroU64, AtomRefError, R> {
        let revision = Arc::clone(&self.revision);
        Effect::sync(move || {
            *revision
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
        })
    }

    pub fn set<R: Send + Sync + 'static>(&self, next: A) -> Effect<PublishReport, AtomRefError, R> {
        let source = self.clone();
        let next = Mutex::new(Some(next));
        self.mutation
            .acquire(NonZeroUsize::MIN)
            .map_error(AtomRefError::Admission)
            .flat_map(move |permit| {
                let source = source.clone();
                let next = next
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .take();
                let mutation = match next {
                    Some(next) => {
                        let upcoming = source
                            .revision
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner)
                            .get()
                            .checked_add(1)
                            .and_then(std::num::NonZeroU64::new);
                        match upcoming {
                            Some(upcoming) => {
                                let revision = Arc::clone(&source.revision);
                                let registry = source.registry.clone();
                                let id = source.id.clone();
                                source
                                    .value
                                    .set_with_commit(next, move |_| {
                                        *revision
                                            .lock()
                                            .unwrap_or_else(std::sync::PoisonError::into_inner) =
                                            upcoming;
                                    })
                                    .map_error(AtomRefError::Publication)
                                    .flat_map(move |report| {
                                        registry
                                            .changed(id.clone())
                                            .map_error(AtomRefError::CommittedWithoutInvalidation)
                                            .map(move |_| report)
                                    })
                            }
                            None => Effect::from_result_once(Err(AtomRefError::RevisionExhausted)),
                        }
                    }
                    None => Effect::from_result_once(Err(AtomRefError::InputConsumed)),
                };
                mutation
                    .uninterruptible()
                    .ensuring(Effect::sync_once(move || drop(permit)))
            })
    }

    #[must_use]
    pub fn stream<R: Send + Sync + 'static>(&self) -> crate::Stream<Arc<A>, PubSubError, R> {
        self.value.stream()
    }

    pub fn dispose<R: Send + Sync + 'static>(
        &self,
        mode: PubSubShutdown,
    ) -> Effect<(), AtomRefError, R> {
        let source = self.clone();
        self.mutation
            .acquire(NonZeroUsize::MIN)
            .map_error(AtomRefError::Admission)
            .flat_map(move |permit| {
                let source = source.clone();
                source
                    .value
                    .shutdown(mode)
                    .map_error(AtomRefError::Publication)
                    .flat_map(move |_| {
                        source
                            .registry
                            .remove(source.id.clone())
                            .map_error(AtomRefError::Registry)
                            .map(|_| ())
                    })
                    .uninterruptible()
                    .ensuring(Effect::sync_once(move || drop(permit)))
            })
    }
}

/// A derived evaluator records only source reads that actually execute. On a
/// successful run, its collected set atomically replaces the previous graph
/// dependencies after cycle validation.
#[derive(Clone)]
pub struct AtomReadContext {
    registry: AtomRegistry,
    dependencies: Arc<Mutex<BTreeSet<AtomId>>>,
}

impl AtomReadContext {
    pub(crate) fn new(registry: AtomRegistry) -> Self {
        Self {
            registry,
            dependencies: Arc::new(Mutex::new(BTreeSet::new())),
        }
    }

    pub fn read<A: Send + Sync + 'static, R: Send + Sync + 'static>(
        &self,
        source: &AtomRef<A>,
    ) -> Effect<Arc<A>, AtomRefError, R> {
        if !self.registry.same_instance(&source.registry) {
            return Effect::from_result_once(Err(AtomRefError::ForeignRegistry));
        }
        let dependencies = Arc::clone(&self.dependencies);
        let id = source.id.clone();
        let source = source.clone();
        Effect::sync(move || {
            dependencies
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .insert(id.clone());
        })
        .flat_map(move |_| source.read())
    }

    pub(crate) fn collected(&self) -> BTreeSet<AtomId> {
        self.dependencies
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    pub fn read_derived<A, E, R>(
        &self,
        derived: &Atom<A, E, R>,
    ) -> Effect<SharedValue<A>, AtomDependencyError<E>, R>
    where
        A: Send + 'static,
        E: Send + 'static,
        R: Send + Sync + 'static,
    {
        if !self.registry.same_instance(derived.registry()) {
            return Effect::from_result_once(Err(AtomDependencyError::ForeignRegistry));
        }
        let dependencies = Arc::clone(&self.dependencies);
        let id = derived.id().clone();
        let derived = derived.clone();
        Effect::sync(move || {
            dependencies
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .insert(id.clone());
        })
        .flat_map(move |_| {
            derived
                .read()
                .map_error(AtomDependencyError::Read)
                .flat_map(|status| {
                    let result = match status.outcome() {
                        Some(Exit::Success(value)) => Ok(value.clone()),
                        Some(Exit::Fail(error)) => Err(AtomDependencyError::Failed(error.clone())),
                        Some(Exit::Die(defect)) => Err(AtomDependencyError::Defect(defect.clone())),
                        Some(Exit::Interrupted) => Err(AtomDependencyError::Interrupted),
                        Some(Exit::FinalizationFailed(details)) => {
                            Err(AtomDependencyError::FinalizationFailed(details.clone()))
                        }
                        None => Err(AtomDependencyError::Unavailable),
                    };
                    Effect::from_result_once(result)
                })
        })
    }
}
