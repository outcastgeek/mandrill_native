//! Shared activation index for the volatile store's message and owner fences.

use super::{ActivationStatus, ClusterAddress, ClusterStoreError};
use crate::memory::{Bytes, FixedVector, MemoryError, PreparedMemory, RegionId};
use std::alloc::Layout;
use std::collections::BTreeMap;
use std::num::NonZeroUsize;

/// Selected activation metadata bounds, including retained inactive fences.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PreparedClusterActivationCapacity {
    /// Addresses remembered by this store, including passivated or abandoned addresses.
    pub addresses: NonZeroUsize,
    /// Combined entity type, entity, shard and current activation identity bytes per address.
    pub identity_bytes_per_address: Bytes,
}

impl PreparedClusterActivationCapacity {
    /// Regional inline ownership records. Stable identity handles keep their
    /// caller-owned backing, bounded separately by `identity_bytes_per_address`.
    pub fn storage_bytes(self) -> Result<Bytes, MemoryError> {
        Layout::array::<ActivationStatus>(self.addresses.get())
            .map(|layout| Bytes::new(layout.size()))
            .map_err(|_| MemoryError::InvalidLayout)
    }
}

pub(super) enum ActivationTable {
    Ordinary(BTreeMap<ClusterAddress, ActivationStatus>),
    Prepared(PreparedTable),
}

pub(super) struct PreparedTable {
    statuses: FixedVector<'static, ActivationStatus>,
    identity_bytes_per_address: Bytes,
}

impl ActivationTable {
    pub(super) const fn ordinary() -> Self {
        Self::Ordinary(BTreeMap::new())
    }

    pub(super) fn prepare(
        memory: &PreparedMemory<'static>,
        region: RegionId,
        capacity: PreparedClusterActivationCapacity,
    ) -> Result<Self, MemoryError> {
        capacity.storage_bytes()?;
        Ok(Self::Prepared(PreparedTable {
            statuses: FixedVector::prepare(memory.allocator(region)?, capacity.addresses.get())?,
            identity_bytes_per_address: capacity.identity_bytes_per_address,
        }))
    }

    pub(super) fn get(&self, address: &ClusterAddress) -> Option<&ActivationStatus> {
        match self {
            Self::Ordinary(statuses) => statuses.get(address),
            Self::Prepared(table) => {
                let index = table.position(address).ok()?;
                table.statuses.as_slice().get(index)
            }
        }
    }

    pub(super) fn get_mut(&mut self, address: &ClusterAddress) -> Option<&mut ActivationStatus> {
        match self {
            Self::Ordinary(statuses) => statuses.get_mut(address),
            Self::Prepared(table) => {
                let index = table.position(address).ok()?;
                table.statuses.as_mut_slice().get_mut(index)
            }
        }
    }

    /// Commit an already fenced status under the same gate as the message index.
    /// Every prepared failure precedes changing any existing status.
    pub(super) fn try_insert(&mut self, status: ActivationStatus) -> Result<(), ClusterStoreError> {
        match self {
            Self::Ordinary(statuses) => {
                statuses.insert(status.lease.owner.address.clone(), status);
                Ok(())
            }
            Self::Prepared(table) => table.try_insert(status),
        }
    }
}

impl PreparedTable {
    fn position(&self, address: &ClusterAddress) -> Result<usize, usize> {
        self.statuses
            .as_slice()
            .binary_search_by(|status| status.lease.owner.address.cmp(address))
    }

    fn try_insert(&mut self, status: ActivationStatus) -> Result<(), ClusterStoreError> {
        let owner = &status.lease.owner;
        let address = &owner.address;
        let identity_bytes = address
            .identity_bytes()
            .and_then(|bytes| bytes.checked_add(Bytes::new(owner.activation.as_bytes().len())));
        if !identity_bytes.is_ok_and(|bytes| bytes <= self.identity_bytes_per_address) {
            return Err(ClusterStoreError::CapacityExceeded);
        }
        match self.position(address) {
            Ok(index) => {
                let slot = self
                    .statuses
                    .as_mut_slice()
                    .get_mut(index)
                    .ok_or(ClusterStoreError::WrongOwner)?;
                *slot = status;
            }
            Err(index) => {
                self.statuses
                    .push(status)
                    .map_err(|_| ClusterStoreError::CapacityExceeded)?;
                // The table appended one record, so the insertion position is in range.
                if let Some(suffix) = self.statuses.as_mut_slice().get_mut(index..) {
                    suffix.rotate_right(1);
                }
            }
        }
        Ok(())
    }
}
