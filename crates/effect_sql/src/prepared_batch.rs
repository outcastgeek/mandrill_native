//! Fixed, move-only batches of prepared SQL statements.
use crate::{
    PreparedSqlRows, PreparedSqlRowsBuilder, PreparedSqlRowsCapacity, PreparedSqlRowsPool,
    PreparedSqlStatement, SqlError, SqlStatement,
    statement::MaterializationCharge,
};
use effect::memory::{Admission, Bytes, MemoryError, PreparedMemory, RegionId};

/// Explicit batch concurrency and retained statements per batch.
#[derive(Clone, Copy, Debug)]
pub struct PreparedSqlBatchCapacity {
    /// Simultaneously retained builders and batches.
    pub batches: usize,
    /// Maximum statements in each batch.
    pub statements_per_batch: usize,
}
impl PreparedSqlBatchCapacity {
    /// Actual region vector demand; statement backing, caller Arc and allocator
    /// overhead/rounding are separately owned.
    pub fn demand(self) -> Result<Bytes, MemoryError> {
        PreparedSqlRowsCapacity {
            outputs: self.batches,
            rows_per_output: self.statements_per_batch,
        }
        .demand::<BatchEntry>()
    }
}
/// Startup-prepared batch vectors, distinct from statement storage.
pub struct PreparedSqlBatchPool {
    rows: PreparedSqlRowsPool<BatchEntry>,
}
/// Exclusive batch builder.
pub struct PreparedSqlBatchBuilder {
    rows: PreparedSqlRowsBuilder<BatchEntry>,
}
/// Ordered owned batch retaining each actual prepared statement.
pub struct PreparedSqlBatch {
    rows: PreparedSqlRows<BatchEntry>,
}
enum BatchEntry {
    Prepared {
        source: PreparedSqlStatement,
        charge: Option<MaterializationCharge>,
    },
    Materialized(SqlStatement),
}
/// One output item, retaining the producing statement's position.
pub struct PreparedSqlBatchItem<A> {
    /// Zero-based input statement index.
    pub statement_index: usize,
    /// Native statement result metadata or a decoded row.
    pub value: either::Either<sqlx::TursoQueryResult, A>,
}
impl PreparedSqlBatchPool {
    /// Prepare every batch vector at startup.
    pub fn prepare(
        memory: &PreparedMemory<'static>,
        region: RegionId,
        capacity: PreparedSqlBatchCapacity,
    ) -> Result<Self, MemoryError> {
        Ok(Self {
            rows: crate::sql! { prepared_rows; memory: memory, region: region, capacity: PreparedSqlRowsCapacity { outputs: capacity.batches, rows_per_output: capacity.statements_per_batch }, }?,
        })
    }
    /// Couple each batch vector slot to one Slots work unit. Contained
    /// statements may independently select their own admission.
    pub fn with_admission(mut self, admission: Admission<'static, 1>) -> Result<Self, MemoryError> {
        self.rows = self.rows.with_admission(admission)?;
        Ok(self)
    }
    /// Claim one actual empty batch vector.
    pub fn reserve(&self) -> Result<PreparedSqlBatchBuilder, MemoryError> {
        Ok(PreparedSqlBatchBuilder {
            rows: self.rows.reserve()?,
        })
    }
}
impl PreparedSqlBatchBuilder {
    /// Retain one statement or return the original when full.
    #[expect(
        clippy::unreachable,
        reason = "the builder inserts only prepared entries"
    )]
    pub fn try_push(
        &mut self,
        statement: PreparedSqlStatement,
    ) -> Result<(), PreparedSqlStatement> {
        match self.rows.try_push(BatchEntry::Prepared { source: statement, charge: None }) {
            Ok(()) => Ok(()),
            Err(BatchEntry::Prepared { source, .. }) => Err(source),
            Err(BatchEntry::Materialized(_)) => unreachable!("builder inserts prepared statements"),
        }
    }
    /// Number of retained statements.
    #[must_use]
    pub fn len(&self) -> usize {
        self.rows.len()
    }
    /// Whether the batch is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }
    /// Maximum statements in this actual vector.
    #[must_use]
    pub fn capacity(&self) -> usize {
        self.rows.capacity()
    }
    /// Transfer the same actual vector and statement owners.
    #[must_use]
    pub fn finish(self) -> PreparedSqlBatch {
        PreparedSqlBatch {
            rows: self.rows.finish(),
        }
    }
}
impl PreparedSqlBatch {
    /// Number of retained statements.
    #[must_use]
    pub fn len(&self) -> usize {
        self.rows.len()
    }
    /// Whether the batch is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }
    #[expect(
        clippy::result_large_err,
        reason = "retains owned SQL diagnostics without an extra heap allocation"
    )]
    pub(crate) fn reserve_materializations(&mut self) -> Result<(), SqlError> {
        let result = self.reserve_unclaimed_materializations();
        if result.is_err() {
            for index in 0..self.rows.len() {
                if let Some(BatchEntry::Prepared { charge, .. }) = self.rows.get_mut(index) {
                    *charge = None;
                }
            }
        }
        result
    }

    #[expect(
        clippy::result_large_err,
        reason = "retains owned SQL diagnostics without an extra heap allocation"
    )]
    fn reserve_unclaimed_materializations(&mut self) -> Result<(), SqlError> {
        for index in 0..self.rows.len() {
            let entry = self.rows.get_mut(index).ok_or(SqlError::Memory(MemoryError::InvalidLayout))?;
            if let BatchEntry::Prepared { source, charge } = entry
                && charge.is_none()
            {
                *charge = source.reserve_materialization()?;
            }
        }
        Ok(())
    }

    #[expect(
        clippy::result_large_err,
        reason = "retains owned SQL diagnostics without an extra heap allocation"
    )]
    pub(crate) fn materialize_all(&mut self) -> Result<(), SqlError> {
        self.reserve_materializations()?;
        self.materialize_reserved_all()
    }

    #[expect(
        clippy::result_large_err,
        reason = "retains owned SQL diagnostics without an extra heap allocation"
    )]
    #[expect(
        clippy::unreachable,
        reason = "replacement follows a prepared-entry match"
    )]
    pub(crate) fn materialize_reserved_all(&mut self) -> Result<(), SqlError> {
        for index in 0..self.rows.len() {
            let entry = self
                .rows
                .get_mut(index)
                .ok_or(SqlError::Memory(MemoryError::InvalidLayout))?;
            if let BatchEntry::Prepared { source, charge } = entry {
                let native = source.materialize_reserved(charge.take())?;
                let old = std::mem::replace(entry, BatchEntry::Materialized(native));
                match (old, entry) {
                    (BatchEntry::Prepared { source, .. }, BatchEntry::Materialized(native)) => {
                        native.retention.prepared_source = Some(source);
                    }
                    _ => unreachable!("prepared entry replaced by materialized entry"),
                }
            }
        }
        Ok(())
    }
    pub(crate) fn take_first(&mut self) -> Option<SqlStatement> {
        match self.rows.take_first()? {
            BatchEntry::Materialized(native) => Some(native),
            BatchEntry::Prepared { .. } => None,
        }
    }
}
