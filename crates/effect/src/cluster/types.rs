//! Stable Cluster identities and delivery policies.

use crate::{KeyDomain, StableKey};
use std::num::NonZeroU64;
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// A malformed or incompatible identity used to address cluster work.
pub enum ClusterIdentityError {
    /// The required entity type is empty.
    EmptyEntityType,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
/// The stable type identity used to route messages to an entity handler.
pub struct ClusterEntityType(Arc<str>);

impl ClusterEntityType {
    /// Construct a `ClusterEntityType` from its required inputs and explicit policies.
    ///
    /// # Parameters
    /// - `value` (`impl Into<Arc<str>>`): Value read, transformed, encoded, or stored by this operation.
    ///
    /// # Returns
    /// Type: `Result<Self, ClusterIdentityError>`.
    pub fn new(value: impl Into<Arc<str>>) -> Result<Self, ClusterIdentityError> {
        let value = value.into();
        if value.trim().is_empty() {
            Err(ClusterIdentityError::EmptyEntityType)
        } else {
            Ok(Self(value))
        }
    }
    #[must_use]
    /// Borrow the textual representation without allocating a new string.
    ///
    /// # Returns
    /// Type: `&str`.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

macro_rules! stable_identity {
    ($name:ident) => {
        #[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
        /// Stable Cluster identity encoded as validated, namespaced bytes.
        pub struct $name(Arc<[u8]>);

        impl $name {
            #[must_use]
            /// Wrap a typed stable key while preserving its identity bytes.
            ///
            /// # Type parameters
            /// - `D`: Identity-domain type. Bounds: `KeyDomain`.
            ///
            /// # Parameters
            /// - `key` (`StableKey<D>`): Stable key used to locate or identify the selected value.
            ///
            /// # Returns
            /// Type: `Self`.
            pub fn from_stable<D: KeyDomain>(key: StableKey<D>) -> Self {
                Self(key.into_bytes())
            }

            #[must_use]
            /// Borrow the stable encoded bytes without transferring ownership.
            ///
            /// # Returns
            /// Type: `&[u8]`.
            pub fn as_bytes(&self) -> &[u8] {
                &self.0
            }
        }
    };
}

stable_identity!(ClusterEntityId);
stable_identity!(ClusterShardId);
stable_identity!(ClusterRequestId);
stable_identity!(ClusterObserverId);
stable_identity!(ActivationId);

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
/// An entity type and entity identity combined into a stable delivery address.
pub struct ClusterAddress {
    /// Stable type identity of the addressed entity. Type: `ClusterEntityType`.
    pub entity_type: ClusterEntityType,
    /// Stable identity of the addressed entity. Type: `ClusterEntityId`.
    pub entity: ClusterEntityId,
    /// Shard identity used to locate ownership for this entity. Type: `ClusterShardId`.
    pub shard: ClusterShardId,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
/// A monotonically changing fence that distinguishes a current owner from stale owners.
pub struct OwnershipGeneration(NonZeroU64);

impl OwnershipGeneration {
    #[must_use]
    /// # Parameters
    /// - `value` (`NonZeroU64`): Value read, transformed, encoded, or stored by this operation.
    ///
    /// # Returns
    /// Type: `Self`.
    pub const fn new(value: NonZeroU64) -> Self {
        Self(value)
    }
    #[must_use]
    /// # Returns
    /// Type: `NonZeroU64`.
    pub const fn value(self) -> NonZeroU64 {
        self.0
    }
}

/// Ownership is logical contract data. A provider may use leases, database
/// fencing or another coordinator; the core prescribes no topology or lock.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClusterOwner {
    /// Entity address targeted by this message. Type: `ClusterAddress`.
    pub address: ClusterAddress,
    /// Stable identity of the activation being fenced. Type: `ActivationId`.
    pub activation: ActivationId,
    /// The ownership generation used for fencing. Type: `OwnershipGeneration`.
    pub generation: OwnershipGeneration,
}
