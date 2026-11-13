//! Localized authoring state, signal names, form attributes and client actions.
//!
//! The authoring widget is the localized-tabs child owner's authoring contract:
//!
//! - the project-wide authoring availability switch, set once by the application
//!   (the preserved source-level switch, not a dependency locator);
//! - the authoring props built from the caller's translated text keys;
//! - the deterministic signal names for each widget, language and field;
//! - the form root attributes that bind the submit-blocking/translating state;
//! - the localized field/format attributes and their input/change/focus actions;
//! - the submit control attributes and the submit-attempt guard.
//!
//! Every label is resolved through the caller's translation port. Signal names are
//! normalized to lower-case `[a-z0-9_]` parts and scoped by widget key; the source
//! widget key is used when no widget key is supplied. The `data-signals` object
//! keeps every distinct signal once, including the four form-level signals.
//!
//! The authoring availability switch is the source behavior: it is unset initially,
//! which reads as available; once set, its value is used.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};

use effect_server::Request;

use crate::localization::{NormalizerRef, TranslateKeyFunc, normalize_authoring_language};

use super::{LocalizedTab, LocalizedTabsProps};

static LOCALIZED_AUTHORING_AVAILABILITY: AtomicBool = AtomicBool::new(false);
static LOCALIZED_AUTHORING_AVAILABILITY_SET: AtomicBool = AtomicBool::new(false);

const LOCALIZED_SIGNAL_AUTO_TRANSLATE: &str = "locAutoTranslate";
const LOCALIZED_SIGNAL_SUBMIT_BLOCKED: &str = "locSubmitBlocked";
const LOCALIZED_SIGNAL_SUBMIT_ERROR_OPEN: &str = "locSubmitErrorOpen";
const LOCALIZED_SIGNAL_SUBMIT_ERROR_TEXT: &str = "locSubmitErrorText";
const LOCALIZED_SIGNAL_STATUS_TEXT: &str = "locStatusText";
const LOCALIZED_SIGNAL_SOURCE_LANGUAGE: &str = "locSourceLanguage";
const LOCALIZED_SIGNAL_TRANSLATING: &str = "locTranslating";
const LOCALIZED_SIGNAL_TRANSLATION_READY: &str = "locTranslationAvailable";
const LOCALIZED_SIGNAL_ACTIVE_WIDGET: &str = "locActiveTranslationWidget";

const LOCALIZED_TRANSLATION_SCOPE_FIELD_NAME: &str = "scope";
/// The translation transport widget-key field name.
pub const LOCALIZED_TRANSLATION_WIDGET_KEY_FIELD_NAME: &str = "widget_key";
const LOCALIZED_TRANSLATION_SOURCE_LANGUAGE_FIELD_NAME: &str = "source_language";
const LOCALIZED_TRANSLATION_TARGET_LANGUAGE_FIELD_NAME: &str = "target_language";
const LOCALIZED_TRANSLATION_FIELD_NAME_FIELD_NAME: &str = "field_name";
const LOCALIZED_TRANSLATION_FIELD_VALUE_FIELD_NAME: &str = "field_value";
const LOCALIZED_TRANSLATION_FIELD_FORMAT_FIELD_NAME: &str = "field_format";

/// Configures the authoring widget state.
#[derive(Clone, Default)]
pub struct LocalizedTabsAuthoringOptions {
    /// The authoring scope.
    pub scope: String,
    /// The widget key.
    pub widget_key: String,
    /// The translation stream URL.
    pub stream_url: String,
    /// The auto-translate checkbox name.
    pub checkbox_name: String,
    /// Whether the checkbox starts checked.
    pub checkbox_checked: bool,
    /// Whether translation is available.
    pub translation_available: bool,
    /// The translated text lookup.
    pub text: Option<TranslateKeyFunc<Request>>,
}

/// The authoring widget state.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LocalizedTabsAuthoringProps {
    /// Whether the authoring surface is enabled.
    pub enabled: bool,
    /// Whether translation is available.
    pub translation_available: bool,
    /// The authoring scope.
    pub scope: String,
    /// The widget key.
    pub widget_key: String,
    /// The translation stream URL.
    pub stream_url: String,
    /// The checkbox id.
    pub checkbox_id: String,
    /// The checkbox name.
    pub checkbox_name: String,
    /// Whether the checkbox is checked.
    pub checkbox_checked: bool,
    /// The auto-translate label.
    pub auto_translate_label: String,
    /// The auto-translate help text.
    pub auto_translate_help: String,
    /// The idle status text.
    pub status_idle_text: String,
    /// The off status text.
    pub status_off_text: String,
    /// The working status text.
    pub status_working_text: String,
    /// The done status text.
    pub status_done_text: String,
    /// The error status text.
    pub status_error_text: String,
    /// The unavailable status text.
    pub status_unavailable_text: String,
    /// The required-tab-missing text.
    pub tab_missing_text: String,
    /// The tab-complete text.
    pub tab_complete_text: String,
    /// The empty-tab text.
    pub tab_empty_text: String,
    /// The blocked-submit text.
    pub submit_blocked_text: String,
    /// The translating-submit text.
    pub submit_translating_text: String,
    /// The missing-required-submit text.
    pub submit_error_text: String,
    /// The unavailable-submit text.
    pub submit_unavailable_text: String,
    /// The missing-source-submit text.
    pub submit_empty_source_text: String,
}

/// The authoring widget's signal names.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct LocalizedAuthoringSignals {
    /// The normalized widget key.
    pub widget_key: String,
    /// The auto-translate toggle signal.
    pub auto_translate: String,
    /// The source-language signal.
    pub source_language: String,
    /// The translating signal.
    pub translating: String,
    /// The translation-available signal.
    pub translation_available: String,
    /// The status-text signal.
    pub status_text: String,
}

/// Set the project-wide authoring availability.
pub fn set_localized_authoring_availability(available: bool) {
    LOCALIZED_AUTHORING_AVAILABILITY.store(available, Ordering::SeqCst);
    LOCALIZED_AUTHORING_AVAILABILITY_SET.store(true, Ordering::SeqCst);
}

fn localized_authoring_is_available() -> bool {
    if !LOCALIZED_AUTHORING_AVAILABILITY_SET.load(Ordering::SeqCst) {
        return true;
    }
    LOCALIZED_AUTHORING_AVAILABILITY.load(Ordering::SeqCst)
}

/// Build the authoring widget state.
#[must_use]
pub fn new_localized_tabs_authoring_props(
    request: &Request,
    options: LocalizedTabsAuthoringOptions,
) -> LocalizedTabsAuthoringProps {
    let text = |key: &str| match options.text.as_ref() {
        Some(translate) => translate(request, key),
        None => key.to_owned(),
    };
    let translation_available = options.translation_available && localized_authoring_is_available();
    LocalizedTabsAuthoringProps {
        enabled: true,
        translation_available,
        scope: options.scope.trim().to_owned(),
        widget_key: options.widget_key.trim().to_owned(),
        stream_url: options.stream_url.trim().to_owned(),
        checkbox_id: options.checkbox_name.trim().to_owned(),
        checkbox_name: options.checkbox_name.trim().to_owned(),
        checkbox_checked: options.checkbox_checked,
        auto_translate_label: text("localized_tabs.auto_translate_label"),
        auto_translate_help: text("localized_tabs.auto_translate_help"),
        status_idle_text: text("localized_tabs.auto_translate_ready"),
        status_off_text: text("localized_tabs.auto_translate_off"),
        status_working_text: text("localized_tabs.auto_translate_working"),
        status_done_text: text("localized_tabs.auto_translate_done"),
        status_error_text: text("localized_tabs.auto_translate_failed"),
        status_unavailable_text: text("localized_tabs.auto_translate_unavailable"),
        tab_missing_text: text("localized_tabs.tab_missing_required_content"),
        tab_complete_text: text("localized_tabs.tab_content_complete"),
        tab_empty_text: text("localized_tabs.tab_no_content"),
        submit_blocked_text: text("localized_tabs.submit_blocked_notice"),
        submit_translating_text: text("localized_tabs.submit_translating_notice"),
        submit_error_text: text("localized_tabs.submit_error_missing_required"),
        submit_unavailable_text: text("localized_tabs.submit_error_unavailable"),
        submit_empty_source_text: text("localized_tabs.submit_error_source_missing"),
    }
}

/// The localized form error alert class.
#[must_use]
pub fn localized_form_error_alert_class() -> String {
    web_components::components::shared::build_classes(&[
        "mb-6 rounded-2xl border-2 border-destructive bg-destructive-muted text-foreground shadow-xl ring-2 ring-destructive/35",
        "font-medium [&_a]:text-destructive [&_a]:underline [&_a]:underline-offset-2",
    ])
}

/// Merge the localized submit-control attributes into the caller's attributes.
#[must_use]
pub fn localized_submit_control_attributes(
    extra: BTreeMap<String, String>,
) -> BTreeMap<String, String> {
    let mut attributes = BTreeMap::new();
    attributes.insert("data-localized-submit-control".to_owned(), "true".to_owned());
    attributes.insert(
        "data-attr:aria-disabled".to_owned(),
        format!("${LOCALIZED_SIGNAL_SUBMIT_BLOCKED} ? 'true' : 'false'"),
    );
    attributes.insert(
        "data-attr:disabled".to_owned(),
        format!("${LOCALIZED_SIGNAL_SUBMIT_BLOCKED} ? 'true' : null"),
    );
    attributes.insert(
        "data-class:opacity-55".to_owned(),
        format!("${LOCALIZED_SIGNAL_SUBMIT_BLOCKED}"),
    );
    attributes.insert(
        "data-class:saturate-50".to_owned(),
        format!("${LOCALIZED_SIGNAL_SUBMIT_BLOCKED}"),
    );
    attributes.insert(
        "data-class:cursor-not-allowed".to_owned(),
        format!("${LOCALIZED_SIGNAL_SUBMIT_BLOCKED}"),
    );
    attributes.insert(
        "data-on:click".to_owned(),
        localized_authoring_submit_attempt_action(),
    );
    attributes.extend(extra);
    attributes
}

/// Build the authoring form attributes for one widget.
#[must_use]
pub fn localized_authoring_form_attributes(
    props: &LocalizedTabsProps,
    extra: BTreeMap<String, String>,
) -> BTreeMap<String, String> {
    localized_authoring_form_attributes_for_widgets(std::slice::from_ref(props), extra)
}

/// Build the authoring form attributes for several widgets.
#[must_use]
pub fn localized_authoring_form_attributes_for_widgets(
    props_list: &[LocalizedTabsProps],
    extra: BTreeMap<String, String>,
) -> BTreeMap<String, String> {
    let mut attributes = extra;
    attributes.extend(localized_authoring_form_root_attributes(props_list));
    attributes
}

/// The field value signal name.
#[must_use]
pub fn localized_field_signal_name(
    lang: &str,
    field_key: &str,
    normalize: Option<NormalizerRef<'_>>,
) -> String {
    let normalized_lang = localized_normalize_language(lang, normalize);
    localized_authoring_signal_name("value", normalized_lang.as_str(), field_key)
}

/// The field format signal name.
#[must_use]
pub fn localized_format_signal_name(
    lang: &str,
    field_key: &str,
    normalize: Option<NormalizerRef<'_>>,
) -> String {
    let normalized_lang = localized_normalize_language(lang, normalize);
    localized_authoring_signal_name("format", normalized_lang.as_str(), field_key)
}

/// The has-content signal name for a widget and language.
#[must_use]
pub fn localized_has_content_signal_name(
    widget_key: &str,
    lang: &str,
    normalize: Option<NormalizerRef<'_>>,
) -> String {
    let normalized_lang = localized_normalize_language(lang, normalize);
    let base = localized_authoring_signal_name("has", normalized_lang.as_str(), "");
    localized_authoring_scoped_signal_name(base.as_str(), widget_key)
}

fn localized_authoring_signal_name(prefix: &str, lang: &str, field_key: &str) -> String {
    let mut parts = vec![
        "loc".to_owned(),
        normalize_localized_signal_part(prefix),
        normalize_localized_signal_part(lang),
    ];
    let normalized_field_key = normalize_localized_signal_part(field_key);
    if !normalized_field_key.is_empty() {
        parts.push(normalized_field_key);
    }
    parts.join("_")
}

/// The authoring signal names for one widget.
#[must_use]
pub fn localized_authoring_signals_for_widget(widget_key: &str) -> LocalizedAuthoringSignals {
    let normalized_widget_key = localized_authoring_widget_key(widget_key);
    LocalizedAuthoringSignals {
        widget_key: normalized_widget_key.clone(),
        auto_translate: localized_authoring_scoped_signal_name(
            LOCALIZED_SIGNAL_AUTO_TRANSLATE,
            &normalized_widget_key,
        ),
        source_language: localized_authoring_scoped_signal_name(
            LOCALIZED_SIGNAL_SOURCE_LANGUAGE,
            &normalized_widget_key,
        ),
        translating: localized_authoring_scoped_signal_name(
            LOCALIZED_SIGNAL_TRANSLATING,
            &normalized_widget_key,
        ),
        translation_available: localized_authoring_scoped_signal_name(
            LOCALIZED_SIGNAL_TRANSLATION_READY,
            &normalized_widget_key,
        ),
        status_text: localized_authoring_scoped_signal_name(
            LOCALIZED_SIGNAL_STATUS_TEXT,
            &normalized_widget_key,
        ),
    }
}

fn localized_authoring_widget_key(widget_key: &str) -> String {
    let normalized = normalize_localized_signal_part(widget_key);
    if normalized.is_empty() {
        "authoring".to_owned()
    } else {
        normalized
    }
}

fn localized_authoring_scoped_signal_name(base: &str, widget_key: &str) -> String {
    let base = normalize_localized_signal_part(base);
    let widget_key = localized_authoring_widget_key(widget_key);
    if base.is_empty() {
        return widget_key;
    }
    format!("{base}_{widget_key}")
}

/// Normalize one signal name part.
#[must_use]
pub fn normalize_localized_signal_part(value: &str) -> String {
    let value = value.trim();
    if value.is_empty() {
        return String::new();
    }
    let mut builder = String::new();
    let mut last_underscore = false;
    for character in value.chars() {
        if character.is_alphanumeric() {
            for lower in character.to_lowercase() {
                builder.push(lower);
            }
            last_underscore = false;
        } else if !last_underscore {
            builder.push('_');
            last_underscore = true;
        }
    }
    builder.trim_matches('_').to_owned()
}

fn localized_transport_inactive_expr(widget_key: &str) -> String {
    let signals = localized_authoring_signals_for_widget(widget_key);
    format!(
        "${LOCALIZED_SIGNAL_ACTIVE_WIDGET} !== {}",
        quoted_js(&signals.widget_key)
    )
}

fn localized_transport_disabled_expr(widget_key: &str) -> String {
    format!("{} ? 'true' : null", localized_transport_inactive_expr(widget_key))
}

fn localized_authoring_form_root_attributes(
    props_list: &[LocalizedTabsProps],
) -> BTreeMap<String, String> {
    let Some(primary) = localized_primary_authoring_props(props_list) else {
        return BTreeMap::new();
    };
    let mut attributes = BTreeMap::new();
    attributes.insert("data-localized-authoring".to_owned(), "true".to_owned());
    attributes.insert(
        "data-signals".to_owned(),
        localized_authoring_signals_for_widgets(props_list),
    );
    attributes.insert(
        "data-submit-blocked-text".to_owned(),
        primary.submit_blocked_text.clone(),
    );
    attributes.insert(
        "data-submit-translating-text".to_owned(),
        primary.submit_translating_text.clone(),
    );
    attributes.insert(
        "data-submit-error-text".to_owned(),
        primary.submit_error_text.clone(),
    );
    attributes.insert(
        "data-submit-unavailable-text".to_owned(),
        primary.submit_unavailable_text.clone(),
    );
    attributes.insert(
        "data-submit-empty-source-text".to_owned(),
        primary.submit_empty_source_text.clone(),
    );
    attributes.insert(
        "data-attr:aria-busy".to_owned(),
        format!(
            "{} ? 'true' : 'false'",
            localized_any_widget_translating_expr(props_list)
        ),
    );
    attributes.insert(
        "data-init".to_owned(),
        localized_authoring_init_action(),
    );
    attributes.insert(
        "data-on:input__capture".to_owned(),
        localized_authoring_sync_submit_action(),
    );
    attributes.insert(
        "data-on:change__capture".to_owned(),
        localized_authoring_sync_submit_action(),
    );
    attributes.insert(
        "data-on:datastar-signal-patch".to_owned(),
        localized_authoring_signal_patch_action(),
    );
    attributes
}

fn localized_authoring_signals_for_widgets(props_list: &[LocalizedTabsProps]) -> String {
    let mut signals: Vec<(String, String)> = vec![
        (LOCALIZED_SIGNAL_SUBMIT_BLOCKED.to_owned(), "true".to_owned()),
        (LOCALIZED_SIGNAL_SUBMIT_ERROR_OPEN.to_owned(), "false".to_owned()),
        (
            LOCALIZED_SIGNAL_SUBMIT_ERROR_TEXT.to_owned(),
            quoted_js(""),
        ),
        (
            LOCALIZED_SIGNAL_ACTIVE_WIDGET.to_owned(),
            quoted_js(""),
        ),
    ];
    let mut seen: std::collections::BTreeSet<String> = signals
        .iter()
        .map(|(name, _value)| name.clone())
        .collect();
    for props in props_list {
        let Some(authoring) = props.authoring.as_ref() else {
            continue;
        };
        if !authoring.enabled {
            continue;
        }
        let widget_signals = localized_authoring_signals_for_widget(authoring.widget_key.as_str());
        for (name, value) in [
            (
                widget_signals.auto_translate.clone(),
                authoring.checkbox_checked.to_string(),
            ),
            (
                widget_signals.source_language.clone(),
                quoted_js(localized_tabs_default_value(props).as_str()),
            ),
            (widget_signals.translating.clone(), "false".to_owned()),
            (
                widget_signals.translation_available.clone(),
                authoring.translation_available.to_string(),
            ),
            (
                widget_signals.status_text.clone(),
                quoted_js(localized_tabs_initial_status(props).as_str()),
            ),
        ] {
            if !seen.insert(name.clone()) {
                continue;
            }
            signals.push((name, value));
        }
        for tab in &props.tabs {
            let lang = localized_normalize_language(tab.value.as_str(), props.normalize_language.as_deref());
            let field_key = tab.field_key.trim();
            if field_key.is_empty() || lang.is_empty() {
                continue;
            }
            for (name, value) in [
                (
                    localized_field_signal_name(lang.as_str(), field_key, props.normalize_language.as_deref()),
                    quoted_js_bytes(tab.field_value.as_bytes()),
                ),
                (
                    localized_format_signal_name(lang.as_str(), field_key, props.normalize_language.as_deref()),
                    quoted_js_bytes(tab.format_value.as_bytes()),
                ),
                (
                    localized_has_content_signal_name(
                        widget_signals.widget_key.as_str(),
                        lang.as_str(),
                        props.normalize_language.as_deref(),
                    ),
                    (!crate::validation::trim_unicode_whitespace(tab.field_value.as_bytes()).is_empty()).to_string(),
                ),
            ] {
                if !seen.insert(name.clone()) {
                    continue;
                }
                signals.push((name, value));
            }
        }
    }
    let parts: Vec<String> = signals
        .into_iter()
        .map(|(name, value)| format!("{}:{value}", quoted_js(name.as_str())))
        .collect();
    format!("{{{}}}", parts.join(","))
}

fn localized_authoring_init_action() -> String {
    localized_authoring_recompute_submit_action()
}

fn localized_authoring_sync_submit_action() -> String {
    format!(
        "${LOCALIZED_SIGNAL_SUBMIT_ERROR_OPEN} = false; ${LOCALIZED_SIGNAL_SUBMIT_ERROR_TEXT} = ''; {}",
        localized_authoring_recompute_submit_action()
    )
}

fn localized_authoring_debounced_translate_action(widget_key: &str) -> String {
    let signals = localized_authoring_signals_for_widget(widget_key);
    [
        "const root = el.closest('[data-localized-widget=\"true\"]');",
        "const translationURL = root instanceof HTMLElement ? String(root.dataset.translationUrl || '') : '';",
        "const workingText = root instanceof HTMLElement ? String(root.dataset.statusWorking || 'Translating...') : 'Translating...';",
        "if (translationURL === '') return;",
        &format!(
            "if (!${} || !${}) return;",
            signals.auto_translate, signals.translation_available
        ),
        "if (!(evt.target instanceof HTMLInputElement || evt.target instanceof HTMLTextAreaElement)) return;",
        "if (String(evt.target.value ?? '').trim() === '') return;",
        &format!(
            "${LOCALIZED_SIGNAL_ACTIVE_WIDGET} = {};",
            quoted_js(signals.widget_key.as_str())
        ),
        &format!("${} = true;", signals.translating),
        &format!("${} = workingText;", signals.status_text),
        &format!("${LOCALIZED_SIGNAL_SUBMIT_BLOCKED} = true;"),
        &localized_authoring_post_action("translationURL"),
        &localized_authoring_recompute_submit_action(),
    ]
    .join(" ")
}

fn localized_authoring_toggle_action(widget_key: &str) -> String {
    let signals = localized_authoring_signals_for_widget(widget_key);
    [
        "const root = el.closest('[data-localized-widget=\"true\"]');".to_owned(),
        "const statusIdle = root instanceof HTMLElement ? String(root.dataset.statusIdle || 'Ready to translate') : 'Ready to translate';".to_owned(),
        "const statusOff = root instanceof HTMLElement ? String(root.dataset.statusOff || 'Automatic translation is off') : 'Automatic translation is off';".to_owned(),
        "const statusUnavailable = root instanceof HTMLElement ? String(root.dataset.statusUnavailable || 'Translation is unavailable right now') : 'Translation is unavailable right now';".to_owned(),
        localized_authoring_sync_submit_action(),
        "if (!(evt.target instanceof HTMLInputElement)) return;".to_owned(),
        format!(
            "${LOCALIZED_SIGNAL_ACTIVE_WIDGET} = {};",
            quoted_js(signals.widget_key.as_str())
        ),
        format!("${} = !!evt.target.checked;", signals.auto_translate),
        format!(
            "${} = ${} ? (${} ? statusIdle : statusUnavailable) : statusOff;",
            signals.status_text, signals.auto_translate, signals.translation_available
        ),
    ]
    .join(" ")
}

fn localized_authoring_format_change_action(widget_key: &str) -> String {
    let signals = localized_authoring_signals_for_widget(widget_key);
    [
        "const root = el.closest('[data-localized-widget=\"true\"]');".to_owned(),
        "const translationURL = root instanceof HTMLElement ? String(root.dataset.translationUrl || '') : '';".to_owned(),
        "const workingText = root instanceof HTMLElement ? String(root.dataset.statusWorking || 'Translating...') : 'Translating...';".to_owned(),
        localized_authoring_sync_submit_action(),
        "if (translationURL === '') return;".to_owned(),
        "if (!(evt.target instanceof HTMLSelectElement)) return;".to_owned(),
        format!(
            "if (!${} || !${}) return;",
            signals.auto_translate, signals.translation_available
        ),
        format!(
            "${LOCALIZED_SIGNAL_ACTIVE_WIDGET} = {};",
            quoted_js(signals.widget_key.as_str())
        ),
        format!("${} = true;", signals.translating),
        format!("${} = workingText;", signals.status_text),
        format!("${LOCALIZED_SIGNAL_SUBMIT_BLOCKED} = true;"),
        localized_authoring_post_action("translationURL"),
        localized_authoring_recompute_submit_action(),
    ]
    .join(" ")
}

fn localized_authoring_prepare_run_action(props: &LocalizedTabsProps) -> String {
    let Some(authoring) = props.authoring.as_ref() else {
        return String::new();
    };
    let signals = localized_authoring_signals_for_widget(authoring.widget_key.as_str());
    [
        "evt.preventDefault();".to_owned(),
        format!(
            "${LOCALIZED_SIGNAL_ACTIVE_WIDGET} = {};",
            quoted_js(signals.widget_key.as_str())
        ),
        format!(
            "${} = $activeTab || ${};",
            signals.source_language, signals.source_language
        ),
    ]
    .join(" ")
}

fn localized_authoring_run_action(props: &LocalizedTabsProps) -> String {
    let Some(authoring) = props.authoring.as_ref() else {
        return String::new();
    };
    let signals = localized_authoring_signals_for_widget(authoring.widget_key.as_str());
    [
        "evt.preventDefault();".to_owned(),
        format!(
            "${LOCALIZED_SIGNAL_ACTIVE_WIDGET} = {};",
            quoted_js(signals.widget_key.as_str())
        ),
        format!(
            "${} = $activeTab || ${};",
            signals.source_language, signals.source_language
        ),
        format!("if (!${}) return;", signals.translation_available),
        format!("${} = true;", signals.translating),
        format!(
            "${} = {};",
            signals.status_text,
            quoted_js(authoring.status_working_text.as_str())
        ),
        format!("${LOCALIZED_SIGNAL_SUBMIT_BLOCKED} = true;"),
        localized_authoring_post_action(quoted_js(authoring.stream_url.as_str()).as_str()),
        localized_authoring_recompute_submit_action(),
    ]
    .join(" ")
}

fn localized_authoring_signal_patch_action() -> String {
    localized_authoring_recompute_submit_action()
}

fn localized_authoring_submit_attempt_action() -> String {
    format!(
        "const root = el.closest('[data-localized-authoring=\"true\"]'); \
const form = el.closest('form'); \
if (!(form instanceof HTMLFormElement)) return; \
const submitErrorText = root instanceof HTMLElement ? String(root.dataset.submitErrorText || 'Complete the required fields before submitting.') : 'Complete the required fields before submitting.'; \
const submitUnavailableText = root instanceof HTMLElement ? String(root.dataset.submitUnavailableText || submitErrorText) : submitErrorText; \
const submitEmptySourceText = root instanceof HTMLElement ? String(root.dataset.submitEmptySourceText || submitErrorText) : submitErrorText; \
const widgets = Array.from(form.querySelectorAll('[data-localized-widget=\"true\"]')); \
const anyWidgetTranslating = widgets.some((widget) => widget instanceof HTMLElement && String(widget.dataset.localizedTranslating || '') === 'true'); \
const workingWidget = widgets.find((widget) => widget instanceof HTMLElement && String(widget.dataset.localizedTranslating || '') === 'true'); \
const workingText = workingWidget instanceof HTMLElement ? String(workingWidget.dataset.statusWorking || 'Translating...') : 'Translating...'; \
const missingRequiredProxy = Array.from(form.querySelectorAll('[data-form-required-proxy=\"true\"]')).some((field) => {{ \
if (!(field instanceof HTMLInputElement || field instanceof HTMLTextAreaElement || field instanceof HTMLSelectElement)) return false; \
if (field.disabled) return false; \
return String(field.value ?? '').trim() === ''; \
}}); \
const missingLocalizedRequired = Array.from(form.querySelectorAll('[data-localized-tab-indicator=\"true\"][data-localized-tab-required=\"true\"]')).some((indicator) => {{ \
if (!(indicator instanceof HTMLElement)) return false; \
return String(indicator.dataset.localizedTabState || '') !== 'complete'; \
}}); \
if (anyWidgetTranslating) {{ \
evt.preventDefault(); \
${LOCALIZED_SIGNAL_SUBMIT_ERROR_TEXT} = workingText; \
${LOCALIZED_SIGNAL_SUBMIT_ERROR_OPEN} = true; \
return; \
}} \
const unavailableWidget = widgets.find((widget) => {{ \
if (!(widget instanceof HTMLElement)) return false; \
if (String(widget.dataset.localizedAutoTranslate || '') !== 'true') return false; \
if (String(widget.dataset.localizedTranslationAvailable || '') === 'true') return false; \
return Array.from(widget.querySelectorAll('[data-localized-tab-indicator=\"true\"][data-localized-tab-required=\"true\"]')).some((indicator) => indicator instanceof HTMLElement && String(indicator.dataset.localizedTabState || '') !== 'complete'); \
}}); \
const emptySourceWidget = widgets.find((widget) => {{ \
if (!(widget instanceof HTMLElement)) return false; \
if (String(widget.dataset.localizedAutoTranslate || '') !== 'true') return false; \
const sourceLanguage = String(widget.dataset.localizedSourceLanguage || '').trim(); \
if (sourceLanguage === '') return false; \
const widgetKey = String(widget.dataset.localizedWidgetKey || '').trim(); \
const missingRequired = Array.from(widget.querySelectorAll('[data-localized-tab-indicator=\"true\"][data-localized-tab-required=\"true\"]')).some((indicator) => indicator instanceof HTMLElement && String(indicator.dataset.localizedTabState || '') !== 'complete'); \
if (!missingRequired) return false; \
const sourceFieldSelector = '[data-localized-field=\"true\"][data-localized-widget-key=\"' + widgetKey + '\"][data-localized-lang=\"' + sourceLanguage + '\"]'; \
const sourceField = form.querySelector(sourceFieldSelector); \
return !(sourceField instanceof HTMLInputElement || sourceField instanceof HTMLTextAreaElement) || String(sourceField.value ?? '').trim() === ''; \
}}); \
if (!form.checkValidity() || missingRequiredProxy || missingLocalizedRequired) {{ \
evt.preventDefault(); \
if (typeof form.reportValidity === 'function') {{ form.reportValidity(); }} \
if (unavailableWidget) {{ \
${LOCALIZED_SIGNAL_SUBMIT_ERROR_TEXT} = submitUnavailableText; \
}} else if (emptySourceWidget) {{ \
${LOCALIZED_SIGNAL_SUBMIT_ERROR_TEXT} = submitEmptySourceText; \
}} else {{ \
${LOCALIZED_SIGNAL_SUBMIT_ERROR_TEXT} = submitErrorText; \
}} \
${LOCALIZED_SIGNAL_SUBMIT_ERROR_OPEN} = true; \
return; \
}} \
${LOCALIZED_SIGNAL_SUBMIT_ERROR_OPEN} = false; \
${LOCALIZED_SIGNAL_SUBMIT_ERROR_TEXT} = '';"
    )
}

fn localized_authoring_post_action(url_expr: &str) -> String {
    format!(
        "const form = el.closest('form'); if (!(form instanceof HTMLFormElement)) return; if (!form.id) {{ form.id = 'localized-authoring-form'; }} const previousNoValidate = form.noValidate; form.noValidate = true; @post({url_expr}, {{contentType: 'form'}}); form.noValidate = previousNoValidate;"
    )
}

/// Build the localized field attributes.
#[must_use]
pub fn localized_field_attributes(
    widget_key: &str,
    lang: &str,
    field_key: &str,
    normalize: Option<NormalizerRef<'_>>,
) -> BTreeMap<String, String> {
    let normalized_lang = localized_normalize_language(lang, normalize);
    let signals = localized_authoring_signals_for_widget(widget_key);
    let mut attributes = BTreeMap::new();
    attributes.insert("data-localized-field".to_owned(), "true".to_owned());
    attributes.insert(
        "data-localized-widget-key".to_owned(),
        signals.widget_key.clone(),
    );
    attributes.insert("data-localized-lang".to_owned(), normalized_lang.clone());
    attributes.insert("data-localized-field-key".to_owned(), field_key.to_owned());
    attributes.insert(
        "data-bind".to_owned(),
        localized_field_signal_name(normalized_lang.as_str(), field_key, normalize),
    );
    attributes.insert(
        "data-on:focusin".to_owned(),
        format!(
            "${LOCALIZED_SIGNAL_ACTIVE_WIDGET} = {}; ${} = {};",
            quoted_js(signals.widget_key.as_str()),
            signals.source_language,
            quoted_js(normalized_lang.as_str())
        ),
    );
    attributes.insert(
        "data-on:input".to_owned(),
        localized_authoring_field_input_action(signals.widget_key.as_str(), normalized_lang.as_str()),
    );
    attributes.insert(
        "data-on:input__debounce.500ms".to_owned(),
        localized_authoring_debounced_translate_action(signals.widget_key.as_str()),
    );
    attributes
}

/// Build the localized content-format attributes.
#[must_use]
pub fn localized_format_attributes(
    widget_key: &str,
    lang: &str,
    field_key: &str,
    normalize: Option<NormalizerRef<'_>>,
) -> BTreeMap<String, String> {
    let normalized_lang = localized_normalize_language(lang, normalize);
    let signals = localized_authoring_signals_for_widget(widget_key);
    let mut attributes = BTreeMap::new();
    attributes.insert("data-localized-format".to_owned(), "true".to_owned());
    attributes.insert(
        "data-localized-widget-key".to_owned(),
        signals.widget_key.clone(),
    );
    attributes.insert("data-localized-lang".to_owned(), normalized_lang.clone());
    attributes.insert("data-localized-field-key".to_owned(), field_key.to_owned());
    attributes.insert(
        "data-bind".to_owned(),
        localized_format_signal_name(normalized_lang.as_str(), field_key, normalize),
    );
    attributes.insert(
        "data-on:focusin".to_owned(),
        format!(
            "${LOCALIZED_SIGNAL_ACTIVE_WIDGET} = {}; ${} = {};",
            quoted_js(signals.widget_key.as_str()),
            signals.source_language,
            quoted_js(normalized_lang.as_str())
        ),
    );
    attributes.insert(
        "data-on:change".to_owned(),
        localized_authoring_format_change_action(signals.widget_key.as_str()),
    );
    attributes
}

/// Normalize one authoring language through the caller's normalizer.
#[must_use]
pub fn localized_normalize_language(lang: &str, normalize: Option<NormalizerRef<'_>>) -> String {
    normalize_authoring_language(lang, normalize)
}

fn localized_authoring_field_input_action(widget_key: &str, normalized_lang: &str) -> String {
    let signals = localized_authoring_signals_for_widget(widget_key);
    [
        format!(
            "${LOCALIZED_SIGNAL_ACTIVE_WIDGET} = {}",
            quoted_js(signals.widget_key.as_str())
        ),
        format!(
            "${} = {}",
            signals.source_language,
            quoted_js(normalized_lang)
        ),
        format!(
            "${} = String(evt.target.value ?? '').trim() !== ''",
            localized_has_content_signal_name(
                signals.widget_key.as_str(),
                normalized_lang,
                None
            )
        ),
        localized_authoring_sync_submit_action(),
    ]
    .join("; ")
}

fn localized_authoring_recompute_submit_action() -> String {
    format!(
        "${LOCALIZED_SIGNAL_SUBMIT_BLOCKED} = (() => {{ \
const form = el.closest('form'); \
const anyWidgetTranslating = form instanceof HTMLFormElement && Array.from(form.querySelectorAll('[data-localized-widget=\"true\"]')).some((widget) => widget instanceof HTMLElement && String(widget.dataset.localizedTranslating || '') === 'true'); \
const missingRequiredProxy = form instanceof HTMLFormElement && Array.from(form.querySelectorAll('[data-form-required-proxy=\"true\"]')).some((field) => {{ \
if (!(field instanceof HTMLInputElement || field instanceof HTMLTextAreaElement || field instanceof HTMLSelectElement)) return false; \
if (field.disabled) return false; \
return String(field.value ?? '').trim() === ''; \
}}); \
const missingLocalizedRequired = form instanceof HTMLFormElement && Array.from(form.querySelectorAll('[data-localized-tab-indicator=\"true\"][data-localized-tab-required=\"true\"]')).some((indicator) => {{ \
if (!(indicator instanceof HTMLElement)) return false; \
return String(indicator.dataset.localizedTabState || '') !== 'complete'; \
}}); \
return anyWidgetTranslating || !(form instanceof HTMLFormElement && form.checkValidity()) || missingRequiredProxy || missingLocalizedRequired; \
}})();"
    )
}

fn localized_primary_authoring_props(
    props_list: &[LocalizedTabsProps],
) -> Option<&LocalizedTabsAuthoringProps> {
    props_list
        .iter()
        .find_map(|props| props.authoring.as_deref().filter(|authoring| authoring.enabled))
}

fn localized_any_widget_translating_expr(props_list: &[LocalizedTabsProps]) -> String {
    let mut terms: Vec<String> = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for props in props_list {
        let Some(authoring) = props.authoring.as_deref() else {
            continue;
        };
        if !authoring.enabled {
            continue;
        }
        let signals = localized_authoring_signals_for_widget(authoring.widget_key.as_str());
        if !seen.insert(signals.translating.clone()) {
            continue;
        }
        terms.push(format!("${}", signals.translating));
    }
    if terms.is_empty() {
        "false".to_owned()
    } else {
        format!("({})", terms.join(" || "))
    }
}

pub(crate) fn localized_tabs_default_value(props: &LocalizedTabsProps) -> String {
    if !props.default_value.is_empty() {
        return props.default_value.clone();
    }
    props
        .tabs
        .first()
        .map(|tab| tab.value.clone())
        .unwrap_or_default()
}

pub(crate) fn localized_tabs_has_authoring(props: &LocalizedTabsProps) -> bool {
    props
        .authoring
        .as_ref()
        .is_some_and(|authoring| authoring.enabled && !authoring.stream_url.is_empty())
}

pub(crate) fn localized_tabs_initial_status(props: &LocalizedTabsProps) -> String {
    let Some(authoring) = props.authoring.as_ref() else {
        return String::new();
    };
    if !localized_tabs_has_authoring(props) {
        return String::new();
    }
    if authoring.checkbox_checked && !authoring.translation_available {
        return authoring.status_unavailable_text.clone();
    }
    if authoring.checkbox_checked {
        return authoring.status_idle_text.clone();
    }
    authoring.status_off_text.clone()
}

pub(crate) fn localized_tabs_root_attributes(
    props: &LocalizedTabsProps,
) -> BTreeMap<String, String> {
    let Some(authoring) = props.authoring.as_ref() else {
        return BTreeMap::new();
    };
    if !localized_tabs_has_authoring(props) {
        return BTreeMap::new();
    }
    let signals = localized_authoring_signals_for_widget(authoring.widget_key.as_str());
    let mut attributes = BTreeMap::new();
    attributes.insert("data-localized-widget".to_owned(), "true".to_owned());
    attributes.insert(
        "data-localized-widget-key".to_owned(),
        signals.widget_key.clone(),
    );
    attributes.insert(
        "data-translation-url".to_owned(),
        authoring.stream_url.clone(),
    );
    attributes.insert(
        "data-translation-scope".to_owned(),
        authoring.scope.clone(),
    );
    attributes.insert(
        "data-status-idle".to_owned(),
        authoring.status_idle_text.clone(),
    );
    attributes.insert(
        "data-status-off".to_owned(),
        authoring.status_off_text.clone(),
    );
    attributes.insert(
        "data-status-working".to_owned(),
        authoring.status_working_text.clone(),
    );
    attributes.insert(
        "data-status-done".to_owned(),
        authoring.status_done_text.clone(),
    );
    attributes.insert(
        "data-status-error".to_owned(),
        authoring.status_error_text.clone(),
    );
    attributes.insert(
        "data-status-unavailable".to_owned(),
        authoring.status_unavailable_text.clone(),
    );
    attributes.insert(
        "data-translation-available".to_owned(),
        authoring.translation_available.to_string(),
    );
    attributes.insert(
        "data-tab-status-missing".to_owned(),
        authoring.tab_missing_text.clone(),
    );
    attributes.insert(
        "data-tab-status-complete".to_owned(),
        authoring.tab_complete_text.clone(),
    );
    attributes.insert(
        "data-tab-status-empty".to_owned(),
        authoring.tab_empty_text.clone(),
    );
    attributes.insert(
        "data-attr:data-localized-auto-translate".to_owned(),
        format!("${} ? 'true' : 'false'", signals.auto_translate),
    );
    attributes.insert(
        "data-attr:data-localized-source-language".to_owned(),
        format!("${} || ''", signals.source_language),
    );
    attributes.insert(
        "data-attr:data-localized-translating".to_owned(),
        format!("${} ? 'true' : 'false'", signals.translating),
    );
    attributes.insert(
        "data-attr:data-localized-translation-available".to_owned(),
        format!("${} ? 'true' : 'false'", signals.translation_available),
    );
    attributes.insert(
        "data-attr:aria-busy".to_owned(),
        format!("${} ? 'true' : 'false'", signals.translating),
    );
    attributes
}

pub(crate) fn localized_tab_trigger_attributes(tab: &LocalizedTab) -> BTreeMap<String, String> {
    if tab.field_id.is_empty() {
        return BTreeMap::new();
    }
    let mut attributes = BTreeMap::new();
    attributes.insert("id".to_owned(), format!("{}-tab", tab.field_id));
    attributes.insert("aria-controls".to_owned(), format!("{}-panel", tab.field_id));
    attributes
}

pub(crate) fn localized_tab_panel_attributes(tab: &LocalizedTab) -> BTreeMap<String, String> {
    if tab.field_id.is_empty() {
        return BTreeMap::new();
    }
    let mut attributes = BTreeMap::new();
    attributes.insert("id".to_owned(), format!("{}-panel", tab.field_id));
    attributes.insert(
        "aria-labelledby".to_owned(),
        format!("{}-tab", tab.field_id),
    );
    attributes
}

pub(crate) fn localized_tab_required_value(required: bool) -> &'static str {
    if required { "true" } else { "false" }
}

pub(crate) fn localized_tab_field_label(tab: &LocalizedTab) -> String {
    let label = tab.field_label.trim();
    if !label.is_empty() {
        return label.to_owned();
    }
    let label = tab.label.trim();
    if !label.is_empty() {
        return label.to_owned();
    }
    tab.value.trim().to_uppercase()
}

pub(crate) fn localized_tab_indicator_class_expr(
    props: &LocalizedTabsProps,
    tab: &LocalizedTab,
) -> String {
    let has_content_signal = format!(
        "${}",
        localized_has_content_signal_name(
            localized_tabs_widget_key(props).as_str(),
            tab.value.as_str(),
            props.normalize_language.as_deref()
        )
    );
    if tab.required {
        format!(
            "{has_content_signal} ? 'h-1.5 w-1.5 rounded-full flex-shrink-0 bg-success shadow-[0_0_4px_rgba(34,197,94,0.5)]' : 'h-1.5 w-1.5 rounded-full flex-shrink-0 bg-warning shadow-[0_0_4px_rgba(234,179,8,0.5)]'"
        )
    } else {
        format!(
            "{has_content_signal} ? 'h-1.5 w-1.5 rounded-full flex-shrink-0 bg-success shadow-[0_0_4px_rgba(34,197,94,0.5)]' : 'h-1.5 w-1.5 rounded-full flex-shrink-0 bg-muted-foreground/30'"
        )
    }
}

pub(crate) fn localized_tab_indicator_state_expr(
    props: &LocalizedTabsProps,
    tab: &LocalizedTab,
) -> String {
    let has_content_signal = format!(
        "${}",
        localized_has_content_signal_name(
            localized_tabs_widget_key(props).as_str(),
            tab.value.as_str(),
            props.normalize_language.as_deref()
        )
    );
    if tab.required {
        format!("{has_content_signal} ? 'complete' : 'missing'")
    } else {
        format!("{has_content_signal} ? 'complete' : 'empty'")
    }
}

pub(crate) fn localized_tab_indicator_title_expr(
    props: &LocalizedTabsProps,
    tab: &LocalizedTab,
) -> String {
    let has_content_signal = format!(
        "${}",
        localized_has_content_signal_name(
            localized_tabs_widget_key(props).as_str(),
            tab.value.as_str(),
            props.normalize_language.as_deref()
        )
    );
    let Some(authoring) = props.authoring.as_deref() else {
        if tab.required {
            return format!(
                "{has_content_signal} ? {} : {}",
                quoted_js("Content complete"),
                quoted_js("Required content missing")
            );
        }
        return format!(
            "{has_content_signal} ? {} : {}",
            quoted_js("Content complete"),
            quoted_js("No content yet")
        );
    };
    if tab.required {
        format!(
            "{has_content_signal} ? {} : {}",
            quoted_js(authoring.tab_complete_text.as_str()),
            quoted_js(authoring.tab_missing_text.as_str())
        )
    } else {
        format!(
            "{has_content_signal} ? {} : {}",
            quoted_js(authoring.tab_complete_text.as_str()),
            quoted_js(authoring.tab_empty_text.as_str())
        )
    }
}

pub(crate) fn localized_tabs_widget_key(props: &LocalizedTabsProps) -> String {
    props
        .authoring
        .as_ref()
        .map(|authoring| authoring.widget_key.clone())
        .unwrap_or_default()
}

pub(crate) fn localized_tabs_language(props: &LocalizedTabsProps, lang: &str) -> String {
    normalize_authoring_language(lang, props.normalize_language.as_deref())
}

fn quoted_js_bytes(mut value: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut output = String::from("\"");
    loop {
        let (prefix, invalid) = match std::str::from_utf8(value) {
            Ok(text) => (text, false),
            Err(error) => (std::str::from_utf8(&value[..error.valid_up_to()]).expect("validated prefix"), true),
        };
        let quoted = serde_json::to_string(prefix).expect("string serialization");
        output.push_str(&quoted[1..quoted.len() - 1]);
        value = &value[prefix.len()..];
        if !invalid { break; }
        let _ = write!(output, "\\x{:02x}", value[0]);
        value = &value[1..];
    }
    output.push('"');
    output
}

pub(crate) fn quoted_js(value: &str) -> String {
    serde_json::to_string(value.trim()).unwrap_or_else(|_| "\"\"".to_owned())
}

pub(crate) fn localized_transport_disabled_expr_for(widget_key: &str) -> String {
    localized_transport_disabled_expr(widget_key)
}

pub(crate) fn localized_transport_inactive_expr_for(widget_key: &str) -> String {
    localized_transport_inactive_expr(widget_key)
}

pub(crate) fn localized_authoring_toggle_action_for(widget_key: &str) -> String {
    localized_authoring_toggle_action(widget_key)
}

pub(crate) fn localized_authoring_prepare_run_action_for(props: &LocalizedTabsProps) -> String {
    localized_authoring_prepare_run_action(props)
}

pub(crate) fn localized_authoring_run_action_for(props: &LocalizedTabsProps) -> String {
    localized_authoring_run_action(props)
}

pub(crate) const fn localized_translation_scope_field_name() -> &'static str {
    LOCALIZED_TRANSLATION_SCOPE_FIELD_NAME
}

pub(crate) const fn localized_translation_source_language_field_name() -> &'static str {
    LOCALIZED_TRANSLATION_SOURCE_LANGUAGE_FIELD_NAME
}

pub(crate) const fn localized_translation_target_language_field_name() -> &'static str {
    LOCALIZED_TRANSLATION_TARGET_LANGUAGE_FIELD_NAME
}

pub(crate) const fn localized_translation_field_name_field_name() -> &'static str {
    LOCALIZED_TRANSLATION_FIELD_NAME_FIELD_NAME
}

pub(crate) const fn localized_translation_field_value_field_name() -> &'static str {
    LOCALIZED_TRANSLATION_FIELD_VALUE_FIELD_NAME
}

pub(crate) const fn localized_translation_field_format_field_name() -> &'static str {
    LOCALIZED_TRANSLATION_FIELD_FORMAT_FIELD_NAME
}
