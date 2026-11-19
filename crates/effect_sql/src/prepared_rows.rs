//! Fixed, exclusively owned SQL result storage.
use effect::memory::{
    Admission, BackedAdmission, Bytes, CapacityUnit, FixedVector, MemoryError, PreparedMemory,
    RegionId, SlotLease, SlotPool,
};
use std::{
    alloc::Layout,
    ops::{Deref, DerefMut},
    sync::{Arc, atomic::{AtomicBool, Ordering}},
};

/// Explicit concurrent outputs and inline rows per output.
#[derive(Clone, Copy, Debug)]
pub struct PreparedSqlRowsCapacity {
    /// Simultaneously retained builders or completed outputs.
    pub outputs: usize,
    /// Maximum number of inline rows in each output.
    pub rows_per_output: usize,
}
impl PreparedSqlRowsCapacity {
    /// Actual region slot and vector element bytes; excludes allocator overhead,
    /// alignment rounding, any caller-owned Arc and allocations owned by A.
    pub fn demand<A: Send + 'static>(self) -> Result<Bytes, MemoryError> {
        let layout = std::alloc::Layout::array::<A>(self.rows_per_output)
            .map_err(|_| MemoryError::InvalidLayout)?;
        SlotPool::<FixedVector<'static, A>>::storage_bytes(self.outputs)?
            .checked_add(Bytes::new(layout.size()).checked_mul(self.outputs)?)
    }
}

/// Startup-prepared fixed vectors for owned SQL results.
pub struct PreparedSqlRowsPool<A: Send + 'static> {
    vectors: SlotPool<'static, FixedVector<'static, A>>,
    admission: Option<Admission<'static, 1>>,
}
/// Exclusive construction owner. It preserves each rejected row on refusal.
pub struct PreparedSqlRowsBuilder<A: Send + 'static> {
    rows: PreparedSqlRows<A>,
}
/// Immutable owned rows retaining the actual prepared vector lease.
pub struct PreparedSqlRows<A: Send + 'static> {
    vector: SqlPreparedSlot<FixedVector<'static, A>>,
}
pub(crate) enum SqlPreparedSlot<T: Send + 'static> {
    Standalone(SlotLease<'static, T>),
    Backed(BackedAdmission<'static, 1, SlotLease<'static, T>>),
}
impl<T: Send + 'static> SqlPreparedSlot<T> {
    pub(crate) fn checkout(
        pool: &SlotPool<'static, T>,
        admission: Option<&Admission<'static, 1>>,
    ) -> Result<Self, MemoryError> {
        match admission {
            Some(admission) => Ok(Self::Backed(
                admission.reserve_backed([1], [0], || pool.try_checkout())?,
            )),
            None => Ok(Self::Standalone(pool.try_checkout()?)),
        }
    }
}
pub(crate) fn validate_slot_admission(
    admission: &Admission<'static, 1>,
) -> Result<(), MemoryError> {
    if admission.capacities().first().map(|capacity| capacity.unit) != Some(CapacityUnit::Slots) {
        return Err(MemoryError::UnsupportedRunContract);
    }
    Ok(())
}
impl<T: Send + 'static> Deref for SqlPreparedSlot<T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        match self {
            Self::Standalone(vector) => vector,
            Self::Backed(vector) => vector.owner(),
        }
    }
}
impl<T: Send + 'static> DerefMut for SqlPreparedSlot<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        match self {
            Self::Standalone(vector) => vector,
            Self::Backed(vector) => vector.slot_value_mut(),
        }
    }
}
impl<A: Send + 'static> PreparedSqlRowsPool<A> {
    /// Prepare every vector in the selected region before operations begin.
    pub fn prepare(
        memory: &PreparedMemory<'static>,
        region: RegionId,
        capacity: PreparedSqlRowsCapacity,
    ) -> Result<Self, MemoryError> {
        let _ = capacity.demand::<A>()?;
        let vectors = memory.slots(region, capacity.outputs, |_| {
            FixedVector::prepare(memory.allocator(region)?, capacity.rows_per_output)
        })?;
        Ok(Self {
            vectors,
            admission: None,
        })
    }
    /// Couple each checked-out output slot to one unit of work admission.
    /// The selected coordinator must describe one slot resource; this does not
    /// account for memory owned by a row value or its pointees.
    pub fn with_admission(mut self, admission: Admission<'static, 1>) -> Result<Self, MemoryError> {
        validate_slot_admission(&admission)?;
        self.admission = Some(admission);
        Ok(self)
    }
    /// Claim one actual empty output vector.
    pub fn reserve(&self) -> Result<PreparedSqlRowsBuilder<A>, MemoryError> {
        let vector = SqlPreparedSlot::checkout(&self.vectors, self.admission.as_ref())?;
        Ok(PreparedSqlRowsBuilder {
            rows: PreparedSqlRows { vector },
        })
    }
}
fn drain<A>(vector: &mut FixedVector<'_, A>) {
    let mut continuation = std::mem::DropGuard::new(vector, |vector| {
        while let Some(value) = vector.pop() {
            drop(value);
        }
    });
    while let Some(value) = continuation.pop() {
        drop(value);
    }
    let _ = std::mem::DropGuard::dismiss(continuation);
}
impl<A: Send + 'static> PreparedSqlRowsBuilder<A> {
    pub(crate) fn get_mut(&mut self, index: usize) -> Option<&mut A> {
        self.rows.vector.as_mut_slice().get_mut(index)
    }
    pub(crate) fn pop(&mut self) -> Option<A> {
        self.rows.vector.pop()
    }
    /// Number of initialized rows.
    #[must_use]
    pub fn len(&self) -> usize {
        self.rows.vector.len()
    }
    /// Whether the builder is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    /// Physical maximum row count.
    #[must_use]
    pub fn capacity(&self) -> usize {
        self.rows.vector.capacity()
    }
    /// Available row positions.
    #[must_use]
    pub fn remaining(&self) -> usize {
        self.capacity() - self.len()
    }
    /// Push one row or return the unchanged value when full.
    pub fn try_push(&mut self, value: A) -> Result<(), A> {
        self.rows.vector.push(value)
    }
    /// Transfer the same actual vector lease into an immutable result.
    #[must_use]
    pub fn finish(self) -> PreparedSqlRows<A> {
        self.rows
    }
}
impl<A: Send + 'static> PreparedSqlRows<A> {
    pub(crate) fn get_mut(&mut self, index: usize) -> Option<&mut A> {
        self.vector.as_mut_slice().get_mut(index)
    }
    pub(crate) fn take_first(&mut self) -> Option<A> {
        self.vector.remove_first()
    }
    /// Borrow rows in their insertion order.
    #[must_use]
    pub fn as_slice(&self) -> &[A] {
        self.vector.as_slice()
    }
    /// Number of retained rows.
    #[must_use]
    pub fn len(&self) -> usize {
        self.vector.len()
    }
    /// Whether there are no retained rows.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.vector.is_empty()
    }
    /// Iterate borrowed rows in order.
    pub fn iter(&self) -> std::slice::Iter<'_, A> {
        self.as_slice().iter()
    }
}
impl<A: Send + 'static> Drop for PreparedSqlRows<A> {
    fn drop(&mut self) {
        drain(&mut self.vector);
    }
}

/// Setup failure for prepared native row backing.
#[derive(Debug)]
pub enum PreparedNativeRowsError {
    /// The selected memory region could not be opened.
    Region(effect::memory::MemoryError),
    /// The row pool's requested backing could not be prepared.
    Rows(sqlx::PreparedTursoRowMemoryError),
}
impl std::fmt::Display for PreparedNativeRowsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Region(error) => error.fmt(f),
            Self::Rows(error) => error.fmt(f),
        }
    }
}
impl std::error::Error for PreparedNativeRowsError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Region(error) => Some(error),
            Self::Rows(error) => Some(error),
        }
    }
}
/// Prepare actual retained native row backing from the selected memory region.
pub fn prepare_native_rows(
    memory: &effect::memory::PreparedMemory<'static>,
    region: effect::memory::RegionId,
    capacity: sqlx::PreparedTursoRowsCapacity,
) -> Result<sqlx::PreparedTursoRowsPool, PreparedNativeRowsError> {
    let allocator = memory
        .allocator(region)
        .map_err(PreparedNativeRowsError::Region)?;
    sqlx::PreparedTursoRowsPool::prepare(allocator, capacity).map_err(PreparedNativeRowsError::Rows)
}

type DriverRows = Arc<sqlx::PreparedTursoRowsPool>;
struct RowLane {
    in_use: AtomicBool,
    driver: DriverRows,
}

/// Fixed cursor lanes with one private driver row registry per lane.
pub struct PreparedSqlRowLanes {
    lanes: FixedVector<'static, RowLane>,
}

/// Startup backing for exclusive native row lanes.
#[derive(Clone, Copy, Debug)]
pub struct PreparedSqlRowLanesDemand {
    /// Requested lane control and driver vector bytes.
    pub region_bytes: Bytes,
    /// One native pool Arc per lane, in addition to the driver's controls.
    pub lane_pool_controls: usize,
    /// Payload layout of each native pool Arc.
    pub lane_pool_payload: Layout,
    /// Driver layouts and startup control count for one lane.
    pub driver_per_lane: sqlx::PreparedTursoRowsDemand,
}

impl PreparedSqlRowLanes {
    /// Calculate requested storage. Arc headers and allocator rounding are separate.
    pub fn demand(
        lanes: usize,
        per_lane: sqlx::PreparedTursoRowsCapacity,
    ) -> Result<PreparedSqlRowLanesDemand, PreparedNativeRowsError> {
        let driver_per_lane = per_lane.demand::<effect::memory::RegionAllocator<'static>>()
            .map_err(PreparedNativeRowsError::Rows)?;
        let region_bytes = (|| {
            let index = Layout::array::<RowLane>(lanes).map_err(|_| MemoryError::InvalidLayout)?;
            let per_lane = Bytes::new(driver_per_lane.requested_vector_bytes);
            Bytes::new(index.size()).checked_add(per_lane.checked_mul(lanes)?)
        })()
        .map_err(PreparedNativeRowsError::Region)?;
        Ok(PreparedSqlRowLanesDemand {
            region_bytes,
            lane_pool_controls: lanes,
            lane_pool_payload: Layout::new::<sqlx::PreparedTursoRowsPool>(),
            driver_per_lane,
        })
    }

    /// Build independent driver row registries before the memory region is sealed.
    pub fn prepare(
        memory: &PreparedMemory<'static>,
        region: RegionId,
        lanes: usize,
        per_lane: sqlx::PreparedTursoRowsCapacity,
    ) -> Result<Self, PreparedNativeRowsError> {
        Self::demand(lanes, per_lane)?;
        let region_error = PreparedNativeRowsError::Region;
        let allocator = memory.allocator(region).map_err(region_error)?;
        let mut prepared = FixedVector::prepare(allocator, lanes).map_err(region_error)?;
        for _ in 0..lanes {
            let lane = RowLane {
                in_use: AtomicBool::new(false),
                driver: Arc::new(prepare_native_rows(memory, region, per_lane)?),
            };
            prepared.push(lane).map_err(|_| region_error(MemoryError::NoSlot))?;
        }
        Ok(Self { lanes: prepared })
    }

    #[expect(clippy::result_large_err, reason = "preserves the owned SQL error representation")]
    fn reserve(self: &Arc<Self>) -> Result<ReservedNativeRows, crate::SqlError> {
        let mut backing_exhausted = false;
        for (index, lane) in self.lanes.as_slice().iter().enumerate() {
            if lane
                .in_use
                .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
                .is_err()
            {
                continue;
            }
            let lease = RowLaneLease { owner: self.clone(), index };
            match lane.driver.reserve_cursor() {
                Ok(permit) => return Ok(ReservedNativeRows { permit, lane: Some(lease) }),
                Err(sqlx::PreparedTursoRowMemoryError::NoSlot) => backing_exhausted = true,
                Err(error) => return Err(crate::SqlError::PreparedRow(error)),
            }
        }
        if backing_exhausted {
            Err(crate::SqlError::PreparedRow(sqlx::PreparedTursoRowMemoryError::NoSlot))
        } else {
            Err(crate::SqlError::Memory(MemoryError::NoSlot))
        }
    }
}

#[derive(Clone)]
pub(crate) enum NativeRows {
    Shared(DriverRows),
    Lanes(Arc<PreparedSqlRowLanes>),
}
impl NativeRows {
    #[expect(clippy::result_large_err, reason = "preserves the owned SQL error representation")]
    pub(crate) fn reserve(&self) -> Result<ReservedNativeRows, crate::SqlError> {
        match self {
            Self::Shared(pool) => {
                let permit = pool.reserve_cursor().map_err(crate::SqlError::PreparedRow)?;
                Ok(ReservedNativeRows { permit, lane: None })
            }
            Self::Lanes(lanes) => lanes.reserve(),
        }
    }
}

pub(crate) struct RowLaneLease {
    owner: Arc<PreparedSqlRowLanes>,
    index: usize,
}
impl Drop for RowLaneLease {
    #[expect(clippy::expect_used, reason = "the lane index comes from the immutable lane vector")]
    fn drop(&mut self) {
        self.owner.lanes.as_slice().get(self.index).expect("live row lane")
            .in_use.store(false, Ordering::Release);
    }
}

pub(crate) struct ReservedNativeRows {
    // Unopened backing returns to its registry before the lane becomes available.
    pub(crate) permit: sqlx::PreparedTursoCursorReservation,
    pub(crate) lane: Option<RowLaneLease>,
}
