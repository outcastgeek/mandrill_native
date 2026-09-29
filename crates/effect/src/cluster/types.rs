use crate::{KeyDomain, StableKey};
use std::num::NonZeroU64;
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClusterIdentityError {
    EmptyEntityType,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ClusterEntityType(Arc<str>);

impl ClusterEntityType {
    pub fn new(value: impl Into<Arc<str>>) -> Result<Self, ClusterIdentityError> {
        let value = value.into();
        if value.trim().is_empty() {
            Err(ClusterIdentityError::EmptyEntityType)
        } else {
            Ok(Self(value))
        }
    }
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

macro_rules! stable_identity {
    ($name:ident) => {
        #[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(Arc<[u8]>);

        impl $name {
            #[must_use]
            pub fn from_stable<D: KeyDomain>(key: StableKey<D>) -> Self {
                Self(key.into_bytes())
            }

            #[must_use]
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
pub struct ClusterAddress {
    pub entity_type: ClusterEntityType,
    pub entity: ClusterEntityId,
    pub shard: ClusterShardId,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct OwnershipGeneration(NonZeroU64);

impl OwnershipGeneration {
    #[must_use]
    pub const fn new(value: NonZeroU64) -> Self {
        Self(value)
    }
    #[must_use]
    pub const fn value(self) -> NonZeroU64 {
        self.0
    }
}

/// Ownership is logical contract data. A provider may use leases, database
/// fencing or another coordinator; the core prescribes no topology or lock.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClusterOwner {
    pub address: ClusterAddress,
    pub activation: ActivationId,
    pub generation: OwnershipGeneration,
}
