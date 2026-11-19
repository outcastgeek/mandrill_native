//! Native Identity entry points and local fixture acquisition for the pickup story.

use super::*;
use effect::{effect_die, effect_map_error, effect_sync, effect_try_sync, service};
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
                bytes(bytes::Bytes::from_static(b"Signed in. Your pickup order is ready to view."), http::HeaderValue::from_static("text/plain; charset=utf-8"));
            }; response
        } else {
            let status = effect_try_sync! { once; body { effect_server::BodyStatus::new(http::StatusCode::UNAUTHORIZED) }, on_error |error| { cafe_error(CafeProblem::Http(error)) }, }?;
            let response = reply! {
                status(status);
                header(http::header::CACHE_CONTROL, http::HeaderValue::from_static("no-store"));
                bytes(bytes::Bytes::from_static(b"Username or password incorrect. Check both fields and try again."), http::HeaderValue::from_static("text/plain; charset=utf-8"));
            }; response
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
fn sql_unavailable(error: &effect_sql::SqlError) -> bool {
    matches!(
        error,
        effect_sql::SqlError::Memory(
            MemoryError::NoSlot
                | MemoryError::Busy
                | MemoryError::Closed
                | MemoryError::CapacityExceeded { .. },
        ) | effect_sql::SqlError::PreparedRow(
            sqlx::PreparedTursoRowMemoryError::NoSlot | sqlx::PreparedTursoRowMemoryError::Capacity,
        )
    )
}

/// Map actual typed request failures to useful status codes without exposing diagnostics.
pub(super) fn error_reply(
    error: CafeError,
) -> Effect<Reply<CafeError, Environment>, CafeError, Environment> {
    effect! {
        let error = error.clone();
        let (status, error_page) = match error.with(|error| {
            if let DependencyCacheError::Load(CafeProblem::Identity(error)) = error {
                if matches!(error.as_ref(), IdentityError::Unauthorized | IdentityError::StaleGrant | IdentityError::Policy(http::StatusCode::UNAUTHORIZED)) {
                    return (http::StatusCode::UNAUTHORIZED, Some(pickup::ErrorPage::SIGN_IN));
                }
                if matches!(error.as_ref(), IdentityError::Admission(_))
                    || matches!(error.as_ref(), IdentityError::Sql(error) if sql_unavailable(error))
                {
                    return (http::StatusCode::SERVICE_UNAVAILABLE, None);
                }
                if let IdentityError::Policy(status) = error.as_ref() {
                    return (*status, Some(pickup::ErrorPage::SECURITY));
                }
            }
            if let DependencyCacheError::Load(CafeProblem::Web(WebError::Rejected("authentication"))) = error {
                return (http::StatusCode::UNAUTHORIZED, Some(pickup::ErrorPage::SIGN_IN));
            }
            if let DependencyCacheError::Load(CafeProblem::Web(WebError::Rejected("antiforgery"))) = error {
                return (http::StatusCode::FORBIDDEN, Some(pickup::ErrorPage::SECURITY));
            }
            if let DependencyCacheError::Load(CafeProblem::Http(ServerError::Invalid(_))) = error {
                return (http::StatusCode::BAD_REQUEST, Some(pickup::ErrorPage::LOGIN_FIELDS));
            }
            if let DependencyCacheError::Load(CafeProblem::Http(ServerError::Limit(_))) = error {
                return (http::StatusCode::PAYLOAD_TOO_LARGE, Some(pickup::ErrorPage::LOGIN_SIZE));
            }
            if matches!(error, DependencyCacheError::Load(CafeProblem::Sql(error)) if sql_unavailable(error))
                || matches!(error,
                DependencyCacheError::Load(CafeProblem::Busy(effect::AdmissionError::Full | effect::AdmissionError::Closed)
                    | CafeProblem::Web(WebError::Memory(effect::memory::MemoryError::NoSlot
                        | effect::memory::MemoryError::Busy | effect::memory::MemoryError::CapacityExceeded { .. })))
            ) {
                return (http::StatusCode::SERVICE_UNAVAILABLE, None);
            }
            if let DependencyCacheError::Load(CafeProblem::MissingOrder) = error {
                return (http::StatusCode::NOT_FOUND, Some(pickup::ErrorPage::NO_ORDER));
            }
            (http::StatusCode::INTERNAL_SERVER_ERROR, Some(pickup::ErrorPage::UNKNOWN))
        }) {
            Ok(reply) => reply,
            Err(defect) => effect_die! { defect: defect, }?,
        };
        let status = effect_try_sync! { once; body { effect_server::BodyStatus::new(status) }, on_error |error| { cafe_error(CafeProblem::Http(error)) }, }?;
        if let Some(error_page) = error_page {
            let request = service! { env |env: &Environment| { env.request().clone() }, }?;
            let page = effect_try_sync! { once; body: move || pickup::error_page(&request, error_page), on_error: std::convert::identity, }?;
            reply! { status(status); header(http::header::CACHE_CONTROL, http::HeaderValue::from_static("no-store")); source(page); }
        } else {
            reply! {
                status(status);
                header(http::header::CACHE_CONTROL, http::HeaderValue::from_static("no-store"));
                bytes(effect_server::native::bytes::Bytes::from_static("The café is busy. Try again shortly.\n".as_bytes()), http::HeaderValue::from_static("text/plain; charset=utf-8"));
            }
        }
    }
}
