use std::{
    collections::VecDeque,
    fmt, future,
    marker::PhantomData,
    ops::{Deref, DerefMut},
};

#[cfg(feature = "migrate")]
use std::{
    fs,
    hash::{DefaultHasher, Hash, Hasher},
    path::PathBuf,
};

use sqlx_core::{
    common::StatementCache, connection::Connection, error::Error, transaction::Transaction,
};

use crate::{Turso, TursoStatement, driver::TursoDriverConnection, options::TursoConnectOptions};

#[cfg(feature = "migrate")]
use crate::TursoDatabaseTarget;

/// SQLx connection handle for Turso databases
pub struct TursoConnection {
    options: TursoConnectOptions,
    raw: turso::Connection,
    #[cfg(feature = "sync")]
    sync: Option<turso::sync::Database>,
    statements: StatementCache<TursoStatement>,
    transaction_state: TransactionState,
    #[cfg(feature = "migrate")]
    migration_lock: Option<fs::File>,
}

impl fmt::Debug for TursoConnection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TursoConnection")
            .field("options", &self.options)
            .field("transaction_state", &self.transaction_state)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Default)]
struct TransactionState {
    depth: usize,
    native_outer_depth: usize,
    pending_rollback_depths: VecDeque<usize>,
    rollback_failed: bool,
    #[cfg(feature = "migrate")]
    migration_operation_active: bool,
}

impl TransactionState {
    fn depth(&self) -> usize {
        self.depth
    }

    fn increment_depth(&mut self) {
        self.depth += 1;
    }

    fn decrement_depth(&mut self) {
        if self.depth > self.native_outer_depth {
            self.depth -= 1;
        }
    }

    fn with_native_outer_transaction() -> Self {
        Self {
            depth: 1,
            native_outer_depth: 1,
            ..Self::default()
        }
    }

    fn native_outer_depth(&self) -> usize {
        self.native_outer_depth
    }

    fn adopt_native_outer_transaction(&mut self) {
        self.depth = 1;
        self.native_outer_depth = 1;
    }

    fn mark_rollback_needed(&mut self) {
        if self.depth <= self.native_outer_depth {
            return;
        }

        self.pending_rollback_depths.push_back(self.depth);
        self.decrement_depth();
    }

    fn pop_pending_rollback(&mut self) -> Option<usize> {
        self.pending_rollback_depths.pop_front()
    }

    fn has_failed_rollback(&self) -> bool {
        self.rollback_failed
    }

    fn mark_rollback_failed(&mut self) {
        self.rollback_failed = true;
        self.pending_rollback_depths.clear();
    }
}

impl TursoConnection {
    pub(crate) fn new(options: TursoConnectOptions, connection: TursoDriverConnection) -> Self {
        Self::new_with_transaction_state(options, connection, TransactionState::default())
    }

    fn new_with_transaction_state(
        options: TursoConnectOptions,
        connection: TursoDriverConnection,
        transaction_state: TransactionState,
    ) -> Self {
        Self {
            statements: StatementCache::new(options.get_statement_cache_capacity()),
            options,
            raw: connection.raw,
            #[cfg(feature = "sync")]
            sync: connection.sync,
            transaction_state,
            #[cfg(feature = "migrate")]
            migration_lock: None,
        }
    }

    /// Adopts an existing native Turso connection for SQLx use.
    pub fn from_native_connection(connection: turso::Connection) -> Result<Self, Error> {
        let transaction_state = if connection
            .is_autocommit()
            .map_err(crate::executor::map_turso_error)?
        {
            TransactionState::default()
        } else {
            TransactionState::with_native_outer_transaction()
        };
        Ok(Self::new_with_transaction_state(
            TursoConnectOptions::new(),
            TursoDriverConnection {
                raw: connection,
                #[cfg(feature = "sync")]
                sync: None,
            },
            transaction_state,
        ))
    }

    /// Adopts an existing native sync connection together with its database owner.
    #[cfg(feature = "sync")]
    pub fn from_native_sync_parts(
        database: turso::sync::Database,
        connection: turso::Connection,
    ) -> Result<Self, Error> {
        let transaction_state = if connection
            .is_autocommit()
            .map_err(crate::executor::map_turso_error)?
        {
            TransactionState::default()
        } else {
            TransactionState::with_native_outer_transaction()
        };
        Ok(Self::new_with_transaction_state(
            TursoConnectOptions::from_native_sync_database(database.clone()),
            TursoDriverConnection {
                raw: connection,
                sync: Some(database),
            },
            transaction_state,
        ))
    }

    /// Creates a lifetime-bound SQLx executor for an open native transaction.
    /// SQLx transactions started through this executor are savepoints; the
    /// native transaction remains the owner of the outer commit or rollback.
    #[must_use]
    pub fn from_native_transaction<'transaction>(
        transaction: &'transaction turso::transaction::Transaction<'_>,
    ) -> TursoNativeTransaction<'transaction> {
        let connection = Self::new_with_transaction_state(
            TursoConnectOptions::new(),
            TursoDriverConnection {
                raw: Deref::deref(transaction).clone(),
                #[cfg(feature = "sync")]
                sync: None,
            },
            TransactionState::with_native_outer_transaction(),
        );
        TursoNativeTransaction {
            connection,
            native_transaction: PhantomData,
        }
    }

    /// Executes one native Turso statement on this checked-out connection.
    ///
    /// The adapter completes pending SQLx transaction cleanup first and keeps
    /// the native handle private, so a clone or statement cannot outlive the
    /// SQLx connection and later race a pool borrower. Use
    /// [`Self::into_native_connection`] when unrestricted native access is
    /// required.
    pub async fn native_execute(
        &mut self,
        sql: impl AsRef<str>,
        params: impl turso::IntoParams,
    ) -> Result<u64, Error> {
        self.clear_pending_rollback().await?;
        self.raw
            .execute(sql, params)
            .await
            .map_err(crate::executor::map_turso_error)
    }

    /// Executes native Turso batch SQL on this checked-out connection.
    pub async fn native_execute_batch(&mut self, sql: impl AsRef<str>) -> Result<(), Error> {
        self.clear_pending_rollback().await?;
        self.raw
            .execute_batch(sql)
            .await
            .map_err(crate::executor::map_turso_error)
    }

    /// Executes native Turso batch statements on this checked-out connection.
    pub async fn native_batch<I>(&mut self, statements: I) -> Result<Vec<turso::BatchResult>, Error>
    where
        I: IntoIterator,
        I::Item: turso::IntoBatchStatement,
    {
        self.clear_pending_rollback().await?;
        self.raw
            .batch(statements)
            .await
            .map_err(crate::executor::map_turso_error)
    }

    /// Executes an atomic native Turso batch on this checked-out connection.
    pub async fn native_transactional_batch<I>(
        &mut self,
        statements: I,
        behavior: turso::transaction::TransactionBehavior,
    ) -> Result<Vec<turso::BatchResult>, Error>
    where
        I: IntoIterator,
        I::Item: turso::IntoBatchStatement,
    {
        self.clear_pending_rollback().await?;
        self.raw
            .transactional_batch(statements, behavior)
            .await
            .map_err(crate::executor::map_turso_error)
    }

    /// Transfers the native connection out of this SQLx connection after
    /// completing pending SQLx transaction cleanup.
    pub async fn into_native_connection(mut self) -> Result<turso::Connection, Error> {
        self.clear_pending_rollback().await?;
        Ok(self.raw)
    }

    /// Transfers a sync-backed native connection and its retained database owner together.
    #[cfg(feature = "sync")]
    pub async fn into_native_sync_parts(
        mut self,
    ) -> Result<(turso::sync::Database, turso::Connection), Error> {
        self.clear_pending_rollback().await?;
        let database = self.sync.take().ok_or_else(|| {
            crate::error::unsupported_sqlx("native sync transfer from a local Turso connection")
        })?;
        Ok((database, self.raw))
    }

    pub(crate) fn transaction_depth(&self) -> usize {
        self.transaction_state.depth()
    }

    pub(crate) fn increment_transaction_depth(&mut self) {
        self.transaction_state.increment_depth();
    }

    pub(crate) fn decrement_transaction_depth(&mut self) {
        self.transaction_state.decrement_depth();
    }

    pub(crate) fn native_outer_transaction_depth(&self) -> usize {
        self.transaction_state.native_outer_depth()
    }

    pub(crate) fn mark_rollback_needed(&mut self) {
        self.transaction_state.mark_rollback_needed();
    }

    pub(crate) async fn clear_pending_rollback(&mut self) -> Result<(), Error> {
        if self.transaction_state.has_failed_rollback() {
            return Err(Error::WorkerCrashed);
        }

        #[cfg(feature = "migrate")]
        if self.migration_lock.is_some() && !self.transaction_state.migration_operation_active {
            self.release_migration_lock()?;
        }

        while let Some(depth) = self.transaction_state.pop_pending_rollback() {
            let sql = sqlx_core::transaction::rollback_ansi_transaction_sql(depth);
            if let Err(error) = self.raw().execute(sql.as_str(), ()).await {
                if depth == 1 && rollback_error_is_inactive_transaction(&error) {
                    continue;
                }

                self.transaction_state.mark_rollback_failed();
                return Err(crate::executor::map_turso_error(error));
            }
        }

        let is_autocommit = self
            .raw
            .is_autocommit()
            .map_err(crate::executor::map_turso_error)?;
        if self.transaction_state.depth() > 0 && is_autocommit {
            self.transaction_state.mark_rollback_failed();
            return Err(Error::Protocol(
                "native transaction control ended an active SQLx transaction".to_owned(),
            ));
        }
        if self.transaction_state.depth() == 0 && !is_autocommit {
            self.transaction_state.adopt_native_outer_transaction();
        }

        Ok(())
    }

    #[cfg(feature = "migrate")]
    pub(crate) async fn acquire_migration_lock(&mut self) -> Result<(), Error> {
        self.clear_pending_rollback().await?;
        if self.transaction_state.depth() != 0 {
            return Err(Error::Protocol(
                "cannot acquire the migration lock inside an active transaction".to_owned(),
            ));
        }

        let path = migration_lock_path(&self.options)?;
        let file = tokio::task::spawn_blocking(move || {
            let file = fs::OpenOptions::new()
                .create(true)
                .truncate(false)
                .read(true)
                .write(true)
                .open(path)?;
            fs::File::lock(&file)?;
            Ok::<_, std::io::Error>(file)
        })
        .await
        .map_err(|error| Error::Protocol(format!("migration lock task failed: {error}")))??;
        self.migration_lock = Some(file);
        Ok(())
    }

    #[cfg(feature = "migrate")]
    pub(crate) fn release_migration_lock(&mut self) -> Result<(), Error> {
        if let Some(file) = self.migration_lock.take() {
            fs::File::unlock(&file).map_err(Error::Io)?;
        }
        Ok(())
    }

    #[cfg(feature = "migrate")]
    pub(crate) fn begin_migration_operation(&mut self) -> Result<(), Error> {
        self.transaction_state.migration_operation_active = self.migration_lock.is_some();
        Ok(())
    }

    #[cfg(feature = "migrate")]
    pub(crate) fn end_migration_operation(&mut self) {
        self.transaction_state.migration_operation_active = false;
    }

    /// Returns the options used to create this connection
    pub fn options(&self) -> &TursoConnectOptions {
        &self.options
    }

    pub(crate) fn raw(&self) -> &turso::Connection {
        &self.raw
    }

    /// Pushes local sync changes to the configured remote
    #[cfg(feature = "sync")]
    pub async fn sync_push(&self) -> Result<(), Error> {
        self.sync_database()?
            .push()
            .await
            .map_err(|error| crate::executor::map_turso_error_with_options(error, &self.options))
    }

    /// Pulls remote sync changes and applies them locally
    #[cfg(feature = "sync")]
    pub async fn sync_pull(&self) -> Result<bool, Error> {
        self.sync_database()?
            .pull()
            .await
            .map_err(|error| crate::executor::map_turso_error_with_options(error, &self.options))
    }

    /// Checkpoints the synced database WAL
    #[cfg(feature = "sync")]
    pub async fn sync_checkpoint(&self) -> Result<(), Error> {
        self.sync_database()?
            .checkpoint()
            .await
            .map_err(|error| crate::executor::map_turso_error_with_options(error, &self.options))
    }

    /// Returns synced database statistics
    #[cfg(feature = "sync")]
    pub async fn sync_stats(&self) -> Result<turso::sync::DatabaseSyncStats, Error> {
        self.sync_database()?
            .stats()
            .await
            .map_err(|error| crate::executor::map_turso_error_with_options(error, &self.options))
    }

    #[cfg(feature = "sync")]
    fn sync_database(&self) -> Result<&turso::sync::Database, Error> {
        self.sync
            .as_ref()
            .ok_or_else(|| crate::error::unsupported_sqlx("non-sync Turso connections"))
    }

    /// Returns the native synced database owner used by this connection.
    #[cfg(feature = "sync")]
    pub fn native_sync_database(&self) -> Option<&turso::sync::Database> {
        self.sync.as_ref()
    }

    pub(crate) fn cached_statement(&mut self, sql: &str) -> Option<TursoStatement> {
        self.statements.get_mut(sql).cloned()
    }

    pub(crate) fn cache_statement(&mut self, sql: &str, statement: TursoStatement) {
        if self.statements.is_enabled() {
            self.statements.insert(sql, statement);
        }
    }
}

#[cfg(feature = "migrate")]
fn migration_lock_path(options: &TursoConnectOptions) -> Result<PathBuf, Error> {
    match options.target() {
        TursoDatabaseTarget::File(path) => {
            let canonical = fs::canonicalize(path).map_err(Error::Io)?;
            let mut lock_path = canonical.as_os_str().to_owned();
            lock_path.push(".sqlx-migrate.lock");
            Ok(PathBuf::from(lock_path))
        }
        TursoDatabaseTarget::Memory { name } => {
            let mut hasher = DefaultHasher::new();
            name.hash(&mut hasher);
            Ok(std::env::temp_dir().join(format!(
                "sqlx-turso-memory-{:016x}.migrate.lock",
                hasher.finish()
            )))
        }
    }
}

/// SQLx executor tied to the lifetime of an open native Turso transaction.
pub struct TursoNativeTransaction<'transaction> {
    connection: TursoConnection,
    native_transaction: PhantomData<&'transaction ()>,
}

impl Deref for TursoNativeTransaction<'_> {
    type Target = TursoConnection;

    fn deref(&self) -> &Self::Target {
        &self.connection
    }
}

impl DerefMut for TursoNativeTransaction<'_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.connection
    }
}

fn rollback_error_is_inactive_transaction(error: &turso::Error) -> bool {
    error
        .to_string()
        .contains("cannot rollback - no transaction is active")
}

impl Connection for TursoConnection {
    type Database = Turso;
    type Options = TursoConnectOptions;

    async fn close(self) -> Result<(), Error> {
        Ok(())
    }

    async fn close_hard(self) -> Result<(), Error> {
        Ok(())
    }

    async fn ping(&mut self) -> Result<(), Error> {
        self.clear_pending_rollback().await?;
        let mut statement = self
            .raw()
            .prepare("SELECT 1")
            .await
            .map_err(crate::executor::map_turso_error)?;
        let mut rows = statement
            .query(())
            .await
            .map_err(crate::executor::map_turso_error)?;
        let _ = rows
            .next()
            .await
            .map_err(crate::executor::map_turso_error)?;
        Ok(())
    }

    async fn begin(&mut self) -> Result<Transaction<'_, Self::Database>, Error> {
        Transaction::begin(self, None).await
    }

    fn shrink_buffers(&mut self) {}

    async fn flush(&mut self) -> Result<(), Error> {
        self.clear_pending_rollback().await?;
        self.raw
            .cacheflush()
            .map_err(crate::executor::map_turso_error)
    }

    fn should_flush(&self) -> bool {
        false
    }

    fn cached_statements_size(&self) -> usize
    where
        Self::Database: sqlx_core::database::HasStatementCache,
    {
        self.statements.len()
    }

    fn clear_cached_statements(&mut self) -> impl Future<Output = Result<(), Error>> + Send + '_
    where
        Self::Database: sqlx_core::database::HasStatementCache,
    {
        self.statements.clear();
        future::ready(Ok(()))
    }
}

impl AsRef<TursoConnectOptions> for TursoConnection {
    fn as_ref(&self) -> &TursoConnectOptions {
        &self.options
    }
}
