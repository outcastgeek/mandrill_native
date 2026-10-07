//! Ownership of one logical singleton execution.

use super::{
    ActivationId, ActivationLease, ActivationOutcome, ClusterActivationStore, ClusterAddress,
    ClusterOwner, HasClusterActivationStore, PassivationOutcome,
};
use crate::{Effect, HasWallClock, PrimaryOutcomeRef, Release, UtcMillis, WallClock};
use std::sync::Arc;

#[derive(Debug)]
/// An ownership or execution failure in a cluster singleton operation.
///
/// # Type parameters
/// - `Body`: Workflow or resource body callback type.
/// - `Activation`: Activation provider type.
/// - `Clock`: Clock provider type.
pub enum SingletonError<Body, Activation, Clock> {
    /// The registered application body failed. Retains the `Body` detail.
    Body(Body),
    /// Entity activation or renewal failed. Retains the `Activation` detail.
    Activation(Activation),
    /// The selected clock could not schedule or observe the requested time. Retains the `Clock` detail.
    Clock(Clock),
    /// Passivation may have committed; the owner state could not be confirmed. Retains the `Activation` detail.
    UncertainPassivation(Activation),
    /// Abandonment may have committed; the owner state could not be confirmed. Retains the `Activation` detail.
    UncertainAbandonment(Activation),
    /// Singleton handover must wait for outstanding claims to settle.
    OutstandingClaims {
        /// Number of unfinished claims. Type: `usize`.
        count: usize,
    },
}

/// Whether this owner acquired, retained, or yielded a singleton activation.
///
/// # Type parameters
/// - `A`: Application value type.
pub enum SingletonOutcome<A> {
    /// The singleton operation executed under the current owner. Contains `A`.
    Ran(A),
    /// The requested work is already in progress. Contains `ActivationLease`.
    Busy(ActivationLease),
}

/// Logical singleton execution. A provider grants an activation lease; this
/// description neither chooses a process nor starts an OS scheduler.
pub struct ClusterSingleton {
    address: ClusterAddress,
}

impl ClusterSingleton {
    #[must_use]
    /// Construct a `ClusterSingleton` from its required inputs and explicit policies.
    ///
    /// # Parameters
    /// - `address` (`ClusterAddress`): Stable destination address of the entity receiving this request.
    ///
    /// # Returns
    /// Type: `Self`.
    pub fn new(address: ClusterAddress) -> Self {
        Self { address }
    }

    /// Lazily claim this singleton activation until lease_until, then run body with its exact owner.
    ///
    /// R supplies activation storage and wall time. Body construction and execution
    /// remain within protected ownership cleanup; release failure is retained with
    /// the primary body outcome, including interruption. This does not start a process
    /// scheduler or make an in-memory provider physically durable.
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
                                    let body=Arc::clone(&body);
                                    Effect::defer(move || body(owner.clone())).map_error(SingletonError::Body)
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
