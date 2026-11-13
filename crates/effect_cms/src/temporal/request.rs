//! Byte-preserving duration request readers over the native merged form cache.
//! URL-encoded body values precede query values; multipart values follow query
//! values. Parsing diagnostics are ignored while successfully decoded pairs remain.
//! All transport limits are supplied by the consumer.

use super::duration::normalize_duration_form_values;
use crate::commands::InputBytes;
pub use crate::form_media::MergedFormPolicy as DurationFormPolicy;
use crate::validation::trim_unicode_whitespace;
use effect::{Effect, effect, service};
use effect_server::{BytePairs, HasRequest, ServerError};
use std::sync::Arc;

pub const DURATION_FIELD: &str = "duration";
pub const DURATION_TEXT_FIELD: &str = "duration_text";
pub const DURATION_WEEKS_FIELD: &str = "duration_weeks";
pub const DURATION_DAYS_FIELD: &str = "duration_days";
pub const DURATION_HOURS_FIELD: &str = "duration_hours";
pub const DURATION_MINUTES_FIELD: &str = "duration_minutes";

/// Borrowed decoded values; the first body entry wins even when empty.
#[derive(Clone, Copy, Debug)]
pub struct DurationFormValues<'a> {
    body: Option<&'a BytePairs>,
    query: Option<&'a BytePairs>,
}
impl<'a> DurationFormValues<'a> {
    #[must_use]
    pub fn new(body: Option<&'a BytePairs>, query: Option<&'a BytePairs>) -> Self {
        Self { body, query }
    }
    #[must_use]
    pub fn first(&self, name: &str) -> Option<&'a [u8]> {
        self.body
            .and_then(|v| v.get(name.as_bytes()))
            .and_then(|v| v.first())
            .or_else(|| {
                self.query
                    .and_then(|v| v.get(name.as_bytes()))
                    .and_then(|v| v.first())
            })
            .map(Vec::as_slice)
    }
}
fn value<'a>(values: Option<&DurationFormValues<'a>>, name: &str) -> &'a [u8] {
    trim_unicode_whitespace(values.and_then(|v| v.first(name)).unwrap_or_default())
}
/// Normalize selected fields with text, structured-parts, canonical precedence.
#[must_use]
pub fn normalize_duration_form_request_from_decoded(
    values: Option<&DurationFormValues<'_>>,
) -> (String, bool) {
    let canonical = value(values, DURATION_FIELD);
    let text = value(values, DURATION_TEXT_FIELD);
    let parts = [
        value(values, DURATION_WEEKS_FIELD),
        value(values, DURATION_DAYS_FIELD),
        value(values, DURATION_HOURS_FIELD),
        value(values, DURATION_MINUTES_FIELD),
    ];
    // Malformed bytes cannot be duration syntax. Only the branch selected by the
    // source precedence is decoded; invalid lower-priority fields do not reject it.
    if !text.is_empty() {
        return match std::str::from_utf8(text) {
            Ok(text) => normalize_duration_form_values("", text, "", "", "", ""),
            Err(_) => (String::new(), false),
        };
    }
    if parts.iter().any(|v| !v.is_empty()) {
        let decoded = parts.map(std::str::from_utf8);
        return match decoded {
            [Ok(w), Ok(d), Ok(h), Ok(m)] => normalize_duration_form_values("", "", w, d, h, m),
            _ => (String::new(), false),
        };
    }
    match std::str::from_utf8(canonical) {
        Ok(raw) => normalize_duration_form_values(raw, "", "", "", "", ""),
        Err(_) => (String::new(), false),
    }
}
/// Return canonical minutes, or the original trimmed hidden field on failure.
#[must_use]
pub fn requested_duration_form_value_from_decoded(
    values: Option<&DurationFormValues<'_>>,
) -> InputBytes {
    let (canonical, valid) = normalize_duration_form_request_from_decoded(values);
    if valid {
        canonical.into()
    } else {
        value(values, DURATION_FIELD).into()
    }
}
/// Retain the exact invalid typed text, including malformed UTF-8 bytes.
#[must_use]
pub fn invalid_duration_text_from_decoded(values: Option<&DurationFormValues<'_>>) -> InputBytes {
    let raw = value(values, DURATION_TEXT_FIELD);
    if raw.is_empty()
        || std::str::from_utf8(raw)
            .ok()
            .and_then(super::duration::normalize_duration_value)
            .is_some()
    {
        InputBytes::default()
    } else {
        raw.into()
    }
}
pub fn normalize_duration_form_request<E, R>(
    policy: DurationFormPolicy,
    on_error: impl Fn(ServerError) -> E + Send + Sync + 'static,
) -> Effect<(String, bool), E, R>
where
    E: Send + 'static,
    R: HasRequest + Send + Sync + 'static,
{
    duration_read_effect(
        policy,
        on_error,
        normalize_duration_form_request_from_decoded,
    )
}
pub fn requested_duration_form_value<E, R>(
    policy: DurationFormPolicy,
    on_error: impl Fn(ServerError) -> E + Send + Sync + 'static,
) -> Effect<InputBytes, E, R>
where
    E: Send + 'static,
    R: HasRequest + Send + Sync + 'static,
{
    duration_read_effect(policy, on_error, requested_duration_form_value_from_decoded)
}
pub fn invalid_duration_text_from_request<E, R>(
    policy: DurationFormPolicy,
    on_error: impl Fn(ServerError) -> E + Send + Sync + 'static,
) -> Effect<InputBytes, E, R>
where
    E: Send + 'static,
    R: HasRequest + Send + Sync + 'static,
{
    duration_read_effect(policy, on_error, invalid_duration_text_from_decoded)
}
fn duration_read_effect<E, R, A>(
    policy: DurationFormPolicy,
    on_error: impl Fn(ServerError) -> E + Send + Sync + 'static,
    project: impl Fn(Option<&DurationFormValues<'_>>) -> A + Send + Sync + 'static,
) -> Effect<A, E, R>
where
    E: Send + 'static,
    R: HasRequest + Send + Sync + 'static,
    A: Send + 'static,
{
    let on_error = Arc::new(on_error);
    let project = Arc::new(project);
    effect! {
        let on_error = on_error.clone();
        let project = project.clone();
        let request = service! { env: |env: &R| env.request().clone(), }?;
        let (form, _diagnostic) = crate::form_media::initialize_merged_form(&request, policy, crate::form_media::InitializeForm::FormValue, move |error| on_error(error))?;
        project(Some(&DurationFormValues::new(Some(&form.values), None)))
    }
}
