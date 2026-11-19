//! Region-backed SQL text and explicitly typed binding descriptions.
use crate::{
    SqlError, SqlStatement,
    prepared_rows::{SqlPreparedSlot, validate_slot_admission},
    statement::MaterializationCharge,
};
use effect::memory::{
    Admission, Buffer, Bytes, CapacityUnit, FixedVector, MemoryError, PreparedMemory, RegionId, SlotPool, Text,
};
use sqlx::Arguments;
use std::ops::Range;
use std::sync::Arc;

/// Explicit concurrent statements and per-statement storage.
#[derive(Clone, Copy, Debug)]
pub struct PreparedSqlStatementCapacity {
    /// Simultaneous builders or retained statements.
    pub statements: usize,
    /// Maximum UTF-8 SQL bytes per statement.
    pub sql_bytes: usize,
    /// Maximum typed bindings per statement.
    pub bindings: usize,
    /// Maximum combined text/blob binding bytes per statement.
    pub binding_bytes: usize,
}
impl PreparedSqlStatementCapacity {
    /// Region slots, descriptors and payload bytes; excludes allocator overhead,
    /// alignment rounding, outer Arc controls and native materialization.
    pub fn demand(self) -> Result<Bytes, MemoryError> {
        let descriptors = std::alloc::Layout::array::<Binding>(self.bindings)
            .map_err(|_| MemoryError::InvalidLayout)?;
        let payload = Bytes::new(self.sql_bytes)
            .checked_add(Bytes::new(self.binding_bytes))?
            .checked_add(Bytes::new(descriptors.size()))?;
        SlotPool::<StatementStorage>::storage_bytes(self.statements)?
            .checked_add(payload.checked_mul(self.statements)?)
    }
}
/// A prepared-construction failure; no dynamic error allocation is needed.
#[derive(Clone, Copy, Debug)]
pub enum PreparedSqlStatementError {
    /// Actual backing or arithmetic refusal.
    Memory(MemoryError),
    /// Empty or NUL-containing identifier component.
    InvalidIdentifier,
    /// Empty or NUL-containing SQL text.
    InvalidSql,
}
impl std::fmt::Display for PreparedSqlStatementError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Memory(error) => error.fmt(f),
            Self::InvalidIdentifier => f.write_str("invalid SQL identifier"),
            Self::InvalidSql => f.write_str("invalid SQL text"),
        }
    }
}
impl std::error::Error for PreparedSqlStatementError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Memory(error) => Some(error),
            Self::InvalidIdentifier | Self::InvalidSql => None,
        }
    }
}
impl From<MemoryError> for PreparedSqlStatementError {
    fn from(error: MemoryError) -> Self {
        Self::Memory(error)
    }
}

enum Binding {
    Null,
    Integer(i64),
    Real(f64),
    Text(Range<usize>),
    Blob(Range<usize>),
}
struct StatementStorage {
    sql: Text<'static>,
    bytes: Buffer<'static>,
    bindings: FixedVector<'static, Binding>,
}
impl StatementStorage {
    fn clear(&mut self) {
        self.sql.clear();
        self.bytes.clear();
        self.bindings.clear();
    }
    fn sql_room(&self, add: usize) -> Result<(), MemoryError> {
        let requested = self
            .sql
            .as_str()
            .len()
            .checked_add(add)
            .ok_or(MemoryError::ArithmeticOverflow)?;
        let capacity = self.sql.capacity();
        if requested > capacity.get() {
            return Err(MemoryError::ValueTooLarge {
                requested: Bytes::new(requested),
                capacity,
            });
        }
        Ok(())
    }
}
/// Startup-owned physical statement slots.
pub struct PreparedSqlStatementPool {
    slots: SlotPool<'static, StatementStorage>,
    admission: Option<Admission<'static, 1>>,
    materialization_admission: Option<Arc<Admission<'static, 1>>>,
}
/// Builder holding an exclusive statement slot until its first error.
pub struct PreparedSqlStatementBuilder {
    statement: Result<PreparedSqlStatement, PreparedSqlStatementError>,
}
/// Immutable SQL and typed bindings retaining the actual prepared storage.
pub struct PreparedSqlStatement {
    storage: SqlPreparedSlot<StatementStorage>,
    materialization_admission: Option<Arc<Admission<'static, 1>>>,
}
impl PreparedSqlStatementPool {
    /// Prepare every fixed SQL, binding-byte and descriptor buffer at startup.
    pub fn prepare(
        memory: &PreparedMemory<'static>,
        region: RegionId,
        capacity: PreparedSqlStatementCapacity,
    ) -> Result<Self, MemoryError> {
        let _ = capacity.demand()?;
        let slots = memory.slots(region, capacity.statements, |_| {
            Ok(StatementStorage {
                sql: memory.text(region, Bytes::new(capacity.sql_bytes))?,
                bytes: memory.buffer(region, Bytes::new(capacity.binding_bytes))?,
                bindings: FixedVector::prepare(memory.allocator(region)?, capacity.bindings)?,
            })
        })?;
        Ok(Self {
            slots,
            admission: None,
            materialization_admission: None,
        })
    }
    /// Couple each checked-out statement slot to one Slots work unit.
    pub fn with_admission(mut self, admission: Admission<'static, 1>) -> Result<Self, MemoryError> {
        validate_slot_admission(&admission)?;
        self.admission = Some(admission);
        Ok(self)
    }
    /// Select a shared byte admission for each live SQLx materialization.
    pub fn with_materialization_admission(
        mut self,
        admission: Arc<Admission<'static, 1>>,
    ) -> Result<Self, MemoryError> {
        if admission.capacities().first().map(|capacity| capacity.unit) != Some(CapacityUnit::Bytes) {
            return Err(MemoryError::UnsupportedRunContract);
        }
        self.materialization_admission = Some(admission);
        Ok(self)
    }
    /// Claim one physical statement slot without growing backing.
    pub fn reserve(&self) -> Result<PreparedSqlStatementBuilder, MemoryError> {
        Ok(PreparedSqlStatementBuilder {
            statement: Ok(PreparedSqlStatement {
                storage: SqlPreparedSlot::checkout(&self.slots, self.admission.as_ref())?,
                materialization_admission: self.materialization_admission.clone(),
            }),
        })
    }
}
impl PreparedSqlStatementBuilder {
    fn apply(
        mut self,
        operation: impl FnOnce(&mut StatementStorage) -> Result<(), PreparedSqlStatementError>,
    ) -> Self {
        if let Ok(statement) = &mut self.statement
            && let Err(error) = operation(&mut statement.storage)
        {
            self.statement = Err(error);
        }
        self
    }
    /// Append caller-reviewed SQL text, rejecting NUL and overflow.
    #[must_use]
    pub fn text(self, value: &str) -> Self {
        self.apply(|storage| {
            if value.contains('\0') {
                return Err(PreparedSqlStatementError::InvalidSql);
            }
            storage.sql.try_push_str(value)?;
            Ok(())
        })
    }
    /// Append a dotted, SQL-quoted identifier without a temporary String.
    #[must_use]
    pub fn identifier(self, value: &str) -> Self {
        self.apply(|storage| {
            if value.contains('\0') || value.split('.').any(|part| part.trim().is_empty()) {
                return Err(PreparedSqlStatementError::InvalidIdentifier);
            }
            let mut length = value.len();
            for part in value.split('.') {
                length = length
                    .checked_add(2)
                    .ok_or(MemoryError::ArithmeticOverflow)?;
                for byte in part.bytes() {
                    if byte == b'"' {
                        length = length
                            .checked_add(1)
                            .ok_or(MemoryError::ArithmeticOverflow)?;
                    }
                }
            }
            storage.sql_room(length)?;
            for (index, part) in value.split('.').enumerate() {
                if index != 0 {
                    storage.sql.try_push_str(".")?;
                }
                storage.sql.try_push_str("\"")?;
                for (piece_index, piece) in part.split('"').enumerate() {
                    if piece_index != 0 {
                        storage.sql.try_push_str("\"\"")?;
                    }
                    storage.sql.try_push_str(piece)?;
                }
                storage.sql.try_push_str("\"")?;
            }
            Ok(())
        })
    }
    fn bind(self, value: Binding, bytes: &[u8]) -> Self {
        self.apply(|storage| {
            if storage.bindings.len() == storage.bindings.capacity() {
                return Err(MemoryError::NoSlot.into());
            }
            storage.sql_room(1)?;
            let start = storage.bytes.len();
            let end = start
                .checked_add(bytes.len())
                .ok_or(MemoryError::ArithmeticOverflow)?;
            if end > storage.bytes.capacity().get() {
                return Err(MemoryError::ValueTooLarge {
                    requested: Bytes::new(end),
                    capacity: storage.bytes.capacity(),
                }
                .into());
            }
            storage.bytes.extend_from_slice(bytes)?;
            let value = match value {
                Binding::Text(_) => Binding::Text(start..end),
                Binding::Blob(_) => Binding::Blob(start..end),
                other @ (Binding::Null | Binding::Integer(_) | Binding::Real(_)) => other,
            };
            if storage.bindings.push(value).is_err() {
                return Err(MemoryError::NoSlot.into());
            }
            storage.sql.try_push_str("?")?;
            Ok(())
        })
    }
    /// Bind SQL NULL.
    #[must_use]
    pub fn null(self) -> Self {
        self.bind(Binding::Null, &[])
    }
    /// Bind a signed integer.
    #[must_use]
    pub fn integer(self, value: i64) -> Self {
        self.bind(Binding::Integer(value), &[])
    }
    /// Bind a floating-point value.
    #[must_use]
    pub fn real(self, value: f64) -> Self {
        self.bind(Binding::Real(value), &[])
    }
    /// Copy and bind UTF-8 text.
    #[must_use]
    pub fn text_param(self, value: &str) -> Self {
        self.bind(Binding::Text(0..0), value.as_bytes())
    }
    /// Copy and bind binary bytes.
    #[must_use]
    pub fn blob(self, value: &[u8]) -> Self {
        self.bind(Binding::Blob(0..0), value)
    }
    /// Return the first error or the same immutable statement lease.
    pub fn finish(self) -> Result<PreparedSqlStatement, PreparedSqlStatementError> {
        let statement = self.statement?;
        if statement.storage.sql.as_str().trim().is_empty() {
            return Err(PreparedSqlStatementError::InvalidSql);
        }
        Ok(statement)
    }
}
impl PreparedSqlStatement {
    /// Borrow the retained SQL text.
    #[must_use]
    pub fn sql(&self) -> &str {
        self.storage.sql.as_str()
    }
    /// Materialize a native SQLx statement. A selected admission charges each
    /// copy's requested SQL, binding payload and argument descriptor bytes.
    #[expect(
        clippy::result_large_err,
        reason = "retains the owned SQL error representation without an extra heap allocation"
    )]
    pub fn materialize(&self) -> Result<SqlStatement, SqlError> {
        self.materialize_reserved(self.reserve_materialization()?)
    }

    #[expect(
        clippy::result_large_err,
        reason = "retains the owned SQL error representation without an extra heap allocation"
    )]
    pub(crate) fn reserve_materialization(&self) -> Result<Option<MaterializationCharge>, SqlError> {
        if let Some(admission) = self.materialization_admission.as_deref() {
            let layout = std::alloc::Layout::array::<sqlx::TursoValue>(self.storage.bindings.len())
                .map_err(|_| SqlError::Memory(MemoryError::InvalidLayout))?;
            let requested = Bytes::new(self.sql().len())
                .checked_add(Bytes::new(self.storage.bytes.len()))
                .and_then(|bytes| bytes.checked_add(Bytes::new(layout.size())))
                .map_err(SqlError::Memory)?;
            admission.reserve([requested.get()], [0]).map(Some).map_err(SqlError::Memory)
        } else {
            Ok(None)
        }
    }

    #[expect(
        clippy::result_large_err,
        reason = "retains the owned SQL error representation without an extra heap allocation"
    )]
    pub(crate) fn materialize_reserved(&self, charge: Option<MaterializationCharge>) -> Result<SqlStatement, SqlError> {
        let mut arguments = sqlx::TursoArguments::default();
        arguments
            .try_reserve_exact(self.storage.bindings.len())
            .map_err(|_| SqlError::Memory(MemoryError::CollectionReservationFailed))?;
        for binding in self.storage.bindings.as_slice() {
            match binding {
                Binding::Null => arguments.add(None::<i64>),
                Binding::Integer(value) => arguments.add(*value),
                Binding::Real(value) => arguments.add(*value),
                Binding::Text(range) => {
                    let bytes = self
                        .storage
                        .bytes
                        .as_slice()
                        .get(range.clone())
                        .ok_or(SqlError::Memory(MemoryError::InvalidLayout))?;
                    let text = std::str::from_utf8(bytes)
                        .map_err(|_| SqlError::Memory(MemoryError::InvalidLayout))?;
                    let mut owned = String::new();
                    owned
                        .try_reserve_exact(text.len())
                        .map_err(|_| SqlError::Memory(MemoryError::CollectionReservationFailed))?;
                    owned.push_str(text);
                    arguments.add(owned)
                }
                Binding::Blob(range) => {
                    let bytes = self
                        .storage
                        .bytes
                        .as_slice()
                        .get(range.clone())
                        .ok_or(SqlError::Memory(MemoryError::InvalidLayout))?;
                    let mut owned = Vec::new();
                    owned
                        .try_reserve_exact(bytes.len())
                        .map_err(|_| SqlError::Memory(MemoryError::CollectionReservationFailed))?;
                    owned.extend_from_slice(bytes);
                    arguments.add(owned)
                }
            }
            .map_err(SqlError::Binding)?;
        }
        let mut text = String::new();
        text.try_reserve_exact(self.sql().len())
            .map_err(|_| SqlError::Memory(MemoryError::CollectionReservationFailed))?;
        text.push_str(self.sql());
        Ok(SqlStatement::materialized(text, arguments, charge))
    }
}
impl Drop for PreparedSqlStatement {
    fn drop(&mut self) {
        self.storage.clear();
    }
}
