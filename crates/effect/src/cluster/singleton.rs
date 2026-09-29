use super::{
    ActivationId, ActivationLease, ActivationOutcome, ClusterActivationStore, ClusterAddress,
    ClusterOwner, HasClusterActivationStore, PassivationOutcome,
};
use crate::{Effect, HasWallClock, PrimaryOutcomeRef, Release, UtcMillis, WallClock};
use std::sync::Arc;

#[derive(Debug)]
pub enum SingletonError<Body, Activation, Clock> {
    Body(Body),
    Activation(Activation),
    Clock(Clock),
    UncertainPassivation(Activation),
    UncertainAbandonment(Activation),
    OutstandingClaims { count: usize },
}

pub enum SingletonOutcome<A> {
    Ran(A),
    Busy(ActivationLease),
}

/// Logical singleton execution. A provider grants an activation lease; this
/// description neither chooses a process nor starts an OS scheduler.
pub struct ClusterSingleton {
    address: ClusterAddress,
}

impl ClusterSingleton {
    #[must_use]
    pub fn new(address: ClusterAddress) -> Self {
        Self { address }
    }

    pub fn run<A, BodyError, ActivationError, ClockError, R>(
        &self,
        activation: ActivationId,
        lease_until: UtcMillis,
        body: impl Fn(ClusterOwner) -> Effect<A, BodyError, R> + Send + Sync + 'static,
    ) -> Effect<SingletonOutcome<A>, SingletonError<BodyError, ActivationError, ClockError>, R>
    where
        A: Send + 'static,
        BodyError: Send + 'static,
        ActivationError: Send + 'static,
        ClockError: Send + 'static,
        R: HasClusterActivationStore + HasWallClock + Send + Sync + 'static,
        R::Store: ClusterActivationStore<Error = ActivationError>,
        R::Clock: WallClock<Error = ClockError>,
    {
        let address = self.address.clone();
        let body = Arc::new(body);
        Effect::wall_now().map_error(SingletonError::Clock)
            .flat_map(move |now| {
                let address = address.clone();
                let activation = activation.clone();
                let body = Arc::clone(&body);
                Effect::service(|env: &R| env.cluster_activation_store())
                    .flat_map(move |store| {
                        let body = Arc::clone(&body);
                        store.activate(address.clone(), activation.clone(), now, lease_until)
                            .map_error(SingletonError::Activation)
                            .provide_some(|_: &R| ())
                            .flat_map(move |outcome| match outcome {
                                ActivationOutcome::Busy(lease)
                                | ActivationOutcome::AlreadyActive(lease) => {
                                    Effect::sync_once(move || SingletonOutcome::Busy(lease))
                                }
                                ActivationOutcome::Activated { lease, .. } => {
                                    let owner = lease.owner;
                                    let final_store = Arc::clone(&store);
                                    let final_owner = owner.clone();
                                    body(owner).map_error(SingletonError::Body)
                                        .ensuring_with(move |reason| {
                                            let store = Arc::clone(&final_store);
                                            let owner = final_owner.clone();
                                            let succeeded = matches!(
                                                reason, PrimaryOutcomeRef::Succeeded
                                            );
                                            let cleanup = Effect::wall_now()
                                                .map_error(SingletonError::Clock)
                                                .flat_map(move |now| if succeeded {
                                                    store.passivate(owner.clone(), now)
                                                        .map_error(SingletonError::UncertainPassivation)
                                                        .provide_some(|_: &R| ())
                                                        .flat_map(|status| match status {
                                                            PassivationOutcome::Passivated
                                                            | PassivationOutcome::AlreadyPassivated => {
                                                                Effect::sync(|| ())
                                                            }
                                                            PassivationOutcome::OutstandingClaims { count } => {
                                                                Effect::from_result_once(Err(
                                                                    SingletonError::OutstandingClaims { count }
                                                                ))
                                                            }
                                                        })
                                                } else {
                                                    store.abandon(owner.clone(), now)
                                                    .map_error(SingletonError::UncertainAbandonment)
                                                    .provide_some(|_: &R| ())
                                                    .map(|_| ())
                                                });
                                            Release::from_effect(cleanup)
                                        })
                                        .map(SingletonOutcome::Ran)
                                }
                            })
                    })
            })
    }
}
