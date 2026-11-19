//! Startup storage and input limits for one selected command lifetime.
use std::num::NonZeroUsize;

use effect::memory::{Bytes, MemoryError, PreparedMemory, RegionId, SlotLease, SlotPool};
use effect_server::Request;

use crate::definitions::{ResourceAction, ResourceKind};

use super::CommandInput;

/// Caller-selected caps for command metadata and simultaneous mutation work.
#[derive(Clone, Copy, Debug)]
pub struct CommandWorkLimits {
    /// Maximum simultaneous selected commands, including validation and cleanup.
    pub active: NonZeroUsize,
    /// Maximum combined command/query/form payload bytes.
    pub metadata: NonZeroUsize,
    /// Maximum form and filter entries, counting repeated filter values.
    pub values: NonZeroUsize,
}

/// Region-backed admission for command and mutation working lifetimes.
pub struct PreparedCommandWork {
    slots: SlotPool<'static, ()>,
    limits: CommandWorkLimits,
}

impl PreparedCommandWork {
    /// Region demand for the command admission slots.
    pub fn storage_bytes(limits: CommandWorkLimits) -> Result<Bytes, MemoryError> {
        SlotPool::<()>::storage_bytes(limits.active.get())
    }

    /// Provision the working lifetimes before requests begin.
    pub fn prepare(
        memory: &PreparedMemory<'static>,
        region: RegionId,
        limits: CommandWorkLimits,
    ) -> Result<Self, MemoryError> {
        Ok(Self {
            slots: memory.slots(region, limits.active.get(), |_| Ok(()))?,
            limits,
        })
    }

    pub(super) fn reserve(&self, request: &Request) -> Result<SlotLease<'static, ()>, MemoryError> {
        let raw = request
            .uri()
            .path_and_query()
            .map_or(0, |target| target.as_str().len());
        check_size(raw, self.limits.metadata.get())?;
        self.slots.try_checkout()
    }

    pub(super) fn check_identity(&self, kind: &ResourceKind, action: &ResourceAction) -> Result<(), MemoryError> {
        let bytes = kind.as_str().len().checked_add(action.as_str().len())
            .ok_or(MemoryError::ArithmeticOverflow)?;
        check_size(bytes, self.limits.metadata.get())
    }

    pub(super) fn check_input(&self, input: &CommandInput) -> Result<(), MemoryError> {
        let mut bytes = 0;
        let list = &input.list_query;
        let detail = &input.detail_query;
        for length in [
            input.kind.as_str().len(),
            input.action.as_str().len(),
            list.search.len(),
            list.sort.len(),
            detail.id.len(),
            detail.slug.len(),
            detail.parent.len(),
            detail.locale.len(),
        ] {
            add(&mut bytes, length, self.limits.metadata.get())?;
        }

        let mut count = 0;
        for (key, value) in &input.form_values {
            add(&mut count, 1, self.limits.values.get())?;
            add_pair(
                &mut bytes,
                key.len(),
                value.len(),
                self.limits.metadata.get(),
            )?;
        }
        for (key, value) in &list.filters {
            add(&mut count, 1, self.limits.values.get())?;
            add_pair(
                &mut bytes,
                key.as_str().len(),
                value.len(),
                self.limits.metadata.get(),
            )?;
        }
        for (key, values) in &list.filter_values {
            add(&mut count, 1, self.limits.values.get())?;
            add(&mut count, values.len(), self.limits.values.get())?;
            add(&mut bytes, key.as_str().len(), self.limits.metadata.get())?;
            for value in values {
                add(&mut bytes, value.len(), self.limits.metadata.get())?;
            }
        }
        Ok(())
    }
}

fn check_size(requested: usize, capacity: usize) -> Result<(), MemoryError> {
    if requested > capacity {
        return Err(MemoryError::ValueTooLarge {
            requested: Bytes::new(requested),
            capacity: Bytes::new(capacity),
        });
    }
    Ok(())
}

fn add(used: &mut usize, count: usize, capacity: usize) -> Result<(), MemoryError> {
    *used = used
        .checked_add(count)
        .ok_or(MemoryError::ArithmeticOverflow)?;
    check_size(*used, capacity)
}

fn add_pair(
    used: &mut usize,
    first: usize,
    second: usize,
    capacity: usize,
) -> Result<(), MemoryError> {
    add(used, first, capacity)?;
    add(used, second, capacity)
}
