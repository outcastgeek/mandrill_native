//! Typed reactive atom descriptions and dependencies.

use super::{AsyncResult, AtomId, AtomKind, AtomReadContext, AtomRegistry, AtomRegistryError};
use super::{InvalidationBatch, InvalidationError};
use crate::pub_sub::{Subscription, SubscriptionEvent};
use crate::{
    Effect, Exit, Fiber, HasTaskSpawner, PubSubError, PubSubOverflow, PubSubShutdown,
    ResourceValue, Scope, Semaphore, SemaphoreError, SubscriptionRef,
};
use crate::{
    effect, effect_die, effect_map, effect_map_error, effect_resource, effect_scope_acquire,
    effect_sync, effect_try_sync, effect_uninterruptible,
};
use std::num::NonZeroUsize;
use std::sync::{Arc, Mutex};

/// Live evaluators use Rust trait-object dispatch. Hydration transfers only
/// encoded state, never this callback or its process-local vtable.
///
/// # Type parameters
/// - `A`: Application value type.
/// - `E`: Domain-error type.
/// - `R`: Environment type.
type AtomEvaluator<A, E, R> = dyn Fn(AtomReadContext) -> Effect<A, E, R> + Send + Sync;
/// Type alias for atom read effect in reactivity atom.
///
/// # Type parameters
/// - `A`: Application value type.
/// - `E`: Domain-error type.
/// - `R`: Environment type.
pub type AtomReadEffect<A, E, R> = Effect<Arc<AsyncResult<A, AtomError<E>>>, AtomError<E>, R>;

#[derive(Debug)]
/// A typed read, write, or publication failure for an atom.
///
/// # Type parameters
/// - `E`: Domain-error type.
pub enum AtomError<E: Send + 'static> {
    /// Evaluating the reactive definition failed. Retains the `E` detail.
    Evaluation(E),
    /// The identity or definition registry rejected the operation. Retains the `AtomRegistryError` detail.
    Registry(AtomRegistryError),
    /// Publishing the committed change failed. Retains the `PubSubError` detail.
    Publication(PubSubError),
    /// The provider rejected work before accepting ownership. Retains the `SemaphoreError` detail.
    Admission(SemaphoreError),
    /// The status publication committed, but dependent invalidation failed.
    CommittedWithoutInvalidation(AtomRegistryError),
    /// A publication failed after observers acquired shared ownership of the evaluator error.
    ///
    /// Inspect the original error with SharedValue::with; the non-Clone error is retained
    /// instead of replacing it with an input-consumed failure.
    RetainedFailure(crate::SharedValue<AtomError<E>>),
    /// An owned input was already consumed.
    InputConsumed,
}

/// A typed derived atom. The evaluator reads sources through AtomReadContext,
/// so the graph records only dependencies used by that evaluation. Each atom
/// owns its status and commit gate; registries share neither values nor R.
///
/// # Type parameters
/// - `A`: Application value type.
/// - `E`: Domain-error type.
/// - `R`: Environment type.
pub struct Atom<A, E, R>
where
    A: Send + 'static,
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    id: AtomId,
    registry: AtomRegistry,
    owner: Arc<()>,
    status: SubscriptionRef<AsyncResult<A, AtomError<E>>>,
    evaluate: Arc<AtomEvaluator<A, E, R>>,
    commit: Semaphore,
}

impl<A, E, R> Clone for Atom<A, E, R>
where
    A: Send + 'static,
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    ///
    /// # Returns
    /// Type: `Self`.
    fn clone(&self) -> Self {
        Self {
            id: self.id.clone(),
            registry: self.registry.clone(),
            owner: Arc::clone(&self.owner),
            status: self.status.clone(),
            evaluate: Arc::clone(&self.evaluate),
            commit: self.commit.clone(),
        }
    }
}

impl<A, E, R> Atom<A, E, R>
where
    A: Send + 'static,
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    /// Register a fresh derived owner and describe its evaluator; evaluation starts only when refresh executes.
    pub fn new(
        registry: AtomRegistry,
        id: AtomId,
        capacity: NonZeroUsize,
        overflow: PubSubOverflow,
        evaluate: impl Fn(AtomReadContext) -> Effect<A, E, R> + Send + Sync + 'static,
    ) -> Result<Self, AtomRegistryError> {
        let owner = registry.register_owned(id.clone(), AtomKind::Derived)?;
        Ok(Self {
            id,
            registry,
            owner,
            status: SubscriptionRef::new(AsyncResult::Initial, capacity, overflow),
            evaluate: Arc::new(evaluate),
            commit: Semaphore::full(NonZeroUsize::MIN),
        })
    }

    #[must_use]
    /// Return the stable identity associated with this value.
    ///
    /// # Type parameters
    /// - `A`: Application value type.
    /// - `E`: Domain-error type.
    /// - `R`: Environment type.
    ///
    /// # Returns
    /// Type: `&AtomId`.
    pub fn id(&self) -> &AtomId {
        &self.id
    }

    /// Share the registration identity for owner-checked dependency reads; does not borrow or inspect payload state.
    #[must_use]
    pub(crate) fn registration_owner(&self) -> Arc<()> {
        Arc::clone(&self.owner)
    }

    pub(crate) fn registry(&self) -> &AtomRegistry {
        &self.registry
    }

    /// Read the selected state without assuming it has been modified.
    ///
    /// # Type parameters
    /// - `A`: Success or application-value type.
    /// - `E`: Domain-error type.
    /// - `R`: Environment type.
    ///
    /// # Returns
    /// Type: `AtomReadEffect<A, E, R>`.
    pub fn read(&self) -> AtomReadEffect<A, E, R> {
        effect_map_error! { program: self.status.read(), map: AtomError::Publication, }
    }

    #[must_use]
    /// Return the stream associated with this value.
    ///
    /// # Type parameters
    /// - `A`: Success or application-value type.
    /// - `E`: Domain-error type.
    /// - `R`: Environment type.
    ///
    /// # Returns
    /// Type: `crate::Stream<Arc<AsyncResult<A, AtomError<E>>>, PubSubError, R>`.
    pub fn stream(&self) -> crate::Stream<Arc<AsyncResult<A, AtomError<E>>>, PubSubError, R> {
        self.status.stream()
    }

    /// Lazily evaluate and publish a current derived generation using tracked dependency versions.
    ///
    /// Rejects a stale registration owner before calling the evaluator. Successful
    /// builds commit dependency edges and status atomically against observed versions;
    /// a raced/stale build cannot publish its value. A published evaluator failure is
    /// stored in settled status; publication rejection retains evaluator defects,
    /// interruption and cleanup outcomes in the refresh result. Notifications run
    /// outside graph locks and committed notification failures remain distinguishable.
    pub fn refresh(&self) -> Effect<(), AtomError<E>, R> {
        let atom = self.clone();
        {
            let __dsl_program = {
                let __dsl_program = effect_map_error! {
                program: self.commit
                .acquire(NonZeroUsize::MIN), map: AtomError::Admission, };
                effect! {
                    let atom = atom.clone(); let permit = __dsl_program.clone()?; {
                    let atom = atom.clone();
                    let beginning = {
                        let __dsl_program = effect_try_sync! {
                        body: {
                            let atom = atom.clone();
                            move || {
                                atom.registry
                                    .begin_evaluation_owned(&atom.id, &atom.owner)
                                    .map_err(AtomError::Registry)
                            }
                        }, on_error: std::convert::identity, }; effect! {
                            let atom = atom.clone(); let generation = __dsl_program.clone()?; {
                        let atom = atom.clone();
                        {
                            let __dsl_program = effect_map_error! {
                            program: atom.status
                            .read(), map: AtomError::Publication, }; effect! {
                                let atom = atom.clone(); let snapshot = __dsl_program.clone()?; {
                                let previous = snapshot.current_success().cloned();
                                let running = AsyncResult::Running {
                                    generation,
                                    refreshing: previous.is_some(),
                                    previous_success: previous.clone(),
                                };
                                effect_map! {
                                    program: effect_map_error! {
                                    program: atom.status
                                    .set(running), map: AtomError::Publication, }, map: move |_| (generation, previous.clone()), }
                            }? } }
                    }? } };
                    { let __dsl_program = beginning; let __dsl_cleanup = effect_sync! { once; body: move || drop(permit), }; effect_resource! { acquire: effect_sync! { body: || (), }, release: move |(), _| __dsl_cleanup.clone(), body: move |_| __dsl_program.clone(), } }
                }? }
            };
            let atom = self.clone();
            effect! {
                           let __dsl_program = __dsl_program.clone(); let atom = atom.clone(); let (generation , previous) = __dsl_program?; {
                               let atom = atom.clone();
                               let context = AtomReadContext::new(atom.registry.clone());
                               let evaluator = Arc::clone(&atom.evaluate);
                               {
                                   let __dsl_program = effect_map_error! {
                                   program: (evaluator)(context.clone())
                                   .materialize_exit(), map: |never: std::convert::Infallible| match never {}, }; effect! {
                                       let __dsl_program = __dsl_program.clone(); let atom = atom.clone(); let context = context.clone(); let previous = previous.clone(); let outcome = __dsl_program?; {
                                       let pending = Arc::new(Mutex::new(Some((outcome, previous.clone()))));
                                       let atom = atom.clone();
                                       let context = context.clone();
                                       {
                                           let __dsl_program = effect_map_error! {
                                           program: atom.commit
                                           .acquire(NonZeroUsize::MIN), map: AtomError::Admission, }; effect! {
                                               let __dsl_program = __dsl_program.clone(); let atom = atom.clone(); let context = context.clone(); let pending = pending.clone(); let permit = __dsl_program.clone()?; {
                                               let atom = atom.clone();
                                               let context = context.clone();
                                               let pending = Arc::clone(&pending);
                                               let commit = {
                                                   let __dsl_factory = move || {
                                                   let pending = pending
                                                       .lock()
                                                       .unwrap_or_else(std::sync::PoisonError::into_inner)
                                                       .take();
                                                   let Some((outcome, previous)) = pending else {
                                                       return effect_try_sync! {
                                                           once; body: {
                                                           let __dsl_result = Err(
                                                           AtomError::InputConsumed,
                                                       ); move || __dsl_result }, on_error: ::core::convert::identity, };
                                                   };
                                                   if !atom.registry.is_current(&atom.id, generation) {
                                                       return Effect::from_exit_once(discard_value(outcome.map_error(AtomError::Evaluation)));
                                                   }
                                                   let outcome=outcome.map_error(AtomError::Evaluation);
                                                   let observed=if matches!(&outcome,Exit::Success(_)){Some(context.collected())}else{None};
                                                   let previous_for_stale=previous.clone();let status=AsyncResult::settle(generation,outcome,previous);let rejected=Arc::new(Mutex::new(Some(status.clone())));
                                                   let decision=Arc::new(Mutex::new(None));let captured=Arc::clone(&decision);let registry=atom.registry.clone();let id=atom.id.clone();let commit_registry=registry.clone();let commit_id=id.clone();let atom_for_stale=atom.clone();
                                                   {
                                                       let __dsl_program = effect_map_error! { program: effect_map_error! { program: atom.status.set_with_guard(status,move|_|{let result=commit_registry.commit_evaluation(&commit_id,generation,observed.as_ref());let accepted=matches!(&result,Ok((true,_)));*captured.lock().unwrap_or_else(std::sync::PoisonError::into_inner)=Some(result);accepted}), map: AtomError::Publication, }.materialize_exit(), map: |never:std::convert::Infallible|match never{}, }; effect! {
                                                       let __dsl_program = __dsl_program.clone(); let atom_for_stale = atom_for_stale.clone(); let decision = decision.clone(); let id = id.clone(); let previous_for_stale = previous_for_stale.clone(); let registry = registry.clone(); let rejected = rejected.clone(); let publication = __dsl_program?; (move || {
                                                       let retained=rejected.lock().unwrap_or_else(std::sync::PoisonError::into_inner).take();
                                                       let decision=decision.lock().unwrap_or_else(std::sync::PoisonError::into_inner).take();
                                                       if !matches!(&publication,Exit::Success(_)){let outcome=rejected_outcome(retained);let result={ let __dsl_program = Effect::from_exit_once(outcome); let __dsl_cleanup = Effect::from_exit_once(discard_value(publication)); effect_resource! { acquire: effect_sync! { body: || (), }, release: move |(), _| __dsl_cleanup.clone(), body: move |_| __dsl_program.clone(), } };return match decision{Some(Ok((true,keys)))=>{ let __dsl_program = result; let __dsl_cleanup = effect_map_error! { program: registry.notify(keys), map: AtomError::CommittedWithoutInvalidation, }; effect_resource! { acquire: effect_sync! { body: || (), }, release: move |(), _| __dsl_cleanup.clone(), body: move |_| __dsl_program.clone(), } },Some(Err(error))=>{ let __dsl_program = result; let __dsl_cleanup = effect_try_sync! { once; body: { let __dsl_result = Err(AtomError::Registry(error)); move || __dsl_result }, on_error: ::core::convert::identity, }; effect_resource! { acquire: effect_sync! { body: || (), }, release: move |(), _| __dsl_cleanup.clone(), body: move |_| __dsl_program.clone(), } },Some(Ok((false,_)))|None=>result};}
                                                       match decision{
                                                           Some(Ok((true,keys)))=>effect_map_error! { program: registry.notify(keys), map: AtomError::CommittedWithoutInvalidation, },
                                                           Some(Ok((false,keys)))=>{
                                                               let outcome=rejected_outcome(retained);
                                                               if !registry.owns_evaluation(&id,generation){return Effect::from_exit_once(outcome);}
                                                               let registry=registry.clone();let previous=previous_for_stale.clone();
                                                               {
                                                                   let __dsl_program = Effect::from_exit_once(outcome); let __dsl_cleanup = effect! {
            bind;
            program: effect_map_error! { program: atom_for_stale.status.set(AsyncResult::settle(generation,Exit::Interrupted,previous)), map: AtomError::Publication, },
            body: move|_|effect_map_error! { program: registry.notify(keys.clone()), map: AtomError::CommittedWithoutInvalidation, },
            }; effect_resource! { acquire: effect_sync! { body: || (), }, release: move |(), _| __dsl_cleanup.clone(), body: move |_| __dsl_program.clone(), } }
                                                           },
                                                           Some(Err(error))=>{
                                                               let error=AtomError::Registry(error);let previous=previous_for_stale.clone();
                                                               // Preserve evaluator failures and append registry/settlement failures.
                                                               { let __dsl_program = { let __dsl_program = Effect::from_exit_once(rejected_outcome(retained)); let __dsl_cleanup = effect_try_sync! { once; body: { let __dsl_result = Err(error); move || __dsl_result }, on_error: ::core::convert::identity, }; effect_resource! { acquire: effect_sync! { body: || (), }, release: move |(), _| __dsl_cleanup.clone(), body: move |_| __dsl_program.clone(), } }; let __dsl_cleanup = effect_map! { program: effect_map_error! { program: atom_for_stale.status.set(AsyncResult::settle(generation,Exit::Interrupted,previous)), map: AtomError::Publication, }, map: |_|(), }; effect_resource! { acquire: effect_sync! { body: || (), }, release: move |(), _| __dsl_cleanup.clone(), body: move |_| __dsl_program.clone(), } }
                                                           },
                                                           None=>effect_try_sync! { once; body: { let __dsl_result = Err(AtomError::InputConsumed); move || __dsl_result }, on_error: ::core::convert::identity, },
                                                       }
                                                   })()? } }
                                               }; effect! { __dsl_factory()? } };
                                               { let __dsl_program = effect_uninterruptible! { program: commit, }; let __dsl_cleanup = effect_sync! { once; body: move || drop(permit), }; effect_resource! { acquire: effect_sync! { body: || (), }, release: move |(), _| __dsl_cleanup.clone(), body: move |_| __dsl_program.clone(), } }
                                           }? } }
                                   }? } }
                           }? }
        }
    }

    /// Lazily shut down this status channel and remove only the registration owned by this handle.
    ///
    /// A replacement ID is preserved. Shutdown and graph notification failures retain
    /// their typed channels, and cleanup is protected from interruption.
    pub fn dispose(&self, mode: PubSubShutdown) -> Effect<(), AtomError<E>, R> {
        let atom = self.clone();
        {
            let __dsl_program = effect_map_error! {
            program: self.commit
            .acquire(NonZeroUsize::MIN), map: AtomError::Admission, };
            effect! {
                let atom = atom.clone(); let permit = __dsl_program.clone()?; {
                let atom = atom.clone();
                let release = {
                    let __dsl_program = effect_map_error! {
                    program: atom
                    .status
                    .shutdown(mode), map: AtomError::Publication, }; effect! {
                        let atom = atom.clone(); let _ = __dsl_program.clone()?; {
                        effect_map! {
                            program: effect_map_error! {
                            program: atom.registry
                            .remove_owned(atom.id.clone(), Some(Arc::clone(&atom.owner))), map: AtomError::Registry, }, map: |_| (), }
                    }? } };
                { let __dsl_program = effect_uninterruptible! { program: release, }; let __dsl_cleanup = effect_sync! { once; body: move || drop(permit), }; effect_resource! { acquire: effect_sync! { body: || (), }, release: move |(), _| __dsl_cleanup.clone(), body: move |_| __dsl_program.clone(), } }
            }? }
        }
    }

    /// Register observation before the first refresh. The scope owns the
    /// subscription and the driver fiber; closing it cancels and joins the
    /// driver through the TaskSpawner supplied by R.
    ///
    /// # Type parameters
    /// - `A`: Application value type.
    /// - `E`: Domain-error type.
    /// - `R`: Environment type.
    ///
    /// # Parameters
    /// - `scope` (`&Scope<AtomError<E>>`): Owner that joins child work and releases resources at close.
    ///
    /// # Returns
    /// Type: `Effect<Fiber<(), AtomError<E>>, AtomError<E>, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    pub fn observe(
        &self,
        scope: &Scope<AtomError<E>>,
    ) -> Effect<Fiber<(), AtomError<E>>, AtomError<E>, R>
    where
        R: HasTaskSpawner,
    {
        let atom = self.clone();
        let scope = scope.clone();
        let subscription = effect_map_error! {
        program: self
        .registry
        .invalidations()
        .subscribe(), map: |error| AtomError::Registry(AtomRegistryError::Invalidation(error)), };
        {
            let __dsl_program = effect_scope_acquire! {
            scope: scope, acquire: subscription, release: |subscription| {
                            crate::Release::sync(move || {
                                subscription.close_now();
                            })
                        },
            };
            effect! {
            let atom = atom.clone(); let scope = scope.clone(); let resource = __dsl_program.clone()?; {
                        let atom = atom.clone();
                        let driver = { let __dsl_program = atom.refresh(); let atom = atom.clone(); effect! { let __dsl_program = __dsl_program.clone(); let atom = atom.clone(); let resource = resource.clone(); let _ = __dsl_program?; drive_atom(atom.clone(), resource.clone())? } };
                        scope.fork(driver)
                    }? }
        }
    }
}

///
/// # Type parameters
/// - `A`: Successful application-value type. Additional bound: `Send + 'static`.
/// - `E`: Expected application-error type. Additional bound: `Send + 'static`.
/// - `R`: Environment type supplying required capabilities. Additional bound: `HasTaskSpawner + Send + Sync + 'static`.
///
/// # Parameters
/// - `atom` (`Atom<A, E, R>`): The atom supplied to this operation.
/// - `subscription` (`ResourceValue<Subscription<InvalidationBatch>>`): The subscription supplied to this operation.
///
/// # Returns
/// Type: `Effect<(), AtomError<E>, R>`.
/// Returns a lazy `Effect` with the declared success, error, and environment types.
fn drive_atom<A, E, R>(
    atom: Atom<A, E, R>,
    subscription: ResourceValue<Subscription<InvalidationBatch>>,
) -> Effect<(), AtomError<E>, R>
where
    A: Send + 'static,
    E: Send + 'static,
    R: HasTaskSpawner + Send + Sync + 'static,
{
    {
        let __dsl_factory = move || {
            let atom = atom.clone();
            let subscription = subscription.clone();
            let take = match subscription.with(|value| value.take::<R>()) {
                Ok(take) => effect_map_error! {
                    program: take, map: |error| {
                    AtomError::Registry(AtomRegistryError::Invalidation(InvalidationError::PubSub(
                        error,
                    )))
                }, },
                Err(defect) => return effect_die! { defect: defect, },
            };
            {
                let __dsl_program = take;
                effect! {
                    let __dsl_program = __dsl_program.clone(); let atom = atom.clone(); let subscription = subscription.clone(); let event = __dsl_program.clone()?; (move || {
                    let relevant = match event {
                        SubscriptionEvent::Item(batch) => {
                            let dependencies = match atom.registry.dependencies_of(&atom.id) {
                                Ok(dependencies) => dependencies,
                                Err(error) => {
                                    return effect_try_sync! { once; body: { let __dsl_result = Err(AtomError::Registry(error)); move || __dsl_result }, on_error: ::core::convert::identity, };
                                }
                            };
                            batch.keys.iter().any(|key| {
                                dependencies
                                    .iter()
                                    .any(|dependency| dependency.invalidation_key() == *key)
                            })
                        }
                        SubscriptionEvent::Lagged(_) => true,
                        SubscriptionEvent::End => return effect_sync! { body: || (), },
                    };
                    if relevant {
                        let atom_for_next = atom.clone();
                        let subscription_for_next = subscription.clone();
                        {
                            let __dsl_program = atom.refresh(); effect! {
                            let atom_for_next = atom_for_next.clone(); let subscription_for_next = subscription_for_next.clone(); let _ = __dsl_program.clone()?; {
                            drive_atom(atom_for_next.clone(), subscription_for_next.clone())
                        }? } }
                    } else {
                        drive_atom(atom.clone(), subscription.clone())
                    }
                })()? }
            }
        };
        effect! { __dsl_factory()? }
    }
}

/// Recover a rejected evaluator's full outcome while discarding only its success
/// value. Retain a shared error if publication already transferred ownership.
fn rejected_outcome<A: Send + 'static, E: Send + 'static>(
    status: Option<AsyncResult<A, AtomError<E>>>,
) -> Exit<(), AtomError<E>> {
    match status {
        Some(AsyncResult::Settled { outcome, .. }) => discard_value(outcome).map_error(|error| {
            error
                .try_into_owned()
                .unwrap_or_else(AtomError::RetainedFailure)
        }),
        Some(AsyncResult::Initial | AsyncResult::Running { .. }) | None => {
            Exit::Fail(AtomError::InputConsumed)
        }
    }
}

fn discard_value<A, E>(outcome: Exit<A, E>) -> Exit<(), E> {
    match outcome {
        Exit::Success(_) => Exit::Success(()),
        Exit::Fail(error) => Exit::Fail(error),
        Exit::Die(defect) => Exit::Die(defect),
        Exit::Interrupted => Exit::Interrupted,
        Exit::FinalizationFailed(details) => Exit::FinalizationFailed(details),
    }
}

#[cfg(test)]
mod correction_tests {
    use crate::{effect_resource, effect_sync, effect_try_sync, reactive_derived};
    use std::pin::pin;

    use super::*;
    use crate::{
        CleanupFailure, KeyDomain, KeyNamespace, KeySegment, PrimaryOutcome, PubSubOptions,
        ReplayPolicy, StableKey,
    };
    struct Domain;
    impl KeyDomain for Domain {
        const TAG: &'static str = "rejected-atom-test";
    }
    #[derive(Debug)]
    struct OwnedError(String);
    #[test]
    fn publication_rejection_preserves_owned_evaluator_and_cleanup_failures() -> Result<(), String>
    {
        futures_executor::block_on(async {
            let capacity = NonZeroUsize::MIN;
            let registry = AtomRegistry::new(PubSubOptions {
                capacity,
                overflow: PubSubOverflow::DropOldest,
                replay: ReplayPolicy::None,
            })
            .map_err(|error| format!("atom registry: {error:?}"))?;
            let namespace = KeyNamespace::<Domain>::new("atom")
                .map_err(|error| format!("atom namespace: {error:?}"))?;
            let id = AtomId::from_stable(
                StableKey::new(&namespace, [KeySegment::Text("derived".into())])
                    .map_err(|error| format!("derived atom identity: {error:?}"))?,
            );
            let evaluate = |_| {
                let __dsl_program = effect_try_sync! { body: { let __dsl_error = || OwnedError(String::from("evaluation")); move || Err(__dsl_error()) }, on_error: ::core::convert::identity, };
                let __dsl_cleanup = effect_try_sync! { body: { let __dsl_error = || OwnedError(String::from("cleanup")); move || Err(__dsl_error()) }, on_error: ::core::convert::identity, };
                effect_resource! { acquire: effect_sync! { body: || (), }, release: move |(), _| __dsl_cleanup.clone(), body: move |_| __dsl_program.clone(), }
            };
            let atom: Atom<(), OwnedError, ()> = reactive_derived! {
                registry: registry.clone(),
                id: id.clone(),
                capacity: capacity,
                overflow: PubSubOverflow::Backpressure,
                evaluate: evaluate,
            }
            .map_err(|error| format!("atom construction: {error:?}"))?;
            let Exit::Success(snapshot) = atom.status.subscribe().run(Arc::new(())).await else {
                return Err(String::from("atom status subscription must succeed"));
            };
            let subscription = snapshot.subscription;
            let future = atom.refresh().run(Arc::new(()));
            let future = pin!(future);
            let drain = async {
                let first = subscription.take::<()>().run(Arc::new(())).await;
                assert!(matches!(first, Exit::Success(_)));
                registry
                    .begin_evaluation(&id)
                    .map_err(|error| format!("begin evaluation: {error:?}"))?;
                while matches!(
                    subscription.take::<()>().run(Arc::new(())).await,
                    Exit::Success(SubscriptionEvent::Item(_))
                ) {}
                Ok::<(), String>(())
            };
            let drain = pin!(drain);
            let outcome = match futures_util::future::select(future, drain).await {
                futures_util::future::Either::Left((outcome, _)) => outcome,
                futures_util::future::Either::Right((result, _)) => {
                    result?;
                    return Err(String::from("drain cannot finish first"));
                }
            };
            assert!(
                matches!(outcome,Exit::FinalizationFailed(ref details) if matches!(&details.primary,PrimaryOutcome::Failed(AtomError::Evaluation(OwnedError(message))) if message=="evaluation")&&matches!(&details.first,CleanupFailure::ReleaseFailure(AtomError::Evaluation(OwnedError(message))) if message=="cleanup"))
            );
            Ok(())
        })
    }
    #[test]
    fn publication_failure_after_sharing_retains_the_original_typed_failure() -> Result<(), String>
    {
        let status = AsyncResult::<(), AtomError<OwnedError>>::settle(
            std::num::NonZeroU64::MIN,
            Exit::Fail(AtomError::Evaluation(OwnedError(String::from("published")))),
            None,
        );
        let published = status.clone();
        let outcome = rejected_outcome(Some(status));
        let Exit::Fail(AtomError::RetainedFailure(error)) = outcome else {
            return Err(String::from(
                "shared publication must retain its typed failure",
            ));
        };
        assert!(error
            .with(|error| matches!(error, AtomError::Evaluation(OwnedError(message)) if message == "published"))
            .map_err(|error| format!("retained failure: {error:?}"))?);
        assert!(matches!(
            published,
            AsyncResult::Settled {
                outcome: Exit::Fail(_),
                ..
            }
        ));
        Ok(())
    }
}
