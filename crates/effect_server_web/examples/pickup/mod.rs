//! Customer-owned pickup data and its finite collection workflow.

mod pages;

use super::{CafeError, CafeProblem, cafe_error, pickup_announcement};
use effect::{
    Effect, effect, effect_from_option, effect_map_error, effect_try_future, effect_try_sync,
    service,
};
use effect_server::{BodyLimit, HasRequest, HtmlContent, Reply, reply};
use effect_server_identity::authority::SelfGrant;
use effect_sql::{CellCodec, RowDecoder, SqlClient, row, sql};
use std::num::NonZeroUsize;

/// Local demonstration credentials; native Identity still verifies the password.
pub const DEMO_PASSWORD: &str = "Cafe-Demo-Pickup-42!";
/// One customer's persisted order, selected by verified owner rather than request ID.
struct PickupOrder {
    /// Café ticket printed on the customer's receipt.
    ticket: i64,
    /// Display name stored with the order.
    customer: String,
    /// Ordered drink name.
    drink: String,
    /// Number of cups to collect.
    cups: i64,
    /// Acknowledged collection state; reloading never resets it.
    collected: bool,
}
/// A bounded, strict login payload sent by the visible HTML form.
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Credentials {
    /// Native account lookup name, not a claimed order owner.
    pub username: String,
    /// Password moved directly into the selected native login workflow.
    pub password: String,
}
/// Decode one finite browser form submission under the existing request/body owner.
pub fn credentials<R: HasRequest + Send + Sync + 'static>(
    bytes: NonZeroUsize,
) -> Effect<Credentials, CafeError, R> {
    effect! {
        let request = service! { env |env: &R| { env.request().clone() }, }?;
        if request.headers().get(http::header::CONTENT_TYPE).is_none_or(|value| value != "application/json") {
            effect_try_sync! { body { Err::<(), _>(cafe_error(CafeProblem::Http(effect_server::ServerError::Invalid("login content type".into())))) }, }?;
        }
        let text = effect_try_future! { once; body |_cancel| { async move { request.read_text(BodyLimit::new(bytes)).await } }, on_error |error| { cafe_error(CafeProblem::Http(error)) }, }?;
        effect_try_sync! { once; body { serde_json::from_str(&text) }, on_error |_error| { cafe_error(CafeProblem::Http(effect_server::ServerError::Invalid("login fields".into()))) }, }?
    }
}
/// The entry page explains the local task and provides actual browser controls.
pub fn welcome<R: HasRequest + Send + Sync + 'static>() -> Effect<Reply<CafeError, R>, CafeError, R>
{
    effect! {
        let nonce = response_nonce::<R>()?;
        let page = pages::welcome(&nonce);
        let response = reply! { header(http::header::CACHE_CONTROL, http::HeaderValue::from_static("no-store")); html(HtmlContent::trusted(page)); };
        response
    }
}
/// Read only the order belonging to the native cookie's privately verified actor.
pub fn show<R: HasRequest + Send + Sync + 'static>(
    sql: SqlClient,
) -> Effect<Reply<CafeError, R>, CafeError, R> {
    effect! {
        let sql = sql.clone();
        let actor = actor::<R>()?;
        let order = load::<R>(sql, actor)?;
        let nonce = response_nonce::<R>()?;
        let announcement = if order.collected {
            String::from("Your collection has been recorded. Thank you!")
        } else {
            pickup_announcement::<R>(order.customer.clone())?
        };
        let page = pages::order(order.ticket, &order.drink, order.cups, order.collected, &announcement, &nonce);
        let response = reply! { header(http::header::CACHE_CONTROL, http::HeaderValue::from_static("no-store")); html(HtmlContent::trusted(page)); };
        response
    }
}
/// Atomically move ready to collected once; retries read the already collected state.
/// Web's selected native antiforgery stage validates the actor/method/URI before entry.
pub fn collect<R: HasRequest + Send + Sync + 'static>(
    sql: SqlClient,
) -> Effect<Reply<CafeError, R>, CafeError, R> {
    effect! {
        let sql = sql.clone();
        let actor = actor::<R>()?;
        let owner = actor.user_id().to_string();
        let changed = effect_map_error! {
            program: sql.execute(sql! {
            text("UPDATE cafe_pickups SET collected=1, collections=collections+1 WHERE customer_id="); param(owner);
            text(" AND collected=0");
        }), map |error| { cafe_error(CafeProblem::Sql(error)) }, }?;
        let _order = load::<R>(sql, actor)?;
        let message = if changed == 1 { "Collection recorded. Enjoy your drinks!" } else { "Already collected; no second collection was recorded." };
        let response = reply! { header(http::header::CACHE_CONTROL, http::HeaderValue::from_static("no-store")); text(message.to_owned()); };
        response
    }
}
/// Pull native proof from request metadata; clients cannot construct this grant.
fn actor<R: HasRequest + Send + Sync + 'static>() -> Effect<SelfGrant, CafeError, R> {
    effect! {
        let actor = service! { env |env: &R| { env.request().with_extensions(|extensions| extensions.get::<SelfGrant>().cloned()) }, }?;
        effect_from_option! { once; value: actor, missing { cafe_error(CafeProblem::Identity(Box::new(effect_server_identity::IdentityError::Unauthorized))) }, }?
    }
}
/// Bind response markup to the same per-response nonce used by the native CSP header.
fn response_nonce<R: HasRequest + Send + Sync + 'static>() -> Effect<String, CafeError, R> {
    effect! {
        let nonce = service! { env |env: &R| { env.request().with_extensions(|extensions| extensions.get::<effect_server_web::csp::ResponseNonce>().map(|nonce| nonce.as_str().to_owned())) }, }?;
        effect_from_option! { once; value: nonce, missing { cafe_error(CafeProblem::Web(effect_server_web::WebError::Metadata("response nonce".into()))) }, }?
    }
}
/// Typed row decoding and owner-bound SQL share the host's selected database service.
fn load<R: Send + Sync + 'static>(
    sql: SqlClient,
    actor: SelfGrant,
) -> Effect<PickupOrder, CafeError, R> {
    let decoder: RowDecoder<PickupOrder, R> = row! {
        let ticket = native_column::<i64, R>("ticket")?;
        let customer = native_column::<String, R>("customer")?;
        let drink = native_column::<String, R>("drink")?;
        let cups = native_column::<i64, R>("cups")?;
        let collected = column("collected", CellCodec::<bool, i64, R>::boolean())?;
        PickupOrder { ticket, customer, drink, cups, collected }
    };
    effect! {
        let sql = sql.clone(); let decoder = decoder.clone();
        let owner = actor.user_id().to_string();
        let order = effect_map_error! { program: sql.first(sql! { text("SELECT ticket,customer,drink,cups,collected FROM cafe_pickups WHERE customer_id="); param(owner); }, decoder), map |error| { cafe_error(CafeProblem::Sql(error)) }, }?;
        effect_from_option! { once; value: order, missing { cafe_error(CafeProblem::MissingOrder) }, }?
    }
}

/// Safe error markup gives anonymous or expired browser sessions a sign-in destination.
#[must_use]
pub fn error_page(message: &str) -> String {
    pages::failure(message)
}
