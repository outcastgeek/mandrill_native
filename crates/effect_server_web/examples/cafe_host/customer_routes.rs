//! Native Identity entry points and local fixture acquisition for the pickup story.

use super::*;
use effect::{effect_die, effect_map_error, effect_sync, effect_try_sync};
use effect_server::reply;
use effect_server_identity::IdentityError;
use effect_sql::sql;

/// Render the native login result without exposing credentials or internal outcomes.
pub(super) fn login_reply(
    cookie: Option<http::HeaderValue>,
) -> Effect<Reply<CafeError, Environment>, CafeError, Environment> {
    let cookie = effect_sync! { once; body { cookie }, };
    effect! {
        let cookie = cookie.clone();
        let cookie = cookie?;
        if let Some(cookie) = cookie {
            let response = reply! {
                header(http::header::CACHE_CONTROL, http::HeaderValue::from_static("no-store"));
                header(http::header::SET_COOKIE, cookie);
                text(String::from("Signed in. Your pickup order is ready to view."));
            }; response
        } else {
            let status = effect_try_sync! { once; body { effect_server::BodyStatus::new(http::StatusCode::UNAUTHORIZED) }, on_error |error| { cafe_error(CafeProblem::Http(error)) }, }?;
            let response = reply! { status(status); header(http::header::CACHE_CONTROL, http::HeaderValue::from_static("no-store")); text(String::from("Username or password incorrect. Check both fields and try again.")); }; response
        }
    }
}
/// Seed real native accounts and owner-bound orders once; restarts retain collection.
pub(super) fn seed(
    account: impl Fn(
        &'static str,
    ) -> Effect<effect_server_identity::native::identity::models::UserId, CafeError>
    + Send
    + Sync
    + 'static,
    sql: SqlClient,
) -> Effect<(), CafeError> {
    let account = Arc::new(account);
    effect! {
        let account = account.clone(); let sql = sql.clone();
        effect_map_error! { program: sql.execute(sql! { text("CREATE TABLE IF NOT EXISTS cafe_pickups(ticket INTEGER PRIMARY KEY, customer_id TEXT NOT NULL UNIQUE, customer TEXT NOT NULL, drink TEXT NOT NULL, cups INTEGER NOT NULL CHECK(cups>0), collected INTEGER NOT NULL DEFAULT 0 CHECK(collected IN(0,1)), collections INTEGER NOT NULL DEFAULT 0 CHECK(collections IN(0,1)))"); }), map |error| { cafe_error(CafeProblem::Sql(error)) }, }?;
        for (name, customer, ticket, drink, cups) in [("maya", "Maya", 104i64, "Oat latte", 2i64), ("lee", "Lee", 105i64, "Espresso", 1i64)] {
            let owner = account(name)?;
            effect_map_error! {
                program: sql.execute(sql! {
                text("INSERT OR IGNORE INTO cafe_pickups(ticket,customer_id,customer,drink,cups) VALUES(");
                param(ticket); text(","); param(owner.to_string()); text(","); param(customer); text(","); param(drink); text(","); param(cups); text(")");
            }), map |error| { cafe_error(CafeProblem::Sql(error)) }, }?;
        }
    }
}
/// Map actual typed request failures to useful status codes without exposing diagnostics.
pub(super) fn error_reply(
    error: CafeError,
) -> Effect<Reply<CafeError, Environment>, CafeError, Environment> {
    effect! {
        let error = error.clone();
        let (status, message) = match error.with(|error| {
            if let DependencyCacheError::Load(CafeProblem::Identity(error)) = error {
                if matches!(error.as_ref(), IdentityError::Unauthorized | IdentityError::StaleGrant | IdentityError::Policy(http::StatusCode::UNAUTHORIZED)) {
                    return (http::StatusCode::UNAUTHORIZED, "Sign in to view your own pickup order.");
                }
                if let IdentityError::Policy(status) = error.as_ref() {
                    return (*status, "The secure request was rejected. Reload the page and try again.");
                }
            }
            if let DependencyCacheError::Load(CafeProblem::Web(WebError::Rejected("authentication"))) = error {
                return (http::StatusCode::UNAUTHORIZED, "Sign in to view your own pickup order.");
            }
            if let DependencyCacheError::Load(CafeProblem::Web(WebError::Rejected("antiforgery"))) = error {
                return (http::StatusCode::FORBIDDEN, "The secure request was rejected. Reload the page and try again.");
            }
            if let DependencyCacheError::Load(CafeProblem::Http(ServerError::Invalid(_))) = error {
                return (http::StatusCode::BAD_REQUEST, "Invalid login fields. Enter a username and password.");
            }
            if let DependencyCacheError::Load(CafeProblem::Http(ServerError::Limit(_))) = error {
                return (http::StatusCode::PAYLOAD_TOO_LARGE, "The login request is too large. Shorten the fields and try again.");
            }
            if let DependencyCacheError::Load(CafeProblem::MissingOrder) = error {
                return (http::StatusCode::NOT_FOUND, "No pickup order belongs to this account.");
            }
            (http::StatusCode::INTERNAL_SERVER_ERROR, "The café could not complete this request. Try again.")
        }) {
            Ok(reply) => reply,
            Err(defect) => effect_die! { defect: defect, }?,
        };
        let status = effect_try_sync! { once; body { effect_server::BodyStatus::new(status) }, on_error |error| { cafe_error(CafeProblem::Http(error)) }, }?;
        let response = reply! { status(status); header(http::header::CACHE_CONTROL, http::HeaderValue::from_static("no-store")); html(effect_server::HtmlContent::trusted(pickup::error_page(message))); };
        response
    }
}
