//! Finite local scenario policy shared by the named V03 contracts.
use effect_sql::sqlx::{TursoConnectOptions, TursoPool, TursoPoolOptions};
use effect_sql::{SqlClient, SqlError};
use std::{
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
/// Allocation identity only; this is not an iteration or stress target.
static NEXT_DATABASE: AtomicU64 = AtomicU64::new(0);
/// Exactly two independent leases are needed to exercise native database authority.
const CONNECTIONS: u32 = 2;
/// Fail a competing write immediately while a deterministic barrier owns the first writer.
const LOCK_WAIT: Duration = Duration::ZERO;
/// One test-owned database directory; normal closure joins the pool before removing files.
pub struct Database {
    /// SQL service selected for this fixture.
    pub client: SqlClient,
    /// Native pool joined before filesystem cleanup.
    pub pool: TursoPool,
    /// Temporary directory owned exclusively by this fixture.
    directory: PathBuf,
}
impl Database {
    /// Acquire an isolated real native database using the declared finite scenario policy.
    pub async fn open() -> Result<Self, SqlError> {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| SqlError::Configuration("fixture clock precedes epoch".into()))?
            .as_nanos();
        let sequence = NEXT_DATABASE.fetch_add(1, Ordering::Relaxed);
        let directory = std::env::temp_dir().join(format!(
            "effect-sql-v03-{}-{stamp}-{sequence}",
            std::process::id()
        ));
        std::fs::create_dir(&directory)
            .map_err(|error| SqlError::Configuration(error.to_string()))?;
        let options = TursoConnectOptions::new()
            .filename(directory.join("contract.db"))
            .create_if_missing(true)
            .busy_timeout(LOCK_WAIT);
        let pool = TursoPoolOptions::new()
            .max_connections(CONNECTIONS)
            .connect_with(options)
            .await?;
        Ok(Self {
            client: SqlClient::new(pool.clone()),
            pool,
            directory,
        })
    }
    /// Await native disposal before deleting only this fixture's owned files.
    pub async fn close(self) -> Result<(), SqlError> {
        self.pool.close().await;
        std::fs::remove_dir_all(&self.directory)
            .map_err(|error| SqlError::Configuration(error.to_string()))
    }
}
impl Drop for Database {
    /// Filesystem fallback after an assertion; normal closure awaits native owners.
    fn drop(&mut self) {
        let _removed = std::fs::remove_dir_all(&self.directory);
    }
}
