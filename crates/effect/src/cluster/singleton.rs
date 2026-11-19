//! Ownership of one logical singleton execution.

use super::{
    ActivationId, ActivationLease, ActivationOutcome, ClusterActivationStore, ClusterAddress,
    ClusterOwner, HasClusterActivationStore, PassivationOutcome,
};
use crate::{Effect, HasWallClock, PrimaryOutcomeRef, UtcMillis, WallClock};
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
        crate::effect! {
            bind;
            program: crate::effect_map_error! {
                program: crate::effect! { wall_now; },
                map: SingletonError::Clock,
            },
            body: move |now| {
                let address = address.clone();
                let activation = activation.clone();
                let body = Arc::clone(&body);
                crate::effect! {
                    bind;
                    program: crate::service! { env: |env: &R| env.cluster_activation_store(), },
                    body: move |store| {
                        let body = Arc::clone(&body);
                        crate::effect! {
                            bind;
                            program: crate::effect_provide_some! {
                                program: crate::effect_map_error! {
                                    program: store.activate(address.clone(), activation.clone(), now, lease_until),
                                    map: SingletonError::Activation,
                                },
                                project: |_: &R| (),
                            },
                            body: move |outcome| match outcome {
                                ActivationOutcome::Busy(lease) | ActivationOutcome::AlreadyActive(lease) => {
                                    crate::effect_sync! { once; body: move || SingletonOutcome::Busy(lease), }
                                }
                                ActivationOutcome::Activated { lease, .. } => {
                                    let owner = lease.owner;
                                    let final_store = Arc::clone(&store);
                                    let final_owner = owner.clone();
                                    let body = Arc::clone(&body);
                                    crate::effect_map! {
                                        program: crate::effect_resource! {
                                            ensuring;
                                            program: crate::effect_map_error! {
                                                program: crate::effect! { defer; body: move || body(owner.clone()), },
                                                map: SingletonError::Body,
                                            },
                                            release: move |reason| {
                                                let store = Arc::clone(&final_store);
                                                let owner = final_owner.clone();
                                                let succeeded = matches!(reason, PrimaryOutcomeRef::Succeeded);
                                                crate::effect! {
                                                    bind;
                                                    program: crate::effect_map_error! {
                                                        program: crate::effect! { wall_now; },
                                                        map: SingletonError::Clock,
                                                    },
                                                    body: move |now| if succeeded {
                                                        crate::effect! {
                                                            bind;
                                                            program: crate::effect_provide_some! {
                                                                program: crate::effect_map_error! {
                                                                    program: store.passivate(owner.clone(), now),
                                                                    map: SingletonError::UncertainPassivation,
                                                                },
                                                                project: |_: &R| (),
                                                            },
                                                            body: |status| match status {
                                                                PassivationOutcome::Passivated | PassivationOutcome::AlreadyPassivated => {
                                                                    crate::effect_sync! { body: || (), }
                                                                }
                                                                PassivationOutcome::OutstandingClaims { count } => {
                                                                    crate::effect_from_result! {
                                                                        once; result: Err(SingletonError::OutstandingClaims { count }),
                                                                    }
                                                                }
                                                            },
                                                        }
                                                    } else {
                                                        crate::effect_map! {
                                                            program: crate::effect_provide_some! {
                                                                program: crate::effect_map_error! {
                                                                    program: store.abandon(owner.clone(), now),
                                                                    map: SingletonError::UncertainAbandonment,
                                                                },
                                                                project: |_: &R| (),
                                                            },
                                                            map: |_| (),
                                                        }
                                                    },
                                                }
                                            },
                                        },
                                        map: SingletonOutcome::Ran,
                                    }
                                }
                            },
                        }
                    },
                }
            },
        }
    }
}
