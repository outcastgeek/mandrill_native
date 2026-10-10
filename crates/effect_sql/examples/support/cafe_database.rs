//! Native setup for the café demonstration; business workflows stay in the SQL DSL.

use effect_sql::sqlx::{
    SqlSafeStr, TursoConnectOptions, TursoPool, TursoPoolOptions,
    migrate::{Migration, MigrationType},
};
use effect_sql::{
    MigrationCatalog, MigrationStep, Rollback, SQLX_JOURNAL, SqlClient, SqlError, SqlMigrator,
};
use std::{
    borrow::Cow,
    path::PathBuf,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

/// Demonstration-only pool admission and native lock-wait policy, not library defaults.
const DEMO_POOL_CONNECTIONS: u32 = 2;
/// A local demonstration may wait this long for another SQLite/Turso writer.
const DEMO_WRITE_WAIT: Duration = Duration::from_secs(2);
/// Stable application schema identity, shared by up and down declarations.
const CAFE_SCHEMA_VERSION: i64 = 1;

/// One owned temporary directory; pool shutdown happens before filesystem disposal.
pub struct CafeDatabase {
    /// Native pool retained by the selected SQL client and the outer host.
    pub pool: TursoPool,
    /// Configuration-selected SQL service, with no independent Effect runtime.
    pub client: SqlClient,
    /// Owns only this demonstration's directory.
    directory: PathBuf,
}
impl CafeDatabase {
    /// Acquire a fresh file-backed local database and apply the declared application migration.
    pub async fn acquire() -> Result<Self, SqlError> {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| SqlError::Configuration("demo clock precedes epoch".into()))?
            .as_nanos();
        let directory =
            std::env::temp_dir().join(format!("effect-sql-cafe-{}-{stamp}", std::process::id()));
        std::fs::create_dir(&directory)
            .map_err(|error| SqlError::Configuration(error.to_string()))?;
        let options = TursoConnectOptions::new()
            .filename(directory.join("cafe.db"))
            .create_if_missing(true)
            .busy_timeout(DEMO_WRITE_WAIT);
        let pool = TursoPoolOptions::new()
            .max_connections(DEMO_POOL_CONNECTIONS)
            .connect_with(options)
            .await?;
        let client = SqlClient::new(pool.clone());
        let database = Self {
            pool,
            client,
            directory,
        };
        let rollback = Migration::new(
            CAFE_SCHEMA_VERSION,
            Cow::Borrowed("cafe orders"),
            MigrationType::ReversibleDown,
            "DROP TABLE coffee_orders".into_sql_str(),
            false,
        );
        let catalog = MigrationCatalog::new(
            SQLX_JOURNAL,
            vec![MigrationStep::transactional(
                CAFE_SCHEMA_VERSION,
                "cafe orders",
                "CREATE TABLE coffee_orders(id INTEGER PRIMARY KEY, customer_id INTEGER NOT NULL, drink TEXT NOT NULL, price_cents INTEGER NOT NULL CHECK(price_cents>=0), ready INTEGER NOT NULL CHECK(ready IN (0,1)))",
                Rollback::Reversible(rollback),
            )],
        )?;
        if let Err(error) = SqlMigrator::new(database.client.kernel().clone(), catalog)?
            .up()
            .await
        {
            database.pool.close().await;
            return Err(error);
        }
        Ok(database)
    }
    /// Join native pool shutdown before removing the temporary demonstration directory.
    pub async fn close(self) -> Result<(), SqlError> {
        self.pool.close().await;
        std::fs::remove_dir_all(&self.directory)
            .map_err(|error| SqlError::Configuration(error.to_string()))
    }
}
impl Drop for CafeDatabase {
    /// Best-effort filesystem fallback after an early host error; the normal path awaits pool close.
    fn drop(&mut self) {
        let _removed = std::fs::remove_dir_all(&self.directory);
    }
}
