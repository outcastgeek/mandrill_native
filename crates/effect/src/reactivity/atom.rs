//! Typed reactive atom descriptions and dependencies.

use super::{AsyncResult, AtomId, AtomKind, AtomReadContext, AtomRegistry, AtomRegistryError};
use super::{InvalidationBatch, InvalidationError};
use crate::pub_sub::{Subscription, SubscriptionEvent};
use crate::{
    Effect, Exit, Fiber, HasTaskSpawner, PubSubError, PubSubOverflow, PubSubShutdown, Release,
    ResourceValue, Scope, Semaphore, SemaphoreError, SubscriptionRef,
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
        self.status.read().map_error(AtomError::Publication)
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
        self.commit
            .acquire(NonZeroUsize::MIN)
            .map_error(AtomError::Admission)
            .flat_map(move |permit| {
                let atom = atom.clone();
                let beginning = Effect::try_sync(
                    {
                        let atom = atom.clone();
                        move || {
                            atom.registry
                                .begin_evaluation_owned(&atom.id, &atom.owner)
                                .map_err(AtomError::Registry)
                        }
                    },
                    std::convert::identity,
                )
                .flat_map(move |generation| {
                    let atom = atom.clone();
                    atom.status
                        .read()
                        .map_error(AtomError::Publication)
                        .flat_map(move |snapshot| {
                            let previous = snapshot.current_success().cloned();
                            let running = AsyncResult::Running {
                                generation,
                                refreshing: previous.is_some(),
                                previous_success: previous.clone(),
                            };
                            atom.status
                                .set(running)
                                .map_error(AtomError::Publication)
                                .map(move |_| (generation, previous.clone()))
                        })
                });
                beginning.ensuring(Effect::sync_once(move || drop(permit)))
            })
            .flat_map({
                let atom = self.clone();
                move |(generation, previous)| {
                    let atom = atom.clone();
                    let context = AtomReadContext::new(atom.registry.clone());
                    let evaluator = Arc::clone(&atom.evaluate);
                    (evaluator)(context.clone())
                        .materialize_exit()
                        .map_error(|never: std::convert::Infallible| match never {})
                        .flat_map(move |outcome| {
                            let pending = Arc::new(Mutex::new(Some((outcome, previous.clone()))));
                            let atom = atom.clone();
                            let context = context.clone();
                            atom.commit
                                .acquire(NonZeroUsize::MIN)
                                .map_error(AtomError::Admission)
                                .flat_map(move |permit| {
                                    let atom = atom.clone();
                                    let context = context.clone();
                                    let pending = Arc::clone(&pending);
                                    let commit = Effect::defer(move || {
                                        let pending = pending
                                            .lock()
                                            .unwrap_or_else(std::sync::PoisonError::into_inner)
                                            .take();
                                        let Some((outcome, previous)) = pending else {
                                            return Effect::from_result_once(Err(
                                                AtomError::InputConsumed,
                                            ));
                                        };
                                        if !atom.registry.is_current(&atom.id, generation) {
                                            return Effect::from_exit_once(discard_value(outcome.map_error(AtomError::Evaluation)));
                                        }
                                        let outcome=outcome.map_error(AtomError::Evaluation);
                                        let observed=if matches!(&outcome,Exit::Success(_)){Some(context.collected())}else{None};
                                        let previous_for_stale=previous.clone();let status=AsyncResult::settle(generation,outcome,previous);let rejected=Arc::new(Mutex::new(Some(status.clone())));
                                        let decision=Arc::new(Mutex::new(None));let captured=Arc::clone(&decision);let registry=atom.registry.clone();let id=atom.id.clone();let commit_registry=registry.clone();let commit_id=id.clone();let atom_for_stale=atom.clone();
                                        atom.status.set_with_guard(status,move|_|{let result=commit_registry.commit_evaluation(&commit_id,generation,observed.as_ref());let accepted=matches!(&result,Ok((true,_)));*captured.lock().unwrap_or_else(std::sync::PoisonError::into_inner)=Some(result);accepted})
                                        .map_error(AtomError::Publication).materialize_exit().map_error(|never:std::convert::Infallible|match never{}).flat_map(move|publication|{
                                            let retained=rejected.lock().unwrap_or_else(std::sync::PoisonError::into_inner).take();
                                            let decision=decision.lock().unwrap_or_else(std::sync::PoisonError::into_inner).take();
                                            if !matches!(&publication,Exit::Success(_)){let outcome=rejected_outcome(retained);let result=Effect::from_exit_once(outcome).ensuring(Effect::from_exit_once(discard_value(publication)));return match decision{Some(Ok((true,keys)))=>result.ensuring(registry.notify(keys).map_error(AtomError::CommittedWithoutInvalidation)),Some(Err(error))=>result.ensuring(Effect::from_result_once(Err(AtomError::Registry(error)))),Some(Ok((false,_)))|None=>result};}
                                            match decision{
                                                Some(Ok((true,keys)))=>registry.notify(keys).map_error(AtomError::CommittedWithoutInvalidation),
                                                Some(Ok((false,keys)))=>{
                                                    let outcome=rejected_outcome(retained);
                                                    if !registry.owns_evaluation(&id,generation){return Effect::from_exit_once(outcome);}
                                                    let registry=registry.clone();let previous=previous_for_stale.clone();
                                                    Effect::from_exit_once(outcome).ensuring(atom_for_stale.status.set(AsyncResult::settle(generation,Exit::Interrupted,previous)).map_error(AtomError::Publication).flat_map(move|_|registry.notify(keys.clone()).map_error(AtomError::CommittedWithoutInvalidation)))
                                                },
                                                Some(Err(error))=>{
                                                    let error=AtomError::Registry(error);let previous=previous_for_stale.clone();
                                                    // Preserve evaluator failures and append registry/settlement failures.
                                                    Effect::from_exit_once(rejected_outcome(retained)).ensuring(Effect::from_result_once(Err(error))).ensuring(atom_for_stale.status.set(AsyncResult::settle(generation,Exit::Interrupted,previous)).map_error(AtomError::Publication).map(|_|()))
                                                },
                                                None=>Effect::from_result_once(Err(AtomError::InputConsumed)),
                                            }
                                        })
                                    });
                                    commit
                                        .uninterruptible()
                                        .ensuring(Effect::sync_once(move || drop(permit)))
                                })
                        })
                }
            })
    }

    /// Lazily shut down this status channel and remove only the registration owned by this handle.
    ///
    /// A replacement ID is preserved. Shutdown and graph notification failures retain
    /// their typed channels, and cleanup is protected from interruption.
    pub fn dispose(&self, mode: PubSubShutdown) -> Effect<(), AtomError<E>, R> {
        let atom = self.clone();
        self.commit
            .acquire(NonZeroUsize::MIN)
            .map_error(AtomError::Admission)
            .flat_map(move |permit| {
                let atom = atom.clone();
                let release = atom
                    .status
                    .shutdown(mode)
                    .map_error(AtomError::Publication)
                    .flat_map(move |_| {
                        atom.registry
                            .remove_owned(atom.id.clone(), Some(Arc::clone(&atom.owner)))
                            .map_error(AtomError::Registry)
                            .map(|_| ())
                    });
                release
                    .uninterruptible()
                    .ensuring(Effect::sync_once(move || drop(permit)))
            })
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
        let subscription = self
            .registry
            .invalidations()
            .subscribe()
            .map_error(|error| AtomError::Registry(AtomRegistryError::Invalidation(error)));
        scope
            .acquire(subscription, |subscription| {
                Release::sync(move || {
                    subscription.close_now();
                })
            })
            .flat_map(move |resource| {
                let atom = atom.clone();
                let driver = atom.refresh().flat_map({
                    let atom = atom.clone();
                    move |_| drive_atom(atom.clone(), resource.clone())
                });
                scope.fork(driver)
            })
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
    Effect::defer(move || {
        let atom = atom.clone();
        let subscription = subscription.clone();
        let take = match subscription.with(|value| value.take::<R>()) {
            Ok(take) => take.map_error(|error| {
                AtomError::Registry(AtomRegistryError::Invalidation(InvalidationError::PubSub(
                    error,
                )))
            }),
            Err(defect) => return Effect::die(defect),
        };
        take.flat_map(move |event| {
            let relevant = match event {
                SubscriptionEvent::Item(batch) => {
                    let dependencies = match atom.registry.dependencies_of(&atom.id) {
                        Ok(dependencies) => dependencies,
                        Err(error) => {
                            return Effect::from_result_once(Err(AtomError::Registry(error)));
                        }
                    };
                    batch.keys.iter().any(|key| {
                        dependencies
                            .iter()
                            .any(|dependency| dependency.invalidation_key() == *key)
                    })
                }
                SubscriptionEvent::Lagged(_) => true,
                SubscriptionEvent::End => return Effect::sync(|| ()),
            };
            if relevant {
                let atom_for_next = atom.clone();
                let subscription_for_next = subscription.clone();
                atom.refresh().flat_map(move |_| {
                    drive_atom(atom_for_next.clone(), subscription_for_next.clone())
                })
            } else {
                drive_atom(atom.clone(), subscription.clone())
            }
        })
    })
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
    fn publication_rejection_preserves_owned_evaluator_and_cleanup_failures() {
        futures_executor::block_on(async {
            let capacity = NonZeroUsize::MIN;
            let registry = AtomRegistry::new(PubSubOptions {
                capacity,
                overflow: PubSubOverflow::DropOldest,
                replay: ReplayPolicy::None,
            })
            .unwrap();
            let namespace = KeyNamespace::<Domain>::new("atom").unwrap();
            let id = AtomId::from_stable(
                StableKey::new(&namespace, [KeySegment::Text("derived".into())]).unwrap(),
            );
            let atom = Atom::<(), OwnedError, ()>::new(
                registry.clone(),
                id.clone(),
                capacity,
                PubSubOverflow::Backpressure,
                |_| {
                    Effect::fail_with(|| OwnedError(String::from("evaluation")))
                        .ensuring(Effect::fail_with(|| OwnedError(String::from("cleanup"))))
                },
            )
            .unwrap();
            let snapshot = match atom.status.subscribe().run(Arc::new(())).await {
                Exit::Success(value) => value,
                _ => panic!("subscribe"),
            };
            let subscription = snapshot.subscription;
            let future = atom.refresh().run(Arc::new(()));
            let future = std::pin::pin!(future);
            let drain = async {
                let first = subscription.take::<()>().run(Arc::new(())).await;
                assert!(matches!(first, Exit::Success(_)));
                registry.begin_evaluation(&id).unwrap();
                while matches!(
                    subscription.take::<()>().run(Arc::new(())).await,
                    Exit::Success(SubscriptionEvent::Item(_))
                ) {}
            };
            let drain = std::pin::pin!(drain);
            let outcome = match futures_util::future::select(future, drain).await {
                futures_util::future::Either::Left((outcome, _)) => outcome,
                futures_util::future::Either::Right(_) => panic!("drain cannot finish first"),
            };
            assert!(
                matches!(outcome,Exit::FinalizationFailed(ref details) if matches!(&details.primary,PrimaryOutcome::Failed(AtomError::Evaluation(OwnedError(message))) if message=="evaluation")&&matches!(&details.first,CleanupFailure::ReleaseFailure(AtomError::Evaluation(OwnedError(message))) if message=="cleanup"))
            );
        });
    }
    #[test]
    fn publication_failure_after_sharing_retains_the_original_typed_failure() {
        let status = AsyncResult::<(), AtomError<OwnedError>>::settle(
            std::num::NonZeroU64::MIN,
            Exit::Fail(AtomError::Evaluation(OwnedError(String::from("published")))),
            None,
        );
        let published = status.clone();
        let outcome = rejected_outcome(Some(status));
        assert!(
            matches!(outcome,Exit::Fail(AtomError::RetainedFailure(ref error)) if error.with(|error|matches!(error,AtomError::Evaluation(OwnedError(message)) if message=="published")).unwrap())
        );
        assert!(matches!(
            published,
            AsyncResult::Settled {
                outcome: Exit::Fail(_),
                ..
            }
        ));
    }
}
