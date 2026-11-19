//! Native Identity entry points and local fixture acquisition for the pickup story.

use super::*;
use effect::{effect_die, effect_sync, effect_try_sync};
use effect_server::reply;
use effect_server_identity::IdentityError;
use http::StatusCode;

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

fn pickup_failure(
    error: &DependencyCacheError<CafeProblem>,
) -> (StatusCode, Option<pickup::ErrorPage>) {
    let unknown = (
        StatusCode::INTERNAL_SERVER_ERROR,
        Some(pickup::ErrorPage::UNKNOWN),
    );
    let DependencyCacheError::Load(problem) = error else {
        return unknown;
    };
    let sign_in = (StatusCode::UNAUTHORIZED, Some(pickup::ErrorPage::SIGN_IN));
    let unavailable = (StatusCode::SERVICE_UNAVAILABLE, None);
    match problem {
        CafeProblem::Identity(error) => match error.as_ref() {
            IdentityError::Unauthorized
            | IdentityError::StaleGrant
            | IdentityError::Policy(StatusCode::UNAUTHORIZED) => sign_in,
            IdentityError::Admission(_) => unavailable,
            IdentityError::Sql(error) if sql_unavailable(error) => unavailable,
            IdentityError::Policy(status) => (*status, Some(pickup::ErrorPage::SECURITY)),
            IdentityError::Configuration(_)
            | IdentityError::MissingCapability(_)
            | IdentityError::MissingPolicy(_)
            | IdentityError::WrongConfiguration
            | IdentityError::Native(_)
            | IdentityError::Store(_)
            | IdentityError::Sql(_)
            | IdentityError::Record(_)
            | IdentityError::Input(_)
            | IdentityError::Bearer(_)
            | IdentityError::Ticket(_)
            | IdentityError::Token(_) => unknown,
        },
        CafeProblem::Web(WebError::Rejected("authentication")) => sign_in,
        CafeProblem::Web(WebError::Rejected("antiforgery")) => {
            (StatusCode::FORBIDDEN, Some(pickup::ErrorPage::SECURITY))
        }
        CafeProblem::Http(ServerError::Invalid(_)) => (
            StatusCode::BAD_REQUEST,
            Some(pickup::ErrorPage::LOGIN_FIELDS),
        ),
        CafeProblem::Http(ServerError::Limit(_)) => (
            StatusCode::PAYLOAD_TOO_LARGE,
            Some(pickup::ErrorPage::LOGIN_SIZE),
        ),
        CafeProblem::Sql(error) if sql_unavailable(error) => unavailable,
        CafeProblem::Busy(effect::AdmissionError::Full | effect::AdmissionError::Closed)
        | CafeProblem::Web(WebError::Memory(
            MemoryError::NoSlot | MemoryError::Busy | MemoryError::CapacityExceeded { .. },
        )) => unavailable,
        CafeProblem::MissingOrder => (StatusCode::NOT_FOUND, Some(pickup::ErrorPage::NO_ORDER)),
        CafeProblem::Web(_)
        | CafeProblem::Language(_)
        | CafeProblem::Http(_)
        | CafeProblem::Availability(_)
        | CafeProblem::Busy(_)
        | CafeProblem::MenuOrigin(_)
        | CafeProblem::MenuValue(_)
        | CafeProblem::Sql(_)
        | CafeProblem::Otel(_)
        | CafeProblem::Lifecycle(_) => unknown,
    }
}

/// Map actual typed request failures to useful status codes without exposing diagnostics.
pub(super) fn error_reply(
    error: CafeError,
) -> Effect<Reply<CafeError, Environment>, CafeError, Environment> {
    effect! {
        let error = error.clone();
        let (status, error_page) = match error.with(pickup_failure) {
            Ok(reply) => reply,
            Err(defect) => effect_die! { defect: defect, }?,
        };
        let status = effect_try_sync! {
            once; body { effect_server::BodyStatus::new(status) },
            on_error |error| { cafe_error(CafeProblem::Http(error)) },
        }?;
        if let Some(error_page) = error_page {
            let request = effect_server::server! { request; }?;
            let page = effect_try_sync! { once; body: move || pickup::error_page(&request, error_page), }?;
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
