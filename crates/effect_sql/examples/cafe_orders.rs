//! Save a flat-white order, correct its price, and reload the saved customer record.
//!
//! Run: `DATABASE_URL=turso::memory: cargo +1.100.0 run -p effect-sql --features turso,macros --example cafe_orders`.
//! Input: ticket 1 for customer 7, entered at 400 cents. Expected output:
//! `Order 1: flat white, 425 cents, ready=false; 1 saved order.`
//!
//! Requires `turso,macros`. The static checked query describes only a literal,
//! so the compiler's `DATABASE_URL=turso::memory:` needs no application schema.
//! The support module owns an explicit temporary database/pool policy. The host
//! supplies Tokio once; SQL and Effect create no runtime of their own.

use effect::memory::{
    Admission, Buffer, BufferLease, Bytes, CapacityUnit, RegionId, ResourceCapacity, SlotPool,
};
use effect::{effect_die, effect_map_error, memory, service};
#[path = "support/cafe_database.rs"]
/// Native setup and teardown for the temporary café database.
mod cafe_database;

use effect::{Effect, Exit, SchemaIssue, SchemaIssueKind, effect};
use effect_sql::effect_from_option;
use effect_sql::effect_try_sync;
use effect_sql::{
    CellCodec, ColumnKey, PreparedNativeRowsError, PreparedSqlBatchCapacity, PreparedSqlBatchItem,
    PreparedSqlBatchPool, PreparedSqlBytesKind, PreparedSqlRowsCapacity, PreparedSqlRowsPool,
    PreparedSqlStatementCapacity, PreparedSqlStatementPool, RowDecoder, SqlClient, SqlError,
    SqlPredicate, SqlStartupError, SqlTransaction, row, sql, sql_insert, sql_update, sql_with,
};
use effect_sql::sqlx::PreparedTursoRowsCapacity;
use std::sync::Arc;

/// Application order identity, kept distinct from the library's native scalar.
#[derive(Debug, PartialEq, Eq)]
struct OrderId(#[doc = "Persisted native order key."] i64);
/// Application customer identity; no library defaults create customers.
struct CustomerId(#[doc = "Persisted native customer key."] i64);
/// Owned domain payload intentionally has no Clone implementation.
struct NewOrder {
    /// Application-assigned order identity.
    id: OrderId,
    /// Customer placing this order.
    customer: CustomerId,
    /// Owned drink name moved into its native binding.
    drink: String,
    /// Price in integer cents, validated before insertion.
    price_cents: i64,
}
/// Decoded application view, independent of native rows and cursor ownership.
#[derive(Debug, PartialEq, Eq)]
struct CoffeeOrder {
    /// Decoded application order identity.
    id: OrderId,
    /// Owned drink name decoded from the native row.
    drink: String,
    /// Validated nonnegative price in cents.
    price_cents: i64,
    /// Preparation state decoded with the strict boolean codec.
    ready: bool,
}
/// The application's declared error boundary, including expected absence and invalid input.
#[derive(Debug)]
enum CafeError {
    /// Explicit SQL-to-application error boundary.
    Sql(#[doc = "Original typed SQL failure."] SqlError),
    /// SQL startup validation or scoped acquisition failed.
    SqlStartup(#[doc = "Original typed SQL startup failure."] SqlStartupError),
    /// A required application order was absent.
    MissingOrder,
    /// Input or the demonstration's expected result was invalid.
    InvalidOrder,
    /// Complete non-success host outcome, including cleanup and interruption.
    Execution(#[doc = "Owned terminal Effect outcome."] Box<Exit<(), CafeError>>),
}
impl std::fmt::Display for CafeError {
    /// Keep provider details redacted while retaining the complete host outcome.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Sql(error) => std::fmt::Display::fmt(error, f),
            Self::SqlStartup(error) => std::fmt::Display::fmt(error, f),
            Self::MissingOrder => f.write_str("café order is absent"),
            Self::InvalidOrder => f.write_str("invalid café order"),
            Self::Execution(outcome) => match outcome.primary() {
                effect::PrimaryOutcomeRef::Succeeded => f.write_str("café cleanup failed"),
                effect::PrimaryOutcomeRef::Failed(_) => f.write_str("café execution failed"),
                effect::PrimaryOutcomeRef::Defected(_) => f.write_str("café execution defected"),
                effect::PrimaryOutcomeRef::WasInterrupted => {
                    f.write_str("café execution interrupted")
                }
            },
        }
    }
}
impl std::error::Error for CafeError {}
/// Selected outer application environment; D06 explicitly projects its SQL service.
struct CafeEnv {
    /// Shared configuration explicitly selected by D06.
    sql: Arc<SqlClient>,
}

/// Startup storage for the café's two prepared drink projections.
struct DrinkProjection {
    /// Shared prepared statement backing for the blob batch.
    statements: Arc<PreparedSqlStatementPool>,
    /// Batch slot retaining the prepared blob statement.
    batches: PreparedSqlBatchPool,
    /// One output position for the single saved drink.
    text_rows: Arc<PreparedSqlRowsPool<BufferLease<'static>>>,
    /// Space for both a blob row and the statement's result metadata.
    blob_items: Arc<PreparedSqlRowsPool<PreparedSqlBatchItem<BufferLease<'static>>>>,
    /// Shared byte buffers for both sequential reads.
    buffers: Arc<SlotPool<'static, Buffer<'static>>>,
}

/// Decode a native row, retaining structured refinement and strict boolean policy.
fn order_row() -> RowDecoder<CoffeeOrder, SqlClient> {
    row! {
        let id = native_column::<i64, SqlClient>("id")?;
        let drink = native_column::<String, SqlClient>("drink")?;
        let price_cents = native_column::<i64, SqlClient>("price_cents")?;
        let price_cents = refine(price_cents, |value: &i64| if *value >= 0 { Ok(()) } else { Err(SchemaIssue::new(SchemaIssueKind::OutOfRange)) }, "price_cents")?;
        let ready = column("ready", CellCodec::<bool, i64, SqlClient>::boolean())?;
        CoffeeOrder { id: OrderId(id), drink, price_cents, ready }
    }
}

/// Update the operator-corrected price using the transaction's checked handle.
/// Each invocation shares the checked transaction owner through a handle clone.
#[expect(clippy::result_large_err, reason = "retains the complete SQL failure without boxing")]
fn correct_price(tx: SqlTransaction, id: i64) -> Effect<u64, SqlError, SqlClient> {
    effect! {
        let tx = tx.clone();
        let predicate = effect_try_sync! { once; body: { let __dsl_result = SqlPredicate::equal("id", id); move || __dsl_result }, on_error: ::core::convert::identity, }?;
        let statement = sql_update! { table: "coffee_orders", set("price_cents", 425i64); where(predicate); };
        tx.execute(statement)?
    }
}

/// Move the non-Clone order once; all statement construction stays inside execution.
#[expect(clippy::result_large_err, reason = "retains the complete SQL failure without boxing")]
fn save_order(order: NewOrder) -> Effect<OrderId, CafeError, CafeEnv> {
    sql_with! {
        select: |env: &CafeEnv| Arc::clone(&env.sql),
        on_error: CafeError::Sql,
        run |sql| {
            let order = effect_try_sync! {
                once;
                body {
                    if order.price_cents >= 0 && !order.drink.is_empty() { Ok(order) }
                    else { Err(SqlError::Configuration("invalid café order".into())) }
                },
            }?;
            let id = order.id.0;
            let insert = sql_insert! {
                table: "coffee_orders", value("id", id); value("customer_id", order.customer.0);
                value("drink", order.drink); value("price_cents", order.price_cents); value("ready", false);
            };
            sql.execute(insert)?;
            OrderId(id)
        }
    }
}

/// Extract the typed read program; its moved identity is consumed during execution.
fn load_order_program(
    id: OrderId,
    statements: Arc<PreparedSqlStatementPool>,
) -> Effect<Option<CoffeeOrder>, SqlError, SqlClient> {
    effect! {
        let statements = statements.clone();
        let sql = service! { env: |client: &SqlClient| client.clone(), }?;
        let statement = sql! {
            prepared_statement; pool: statements,
            text("SELECT id,drink,price_cents,ready FROM ");
            identifier("coffee_orders");
            text(" WHERE id=");
            integer(id.0);
        };
        sql.first(statement, order_row())?
    }
}

/// Project an extracted Effect into CafeEnv and map SQL failures, preserving true absence.
fn load_order(
    id: OrderId,
    statements: Arc<PreparedSqlStatementPool>,
) -> Effect<Option<CoffeeOrder>, CafeError, CafeEnv> {
    sql_with! {
        project: |env: &CafeEnv| Arc::clone(&env.sql),
        on_error: CafeError::Sql,
        effect: load_order_program(id, statements),
    }
}

/// Extract a client callback, selected by D06's `run: order_count` form in the workflow.
#[expect(clippy::result_large_err, reason = "retains the complete SQL failure without boxing")]
fn order_count(sql: SqlClient) -> Effect<i64, SqlError, SqlClient> {
    sql.scalar(|| sql! { text("SELECT COUNT(*) FROM coffee_orders"); })
}

/// Read the saved drink into prepared buffers using both atomic SQL forms.
#[expect(clippy::result_large_err, reason = "retains the complete SQL failure without boxing")]
fn verify_drink(sql: SqlClient, storage: Arc<DrinkProjection>) -> Effect<(), SqlError, SqlClient> {
    effect! {
        let sql = sql.clone();
        let storage = storage.clone();
        let text = sql! {
            atomic_query;
            client: sql,
            statement: || sql! { text("SELECT drink FROM coffee_orders"); },
            column: ColumnKey::Ordinal(0),
            kind: PreparedSqlBytesKind::Text,
            rows: storage.text_rows.clone(),
            buffers: storage.buffers.clone(),
        }?;
        let text_matches = matches!(text.as_slice(), [drink] if drink.as_slice() == b"flat white");
        let () = drop(text);
        let batch_storage = storage.clone();
        let batch = effect_try_sync! {
            once;
            body {
                sql! {
                    prepared_batch; pool: batch_storage.batches,
                    statement(sql! {
                        prepared_statement; pool: batch_storage.statements,
                        text("SELECT CAST(drink AS BLOB) FROM coffee_orders");
                    }?);
                }.map_err(SqlError::PreparedStatement)
            },
        }?;
        let blob = sql! {
            atomic_batch;
            client: sql,
            batch: batch,
            column: ColumnKey::Ordinal(0),
            kind: PreparedSqlBytesKind::Blob,
            rows: storage.blob_items.clone(),
            buffers: storage.buffers.clone(),
        }?;
        let blob_matches = blob.as_slice().iter().any(|item| {
            item.value.as_ref().right().is_some_and(|drink| drink.as_slice() == b"flat white")
        });
        effect_try_sync! {
            once;
            body {
                if text_matches && blob_matches { Ok(()) }
                else { Err(SqlError::Configuration("saved drink projection does not match".into())) }
            },
        }?
    }
}

/// Run the finite café story on the caller's Tokio reactor and retain complete exits.
#[tokio::main]
#[expect(clippy::result_large_err, reason = "retains the complete application failure without boxing")]
async fn main() -> Result<(), CafeError> {
    let map_memory = |error| CafeError::Sql(SqlError::Memory(error));
    let plan = memory! {
        total: 32 KiB, mode: prepared, metadata: metadata,
        partition metadata: 16 KiB, partition sql: 16 KiB,
    }.map_err(map_memory)?;
    let memory = plan.prepare().map_err(map_memory)?;
    let metadata = RegionId::named("metadata");
    let sql_region = RegionId::named("sql");
    let pending_checkouts = Arc::new(Admission::prepare(&memory, metadata, [
        ResourceCapacity { unit: CapacityUnit::Slots, work: 2, control: 0 },
    ]).map_err(map_memory)?);
    let materialization = Arc::new(Admission::prepare(&memory, metadata, [
        ResourceCapacity { unit: CapacityUnit::Bytes, work: 512, control: 0 },
    ]).map_err(map_memory)?);
    let statements = Arc::new(sql! {
        prepared_statement_pool; memory: memory, region: sql_region,
        capacity: PreparedSqlStatementCapacity {
            statements: 1, sql_bytes: 128, bindings: 1, binding_bytes: 0,
        },
        materialization_admission: materialization,
    }.map_err(map_memory)?);
    let drink_projection = Arc::new(DrinkProjection {
        statements: statements.clone(),
        batches: sql! {
            prepared_batch_pool; memory: memory, region: sql_region,
            capacity: PreparedSqlBatchCapacity { batches: 1, statements_per_batch: 1 },
        }.map_err(map_memory)?,
        text_rows: Arc::new(sql! {
            prepared_rows; memory: memory, region: sql_region,
            capacity: PreparedSqlRowsCapacity { outputs: 1, rows_per_output: 1 },
        }.map_err(map_memory)?),
        blob_items: Arc::new(sql! {
            prepared_rows; memory: memory, region: sql_region,
            capacity: PreparedSqlRowsCapacity { outputs: 1, rows_per_output: 2 },
        }.map_err(map_memory)?),
        buffers: Arc::new(memory.buffers(sql_region, 2, Bytes::new(32)).map_err(map_memory)?),
    });
    let row_lanes = Arc::new(sql! {
        prepared_native_rows; memory: memory, region: sql_region, lanes: 2,
        capacity: PreparedTursoRowsCapacity { rows: 2, max_columns: 1, bytes_per_row: 32 },
    }.map_err(|error| CafeError::Sql(match error {
        PreparedNativeRowsError::Region(error) => SqlError::Memory(error),
        PreparedNativeRowsError::Rows(error) => SqlError::PreparedRow(error),
    }))?);
    let _memory = memory.seal();
    let database = cafe_database::CafeDatabase::acquire(None)
        .await
        .map_err(CafeError::SqlStartup)?;
    let client = sql! {
        client; pool: database.pool.clone(), pending_checkout_admission: pending_checkouts,
        row_lanes: row_lanes,
    }.map_err(map_memory)?;
    let env = Arc::new(CafeEnv {
        sql: Arc::new(client),
    });
    let selected_sql = env.sql.as_ref().clone();
    let workflow = effect! {
        let drink_projection = drink_projection.clone();
        let statements = statements.clone();
        let selected_sql = selected_sql.clone();
        let id = save_order(NewOrder { id: OrderId(1), customer: CustomerId(7), drink: String::from("flat white"), price_cents: 400 })?;
        effect_map_error! {
            program: sql_with! {
            handle: selected_sql,
            run |sql| { sql.transaction_once(move |tx| correct_price(tx, 1))? },
        }, map: CafeError::Sql, }?;
        let order = load_order(id, statements.clone())?;
        let order = effect_from_option! {
            once; value: order, missing { CafeError::MissingOrder },
        }?;
        let count = sql_with! { select: |env: &CafeEnv| env.sql.clone(), on_error: CafeError::Sql, run: order_count }?;
        let () = sql_with! {
            select: |env: &CafeEnv| env.sql.clone(),
            on_error: CafeError::Sql,
            run: move |sql| verify_drink(sql, drink_projection.clone()),
        }?;
        use std::io::Write;
        let (order, written) = {
            let written = writeln!(std::io::stdout().lock(), "Order {}: {}, {} cents, ready={}; {count} saved order.", order.id.0, order.drink, order.price_cents, order.ready);
            (order, written)
        };
        match written { Ok(()) => (), Err(error) => effect_die! { defect: effect::Defect::from_io(error), }?, };
        effect_try_sync! { once; body: { let __dsl_result = if order.id.0 == 1 && !order.ready && order.drink == "flat white" && order.price_cents == 425 && count == 1 { Ok(()) } else { Err(CafeError::InvalidOrder) }; move || __dsl_result }, on_error: ::core::convert::identity, }?
    };
    let outcome = workflow.run(env.clone()).await;
    let close = database.close().await;
    match outcome {
        Exit::Success(()) => {}
        Exit::Fail(error) => return Err(error),
        failure @ (Exit::Die(_) | Exit::Interrupted | Exit::FinalizationFailed(_)) => {
            return Err(CafeError::Execution(Box::new(failure)));
        }
    }
    close.map_err(CafeError::Sql)
}
