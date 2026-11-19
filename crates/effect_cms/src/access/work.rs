//! Selected admission for the management access workflow.

use std::num::NonZeroUsize;
use std::sync::Arc;

use effect::memory::{Bytes, MemoryError, PreparedMemory, RegionId, SlotLease, SlotPool};
use effect_server::Request;

use super::AccessPrincipal;

/// Limits applied before management access callbacks consume request metadata.
#[derive(Clone, Copy)]
pub struct AccessWorkLimits {
    /// Maximum active guard evaluations, including nested response cleanup.
    pub active: NonZeroUsize,
    /// Maximum encoded path and query bytes presented to the principal resolver.
    pub request: Bytes,
    /// Maximum role count presented to an access rule.
    pub roles: NonZeroUsize,
    /// Maximum identity and role bytes presented to an access rule.
    pub principal: Bytes,
}

/// A region-backed guard admission shared by a management access policy.
pub struct PreparedAccessWork<E> {
    slots: SlotPool<'static, ()>,
    limits: AccessWorkLimits,
    pub(super) on_capacity: Arc<dyn Fn(MemoryError) -> E + Send + Sync>,
}

impl<E> PreparedAccessWork<E> {
    /// Region demand for the simultaneously active access guards.
    pub fn storage_bytes(limits: AccessWorkLimits) -> Result<Bytes, MemoryError> {
        SlotPool::<()>::storage_bytes(limits.active.get())
    }

    /// Provision guard lifetimes before accepting requests.
    pub fn prepare(
        memory: &PreparedMemory<'static>,
        region: RegionId,
        limits: AccessWorkLimits,
        on_capacity: impl Fn(MemoryError) -> E + Send + Sync + 'static,
    ) -> Result<Self, MemoryError> {
        Ok(Self {
            slots: memory.slots(region, limits.active.get(), |_| Ok(()))?,
            limits,
            on_capacity: Arc::new(on_capacity),
        })
    }

    pub(super) fn reserve(&self, request: &Request) -> Result<SlotLease<'static, ()>, MemoryError> {
        let target = request
            .uri()
            .path_and_query()
            .map_or(0, |target| target.as_str().len());
        check_bytes(Bytes::new(target), self.limits.request)?;
        self.slots.try_checkout()
    }

    pub(super) fn check_principal(&self, principal: &AccessPrincipal) -> Result<(), MemoryError> {
        let roles = principal.roles();
        if roles.len() > self.limits.roles.get() {
            return Err(MemoryError::NoSlot);
        }
        let mut values = [
            principal.user_id(),
            principal.user_name(),
            principal.email(),
            principal.security_stamp(),
            principal.authentication_method(),
        ]
        .into_iter()
        .chain(roles.iter().map(String::as_str));
        let bytes = values.try_fold(Bytes::new(0), |bytes, value| {
            bytes.checked_add(Bytes::new(value.len()))
        })?;
        check_bytes(bytes, self.limits.principal)
    }
}

fn check_bytes(requested: Bytes, capacity: Bytes) -> Result<(), MemoryError> {
    if requested.get() > capacity.get() {
        return Err(MemoryError::ValueTooLarge {
            requested,
            capacity,
        });
    }
    Ok(())
}
