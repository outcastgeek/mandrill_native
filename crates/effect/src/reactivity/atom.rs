use super::{AsyncResult, AtomId, AtomKind, AtomReadContext, AtomRegistry, AtomRegistryError};
use super::{InvalidationBatch, InvalidationError};
use crate::pub_sub::{Subscription, SubscriptionEvent};
use crate::{
    Effect, Exit, Fiber, HasTaskSpawner, PubSubError, PubSubOverflow, PubSubShutdown, Release,
    ResourceValue, Scope, Semaphore, SemaphoreError, SubscriptionRef,
};
use std::num::NonZeroUsize;
use std::sync::{Arc, Mutex};

type AtomEvaluator<A, E, R> = dyn Fn(AtomReadContext) -> Effect<A, E, R> + Send + Sync;
pub type AtomReadEffect<A, E, R> = Effect<Arc<AsyncResult<A, AtomError<E>>>, AtomError<E>, R>;

#[derive(Debug)]
pub enum AtomError<E> {
    Evaluation(E),
    Registry(AtomRegistryError),
    Publication(PubSubError),
    Admission(SemaphoreError),
    /// The status publication committed, but dependent invalidation failed.
    CommittedWithoutInvalidation(AtomRegistryError),
    InputConsumed,
}

/// A typed derived atom. The evaluator reads sources through AtomReadContext,
/// so the graph records only dependencies used by that evaluation. Each atom
/// owns its status and commit gate; registries share neither values nor R.
pub struct Atom<A, E, R>
where
    A: Send + 'static,
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    id: AtomId,
    registry: AtomRegistry,
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
    fn clone(&self) -> Self {
        Self {
            id: self.id.clone(),
            registry: self.registry.clone(),
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
    pub fn new(
        registry: AtomRegistry,
        id: AtomId,
        capacity: NonZeroUsize,
        overflow: PubSubOverflow,
        evaluate: impl Fn(AtomReadContext) -> Effect<A, E, R> + Send + Sync + 'static,
    ) -> Result<Self, AtomRegistryError> {
        registry.register(id.clone(), AtomKind::Derived)?;
        Ok(Self {
            id,
            registry,
            status: SubscriptionRef::new(AsyncResult::Initial, capacity, overflow),
            evaluate: Arc::new(evaluate),
            commit: Semaphore::full(NonZeroUsize::MIN),
        })
    }

    #[must_use]
    pub fn id(&self) -> &AtomId {
        &self.id
    }

    #[must_use]
    pub(crate) fn registry(&self) -> &AtomRegistry {
        &self.registry
    }

    pub fn read(&self) -> AtomReadEffect<A, E, R> {
        self.status.read().map_error(AtomError::Publication)
    }

    #[must_use]
    pub fn stream(&self) -> crate::Stream<Arc<AsyncResult<A, AtomError<E>>>, PubSubError, R> {
        self.status.stream()
    }

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
                                .begin_evaluation(&atom.id)
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
                                            return Effect::sync(|| ());
                                        }
                                        let mut outcome = outcome.map_error(AtomError::Evaluation);
                                        if matches!(&outcome, Exit::Success(_))
                                            && let Err(error) = atom
                                                .registry
                                                .replace_dependencies(&atom.id, context.collected())
                                        {
                                            outcome = Exit::Fail(AtomError::Registry(error));
                                        }
                                        let status =
                                            AsyncResult::settle(generation, outcome, previous);
                                        let id = atom.id.clone();
                                        let registry = atom.registry.clone();
                                        atom.status
                                            .set(status)
                                            .map_error(AtomError::Publication)
                                            .flat_map(move |_| {
                                                registry.changed(id.clone()).map_error(
                                                    AtomError::CommittedWithoutInvalidation,
                                                )
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
                            .remove(atom.id.clone())
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
