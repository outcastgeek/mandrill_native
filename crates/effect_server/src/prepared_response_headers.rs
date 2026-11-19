//! Explicit startup controls for the existing public response-header handle.
use effect::memory::{
    Bytes, FixedVector, MemoryError, PreparedMemory, RegionId, SharedSlot, SlotPool,
};
use http::HeaderMap;
use std::sync::{Arc, Mutex};

type Registry = Mutex<FixedVector<'static, Arc<Mutex<HeaderMap>>>>;

/// Physical registry and startup native control demand, excluding Arc headers,
/// allocator rounding, and mutable HeaderMap buckets or values.
pub struct PreparedResponseHeadersDemand {
    /// Region-backed registry and fixed index payload.
    pub region_bytes: Bytes,
    /// Startup Arc controls owned by the registry.
    pub startup_controls: usize,
    /// Mutex and empty HeaderMap payload bytes, excluding Arc headers.
    pub startup_payload_bytes: Bytes,
    /// Additional caller-owned pool Arc required by PreparedServer selection.
    pub caller_pool_arc_controls: usize,
    /// Pool value payload within that Arc, excluding its control header.
    pub caller_pool_arc_payload_bytes: Bytes,
}

/// Finite registry of response-header controls, explicitly selected at startup.
///
/// The registry keeps a strong Arc owner while it lives. Consequently public
/// Arc uniqueness and `try_unwrap` observe that owner, and old Weak handles
/// remain upgradeable and prevent reuse. No mutable HeaderMap growth is bounded.
pub struct PreparedResponseHeadersPool {
    registry: SharedSlot<'static, Registry>,
}
impl PreparedResponseHeadersPool {
    /// Calculate actual region/index and startup native payload demand.
    pub fn demand(controls: usize) -> Result<PreparedResponseHeadersDemand, MemoryError> {
        let index = std::alloc::Layout::array::<Arc<Mutex<HeaderMap>>>(controls)
            .map_err(|_| MemoryError::InvalidLayout)?;
        Ok(PreparedResponseHeadersDemand {
            region_bytes: SlotPool::<Registry>::storage_bytes(1)?
                .checked_add(Bytes::new(index.size()))?,
            startup_controls: controls,
            startup_payload_bytes: Bytes::new(std::mem::size_of::<Mutex<HeaderMap>>())
                .checked_mul(controls)?,
            caller_pool_arc_controls: 1,
            caller_pool_arc_payload_bytes: Bytes::new(std::mem::size_of::<Self>()),
        })
    }

    /// Prepare the fixed region registry and its startup native Arc controls.
    pub fn prepare(
        memory: &PreparedMemory<'static>,
        region: RegionId,
        controls: usize,
    ) -> Result<Self, MemoryError> {
        let _ = Self::demand(controls)?;
        let registry = memory.slots(region, 1, |_| {
            Ok(Mutex::new(FixedVector::prepare(
                memory.allocator(region)?,
                controls,
            )?))
        })?;
        let registry = registry.checkout()?.share();
        for _ in 0..controls {
            let control = Arc::new(Mutex::new(HeaderMap::new()));
            let result = registry
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(control);
            if let Err(control) = result {
                drop(control);
                return Err(MemoryError::NoSlot);
            }
        }
        Ok(Self { registry })
    }

    /// Claim a control only after all previous strong and Weak aliases retire.
    pub(crate) fn reserve(&self) -> Result<crate::ResponseHeaders, MemoryError> {
        let control = {
            let mut registry = crate::try_admission_lock(&self.registry)?;
            registry
                .as_mut_slice()
                .iter_mut()
                .find_map(|cell| {
                    if Arc::get_mut(cell).is_some() {
                        Some(Arc::clone(cell))
                    } else {
                        None
                    }
                })
                .ok_or(MemoryError::NoSlot)?
        };
        let previous = {
            let mut map = control
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            std::mem::take(&mut *map)
        };
        // Keep the claim live while arbitrary old values retire. If a value's
        // destructor unwinds, restore the still-owned map before the claim drops.
        let mut restore = std::mem::DropGuard::new((control, previous), |(control, map)| {
            let displaced = {
                let mut target = control
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                std::mem::replace(&mut *target, map)
            };
            drop(displaced);
        });
        restore.1.clear();
        let (control, map) = std::mem::DropGuard::dismiss(restore);
        let displaced = {
            let mut target = control
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            std::mem::replace(&mut *target, map)
        };
        drop(displaced);
        Ok(crate::ResponseHeaders(control))
    }
}
