//! Authoring signal patches and their Datastar SSE framing.
//!
//! The patch builders preserve the source map assignments in order: unavailable,
//! status and failure patches, the ready patch with its submit-error clearing and
//! per-field values, and the JSON fallback used when a payload cannot be encoded.
//!
//! The response is authored through the native reply contract: SSE framing is the
//! Datastar event text (`event: datastar-patch-signals`, one `data: signals` line
//! per normalized payload line, a terminating blank line), and the source's SSE
//! headers are applied as the final response fields. The source wrote directly to
//! `net/http`'s writer; native transport delivery and its failures belong to the
//! reply/host boundary, so these builders return a completed reply rather than
//! exposing a writer callback.

use std::collections::BTreeMap;

use effect_server::{Reply, reply};
use http::{HeaderName, HeaderValue, header};
use serde::Serialize;

use super::contracts::{AuthoringTranslationPatchOptions, AuthoringTranslationSignals};
use crate::forms::LocalizedAuthoringSignals;

/// One signal-patch value.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(untagged)]
pub enum TranslationSignalValue {
    /// A boolean signal.
    Bool(bool),
    /// A text signal.
    Text(String),
}

/// One signal patch keyed by the consumer's signal names.
pub type AuthoringTranslationPatch = BTreeMap<String, TranslationSignalValue>;

/// The source's SSE media type.
const SSE_CONTENT_TYPE: &str = "text/event-stream";

/// The unavailable patch.
#[must_use]
pub fn authoring_translation_unavailable_patch(
    signals: &AuthoringTranslationSignals,
    status_text: &str,
) -> AuthoringTranslationPatch {
    let mut patch = AuthoringTranslationPatch::new();
    patch.insert(
        signals.translation_available.clone(),
        TranslationSignalValue::Bool(false),
    );
    patch.insert(
        signals.translating.clone(),
        TranslationSignalValue::Bool(false),
    );
    patch.insert(
        signals.status_text.clone(),
        TranslationSignalValue::Text(status_text.to_owned()),
    );
    patch
}

/// The status-only patch.
#[must_use]
pub fn authoring_translation_status_patch(
    signals: &AuthoringTranslationSignals,
    status_text: &str,
) -> AuthoringTranslationPatch {
    let mut patch = AuthoringTranslationPatch::new();
    patch.insert(
        signals.translating.clone(),
        TranslationSignalValue::Bool(false),
    );
    patch.insert(
        signals.status_text.clone(),
        TranslationSignalValue::Text(status_text.to_owned()),
    );
    patch
}

/// The failure patch: the failed status plus the cleared submit-error signals.
#[must_use]
pub fn authoring_translation_failure_patch(
    options: &AuthoringTranslationPatchOptions,
) -> AuthoringTranslationPatch {
    let mut patch = authoring_translation_status_patch(
        &options.signals,
        options.failed_status_text.as_str(),
    );
    if !options.submit_error_open_signal.trim().is_empty() {
        patch.insert(
            options.submit_error_open_signal.clone(),
            TranslationSignalValue::Bool(false),
        );
    }
    if !options.submit_error_text_signal.trim().is_empty() {
        patch.insert(
            options.submit_error_text_signal.clone(),
            TranslationSignalValue::Text(String::new()),
        );
    }
    patch
}

/// The localized-to-authoring signal projection.
#[must_use]
pub fn authoring_translation_signals_from_localized(
    signals: &LocalizedAuthoringSignals,
) -> AuthoringTranslationSignals {
    AuthoringTranslationSignals {
        widget_key: signals.widget_key.clone(),
        translation_available: signals.translation_available.clone(),
        translating: signals.translating.clone(),
        status_text: signals.status_text.clone(),
    }
}

/// Encode one Datastar signal-patch event.
///
/// Line endings are normalized to LF and every payload line receives the
/// `data: signals ` prefix, matching the source event formatter.
fn datastar_signal_patch_event(signals: &str) -> String {
    let normalized = signals.replace("\r\n", "\n").replace('\r', "\n");
    let mut event = String::from("event: datastar-patch-signals\n");
    for line in normalized.split('\n') {
        event.push_str("data: signals ");
        event.push_str(line);
        event.push('\n');
    }
    event.push('\n');
    event
}

/// Encode the payload as the event's signal JSON, using the supplied fallback text
/// when encoding fails.
fn encoded_signals(payload: &AuthoringTranslationPatch, fallback_signals_json: &str) -> String {
    match serde_json::to_string(payload) {
        Ok(raw) => raw,
        Err(_) => fallback_signals_json.to_owned(),
    }
}

/// Write one signal patch as a completed SSE reply.
///
/// The source's marshal-failure fallback keeps the supplied signal JSON; its write
/// failure is retained by native reply delivery rather than reported from here.
#[must_use]
pub fn write_signal_patch<E, R>(
    payload: &AuthoringTranslationPatch,
    fallback_signals_json: &str,
) -> Reply<E, R>
where
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    let event = datastar_signal_patch_event(&encoded_signals(payload, fallback_signals_json));
    reply! {
        header(
            header::CONTENT_TYPE,
            HeaderValue::from_static(SSE_CONTENT_TYPE)
        );
        header(
            header::CACHE_CONTROL,
            HeaderValue::from_static("no-cache")
        );
        header(header::CONNECTION, HeaderValue::from_static("keep-alive"));
        header(
            HeaderName::from_static("x-accel-buffering"),
            HeaderValue::from_static("no")
        );
        text(event);
    }
}

/// The fallback signals JSON for the status patch.
#[must_use]
pub fn authoring_translation_fallback_signals_json(
    signals: &AuthoringTranslationSignals,
    status_text: &str,
) -> String {
    match serde_json::to_string(&authoring_translation_status_patch(signals, status_text)) {
        Ok(raw) => raw,
        Err(_) => "{}".to_owned(),
    }
}

/// Write one authoring translation signal patch as a completed SSE reply.
#[must_use]
pub fn write_authoring_translation_signal_patch<E, R>(
    payload: &AuthoringTranslationPatch,
    signals: &AuthoringTranslationSignals,
    fallback_status_text: &str,
) -> Reply<E, R>
where
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    write_signal_patch(
        payload,
        authoring_translation_fallback_signals_json(signals, fallback_status_text).as_str(),
    )
}
