//! Lifetime-bound SQLx binding helpers for native Identity operations.
//!
//! Transaction ownership and cleanup remain exclusively in SqlKernel. This helper
//! serializes mutable borrowing of that operation's connection, not database writes
//! across processes. Every collection query supplies its native contract bound.

use super::error::{NativeCommandFailure, ProviderFailure};
use sqlx::{Arguments, AssertSqlSafe, TursoArguments, TursoConnection, TursoRow};
use std::sync::{Arc, Mutex as DiagnosticMutex};
use tokio::sync::Mutex;

/// Private connection-operation diagnostic owner, independent of environment state.
#[derive(Clone, Default)]
pub(crate) struct OperationDiagnostics(Arc<DiagnosticMutex<Option<ProviderFailure>>>);
impl OperationDiagnostics {
    /// Retain the first provider failure before projecting its native category.
    fn retain(&self, error: sqlx::Error) -> ProviderFailure {
        let error = ProviderFailure(Arc::new(error));
        let mut retained = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if retained.is_none() {
            *retained = Some(error.clone());
        }
        error
    }
    /// Rejoin a narrowed command rejection with its operation's original source.
    pub(crate) fn command_error(
        &self,
        category: identity_rs::identity::StoreError,
    ) -> effect_sql::SqlError {
        let provider = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        effect_sql::SqlError::Binding(Box::new(NativeCommandFailure { category, provider }))
    }
}

/// One borrowed connection within a kernel-owned operation; it cannot escape run.
pub(crate) struct StoreConnection<'a> {
    /// Borrow serialization makes helper calls share exactly the same native owner.
    connection: Mutex<&'a mut TursoConnection>,
    /// Private per-operation failure retention; never shared by unrelated calls.
    diagnostics: OperationDiagnostics,
}
impl<'a> StoreConnection<'a> {
    /// Borrow an existing kernel owner; this acquires no pool lease or transaction.
    pub(crate) fn new(connection: &'a mut TursoConnection) -> Self {
        Self {
            connection: Mutex::new(connection),
            diagnostics: OperationDiagnostics::default(),
        }
    }
    /// Retain a diagnostic handle across an owned async body without copying payloads.
    pub(crate) fn diagnostics(&self) -> OperationDiagnostics {
        self.diagnostics.clone()
    }
    /// Execute bound trusted schema SQL and preserve acknowledged affected rows.
    pub(crate) async fn execute(
        &self,
        text: &'static str,
        values: impl StoreParams + Send,
    ) -> Result<u64, ProviderFailure> {
        let arguments = values
            .arguments()
            .map_err(|error| self.diagnostics.retain(error))?;
        let mut owner = self.connection.lock().await;
        sqlx::query_with(AssertSqlSafe(text), arguments)
            .execute(&mut **owner)
            .await
            .map(|result| result.rows_affected())
            .map_err(|error| self.diagnostics.retain(error))
    }
    /// Read native rows bounded by each statement's SQL limit/unique key. No cell
    /// conversion or security-payload copying is used to satisfy reusable bounds.
    pub(crate) async fn query(
        &self,
        text: &'static str,
        values: impl StoreParams + Send,
    ) -> Result<StoreRows, ProviderFailure> {
        let arguments = values
            .arguments()
            .map_err(|error| self.diagnostics.retain(error))?;
        let mut owner = self.connection.lock().await;
        let rows = sqlx::query_with(AssertSqlSafe(text), arguments)
            .fetch_all(&mut **owner)
            .await
            .map_err(|error| self.diagnostics.retain(error))?;
        Ok(StoreRows {
            rows: rows.into_iter(),
        })
    }
}

/// Finite native result owner; rows are moved out one at a time without Clone.
pub(crate) struct StoreRows {
    /// SQLx has already settled statement execution before releasing its borrow.
    rows: std::vec::IntoIter<TursoRow>,
}
impl StoreRows {
    /// Move the next native row. Provider failures occurred at the awaited query;
    /// decoding failures are preserved by each native persistence decoder.
    pub(crate) async fn next(&mut self) -> Result<Option<TursoRow>, sqlx::Error> {
        Ok(self.rows.next())
    }
}

/// Move or encode one native binding into SQLx's owned argument representation.
pub(crate) trait StoreValue {
    /// Encode once; SQLx's driver owns any unavoidable borrowed-byte conversion.
    fn append(self, arguments: &mut TursoArguments) -> Result<(), sqlx::Error>;
}
/// Encode the ordered tuple accepted by schema-local native commands.
pub(crate) trait StoreParams {
    /// Build the SQLx buffer without a blanket Clone or untyped value conversion.
    fn arguments(self) -> Result<TursoArguments, sqlx::Error>;
}
impl StoreParams for () {
    /// Empty commands bind no values.
    fn arguments(self) -> Result<TursoArguments, sqlx::Error> {
        Ok(TursoArguments::default())
    }
}

/// Generate concrete SQLx encoding implementations; the native argument buffer
/// performs necessary ownership conversion for borrowed text/blob bindings.
macro_rules! store_value {
    ($($value:ty),+ $(,)?) => { $(impl StoreValue for $value {
        /// Delegate the concrete native representation to SQLx's encoder.
        fn append(self, arguments: &mut TursoArguments) -> Result<(), sqlx::Error> {
            arguments.add(self).map_err(sqlx::Error::Encode)
        }
    })+ };
}
store_value!(
    String,
    Option<String>,
    i64,
    i32,
    u32,
    bool,
    Vec<u8>,
    &str,
    &[u8]
);

/// Tuple arities follow only the native store statements, retaining bind order.
macro_rules! store_params {
    ($($name:ident),+) => {
        impl<$($name: StoreValue),+> StoreParams for ($($name,)+) {
            /// Move each field into the buffer in native placeholder order.
            #[allow(non_snake_case)]
            fn arguments(self) -> Result<TursoArguments, sqlx::Error> {
                let ($($name,)+) = self;
                let mut arguments = TursoArguments::default();
                $($name.append(&mut arguments)?;)+
                Ok(arguments)
            }
        }
    };
}
store_params!(A);
store_params!(A, B);
store_params!(A, B, C);
store_params!(A, B, C, D);
store_params!(A, B, C, D, E);
store_params!(A, B, C, D, E, F);
