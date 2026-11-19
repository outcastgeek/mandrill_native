//! Prepared managed-route descriptors; registration and persistence stay consumer-owned.
use std::alloc::Layout;
use std::num::NonZeroUsize;

use effect::memory::{
    Bytes, FixedVector, MemoryError, PreparedMemory, RegionId, SlotLease, SlotPool,
};

use super::ManagedRoute;

/// Bounds for retained startup route snapshots and their declarative metadata.
#[derive(Clone, Copy, Debug)]
pub struct PreparedManagedRouteCapacity {
    /// Concurrently retained route snapshots, including selected replacements.
    pub snapshots: NonZeroUsize,
    /// Number of declarations in one route snapshot.
    pub routes: NonZeroUsize,
    /// Number of declared permissions across a route snapshot.
    pub permissions: NonZeroUsize,
    /// Total bytes of identifiers, paths, permission strings and display metadata.
    pub metadata: NonZeroUsize,
}

/// Startup provisioned storage for selected route snapshots.
pub struct PreparedManagedRouteStorage<E: Send + 'static, R: Send + Sync + 'static> {
    storage: SlotPool<'static, FixedVector<'static, ManagedRoute<E, R>>>,
    capacity: PreparedManagedRouteCapacity,
}

pub(super) struct RouteTable<E: Send + 'static, R: Send + Sync + 'static> {
    rows: SlotLease<'static, FixedVector<'static, ManagedRoute<E, R>>>,
}

impl<E: Send + 'static, R: Send + Sync + 'static> PreparedManagedRouteStorage<E, R> {
    /// Region payload needed for the retained route descriptor and slot tables.
    pub fn storage_bytes(capacity: PreparedManagedRouteCapacity) -> Result<Bytes, MemoryError> {
        let rows = Layout::array::<ManagedRoute<E, R>>(capacity.routes.get())
            .map_err(|_| MemoryError::InvalidLayout)?;
        SlotPool::<FixedVector<'static, ManagedRoute<E, R>>>::storage_bytes(
            capacity.snapshots.get(),
        )?
        .checked_add(Bytes::new(rows.size()).checked_mul(capacity.snapshots.get())?)
    }

    /// Provision the route tables before registering any consumer callbacks.
    pub fn prepare(
        memory: &PreparedMemory<'static>,
        region: RegionId,
        capacity: PreparedManagedRouteCapacity,
    ) -> Result<Self, MemoryError> {
        let _ = Self::storage_bytes(capacity)?;
        let allocator = memory.allocator(region)?;
        Ok(Self {
            storage: memory.slots(region, capacity.snapshots.get(), |_| {
                FixedVector::prepare(allocator.try_clone()?, capacity.routes.get())
            })?,
            capacity,
        })
    }

    pub(super) fn store(
        &self,
        routes: Vec<ManagedRoute<E, R>>,
    ) -> Result<RouteTable<E, R>, MemoryError> {
        let count = routes.len();
        check_count(0, count, self.capacity.routes.get())?;
        let mut permissions = 0_usize;
        let mut metadata = 0_usize;
        for route in &routes {
            permissions = permissions
                .checked_add(route.permissions.len())
                .ok_or(MemoryError::ArithmeticOverflow)?;
            check_count(1, permissions, self.capacity.permissions.get())?;
            for field in [
                route.name.as_str(),
                route.method.as_str(),
                route.pattern.as_str(),
                route.permission_mode.as_str(),
                route.display_name.as_str(),
                route.description.as_str(),
                route.category.as_str(),
            ]
            .into_iter()
            .chain(route.permissions.iter().map(String::as_str))
            {
                metadata = metadata
                    .checked_add(field.len())
                    .ok_or(MemoryError::ArithmeticOverflow)?;
                if metadata > self.capacity.metadata.get() {
                    return Err(MemoryError::ValueTooLarge {
                        requested: Bytes::new(metadata),
                        capacity: Bytes::new(self.capacity.metadata.get()),
                    });
                }
            }
        }
        let mut table = RouteTable {
            rows: self.storage.try_checkout()?,
        };
        for route in routes {
            table.rows.push(route).map_err(|_| MemoryError::NoSlot)?;
        }
        Ok(table)
    }
}

impl<E: Send + 'static, R: Send + Sync + 'static> RouteTable<E, R> {
    pub(super) fn as_slice(&self) -> &[ManagedRoute<E, R>] {
        self.rows.as_slice()
    }
}

impl<E: Send + 'static, R: Send + Sync + 'static> Drop for RouteTable<E, R> {
    fn drop(&mut self) {
        self.rows.clear();
    }
}

fn check_count(resource: usize, requested: usize, available: usize) -> Result<(), MemoryError> {
    if requested > available {
        return Err(MemoryError::CapacityExceeded {
            resource,
            requested,
            available,
            control: false,
        });
    }
    Ok(())
}
