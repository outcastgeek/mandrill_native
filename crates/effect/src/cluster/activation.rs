//! Activation ownership transitions and fencing.

use super::{ActivationId, ClusterAddress, ClusterOwner};
use crate::{Effect, UtcMillis};
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// The inactive, owned, or transition phase of an entity activation.
pub enum ActivationPhase {
    /// The owner or operation is currently active.
    Active,
    /// The entity has released its active ownership.
    Passivated,
    /// The owner was lost before the operation settled.
    Abandoned,
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// An owner and generation fence with a bounded activation lifetime.
pub struct ActivationLease {
    /// The owner of the current claim or activation. Type: `ClusterOwner`.
    pub owner: ClusterOwner,
    /// Instant at which this claim or ownership lease expires. Type: `UtcMillis`.
    pub lease_until: UtcMillis,
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// The current ownership and lease information for an activation.
pub struct ActivationStatus {
    /// Lease fencing this owner’s access. Type: `ActivationLease`.
    pub lease: ActivationLease,
    /// Current phase of this lifecycle. Type: `ActivationPhase`.
    pub phase: ActivationPhase,
}

/// The store decision after attempting to acquire or renew activation.
pub enum ActivationOutcome {
    /// Ownership was acquired and the entity became active.
    Activated {
        /// Lease fencing this owner’s access. Type: `ActivationLease`.
        lease: ActivationLease,
        /// Claims eligible for recovery by a new owner. Type: `usize`.
        recoverable_claims: usize,
    },
    /// An activation lease is already held by a current owner. Contains `ActivationLease`.
    AlreadyActive(ActivationLease),
    /// The requested work is already in progress. Contains `ActivationLease`.
    Busy(ActivationLease),
}

/// The store decision after attempting to release an activation.
pub enum PassivationOutcome {
    /// The entity has released its active ownership.
    Passivated,
    /// The entity has already relinquished active ownership.
    AlreadyPassivated,
    /// Passivation cannot finish while accepted claims remain outstanding.
    OutstandingClaims {
        /// Number of unfinished claims. Type: `usize`.
        count: usize,
    },
}

/// Logical ownership capability. The provider decides how to prove and
/// coordinate exclusivity; a numeric generation alone does not prescribe a
/// database lock, membership service or process topology.
pub trait ClusterActivationStore: Send + Sync + 'static {
    /// Provider-specific failure type returned by this capability.
    type Error: Send + 'static;

    /// # Parameters
    /// - `address` (`ClusterAddress`): Stable destination address of the entity receiving this request.
    /// - `activation` (`ActivationId`): The activation supplied to this operation.
    /// - `now` (`UtcMillis`): The caller-supplied current time.
    /// - `lease_until` (`UtcMillis`): Instant at which the claim or activation lease expires.
    ///
    /// # Returns
    /// Type: `Effect<ActivationOutcome, Self::Error>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    fn activate(
        &self,
        address: ClusterAddress,
        activation: ActivationId,
        now: UtcMillis,
        lease_until: UtcMillis,
    ) -> Effect<ActivationOutcome, Self::Error>;

    /// # Parameters
    /// - `owner` (`ClusterOwner`): Worker or scope identity used to fence stale operations.
    /// - `now` (`UtcMillis`): The caller-supplied current time.
    /// - `lease_until` (`UtcMillis`): Instant at which the claim or activation lease expires.
    ///
    /// # Returns
    /// Type: `Effect<ActivationLease, Self::Error>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    fn renew(
        &self,
        owner: ClusterOwner,
        now: UtcMillis,
        lease_until: UtcMillis,
    ) -> Effect<ActivationLease, Self::Error>;

    /// # Parameters
    /// - `owner` (`ClusterOwner`): Worker or scope identity used to fence stale operations.
    /// - `now` (`UtcMillis`): The caller-supplied current time.
    ///
    /// # Returns
    /// Type: `Effect<PassivationOutcome, Self::Error>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    fn passivate(
        &self,
        owner: ClusterOwner,
        now: UtcMillis,
    ) -> Effect<PassivationOutcome, Self::Error>;

    /// # Parameters
    /// - `owner` (`ClusterOwner`): Worker or scope identity used to fence stale operations.
    /// - `now` (`UtcMillis`): The caller-supplied current time.
    ///
    /// # Returns
    /// Type: `Effect<bool, Self::Error>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    fn abandon(&self, owner: ClusterOwner, now: UtcMillis) -> Effect<bool, Self::Error>;

    /// # Parameters
    /// - `address` (`ClusterAddress`): Stable destination address of the entity receiving this request.
    ///
    /// # Returns
    /// Type: `Effect<Option<ActivationStatus>, Self::Error>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    fn inspect_activation(
        &self,
        address: ClusterAddress,
    ) -> Effect<Option<ActivationStatus>, Self::Error>;
}

/// Selects the activation-ownership store used by an application.
pub trait HasClusterActivationStore {
    /// Store selected by this environment; its trait bound defines the required guarantees.
    type Store: ClusterActivationStore;
    /// # Returns
    /// Type: `Arc<Self::Store>`.
    fn cluster_activation_store(&self) -> Arc<Self::Store>;
}
