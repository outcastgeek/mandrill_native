use super::{ActivationId, ClusterAddress, ClusterOwner};
use crate::{Effect, UtcMillis};
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActivationPhase {
    Active,
    Passivated,
    Abandoned,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActivationLease {
    pub owner: ClusterOwner,
    pub lease_until: UtcMillis,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActivationStatus {
    pub lease: ActivationLease,
    pub phase: ActivationPhase,
}

pub enum ActivationOutcome {
    Activated {
        lease: ActivationLease,
        recoverable_claims: usize,
    },
    AlreadyActive(ActivationLease),
    Busy(ActivationLease),
}

pub enum PassivationOutcome {
    Passivated,
    AlreadyPassivated,
    OutstandingClaims { count: usize },
}

/// Logical ownership capability. The provider decides how to prove and
/// coordinate exclusivity; a numeric generation alone does not prescribe a
/// database lock, membership service or process topology.
pub trait ClusterActivationStore: Send + Sync + 'static {
    type Error: Send + 'static;

    fn activate(
        &self,
        address: ClusterAddress,
        activation: ActivationId,
        now: UtcMillis,
        lease_until: UtcMillis,
    ) -> Effect<ActivationOutcome, Self::Error>;

    fn renew(
        &self,
        owner: ClusterOwner,
        now: UtcMillis,
        lease_until: UtcMillis,
    ) -> Effect<ActivationLease, Self::Error>;

    fn passivate(
        &self,
        owner: ClusterOwner,
        now: UtcMillis,
    ) -> Effect<PassivationOutcome, Self::Error>;

    fn abandon(&self, owner: ClusterOwner, now: UtcMillis) -> Effect<bool, Self::Error>;

    fn inspect_activation(
        &self,
        address: ClusterAddress,
    ) -> Effect<Option<ActivationStatus>, Self::Error>;
}

pub trait HasClusterActivationStore {
    type Store: ClusterActivationStore;
    fn cluster_activation_store(&self) -> Arc<Self::Store>;
}
