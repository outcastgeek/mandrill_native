//! Customer-owned pickup data and its finite collection workflow.

mod pages;
pub(crate) use pages::ErrorPage;

use super::{CafeError, CafeProblem, cafe_error, pickup_message};
use effect::{
    Effect, effect, effect_from_option, effect_map_error, effect_try_future, effect_try_sync,
    service,
};
use effect_server::{BodyLimit, HasRequest, HtmlContent, Reply, Request, ServerError, reply, server};
use effect_server_identity::authority::SelfGrant;
use effect_server_web::{
    csp::ResponseNonce,
    rendering::{prepared_html, prepared_html_into},
};
use effect_sql::{RowDecoder, RowError, SqlClient, native_column_ref, row, sql};
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
        let request = server! { request; }?;
        if request.headers().get(http::header::CONTENT_TYPE).is_none_or(|value| value != "application/json") {
            effect_try_sync! { body { Err::<(), _>(cafe_error(CafeProblem::Http(effect_server::ServerError::Invalid("login content type".into())))) }, }?;
        }
        let body = effect_try_future! { once; body |_cancel| { async move { request.read_bytes(BodyLimit::new(bytes)).await } }, on_error |error| { cafe_error(CafeProblem::Http(error)) }, }?;
        effect_try_sync! {
            once;
            body: move || {
                let text = std::str::from_utf8(&body).map_err(|_| effect_server::ServerError::Invalid("UTF-8 body".into()))?;
                serde_json::from_str(text).map_err(|_| effect_server::ServerError::Invalid("login fields".into()))
            },
            on_error |error| { cafe_error(CafeProblem::Http(error)) },
        }?
    }
}
/// The entry page explains the local task and provides actual browser controls.
pub fn welcome<R: HasRequest + Send + Sync + 'static>() -> Effect<Reply<CafeError, R>, CafeError, R>
{
    effect! {
        let request = server! { request; }?;
        let nonce = response_nonce::<R>()?;
        let page = effect_try_sync! {
            once;
            body: move || prepared_html::<_, CafeError, R>(&request, nonce, |nonce, out| {
                pages::welcome(out, nonce.as_str()).map_err(pickup_http_error)
            }, pickup_http_error),
        }?;
        reply! { header(http::header::CACHE_CONTROL, http::HeaderValue::from_static("no-store")); source(page); }
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
        let request = server! { request; }?;
        let announcement = if order.collected {
            None
        } else {
            Some(pickup_message::<R>(order.customer)?)
        };
        let page = effect_try_sync! {
            once;
            body: move || prepared_html::<_, CafeError, R>(&request, announcement, |announcement, out| {
                let facts = (order.ticket, order.drink.as_str(), order.cups, order.collected);
                pages::order(out, facts, nonce.as_str(), |out| {
                    if let Some((catalog, message)) = announcement {
                        out.write_message(&catalog, &message).map_err(PickupRenderError::Language)?;
                    } else {
                        out.write_text("Your collection has been recorded. Thank you!")?;
                    }
                    Ok(())
                }).map_err(pickup_render_error)
            }, pickup_http_error),
        }?;
        reply! { header(http::header::CACHE_CONTROL, http::HeaderValue::from_static("no-store")); source(page); }
    }
}

enum PickupRenderError {
    Http(ServerError),
    Language(effect_server_web::localization::LocalizationError),
}

impl From<ServerError> for PickupRenderError {
    fn from(error: ServerError) -> Self {
        Self::Http(error)
    }
}

fn pickup_render_error(error: PickupRenderError) -> CafeError {
    cafe_error(match error {
        PickupRenderError::Http(error) => CafeProblem::Http(error),
        PickupRenderError::Language(error) => CafeProblem::Language(error),
    })
}

fn pickup_http_error(error: ServerError) -> CafeError {
    cafe_error(CafeProblem::Http(error))
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
            program: sql! {
                execute; client: sql, statement: sql! {
                    text("UPDATE cafe_pickups SET collected=1, collections=collections+1 WHERE customer_id="); param(owner);
                    text(" AND collected=0");
                },
            },
            map |error| { cafe_error(CafeProblem::Sql(error)) },
        }?;
        let _order = load::<R>(sql, actor)?;
        let message = if changed == 1 { "Collection recorded. Enjoy your drinks!" } else { "Already collected; no second collection was recorded." };
        let response = reply! {
            header(http::header::CACHE_CONTROL, http::HeaderValue::from_static("no-store"));
            bytes(bytes::Bytes::from_static(message.as_bytes()), http::HeaderValue::from_static("text/plain; charset=utf-8"));
        };
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
fn response_nonce<R: HasRequest + Send + Sync + 'static>() -> Effect<ResponseNonce, CafeError, R> {
    effect! {
        let nonce = service! { env |env: &R| { env.request().with_extensions(|extensions| extensions.get::<ResponseNonce>().cloned()) }, }?;
        effect_from_option! { once; value: nonce, missing { cafe_error(CafeProblem::Web(effect_server_web::WebError::Metadata("response nonce".into()))) }, }?
    }
}
/// Typed row decoding and owner-bound SQL share the host's selected database service.
#[expect(clippy::result_large_err, reason = "the borrowed pickup projection retains the owned row error")]
fn load<R: Send + Sync + 'static>(
    sql: SqlClient,
    actor: SelfGrant,
) -> Effect<PickupOrder, CafeError, R> {
    let decoder: RowDecoder<PickupOrder, R> = row! {
        project: |row| {
            let ticket = native_column_ref(row, "ticket")?;
            let customer = native_column_ref(row, "customer")?;
            let drink = native_column_ref(row, "drink")?;
            let cups = native_column_ref(row, "cups")?;
            let collected = match native_column_ref::<i64>(row, "collected")? {
                0 => false,
                1 => true,
                _ => return Err(RowError::Schema {
                    column: "collected".into(),
                    issue: effect::SchemaIssue::new(effect::SchemaIssueKind::OutOfRange),
                }),
            };
            Ok(PickupOrder { ticket, customer, drink, cups, collected })
        },
    };
    effect! {
        let sql = sql.clone(); let decoder = decoder.clone();
        let owner = actor.user_id().to_string();
        let order = effect_map_error! {
            program: sql! {
                first_prepared; client: sql,
                statement: sql! {
                    text("SELECT ticket,customer,drink,cups,collected FROM cafe_pickups WHERE customer_id="); param(owner);
                },
                decoder: decoder,
            },
            map |error| { cafe_error(CafeProblem::Sql(error)) },
        }?;
        effect_from_option! { once; value: order, missing { cafe_error(CafeProblem::MissingOrder) }, }?
    }
}

/// Safe error markup gives anonymous or expired browser sessions a sign-in destination.
pub(crate) fn error_page<R: Send + Sync + 'static>(
    request: &Request,
    page: ErrorPage,
) -> Result<Reply<CafeError, R>, CafeError> {
    const MEDIA: &str = "text/html; charset=utf-8";
    match request.take_prepared_output_buffer() {
        Ok(output) if output.capacity().get() >= page.html.len() => prepared_html_into(
            output,
            page.html,
            |markup, out| out.write_trusted(markup).map_err(pickup_http_error),
            pickup_http_error,
        ),
        Ok(_) | Err(ServerError::BodyClaimed) => Ok(reply! {
            bytes(effect_server::native::bytes::Bytes::from_static(page.html.as_bytes()), http::HeaderValue::from_static(MEDIA));
        }),
        Err(ServerError::Memory(effect::memory::MemoryError::UnsupportedRunContract)) => {
            Ok(reply! { html(HtmlContent::trusted(pages::failure(page.message))); })
        }
        Err(error) => Err(pickup_http_error(error)),
    }
}
