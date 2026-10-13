//! Observable mutable references for reactive graphs.

use super::{Atom, AtomError, AtomId, AtomKind, AtomRegistry, AtomRegistryError};
use crate::{
    Defect, Effect, Exit, Finalization, PubSubError, PubSubOverflow, PubSubShutdown, PublishReport,
    Semaphore, SemaphoreError, SharedValue, SubscriptionRef,
};
use crate::{
    effect, effect_map, effect_map_error, effect_resource, effect_sync, effect_try_sync,
    effect_uninterruptible,
};
use std::collections::BTreeMap;
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
    owner: Arc<()>,
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
            owner: Arc::clone(&self.owner),
            value: self.value.clone(),
            revision: Arc::clone(&self.revision),
            mutation: self.mutation.clone(),
        }
    }
}

impl<A: Send + Sync + 'static> AtomRef<A> {
    /// Register a fresh source owner and initial shared value with explicit publication capacity; duplicate/disposed registration fails without replacing a live source.
    pub fn new(
        registry: AtomRegistry,
        id: AtomId,
        initial: A,
        capacity: NonZeroUsize,
        overflow: PubSubOverflow,
    ) -> Result<Self, AtomRegistryError> {
        let owner = registry.register_owned(id.clone(), AtomKind::Source)?;
        Ok(Self {
            id,
            registry,
            owner,
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

    /// Lazily borrow the current source value through a shared Arc; this direct read does not register a derived dependency.
    pub fn read<R: Send + Sync + 'static>(&self) -> Effect<Arc<A>, AtomRefError, R> {
        effect_map_error! { program: self.value.read(), map: AtomRefError::Publication, }
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
        effect_sync! {
            body: move || {
            *revision
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
        }, }
    }

    /// Lazily replace one owned source value after acquiring its mutation permit.
    ///
    /// Registration ownership, version change and invalidation metadata are checked in
    /// the publication guard; a stale handle cannot affect a replacement registration.
    /// Notifications and discarded-value cleanup happen outside the graph gate. A
    /// committed notification failure is distinct from a rejected mutation.
    pub fn set<R: Send + Sync + 'static>(&self, next: A) -> Effect<PublishReport, AtomRefError, R> {
        let source = self.clone();
        let next = Mutex::new(Some(next));
        effect! {
        bind;
        program: effect_map_error! {
            program: self.mutation
                   .acquire(NonZeroUsize::MIN), map: AtomRefError::Admission, },
        body: move |permit| {
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
                                       let committed = Arc::new(Mutex::new(None));
                                       let captured = Arc::clone(&committed);
                                       let commit_registry = registry.clone();
                                       let commit_id = id.clone();
                                       let commit_owner = Arc::clone(&source.owner);
                                       {
                                           let __dsl_program = effect_map_error! {
                                           program: effect_map_error! {
                                           program: source
                                           .value
                                           .set_with_guard(next, move |_| {
                                               let result = commit_registry
                                                   .commit_source(&commit_id, &commit_owner);
                                               let accepted = result.is_ok();
                                               if accepted {
                                                   *revision.lock().unwrap_or_else(
                                                       std::sync::PoisonError::into_inner,
                                                   ) = upcoming;
                                               }
                                               *captured
                                                   .lock()
                                                   .unwrap_or_else(std::sync::PoisonError::into_inner) =
                                                   Some(result);
                                               accepted
                                           }), map: AtomRefError::Publication, }
                                           .materialize_exit(), map: |never: std::convert::Infallible| match never {}, }; effect! {
                                               let committed = committed.clone(); let registry = registry.clone(); let publication = __dsl_program.clone()?; {
                                               let decision = committed
                                                   .lock()
                                                   .unwrap_or_else(std::sync::PoisonError::into_inner)
                                                   .take();
                                               match decision {
                                                   Some(Ok(keys)) => { let __dsl_program = Effect::from_exit_once(publication); let __dsl_cleanup = effect_map_error! { program: registry.notify(keys), map: AtomRefError::CommittedWithoutInvalidation, }; effect_resource! { acquire: effect_sync! { body: || (), }, release: move |(), _| __dsl_cleanup.clone(), body: move |_| __dsl_program.clone(), } },
                                                   Some(Err(error)) => {
                                                       let __dsl_program = effect_try_sync! {
                                                       once; body: {
                                                       let __dsl_result = Err(
                                                       AtomRefError::Registry(error),
                                                   ); move || __dsl_result }, on_error: ::core::convert::identity, }; let __dsl_cleanup = effect_map! { program: Effect::from_exit_once(publication), map: |_| (), }; effect_resource! { acquire: effect_sync! { body: || (), }, release: move |(), _| __dsl_cleanup.clone(), body: move |_| __dsl_program.clone(), } },
                                                   None => {
                                                       {
                                                           let __dsl_program = Effect::from_exit_once(publication); effect! {
                                                           let _ = __dsl_program.clone()?; {
                                                           effect_try_sync! {
                                                               once; body: {
                                                               let __dsl_result = Err(
                                                               AtomRefError::InputConsumed,
                                                           ); move || __dsl_result }, on_error: ::core::convert::identity, }
                                                       }? } }
                                                   }
                                               }
                                           }? } }
                                   }
                                   None => effect_try_sync! { once; body: { let __dsl_result = Err(AtomRefError::RevisionExhausted); move || __dsl_result }, on_error: ::core::convert::identity, },
                               }
                           }
                           None => effect_try_sync! { once; body: { let __dsl_result = Err(AtomRefError::InputConsumed); move || __dsl_result }, on_error: ::core::convert::identity, },
                       };
                       { let __dsl_program = effect_uninterruptible! { program: mutation, }; let __dsl_cleanup = effect_sync! { once; body: move || drop(permit), }; effect_resource! { acquire: effect_sync! { body: || (), }, release: move |(), _| __dsl_cleanup.clone(), body: move |_| __dsl_program.clone(), } }
                   },
        }
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

    /// Lazily close this source's publication channel and remove only its owned registration.
    ///
    /// Late disposal preserves a replacement with the same ID. The acquired mutation
    /// permit is released through protected cleanup, including failures.
    pub fn dispose<R: Send + Sync + 'static>(
        &self,
        mode: PubSubShutdown,
    ) -> Effect<(), AtomRefError, R> {
        let source = self.clone();
        let admission = effect_map_error! {
            program: self.mutation.acquire(NonZeroUsize::MIN),
            map: AtomRefError::Admission,
        };
        effect! {
            let source = source.clone();
            let permit = admission.clone()?;
            let shutdown = effect_map_error! {
                program: source.value.shutdown(mode),
                map: AtomRefError::Publication,
            };
            let mutation = effect! {
                let source = source.clone();
                shutdown.clone()?;
                effect_map_error! {
                    program: source.registry.remove_owned(
                        source.id.clone(), Some(Arc::clone(&source.owner)),
                    ),
                    map: AtomRefError::Registry,
                }?;
            };
            let mutation = effect_uninterruptible! { program: mutation, };
            let cleanup = effect_sync! { once; body: move || drop(permit), };
            effect_resource! {
                acquire: effect_sync! { body: || (), },
                release: move |(), _| cleanup.clone(),
                body: move |_| mutation.clone(),
            }?
        }
    }
}

/// Per-evaluation dependency observations; clones share the first version recorded for each actually executed read.
#[derive(Clone)]
pub struct AtomReadContext {
    registry: AtomRegistry,
    dependencies: Arc<Mutex<BTreeMap<AtomId, Arc<()>>>>,
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
            dependencies: Arc::new(Mutex::new(BTreeMap::new())),
        }
    }

    /// Lazily read a source and record its first observed version for this evaluation.
    ///
    /// Holds the source mutation permit across version and value acquisition. Foreign
    /// registries and stale source owners fail before returning a dependency value;
    /// final publication rechecks the version under the graph gate.
    pub fn read<A: Send + Sync + 'static, R: Send + Sync + 'static>(
        &self,
        source: &AtomRef<A>,
    ) -> Effect<Arc<A>, AtomRefError, R> {
        if !self.registry.same_instance(&source.registry) {
            return effect_try_sync! { once; body: { let __dsl_result = Err(AtomRefError::ForeignRegistry); move || __dsl_result }, on_error: ::core::convert::identity, };
        }
        let dependencies = Arc::clone(&self.dependencies);
        let source = source.clone();
        {
            let __dsl_program = effect_map_error! {
            program: source
            .mutation
            .acquire(NonZeroUsize::MIN), map: AtomRefError::Admission, };
            effect! {
                let dependencies = dependencies.clone(); let source = source.clone(); let permit = __dsl_program.clone()?; {
                let source = source.clone();
                let dependencies = Arc::clone(&dependencies);
                let read = {
                    let __dsl_program = effect_try_sync! {
                    body: {
                        let source = source.clone();
                        move || {
                            source
                                .registry
                                .dependency_version(&source.id, &source.owner)
                                .map_err(AtomRefError::Registry)
                        }
                    }, on_error: std::convert::identity, }; effect! {
                        let dependencies = dependencies.clone(); let source = source.clone(); let version = __dsl_program.clone()?; {
                    let dependencies = Arc::clone(&dependencies);
                    let id = source.id.clone();
                    effect_map! {
                        program: source.read(), map: move |value| {
                        dependencies
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner)
                            .entry(id.clone())
                            .or_insert_with(|| Arc::clone(&version));
                        value
                    }, }
                }? } };
                { let __dsl_program = read; let __dsl_cleanup = effect_sync! { once; body: move || drop(permit), }; effect_resource! { acquire: effect_sync! { body: || (), }, release: move |(), _| __dsl_cleanup.clone(), body: move |_| __dsl_program.clone(), } }
            }? }
        }
    }

    /// # Returns
    /// Type: `BTreeSet<AtomId>`.
    pub(crate) fn collected(&self) -> BTreeMap<AtomId, Arc<()>> {
        self.dependencies
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    /// Lazily read a derived dependency from the same registry and record its version.
    ///
    /// Rejects a replaced owner as Unavailable. Version is captured before status
    /// inspection so a concurrent change conservatively rejects the consuming build
    /// at commit, rather than accepting stale state as a current dependency.
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
            return effect_try_sync! { once; body: { let __dsl_result = Err(AtomDependencyError::ForeignRegistry); move || __dsl_result }, on_error: ::core::convert::identity, };
        }
        let dependencies = Arc::clone(&self.dependencies);
        let id = derived.id().clone();
        let owner = derived.registration_owner();
        let derived = derived.clone();
        let registry = self.registry.clone();
        let version = effect_try_sync! {
        body: move || {
            registry
                .dependency_version(&id, &owner)
                .map(|version| (id.clone(), version))
                .map_err(|_| AtomDependencyError::Unavailable)
        }, on_error: std::convert::identity, };
        effect! {
            let dependencies = Arc::clone(&dependencies);
            let derived = derived.clone();
            let (id, version) = version.clone()?;
            dependencies
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .entry(id)
                .or_insert(version);
            let status = effect_map_error! {
                program: derived.read(),
                map: AtomDependencyError::Read,
            }?;
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
            effect_try_sync! {
                once;
                body: move || result,
                on_error: std::convert::identity,
            }?
        }
    }
}
