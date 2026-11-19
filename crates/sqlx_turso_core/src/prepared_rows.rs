//! Fixed-capacity retained row backing; native SDK reads remain separately allocated.
use crate::{
    TursoColumn, TursoRow, TursoTypeInfo, TursoValue,
    value::{BlobValue, TextValue, TursoValueKind},
};
use sqlx_core::{HashMap, column::Column, ext::ustr::UStr};
use std::{
    alloc::{AllocError, Allocator, Layout},
    ptr::NonNull,
    sync::{Arc, Mutex},
};

#[derive(Clone)]
pub(crate) struct SharedAllocator(Arc<dyn Allocator + Send + Sync>);
// SAFETY: every operation delegates to the same retained allocator owner.
unsafe impl Allocator for SharedAllocator {
    fn allocate(&self, layout: Layout) -> Result<NonNull<[u8]>, AllocError> {
        self.0.allocate(layout)
    }
    unsafe fn deallocate(&self, ptr: NonNull<u8>, layout: Layout) {
        unsafe { self.0.deallocate(ptr, layout) }
    }
    unsafe fn grow(
        &self,
        ptr: NonNull<u8>,
        old: Layout,
        new: Layout,
    ) -> Result<NonNull<[u8]>, AllocError> {
        unsafe { self.0.grow(ptr, old, new) }
    }
    unsafe fn shrink(
        &self,
        ptr: NonNull<u8>,
        old: Layout,
        new: Layout,
    ) -> Result<NonNull<[u8]>, AllocError> {
        unsafe { self.0.shrink(ptr, old, new) }
    }
}

type Buffer<T> = Vec<T, SharedAllocator>;
pub(crate) struct RowBytes(pub(crate) Buffer<u8>);
pub(crate) struct RowValues {
    pub(crate) values: Buffer<TursoValue>,
    pending: Buffer<Pending>,
}
impl std::fmt::Debug for RowValues {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_list().entries(&self.values).finish()
    }
}
enum PendingKind {
    Null,
    Integer(i64),
    Real(f64),
    Text(std::ops::Range<usize>),
    Blob(std::ops::Range<usize>),
}
struct Pending {
    kind: PendingKind,
    info: TursoTypeInfo,
}
struct Cell {
    row: Arc<RowValues>,
    bytes: Arc<RowBytes>,
}
type Registry = Mutex<Buffer<Option<Cell>>>;

/// Explicit physical capacities of the retained-row pool.
#[derive(Clone, Copy, Debug)]
pub struct PreparedTursoRowsCapacity {
    pub rows: usize,
    pub max_columns: usize,
    pub bytes_per_row: usize,
}
/// Scalar backing admission failures.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PreparedTursoRowMemoryError {
    NoSlot,
    Capacity,
    ArithmeticOverflow,
    Allocation,
}
/// Failure to copy an SDK row into its reserved backing.
#[derive(Debug)]
pub enum PreparedTursoRowError {
    Memory(PreparedTursoRowMemoryError),
    Native(turso::Error),
}
impl std::fmt::Display for PreparedTursoRowMemoryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "prepared row backing: {self:?}")
    }
}
impl std::error::Error for PreparedTursoRowMemoryError {}
impl std::fmt::Display for PreparedTursoRowError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Memory(e) => e.fmt(f),
            Self::Native(e) => e.fmt(f),
        }
    }
}
impl std::error::Error for PreparedTursoRowError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Memory(e) => Some(e),
            Self::Native(e) => Some(e),
        }
    }
}
/// Checked layouts for region-backed vectors. Arc control allocations and allocator rounding are separate.
#[derive(Clone, Copy, Debug)]
pub struct PreparedTursoRowsDemand {
    pub registry: Layout,
    pub values_per_row: Layout,
    pub descriptors_per_row: Layout,
    pub bytes_per_row: Layout,
    pub row_payload: Layout,
    pub byte_payload: Layout,
    pub registry_payload: Layout,
    pub allocator_payload: Layout,
    pub startup_arc_controls: usize,
    pub requested_vector_bytes: usize,
}
impl PreparedTursoRowsCapacity {
    /// Computes requested vector and startup payload layouts before preparation.
    pub fn demand<A: Allocator + Send + Sync + 'static>(
        self,
    ) -> Result<PreparedTursoRowsDemand, PreparedTursoRowMemoryError> {
        let err = |_| PreparedTursoRowMemoryError::ArithmeticOverflow;
        let startup_arc_controls = self
            .rows
            .checked_mul(2)
            .and_then(|n| n.checked_add(2))
            .ok_or(PreparedTursoRowMemoryError::ArithmeticOverflow)?;
        let registry = Layout::array::<Option<Cell>>(self.rows).map_err(err)?;
        let values_per_row = Layout::array::<TursoValue>(self.max_columns).map_err(err)?;
        let descriptors_per_row = Layout::array::<Pending>(self.max_columns).map_err(err)?;
        let bytes_per_row = Layout::array::<u8>(self.bytes_per_row).map_err(err)?;
        let requested_vector_bytes = values_per_row
            .size()
            .checked_add(descriptors_per_row.size())
            .and_then(|n| n.checked_add(bytes_per_row.size()))
            .and_then(|n| n.checked_mul(self.rows))
            .and_then(|n| n.checked_add(registry.size()))
            .ok_or(PreparedTursoRowMemoryError::ArithmeticOverflow)?;
        Ok(PreparedTursoRowsDemand {
            registry,
            values_per_row,
            descriptors_per_row,
            bytes_per_row,
            registry_payload: Layout::new::<Registry>(),
            allocator_payload: Layout::new::<A>(),
            requested_vector_bytes,
            row_payload: Layout::new::<RowValues>(),
            byte_payload: Layout::new::<RowBytes>(),
            startup_arc_controls,
        })
    }
}
fn vector<T>(
    allocator: SharedAllocator,
    cap: usize,
) -> Result<Buffer<T>, PreparedTursoRowMemoryError> {
    let mut v = Vec::new_in(allocator);
    v.try_reserve_exact(cap)
        .map_err(|_| PreparedTursoRowMemoryError::Allocation)?;
    Ok(v)
}
/// Startup-prepared retained row owners. Allocator and per-row Arc controls are native startup allocations.
pub struct PreparedTursoRowsPool {
    registry: Arc<Registry>,
    capacity: PreparedTursoRowsCapacity,
}
impl PreparedTursoRowsPool {
    /// Allocates all registry, value, descriptor and byte backing using the supplied allocator.
    /// The allocator owner adds one startup Arc with payload `Layout::new::<A>()`;
    /// Arc control overhead and allocator rounding are separate from requested vector bytes.
    pub fn prepare<A: Allocator + Send + Sync + 'static>(
        allocator: A,
        capacity: PreparedTursoRowsCapacity,
    ) -> Result<Self, PreparedTursoRowMemoryError> {
        capacity.demand::<A>()?;
        let allocator = SharedAllocator(Arc::new(allocator));
        let mut cells = vector(allocator.clone(), capacity.rows)?;
        for _ in 0..capacity.rows {
            cells.push(Some(Cell {
                row: Arc::new(RowValues {
                    values: vector(allocator.clone(), capacity.max_columns)?,
                    pending: vector(allocator.clone(), capacity.max_columns)?,
                }),
                bytes: Arc::new(RowBytes(vector(allocator.clone(), capacity.bytes_per_row)?)),
            }));
        }
        Ok(Self {
            registry: Arc::new(Mutex::new(cells)),
            capacity,
        })
    }
    /// Reserve the first row backing before native cursor construction.
    pub fn reserve_cursor(
        self: &Arc<Self>,
    ) -> Result<PreparedTursoCursorReservation, PreparedTursoRowMemoryError> {
        Ok(PreparedTursoCursorReservation {
            permit: self.reserve()?,
            pool: self.clone(),
        })
    }
    /// Claims backing only if the known column count fits.
    pub(crate) fn reserve_columns(
        &self,
        columns: usize,
    ) -> Result<PreparedTursoRowReservation, PreparedTursoRowMemoryError> {
        self.check_columns(columns)?;
        self.reserve()
    }
    pub(crate) fn check_columns(&self, columns: usize) -> Result<(), PreparedTursoRowMemoryError> {
        if columns > self.capacity.max_columns {
            return Err(PreparedTursoRowMemoryError::Capacity);
        }
        Ok(())
    }
    /// Claims an actually reusable row and byte owner. Escaped values prevent byte reuse.
    #[expect(
        clippy::unreachable,
        reason = "the fixed registry index exists and the taken row has no other owners"
    )]
    pub fn reserve(&self) -> Result<PreparedTursoRowReservation, PreparedTursoRowMemoryError> {
        for index in 0..self.capacity.rows {
            let cell = {
                let mut cells = self.registry.lock().unwrap_or_else(|e| e.into_inner());
                let slot = cells
                    .get_mut(index)
                    .unwrap_or_else(|| unreachable!("fixed registry index"));
                if slot
                    .as_mut()
                    .is_some_and(|cell| Arc::get_mut(&mut cell.row).is_some())
                {
                    slot.take()
                } else {
                    None
                }
            };
            if let Some(mut cell) = cell {
                let row = Arc::get_mut(&mut cell.row)
                    .unwrap_or_else(|| unreachable!("claimed row is unique"));
                row.values.clear();
                row.pending.clear();
                if let Some(bytes) = Arc::get_mut(&mut cell.bytes) {
                    bytes.0.clear();
                    return Ok(PreparedTursoRowReservation {
                        registry: self.registry.clone(),
                        index,
                        cell: Some(cell),
                        capacity: self.capacity,
                    });
                }
                let mut cells = self.registry.lock().unwrap_or_else(|e| e.into_inner());
                *cells
                    .get_mut(index)
                    .unwrap_or_else(|| unreachable!("fixed registry index")) = Some(cell);
            }
        }
        Err(PreparedTursoRowMemoryError::NoSlot)
    }
}
/// Exclusive permit for one row; dropping it returns the backing.
#[must_use]
pub struct PreparedTursoRowReservation {
    registry: Arc<Registry>,
    index: usize,
    cell: Option<Cell>,
    capacity: PreparedTursoRowsCapacity,
}
impl PreparedTursoRowReservation {
    /// Copies one SDK row with existing shared metadata. SDK get_value may allocate transient native text/blob values.
    #[expect(
        clippy::unreachable,
        reason = "the exclusive permit owns both unique backing Arcs until publication"
    )]
    pub fn copy_native(
        mut self,
        raw: &turso::Row,
        columns: Arc<[TursoColumn]>,
        names: Arc<HashMap<UStr, usize>>,
    ) -> Result<TursoRow, PreparedTursoRowError> {
        use PreparedTursoRowMemoryError as M;
        if columns.len() > self.capacity.max_columns {
            return Err(PreparedTursoRowError::Memory(M::Capacity));
        }
        let cell = self
            .cell
            .as_mut()
            .unwrap_or_else(|| unreachable!("reservation owns cell"));
        let row =
            Arc::get_mut(&mut cell.row).unwrap_or_else(|| unreachable!("reservation owns row"));
        let bytes =
            Arc::get_mut(&mut cell.bytes).unwrap_or_else(|| unreachable!("reservation owns bytes"));
        for (index, column) in columns.iter().enumerate() {
            let value = raw
                .get_value(index)
                .map_err(PreparedTursoRowError::Native)?;
            let kind = match value {
                turso::Value::Null => PendingKind::Null,
                turso::Value::Integer(v) => PendingKind::Integer(v),
                turso::Value::Real(v) => PendingKind::Real(v),
                turso::Value::Text(v) => {
                    let end = bytes
                        .0
                        .len()
                        .checked_add(v.len())
                        .ok_or(PreparedTursoRowError::Memory(M::ArithmeticOverflow))?;
                    if end > self.capacity.bytes_per_row {
                        return Err(PreparedTursoRowError::Memory(M::Capacity));
                    }
                    let range = bytes.0.len()..end;
                    bytes.0.extend_from_slice(v.as_bytes());
                    PendingKind::Text(range)
                }
                turso::Value::Blob(v) => {
                    let end = bytes
                        .0
                        .len()
                        .checked_add(v.len())
                        .ok_or(PreparedTursoRowError::Memory(M::ArithmeticOverflow))?;
                    if end > self.capacity.bytes_per_row {
                        return Err(PreparedTursoRowError::Memory(M::Capacity));
                    }
                    let range = bytes.0.len()..end;
                    bytes.0.extend_from_slice(&v);
                    PendingKind::Blob(range)
                }
            };
            row.pending.push(Pending {
                kind,
                info: column.type_info().clone(),
            });
        }
        while let Some(pending) = row.pending.pop() {
            let kind = match pending.kind {
                PendingKind::Null => TursoValueKind::Null,
                PendingKind::Integer(v) => TursoValueKind::Integer(v),
                PendingKind::Real(v) => TursoValueKind::Real(v),
                PendingKind::Text(range) => {
                    TursoValueKind::Text(TextValue::Prepared(cell.bytes.clone(), range))
                }
                PendingKind::Blob(range) => {
                    TursoValueKind::Blob(BlobValue::Prepared(cell.bytes.clone(), range))
                }
            };
            let info = if pending.info == TursoTypeInfo::NULL {
                match &kind {
                    TursoValueKind::Null => TursoTypeInfo::NULL,
                    TursoValueKind::Integer(_) => TursoTypeInfo::new("INTEGER"),
                    TursoValueKind::Real(_) => TursoTypeInfo::new("REAL"),
                    TursoValueKind::Text(_) => TursoTypeInfo::new("TEXT"),
                    TursoValueKind::Blob(_) => TursoTypeInfo::new("BLOB"),
                }
            } else {
                pending.info
            };
            row.values.push(TursoValue::prepared(kind, info));
        }
        row.values.reverse();
        let result = TursoRow::prepared(columns, names, cell.row.clone());
        Ok(result)
    }
}
impl Drop for PreparedTursoRowReservation {
    #[expect(
        clippy::unreachable,
        reason = "the permit retains its fixed registry and claimed index"
    )]
    fn drop(&mut self) {
        if let Some(mut cell) = self.cell.take() {
            if let Some(row) = Arc::get_mut(&mut cell.row) {
                row.values.clear();
                row.pending.clear();
                if let Some(bytes) = Arc::get_mut(&mut cell.bytes) {
                    bytes.0.clear();
                }
            }
            let mut cells = self.registry.lock().unwrap_or_else(|e| e.into_inner());
            *cells
                .get_mut(self.index)
                .unwrap_or_else(|| unreachable!("fixed registry index")) = Some(cell);
        }
    }
}

/// First row permit and the exact pool used by a prepared incremental cursor.
#[must_use]
pub struct PreparedTursoCursorReservation {
    pub(crate) permit: PreparedTursoRowReservation,
    pub(crate) pool: Arc<PreparedTursoRowsPool>,
}
impl PreparedTursoRowReservation {
    pub(crate) fn check_columns(&self, columns: usize) -> Result<(), PreparedTursoRowMemoryError> {
        if columns > self.capacity.max_columns {
            Err(PreparedTursoRowMemoryError::Capacity)
        } else {
            Ok(())
        }
    }
}
