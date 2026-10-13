//! Finite V03 SQL contracts; native driver/core tests retain their existing lower-level evidence.
#![cfg(feature = "turso")]

use effect::{effect, effect_die, effect_sync, effect_try_sync};
#[path = "support/database.rs"]
/// Shared native database policy for these finite contracts.
mod database;
use database::Database;
use effect::{
    Cancellation, CleanupFailure, Defect, Effect, Exit, Finalization, HasCancellation,
    PrimaryOutcome,
};
use effect_sql::sqlx::{
    Row, SqlSafeStr,
    migrate::{Migration, MigrationType},
};
use effect_sql::{
    CellCodec, MigrationCatalog, MigrationStep, RecoveryDecision, Rollback, RowDecoder, RowError,
    SQLX_JOURNAL, SqlClient, SqlError, SqlMigrator, SqlPredicate, row, sql, sql_insert, sql_update,
    sql_with,
};
use std::{borrow::Cow, sync::Arc};

/// Move successful test values without requiring Clone or converting another exit channel.
fn success<A>(outcome: Exit<A, SqlError>) -> Result<A, SqlError> {
    match outcome {
        Exit::Success(value) => Ok(value),
        Exit::Fail(error) => Err(error),
        Exit::Die(defect) => Err(SqlError::NativeInvocation(Box::new(Exit::Die(defect)))),
        Exit::Interrupted => Err(SqlError::NativeInvocation(Box::new(Exit::Interrupted))),
        Exit::FinalizationFailed(details) => Err(SqlError::NativeInvocation(Box::new(
            Exit::FinalizationFailed(details),
        ))),
    }
}
/// One owned payload deliberately excludes Clone.
#[derive(Debug, PartialEq, Eq)]
struct Drink(#[doc = "Owned native text moved through SQL binding and decoding."] String);
/// A row program reads native values and refines under the same owned row.
fn drink_row() -> RowDecoder<Drink, SqlClient> {
    row! { let text = native_column::<String, SqlClient>("drink")?; Drink(text) }
}

/// Binding order, quoting, SQL NULL/native mismatch, optional absence, scalar/fold and batch results.
#[tokio::test]
async fn statements_rows_and_result_modes() -> Result<(), SqlError> {
    let database = Database::open().await?;
    let env = Arc::new(database.client.clone());
    success(database.client.execute(|| sql! { text: "CREATE TABLE orders(id INTEGER PRIMARY KEY, drink TEXT, amount INTEGER, ready INTEGER)", }).run(env.clone()).await)?;
    let moved = Drink(String::from("oat latte"));
    let insert = sql_insert! { table: "orders", value("id", 1_i64); value("drink", moved.0); value("amount", 425_i64); value("ready", 1_i64); };
    assert_eq!(
        success(database.client.execute(insert).run(env.clone()).await)?,
        1
    );
    let update = sql_update! { table: "orders", set("amount", 450_i64); where(SqlPredicate::equal("id", 1_i64)?); };
    assert_eq!(
        success(database.client.execute(update).run(env.clone()).await)?,
        1
    );
    let read = sql_with! { handle: database.client.clone(), run |client| { client.first(sql! { text("SELECT drink FROM "); identifier("orders"); text(" WHERE id="); param(1_i64); }, drink_row())? } };
    assert_eq!(
        success(read.run(env.clone()).await)?,
        Some(Drink(String::from("oat latte")))
    );
    let absent = database.client.first(
        || sql! { text: "SELECT drink FROM orders WHERE id=99", },
        drink_row(),
    );
    assert_eq!(success(absent.run(env.clone()).await)?, None);
    let buffered = database
        .client
        .query(|| sql! { text: "SELECT drink FROM orders", }, drink_row());
    assert_eq!(success(buffered.run(env.clone()).await)?.len(), 1);
    let absent_scalar: Effect<i64, SqlError, SqlClient> = database
        .client
        .scalar(|| sql! { text: "SELECT amount FROM orders WHERE id=99", });
    assert!(matches!(
        absent_scalar.run(env.clone()).await,
        Exit::Fail(SqlError::RowAbsent)
    ));
    assert!(matches!(
        database
            .client
            .first(|| sql! { text: "SELECT 'x' AS other", }, drink_row())
            .run(env.clone())
            .await,
        Exit::Fail(SqlError::Decode(RowError::MissingColumn(_)))
    ));
    let scalar: Effect<i64, SqlError, SqlClient> = database
        .client
        .scalar(|| sql! { text: "SELECT amount FROM orders", });
    assert_eq!(success(scalar.run(env.clone()).await)?, 450);
    let fold = database.client.fold_rows(
        || sql! { text: "SELECT drink FROM orders", },
        drink_row(),
        String::new,
        |mut all, Drink(value)| {
            all.push_str(&value);
            Ok(all)
        },
    );
    assert_eq!(success(fold.run(env.clone()).await)?, "oat latte");
    let folded = database.client.fold_rows_effect(
        || sql! { text: "SELECT drink FROM orders", },
        drink_row(),
        || 0_usize,
        |count, _| effect_sync! { once; body: move || count + 1, },
    );
    assert_eq!(success(folded.run(env.clone()).await)?, 1);
    let batch = database.client.batch(|| {
        Ok(vec![
            sql! { text: "SELECT drink FROM orders; SELECT amount FROM orders", }?,
        ])
    });
    let batch = success(batch.run(env.clone()).await)?;
    assert_eq!(
        batch
            .into_iter()
            .flat_map(|result| result.items)
            .filter(|item| matches!(item, either::Either::Left(_)))
            .count(),
        2
    );
    let nullable: RowDecoder<Option<String>, SqlClient> =
        row! { optional_column("drink", CellCodec::<String, String, SqlClient>::identity())? };
    assert_eq!(
        success(
            database
                .client
                .first(|| sql! { text: "SELECT NULL AS drink", }, nullable)
                .run(env.clone())
                .await
        )?,
        Some(None)
    );
    assert!(matches!(
        database
            .client
            .first(|| sql! { text: "SELECT NULL AS drink", }, drink_row())
            .run(env.clone())
            .await,
        Exit::Fail(SqlError::Decode(RowError::NullColumn(_)))
    ));
    assert!(matches!(
        database
            .client
            .first(|| sql! { text: "SELECT 'wrong' AS ready", }, {
                let __dsl_program: RowDecoder<bool, SqlClient> = row! {
                project: |row| row
                                    .try_get::<i64, _>("ready")
                                    .map(|value| value == 1)
                                    .map_err(|source| RowError::Native {
                                        column: "ready".into(),
                                        source
                                    }),
                };
                __dsl_program
            })
            .run(env.clone())
            .await,
        Exit::Fail(SqlError::Decode(RowError::Native { .. }))
    ));
    let quoted = effect_sql::Identifier::new("odd\"name")?;
    assert_eq!(quoted.quoted(), "\"odd\"\"name\"");
    let null = SqlPredicate::equal("drink", Option::<String>::None)?;
    assert!(
        sql! {
            text("SELECT id FROM orders");
            where(null);
        }?
        .sql()
        .ends_with("IS NULL")
    );
    database.close().await
}

/// Savepoint rollback leaves outer writes intact; outer errors/defects settle before handle reuse.
#[tokio::test]
async fn transaction_savepoint_and_stale_handles() -> Result<(), SqlError> {
    let database = Database::open().await?;
    let root = database.client.kernel().session().await?;
    root.execute(sql! { text: "CREATE TABLE writes(id INTEGER PRIMARY KEY)", }?)
        .await?;
    let tx = root.begin().await?;
    tx.execute(sql! { text: "INSERT INTO writes VALUES(1)", }?)
        .await?;
    let nested = tx.begin_savepoint().await?;
    nested
        .execute(sql! { text: "INSERT INTO writes VALUES(2)", }?)
        .await?;
    nested.rollback().await?;
    let next = tx.begin_savepoint().await?;
    assert!(matches!(
        nested.execute(sql! { text: "SELECT 1", }?).await,
        Err(SqlError::InactiveSession)
    ));
    next.commit().await?;
    let cursor = tx.cursor(sql! { text: "SELECT id FROM writes", }?).await?;
    assert!(matches!(
        tx.execute(sql! { text: "SELECT 1", }?).await,
        Err(SqlError::InactiveSession)
    ));
    cursor.close().await?;
    tx.commit().await?;
    let next_tx = root.begin().await?;
    assert!(matches!(
        tx.execute(sql! { text: "SELECT 1", }?).await,
        Err(SqlError::InactiveSession)
    ));
    next_tx.rollback().await?;
    root.close().await?;
    assert!(matches!(
        root.execute(sql! { text: "SELECT 1", }?).await,
        Err(SqlError::InactiveSession)
    ));
    let env = Arc::new(database.client.clone());
    let failed: Effect<(), SqlError, SqlClient> = database.client.transaction(|tx| {
        effect! {
 bind;
 program: tx.execute(|| sql! { text: "INSERT INTO writes VALUES(3)", }),
 body: |_| {
                effect_try_sync! { body: { let __dsl_error = || SqlError::Configuration("deliberate rollback".into()); move || Err(__dsl_error()) }, on_error: ::core::convert::identity, }
            },
 }
    });
    assert!(matches!(
        failed.run(env.clone()).await,
        Exit::Fail(SqlError::Configuration(_))
    ));
    let defected: Effect<(), SqlError, SqlClient> = database.client.transaction(|tx| {
        effect! {
        bind;
        program: tx.execute(|| sql! { text: "INSERT INTO writes VALUES(4)", }),
        body: |_| effect_die! { defect: Defect::new("deliberate body defect"), },
        }
    });
    assert!(matches!(defected.run(env.clone()).await, Exit::Die(_)));
    let count: Effect<i64, SqlError, SqlClient> = database
        .client
        .scalar(|| sql! { text: "SELECT COUNT(*) FROM writes", });
    assert_eq!(success(count.run(env).await)?, 1);
    database.close().await
}

/// External cancellation is requested at a deterministic row boundary; the lease closes afterward.
struct CancelEnv {
    /// Host-owned cancellation source requested at the row boundary.
    cancel: Cancellation,
}
impl HasCancellation for CancelEnv {
    /// Select this fixture's cancellation source without creating another token.
    fn cancellation(&self) -> &Cancellation {
        &self.cancel
    }
}
/// Retain interruption and ordered native cleanup, including uncertain commit acknowledgment.
#[tokio::test]
async fn cancellation_and_native_cleanup_channels() -> Result<(), SqlError> {
    let database = Database::open().await?;
    let env = Arc::new(CancelEnv {
        cancel: Cancellation::new(),
    });
    let (entered, waiting) = tokio::sync::oneshot::channel();
    let entered = Arc::new(std::sync::Mutex::new(Some(entered)));
    let (release, released) = tokio::sync::oneshot::channel();
    let released = Arc::new(tokio::sync::Mutex::new(Some(released)));
    let decoder = row! {
        let entered = entered.clone();
        let released = released.clone();
            if let Some(entered) = entered
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take()
            {
                let _sent = entered.send(());
            }
            if let Some(released) = released.lock().await.take() {
                let _released = released.await;
            }
            1_i64
    };
    let fold = database.client.fold_rows(
        || sql! { text: "SELECT 1 AS value UNION ALL SELECT 2", },
        decoder,
        || 0_i64,
        |sum, value| Ok(sum + value),
    );
    let run = tokio::spawn(fold.run_cancellable(env.clone()));
    waiting
        .await
        .map_err(|error| SqlError::Configuration(error.to_string()))?;
    let _requested = env.cancel.request();
    let _released = release.send(());
    assert!(matches!(
        run.await
            .map_err(|error| SqlError::Configuration(error.to_string()))?,
        Exit::Interrupted
    ));
    let carrier = SqlError::NativeInvocation(Box::new(Exit::FinalizationFailed(Finalization {
        primary: PrimaryOutcome::Defected(Defect::new("body")),
        first: CleanupFailure::ReleaseFailure(SqlError::CommitOutcomeUnknown(
            effect_sql::sqlx::Error::Protocol("acknowledgment unavailable".into()),
        )),
        rest: vec![CleanupFailure::ReleaseInterrupted],
    })));
    assert!(carrier.outcome_uncertain());
    assert!(matches!(
        carrier.into_exit::<()>(),
        Exit::FinalizationFailed(Finalization {
            primary: PrimaryOutcome::Defected(_),
            ..
        })
    ));
    let session = database.client.kernel().session().await?;
    session.execute(sql! { text: "BEGIN", }?).await?;
    session.close().await?;
    let unknown = database
        .client
        .kernel()
        .transaction(|connection| {
            Box::pin(async move {
                connection.native_execute("COMMIT", ()).await?;
                Ok(())
            })
        })
        .await;
    match unknown {
        Err(error) => assert!(error.outcome_uncertain()),
        Ok(()) => {
            return Err(SqlError::Configuration(
                "unacknowledged transaction must not report success".into(),
            ));
        }
    }
    let fresh = database.client.kernel().session().await?;
    fresh.execute(sql! { text: "SELECT 1", }?).await?;
    fresh.close().await?;
    database.close().await
}

/// Stable catalog with an explicit rollback declaration; fixtures never build a second journal.
fn catalog() -> Result<MigrationCatalog, SqlError> {
    let down = Migration::new(
        1,
        Cow::Borrowed("fixture"),
        MigrationType::ReversibleDown,
        "DROP TABLE ledger".into_sql_str(),
        false,
    );
    MigrationCatalog::new(
        SQLX_JOURNAL,
        vec![MigrationStep::transactional(
            1,
            "fixture",
            "CREATE TABLE ledger(id INTEGER PRIMARY KEY)",
            Rollback::Reversible(down),
        )],
    )
}
/// Locked history rejects stale down plans and dirty recovery inspects actual schema deliberately.
#[tokio::test]
async fn migration_plan_dirty_recovery_and_write_authority() -> Result<(), SqlError> {
    let database = Database::open().await?;
    let migrator = SqlMigrator::new(database.client.kernel().clone(), catalog()?)?;
    assert_eq!(migrator.up().await?, 1);
    let stale = migrator.plan_down(1).await?;
    let current = migrator.plan_down(1).await?;
    assert_eq!(migrator.down(current).await?, 1);
    assert!(migrator.down(stale).await.is_err());
    let root = database.client.kernel().session().await?;
    let owner = root.begin().await?;
    assert!(migrator.up().await.is_err());
    owner.rollback().await?;
    root.close().await?;
    assert_eq!(migrator.up().await?, 1);
    database
        .client
        .kernel()
        .with_connection(|connection| {
            Box::pin(async move {
                effect_sql::sqlx::query(
                    "UPDATE _sqlx_migrations SET success=FALSE WHERE version=1",
                )
                .execute(connection)
                .await?;
                Ok(())
            })
        })
        .await?;
    assert!(migrator.inspect_dirty().await?.is_some());
    assert!(migrator.plan_up().await.is_err());
    migrator
        .recover(|_dirty, connection| {
            Box::pin(async move {
                let present: i64 = effect_sql::sqlx::query_scalar(
                    "SELECT COUNT(*) FROM sqlite_schema WHERE type='table' AND name='ledger'",
                )
                .fetch_one(connection)
                .await?;
                if present == 1 {
                    Ok(RecoveryDecision::StepApplied)
                } else {
                    Ok(RecoveryDecision::StepNotApplied)
                }
            })
        })
        .await?;
    assert!(migrator.inspect_dirty().await?.is_none());
    assert!(migrator.plan_up().await?.is_empty());
    database.close().await
}
