//! Prepared ordered attachment rows over checked metadata backing.
use std::alloc::Layout;
use std::num::NonZeroUsize;
use std::ops::Range;

use effect::memory::{
    Buffer, BufferLease, Bytes, FixedVector, MemoryError, PreparedMemory, RegionId, SlotLease,
    SlotPool,
};
use effect_server::ByteFormData;

use crate::validation::trim_unicode_whitespace;

use super::inputs::{
    AttachmentColumns, RoleNormalizerRef, parse_attachment_position, slice_string_value,
};

/// Startup limits for concurrent attachment readers and their retained rows.
#[derive(Clone, Copy, Debug)]
pub struct PreparedAttachmentCapacity {
    /// Maximum concurrent collected snapshots.
    pub snapshots: NonZeroUsize,
    /// Maximum raw column length in one snapshot.
    pub rows: NonZeroUsize,
    /// Maximum combined retained asset and presentation bytes per snapshot.
    pub bytes: NonZeroUsize,
}

struct Row {
    position: i64,
    ordinal: usize,
    asset_id: Range<usize>,
    role: Range<usize>,
    title_override: Range<usize>,
    caption: Range<usize>,
    alt_override: Range<usize>,
}

/// Startup-backed attachment row records and byte snapshots.
pub struct PreparedAttachmentPool {
    rows: SlotPool<'static, FixedVector<'static, Row>>,
    bytes: SlotPool<'static, Buffer<'static>>,
}

/// One ordered snapshot, retaining both physical pools through all borrowed rows.
pub struct PreparedAttachmentInputs {
    rows: SlotLease<'static, FixedVector<'static, Row>>,
    bytes: BufferLease<'static>,
}

/// Borrowed attachment metadata from an admitted snapshot.
pub struct PreparedAttachmentInput<'a> {
    pub asset_id: &'a [u8],
    pub role: &'a [u8],
    pub title_override: &'a [u8],
    pub caption: &'a [u8],
    pub alt_override: &'a [u8],
}

impl PreparedAttachmentPool {
    /// Region payload bytes, excluding region alignment and allocation headers.
    pub fn storage_bytes(capacity: PreparedAttachmentCapacity) -> Result<Bytes, MemoryError> {
        let snapshots = capacity.snapshots.get();
        let rows =
            Layout::array::<Row>(capacity.rows.get()).map_err(|_| MemoryError::InvalidLayout)?;
        SlotPool::<FixedVector<'static, Row>>::storage_bytes(snapshots)?
            .checked_add(SlotPool::<Buffer<'static>>::storage_bytes(snapshots)?)?
            .checked_add(Bytes::new(rows.size()).checked_mul(snapshots)?)?
            .checked_add(Bytes::new(capacity.bytes.get()).checked_mul(snapshots)?)
    }

    /// Provision physical row and metadata backing before requests begin.
    pub fn prepare(
        memory: &PreparedMemory<'static>,
        region: RegionId,
        capacity: PreparedAttachmentCapacity,
    ) -> Result<Self, MemoryError> {
        let _ = Self::storage_bytes(capacity)?;
        let allocator = memory.allocator(region)?;
        Ok(Self {
            rows: memory.slots(region, capacity.snapshots.get(), |_| {
                FixedVector::prepare(allocator.try_clone()?, capacity.rows.get())
            })?,
            bytes: memory.buffers(
                region,
                capacity.snapshots.get(),
                Bytes::new(capacity.bytes.get()),
            )?,
        })
    }

    /// Read the same primary/legacy columns as the owned collector, preserving
    /// duplicates, signed position fallback and original order at equal positions.
    /// A selected normalizer is invoked once for each raw row, including blanks.
    pub fn collect(
        &self,
        form: Option<&ByteFormData>,
        normalize_role: Option<RoleNormalizerRef<'_>>,
    ) -> Result<PreparedAttachmentInputs, MemoryError> {
        let mut rows = self.rows.try_checkout()?;
        let mut bytes = self.bytes.try_checkout()?;
        rows.clear();
        bytes.clear();

        let columns = AttachmentColumns::new(form);
        let count = columns.len();
        if count > rows.capacity() {
            return Err(MemoryError::CapacityExceeded {
                resource: 0,
                requested: count,
                available: rows.capacity(),
                control: false,
            });
        }
        check_raw_bytes(&columns, count, normalize_role.is_none(), bytes.capacity())?;
        for ordinal in 0..count {
            let asset_id = trimmed(columns.asset_ids, ordinal);
            let raw_role = slice_string_value(columns.roles, ordinal);
            let normalized = normalize_role.map(|normalize| normalize(raw_role));
            if asset_id.is_empty() {
                continue;
            }
            let role = trim_unicode_whitespace(
                normalized.as_ref().map_or(raw_role, |role| role.as_bytes()),
            );
            let fallback = i64::try_from(ordinal).map_err(|_| MemoryError::ArithmeticOverflow)?;
            let row = Row {
                position: parse_attachment_position(
                    slice_string_value(columns.positions, ordinal),
                    fallback,
                ),
                ordinal,
                asset_id: append(&mut bytes, asset_id)?,
                role: append(&mut bytes, role)?,
                title_override: append(&mut bytes, trimmed(columns.title_overrides, ordinal))?,
                caption: append(&mut bytes, trimmed(columns.captions, ordinal))?,
                alt_override: append(&mut bytes, trimmed(columns.alt_overrides, ordinal))?,
            };
            rows.push(row).map_err(|_| MemoryError::NoSlot)?;
        }
        rows.as_mut_slice()
            .sort_unstable_by_key(|row| (row.position, row.ordinal));
        Ok(PreparedAttachmentInputs { rows, bytes })
    }
}

fn check_raw_bytes(
    columns: &AttachmentColumns<'_>,
    count: usize,
    include_role: bool,
    capacity: Bytes,
) -> Result<(), MemoryError> {
    let mut demand = Bytes::new(0);
    for ordinal in 0..count {
        let asset = trimmed(columns.asset_ids, ordinal);
        if asset.is_empty() {
            continue;
        }
        for value in [
            asset,
            trimmed(columns.title_overrides, ordinal),
            trimmed(columns.captions, ordinal),
            trimmed(columns.alt_overrides, ordinal),
            if include_role {
                trimmed(columns.roles, ordinal)
            } else {
                &[]
            },
        ] {
            demand = demand.checked_add(Bytes::new(value.len()))?;
        }
        if demand.get() > capacity.get() {
            return Err(MemoryError::ValueTooLarge {
                requested: demand,
                capacity,
            });
        }
    }
    Ok(())
}

impl PreparedAttachmentInputs {
    /// Read the retained rows in their selected order without copying metadata.
    pub fn iter(&self) -> impl ExactSizeIterator<Item = PreparedAttachmentInput<'_>> {
        let bytes = self.bytes.as_slice();
        self.rows
            .as_slice()
            .iter()
            .map(move |row| PreparedAttachmentInput {
                asset_id: segment(bytes, &row.asset_id),
                role: segment(bytes, &row.role),
                title_override: segment(bytes, &row.title_override),
                caption: segment(bytes, &row.caption),
                alt_override: segment(bytes, &row.alt_override),
            })
    }
}

fn trimmed(values: &[Vec<u8>], ordinal: usize) -> &[u8] {
    trim_unicode_whitespace(slice_string_value(values, ordinal))
}

fn append(bytes: &mut Buffer<'static>, value: &[u8]) -> Result<Range<usize>, MemoryError> {
    let start = bytes.len();
    bytes.extend_from_slice(value)?;
    Ok(start..bytes.len())
}

fn segment<'a>(bytes: &'a [u8], range: &Range<usize>) -> &'a [u8] {
    // Construction records only bytes already written into this immutable snapshot.
    bytes.get(range.clone()).unwrap_or_default()
}
