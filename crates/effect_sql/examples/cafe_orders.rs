//! Save and load a café order through D02–D06, with one moved application payload.
//!
//! Requires `turso,macros`. The static checked query describes only a literal,
//! so the compiler's `DATABASE_URL=sqlite::memory:` needs no application schema.
//! The support module owns an explicit temporary database/pool policy. The host
//! supplies Tokio once; SQL and Effect create no runtime of their own.

#[path = "support/cafe_database.rs"]
/// Native setup and teardown for the temporary café database.
mod cafe_database;

use effect::{Effect, Exit, SchemaIssue, SchemaIssueKind, effect};
use effect_sql::{
    CellCodec, RowDecoder, SqlClient, SqlError, SqlPredicate, SqlTransaction, row, sql, sql_insert,
    sql_update, sql_with,
};
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

/// Extracted native savepoint body; a scoped handle remains fenced after this returns.
/// Each invocation shares the checked transaction owner through a handle clone.
fn mark_ready(tx: SqlTransaction, id: i64) -> Effect<u64, SqlError, SqlClient> {
    effect! {
        let tx = tx.clone();
        let predicate = Effect::from_result_once(SqlPredicate::equal("id", id))?;
        let statement = sql_update! { table: "coffee_orders", set("ready", true); where(predicate); };
        tx.execute(statement)?
    }
}

/// Move the non-Clone order once; all statement construction stays inside execution.
fn save_order(order: NewOrder) -> Effect<OrderId, CafeError, CafeEnv> {
    sql_with! {
        select: |env: &CafeEnv| Arc::clone(&env.sql),
        on_error: CafeError::Sql,
        run |sql| {
            let order = Effect::from_result_once(if order.price_cents >= 0 && !order.drink.is_empty() { Ok(order) } else { Err(SqlError::Configuration("invalid café order".into())) })?;
            let selected = sql.clone();
            sql.transaction_once(move |tx| sql_with! {
                handle: selected,
                run |_sql| {
                    let id = order.id.0;
                    let insert = sql_insert! {
                        table: "coffee_orders",
                        value("id", id);
                        value("customer_id", order.customer.0);
                        value("drink", order.drink);
                        value("price_cents", order.price_cents);
                        value("ready", false);
                    };
                    tx.execute(insert)?;
                    tx.savepoint(move |nested| mark_ready(nested, id))?;
                    OrderId(id)
                }
            })?
        }
    }
}

/// Extract the typed read program; its moved identity is consumed during execution.
fn load_order_program(id: OrderId) -> Effect<Option<CoffeeOrder>, SqlError, SqlClient> {
    effect! {
        let sql = Effect::service(|client: &SqlClient| client.clone())?;
        let statement = sql! {
            text("SELECT id,drink,price_cents,ready FROM ");
            identifier("coffee_orders");
            text(" WHERE id=");
            param(id.0);
        };
        sql.first(statement, order_row())?
    }
}

/// Project an extracted Effect into CafeEnv and map SQL failures, preserving true absence.
fn load_order(id: OrderId) -> Effect<Option<CoffeeOrder>, CafeError, CafeEnv> {
    sql_with! {
        project: |env: &CafeEnv| Arc::clone(&env.sql),
        on_error: CafeError::Sql,
        effect: load_order_program(id),
    }
}

/// Extract a client callback, selected by D06's `run: ready_count` form in the workflow.
fn ready_count(sql: SqlClient) -> Effect<i64, SqlError, SqlClient> {
    sql.scalar(|| sql! { text("SELECT COUNT(*) FROM coffee_orders WHERE ready=1"); })
}

/// Exercise the local SQLx checked macro directly beside the domain DSL.
async fn checked_native(client: &SqlClient) -> Result<i64, SqlError> {
    let value = effect_sql::sqlx::query_scalar!("SELECT 1 AS \"schema_version!: i64\"")
        .fetch_one(client.kernel().pool())
        .await?;
    Ok(value)
}

/// Run the finite café story on the caller's Tokio reactor and retain complete exits.
#[tokio::main]
async fn main() -> Result<(), CafeError> {
    let database = cafe_database::CafeDatabase::acquire()
        .await
        .map_err(CafeError::Sql)?;
    let env = Arc::new(CafeEnv {
        sql: Arc::new(database.client.clone()),
    });
    let workflow = effect! {
        let id = save_order(NewOrder { id: OrderId(1), customer: CustomerId(7), drink: String::from("flat white"), price_cents: 425 })?;
        let order = load_order(id)?;
        let order = Effect::from_result_once(order.ok_or(CafeError::MissingOrder))?;
        let count = sql_with! { select: |env: &CafeEnv| env.sql.clone(), on_error: CafeError::Sql, run: ready_count }?;
        Effect::from_result_once(if order.id.0 == 1 && order.ready && order.drink == "flat white" && order.price_cents == 425 && count == 1 { Ok(()) } else { Err(CafeError::InvalidOrder) })?
    };
    let outcome = workflow.run(env.clone()).await;
    let native = checked_native(&database.client).await;
    let close = database.close().await;
    match outcome {
        Exit::Success(()) => {}
        Exit::Fail(error) => return Err(error),
        failure @ (Exit::Die(_) | Exit::Interrupted | Exit::FinalizationFailed(_)) => {
            return Err(CafeError::Execution(Box::new(failure)));
        }
    }
    native.map_err(CafeError::Sql)?;
    close.map_err(CafeError::Sql)
}
