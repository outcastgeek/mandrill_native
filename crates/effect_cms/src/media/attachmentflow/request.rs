//! Request form decoding for the attachment workflows.
//!
//! The decoded view is the merged form map: URL-encoded body values first, then
//! URL query values appended after them, with repeated values preserved in that
//! order. The collection rules read the first column entry, so a body value wins
//! over a query value for the same field.
//!
//! The body is read only for `POST`, `PUT` and `PATCH` and only when the request
//! declares `application/x-www-form-urlencoded`; every other request contributes
//! query values alone. The body parse is the native compatible-form read under the
//! caller's explicit policy and rejects an unencoded semicolon. A body parse
//! failure contributes no body values while the query values still apply, which
//! keeps this adapter's error channel empty. The query map uses the shared query
//! adapter, so its repeated-value order and percent decoding are the shared
//! behavior rather than a local parser.
//!
//! Recorded limits: a `multipart/form-data` body is not decoded here, and a body
//! that is only partially valid yields no body values because the native form read
//! is all-or-nothing. Both limits stay visible as transport work, not owner parity.

use std::collections::BTreeMap;
use std::sync::Arc;

use effect::{Effect, effect, effect_try_future};
use effect_server::{FormData, FormPolicy, Request, ServerError};

use crate::queries::request_query_map;

/// Decode one request's merged form values.
///
/// `None` is an absent request and reads nothing. A present request reads its
/// query always and its body only under the method and media-type gates described
/// in the module documentation. The effect is lazy and reusable; no request body
/// is claimed until the effect runs.
pub(super) fn media_attachment_form<E, R>(
    request: Option<Request>,
    policy: FormPolicy,
    on_error: Arc<dyn Fn(ServerError) -> E + Send + Sync>,
) -> Effect<FormData, E, R>
where
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    effect! {
        let request = request.clone();
        let on_error = on_error.clone();
        let body = match request.clone() {
            Some(request) if reads_body_form(request.method().as_str()) && is_urlencoded_form(&request) => {
                let map = on_error.clone();
                let read = effect_try_future! {
                    body: move |_| {
                        let request = request.clone();
                        async move {
                            request.read_form_rejecting_raw_semicolons(policy).await
                        }
                    },
                    on_error: move |error| map(error),
                };
                let read = effect! { either; program: read, }?;
                read.ok()
            }
            _ => None,
        };
        let mut values: BTreeMap<String, Vec<String>> = match body {
            Some(form) => form.values.clone(),
            None => BTreeMap::new(),
        };
        if let Some(request) = request.as_ref() {
            for (key, repeated) in request_query_map(request) {
                values.entry(key).or_default().extend(repeated);
            }
        }
        FormData { values }
    }
}

/// Whether this method reads a URL-encoded body.
fn reads_body_form(method: &str) -> bool {
    matches!(method, "POST" | "PUT" | "PATCH")
}

/// Whether the request selects a URL-encoded body form.
fn is_urlencoded_form(request: &Request) -> bool {
    request
        .headers()
        .get(http::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| {
            value.split(';').next().is_some_and(|base| {
                base.trim()
                    .eq_ignore_ascii_case("application/x-www-form-urlencoded")
            })
        })
}
