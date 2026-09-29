//! Observable mutable references for reactive graphs.

use super::{Atom, AtomError, AtomId, AtomKind, AtomRegistry, AtomRegistryError};
use crate::{
    Defect, Effect, Exit, Finalization, PubSubError, PubSubOverflow, PubSubShutdown, PublishReport,
    Semaphore, SemaphoreError, SharedValue, SubscriptionRef,
};
use std::collections::BTreeSet;
use std::num::NonZeroUsize;
use std::sync::{Arc, Mutex};

#[derive(Clone, Debug, PartialEq, Eq)]
/// A registry, publication, or admission failure for a mutable atom reference.
pub enum AtomRefError {
    /// The identity or definition registry rejected the operation. Retains the `AtomRegistryError` detail.
    Registry(AtomRegistryError),
    /// Publishing the committed change failed. Retains the `PubSubError` detail.
    Publication(PubSubError),
    /// The source mutation committed, but dependents may not have received its
    /// invalidation. The caller must reconcile through authoritative state.
    CommittedWithoutInvalidation(AtomRegistryError),
    /// The atom belongs to a different registry.
    ForeignRegistry,
    /// The provider rejected work before accepting ownership. Retains the `SemaphoreError` detail.
    Admission(SemaphoreError),
    /// No further revision can be allocated without exceeding its range.
    RevisionExhausted,
    /// An owned input was already consumed.
    InputConsumed,
}

/// A failed or unavailable dependency encountered while evaluating a derived atom.
///
/// # Type parameters
/// - `E`: Domain-error type. Bound or type: `Send + 'static`.
pub enum AtomDependencyError<E: Send + 'static> {
    /// Reading the selected source failed. Retains the `AtomError<E>` detail.
    Read(AtomError<E>),
    /// The atom belongs to a different registry.
    ForeignRegistry,
    /// The required value or capability is unavailable.
    Unavailable,
    /// The underlying operation failed. Retains the `SharedValue<AtomError<E>>` detail.
    Failed(SharedValue<AtomError<E>>),
    /// An unexpected defect occurred. Carries `Defect`.
    Defect(Defect),
    /// Execution was interrupted.
    Interrupted,
    /// The finalization operation failed. Retains the `Finalization<SharedValue<AtomError<E>>>` detail.
    FinalizationFailed(Finalization<SharedValue<AtomError<E>>>),
}

/// A typed source atom. Value snapshots share immutable Arc storage, so
/// consumers never need A: Clone. Every instance belongs to exactly one graph.
///
/// # Type parameters
/// - `A`: Application value type. Bound or type: `Send + Sync + 'static`.
pub struct AtomRef<A: Send + Sync + 'static> {
    pub(super) id: AtomId,
    pub(super) registry: AtomRegistry,
    pub(super) value: SubscriptionRef<A>,
    pub(super) revision: Arc<Mutex<std::num::NonZeroU64>>,
    pub(super) mutation: Semaphore,
}

impl<A: Send + Sync + 'static> Clone for AtomRef<A> {
    ///
    /// # Returns
    /// Type: `Self`.
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
    /// Construct a `AtomRef` from its required inputs and explicit policies.
    ///
    /// # Type parameters
    /// - `A`: Success or application-value type.
    ///
    /// # Parameters
    /// - `registry` (`AtomRegistry`): Registry that owns definitions and rejects identity conflicts.
    /// - `id` (`AtomId`): Stable identity of the selected record or operation.
    /// - `initial` (`A`): Initial value used before any state transition.
    /// - `capacity` (`NonZeroUsize`): The caller-supplied capacity bound.
    /// - `overflow` (`PubSubOverflow`): Behavior when the configured buffer capacity is full.
    ///
    /// # Returns
    /// Type: `Result<Self, AtomRegistryError>`.
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
    /// Return the stable identity associated with this value.
    ///
    /// # Type parameters
    /// - `A`: Application value type. Bound: `Send + Sync + 'static`.
    ///
    /// # Returns
    /// Type: `&AtomId`.
    pub fn id(&self) -> &AtomId {
        &self.id
    }

    /// Read the selected state without assuming it has been modified.
    ///
    /// # Type parameters
    /// - `R`: Environment type. Bounds: `Send + Sync + 'static`.
    /// - `A`: Success or application-value type.
    ///
    /// # Returns
    /// Type: `Effect<Arc<A>, AtomRefError, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    pub fn read<R: Send + Sync + 'static>(&self) -> Effect<Arc<A>, AtomRefError, R> {
        self.value.read().map_error(AtomRefError::Publication)
    }

    /// Return the revision assigned to the current atom value.
    ///
    /// # Type parameters
    /// - `A`: Application value type. Bound: `Send + Sync + 'static`.
    /// - `R`: Environment type. Bounds: `Send + Sync + 'static`.
    ///
    /// # Returns
    /// Type: `Effect<std::num::NonZeroU64, AtomRefError, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
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

    /// Replace the current value through the declared state capability.
    ///
    /// # Type parameters
    /// - `R`: Environment type. Bounds: `Send + Sync + 'static`.
    /// - `A`: Success or application-value type.
    ///
    /// # Parameters
    /// - `next` (`A`): Next value or state produced by the transition.
    ///
    /// # Returns
    /// Type: `Effect<PublishReport, AtomRefError, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
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
    /// Return the stream associated with this value.
    ///
    /// # Type parameters
    /// - `R`: Environment type. Bounds: `Send + Sync + 'static`.
    /// - `A`: Success or application-value type.
    ///
    /// # Returns
    /// Type: `crate::Stream<Arc<A>, PubSubError, R>`.
    pub fn stream<R: Send + Sync + 'static>(&self) -> crate::Stream<Arc<A>, PubSubError, R> {
        self.value.stream()
    }

    /// Release owned native state and await its disposal where required.
    ///
    /// # Type parameters
    /// - `A`: Application value type. Bound: `Send + Sync + 'static`.
    /// - `R`: Environment type. Bounds: `Send + Sync + 'static`.
    ///
    /// # Parameters
    /// - `mode` (`PubSubShutdown`): Selected behavior for this operation.
    ///
    /// # Returns
    /// Type: `Effect<(), AtomRefError, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
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
    /// # Parameters
    /// - `registry` (`AtomRegistry`): Registry that owns definitions and rejects identity conflicts.
    ///
    /// # Returns
    /// Type: `Self`.
    pub(crate) fn new(registry: AtomRegistry) -> Self {
        Self {
            registry,
            dependencies: Arc::new(Mutex::new(BTreeSet::new())),
        }
    }

    /// Read the selected state without assuming it has been modified.
    ///
    /// # Type parameters
    /// - `A`: Application value type. Bounds: `Send + Sync + 'static`.
    /// - `R`: Environment type. Bounds: `Send + Sync + 'static`.
    ///
    /// # Parameters
    /// - `source` (`&AtomRef<A>`): Underlying provider or input from which values are read.
    ///
    /// # Returns
    /// Type: `Effect<Arc<A>, AtomRefError, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
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

    /// # Returns
    /// Type: `BTreeSet<AtomId>`.
    pub(crate) fn collected(&self) -> BTreeSet<AtomId> {
        self.dependencies
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    /// Evaluate a derived atom after checking its dependencies.
    ///
    /// # Type parameters
    /// - `A`: Application value type. Additional bound: `Send + 'static`.
    /// - `E`: Domain-error type. Additional bound: `Send + 'static`.
    /// - `R`: Environment type. Additional bound: `Send + Sync + 'static`.
    ///
    /// # Parameters
    /// - `derived` (`&Atom<A, E, R>`): The derived supplied to this operation.
    ///
    /// # Returns
    /// Type: `Effect<SharedValue<A>, AtomDependencyError<E>, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
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
