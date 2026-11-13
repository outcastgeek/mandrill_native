//! Localized tabs: the localized form-tab contract, its builder surface and the
//! authoring/media helper state.
//!
//! The child owner provides:
//!
//! - the localized tab record: the normalized language value, the localized label,
//!   required/content/error state, the field key/value/format, the field identity and
//!   label, the panel and the after-helpers;
//! - the localized tabs props: the default tab, the localized aria/default-badge
//!   labels, the authoring and media-tools state, the caller's language normalizer and
//!   the tabs;
//! - the runtime: the request, the translation port, the language-name port, the
//!   authoring stream URL and the media-shortcode base URL;
//! - the tab-set builders for localized descriptions and localized content, the tab
//!   label/field label/placeholder policies, the format select helper and the media
//!   tools;
//! - the authoring state and signals declared by the authoring child module.
//!
//! Every optional value is genuinely optional: the runtime request, translator,
//! language-name port and normalizer may be absent, and absent callbacks yield the
//! owner's documented fallback (the trimmed language, the key itself, or the field's
//! default id). The owner opens no store and performs no I/O; the builder functions
//! are pure and are lifted by their callers.

use std::sync::Arc;

use effect_server::Request;
use maud::Markup;
use url::form_urlencoded;

use crate::attachments::{MediaAttachmentFormItem, MediaAttachmentManagerProps};
use crate::commands::InputBytes;
use crate::localization::{
    FormatNormalizer, FormatNormalizerRef, LocalizedContentDraft, LocalizedTextDraft, Normalizer, TranslateKeyFunc,
    normalize_authoring_language,
};
use crate::queries::first_non_blank;
use crate::validation::{ValidationIssueVM, trim_unicode_whitespace};

mod authoring;
mod render;
mod label_format;
use label_format::format_with_language_name;

pub use self::authoring::{
    LOCALIZED_TRANSLATION_WIDGET_KEY_FIELD_NAME, LocalizedAuthoringSignals,
    LocalizedTabsAuthoringOptions, LocalizedTabsAuthoringProps, localized_authoring_form_attributes,
    localized_authoring_form_attributes_for_widgets, localized_authoring_signals_for_widget,
    localized_field_attributes, localized_field_signal_name,
    localized_form_error_alert_class, localized_format_attributes,
    localized_format_signal_name, localized_has_content_signal_name,
    localized_normalize_language, localized_submit_control_attributes,
    new_localized_tabs_authoring_props, normalize_localized_signal_part,
    set_localized_authoring_availability,
};
pub use self::render::{
    localized_form_error_alert, localized_format_select, localized_live_submit_alerts,
    localized_live_submit_blocked_alert, localized_live_submit_error_alert,
    localized_media_insert_helper, localized_tab_error_message, localized_tabs,
    localized_text_area_panel,
};

/// The media shortcode toolbar help-text key.
pub const MEDIA_SHORTCODE_TOOLBAR_HELP_KEY: &str = "media.shortcode.toolbar_help";

/// Resolves a language's display name.
pub type LocalizedLanguageNameFunc =
    Arc<dyn Fn(&Request, &str) -> String + Send + Sync + 'static>;

/// The request-scoped localization runtime.
#[derive(Clone, Default)]
pub struct LocalizedTabsRuntime {
    /// The current request.
    pub request: Option<Request>,
    /// The default tab value.
    pub default_value: String,
    /// The translation port.
    pub translate_key: Option<TranslateKeyFunc<Request>>,
    /// The language display-name port.
    pub language_name: Option<LocalizedLanguageNameFunc>,
    /// The caller's language normalizer.
    pub normalize_language: Option<Normalizer>,
    /// The authoring stream URL.
    pub authoring_stream_url: String,
    /// The media shortcode base URL.
    pub media_shortcode_base_url: String,
}

impl std::fmt::Debug for LocalizedTabsRuntime {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("LocalizedTabsRuntime")
            .field("default_value", &self.default_value)
            .field("authoring_stream_url", &self.authoring_stream_url)
            .field("media_shortcode_base_url", &self.media_shortcode_base_url)
            .finish_non_exhaustive()
    }
}

/// Configures the authoring widget.
#[derive(Clone, Debug, Default)]
pub struct LocalizedTabsAuthoringConfig {
    /// The authoring scope.
    pub scope: String,
    /// The widget key.
    pub widget_key: String,
    /// The translation stream URL.
    pub stream_url: String,
    /// The auto-translate checkbox name.
    pub checkbox_name: String,
    /// Whether the checkbox is checked.
    pub checkbox_checked: bool,
    /// Whether translation is available.
    pub translation_available: bool,
}

/// Configures the media tools.
#[derive(Clone, Default)]
pub struct LocalizedTabsMediaToolsConfig {
    /// The request that overrides the runtime request.
    pub request: Option<Request>,
    /// The media shortcode base URL.
    pub shortcode_base_url: String,
    /// The localized field key.
    pub field_key: String,
    /// An explicit help text.
    pub help_text: String,
    /// The help-text key.
    pub help_text_key: String,
    /// The attachment manager props.
    pub manager: Option<Arc<MediaAttachmentManagerProps>>,
}

impl std::fmt::Debug for LocalizedTabsMediaToolsConfig {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("LocalizedTabsMediaToolsConfig")
            .field("shortcode_base_url", &self.shortcode_base_url)
            .field("field_key", &self.field_key)
            .field("help_text", &self.help_text)
            .field("help_text_key", &self.help_text_key)
            .field("manager", &self.manager.is_some())
            .finish_non_exhaustive()
    }
}

/// Configures the localized tabs props.
#[derive(Clone, Debug, Default)]
pub struct LocalizedTabsPropsOptions {
    /// The localization runtime.
    pub runtime: LocalizedTabsRuntime,
    /// The default tab value.
    pub default_value: String,
    /// The aria-label key.
    pub aria_label_key: String,
    /// The default-badge key.
    pub default_badge_text_key: String,
    /// The authoring configuration.
    pub authoring: Option<LocalizedTabsAuthoringConfig>,
    /// The media-tools configuration.
    pub media_tools: Option<LocalizedTabsMediaToolsConfig>,
    /// The tabs.
    pub tabs: Vec<LocalizedTab>,
}

/// How a tab label is produced.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct LocalizedTabLabelMode(String);

impl LocalizedTabLabelMode {
    /// The language name mode.
    pub const LANGUAGE_NAME: &'static str = "language_name";
    /// The translated-label mode.
    pub const TRANSLATED: &'static str = "translated";
    /// The translated-label-with-language-name mode.
    pub const TRANSLATED_WITH_LANGUAGE_NAME: &'static str = "translated_with_language_name";

    /// Admit a label mode.
    #[must_use]
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// The value as written.
    #[must_use]
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl From<&str> for LocalizedTabLabelMode {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

/// The language-name tab label mode.
pub const LOCALIZED_TAB_LABEL_LANGUAGE_NAME: &str = LocalizedTabLabelMode::LANGUAGE_NAME;
/// The translated tab label mode.
pub const LOCALIZED_TAB_LABEL_TRANSLATED: &str = LocalizedTabLabelMode::TRANSLATED;
/// The translated-with-language-name tab label mode.
pub const LOCALIZED_TAB_LABEL_TRANSLATED_WITH_LANGUAGE_NAME: &str =
    LocalizedTabLabelMode::TRANSLATED_WITH_LANGUAGE_NAME;

/// Configures one tab/field label.
#[derive(Clone, Debug, Default)]
pub struct LocalizedTabLabelOptions {
    /// The label mode.
    pub mode: LocalizedTabLabelMode,
    /// The label key.
    pub key: String,
}

/// Configures one localized text tab set.
#[derive(Clone, Default)]
pub struct LocalizedTextTabSetOptions {
    /// The localization runtime.
    pub runtime: LocalizedTabsRuntime,
    /// The localized field key.
    pub field_key: String,
    /// The widget key.
    pub widget_key: String,
    /// The text-area rows.
    pub rows: i64,
    /// The tab-label policy.
    pub tab_label: LocalizedTabLabelOptions,
    /// The field-label policy.
    pub field_label: LocalizedTabLabelOptions,
    /// The placeholder key.
    pub placeholder_key: String,
    /// The panel renderer.
    pub panel: Option<Arc<dyn Fn(&LocalizedTextAreaPanelVM) -> Markup + Send + Sync + 'static>>,
}

impl std::fmt::Debug for LocalizedTextTabSetOptions {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("LocalizedTextTabSetOptions")
            .field("field_key", &self.field_key)
            .field("widget_key", &self.widget_key)
            .field("rows", &self.rows)
            .field("tab_label", &self.tab_label)
            .field("field_label", &self.field_label)
            .field("placeholder_key", &self.placeholder_key)
            .finish_non_exhaustive()
    }
}

/// Configures one localized content tab set.
#[derive(Clone, Default)]
pub struct LocalizedContentTabSetOptions {
    /// The text tab options.
    pub text: LocalizedTextTabSetOptions,
    /// The content-format field key.
    pub format_field_key: String,
    /// The caller's content-format normalizer.
    pub normalize_format: Option<FormatNormalizer>,
    /// The format select configuration.
    pub format_select: Option<LocalizedFormatSelectConfig>,
}

impl std::fmt::Debug for LocalizedContentTabSetOptions {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("LocalizedContentTabSetOptions")
            .field("text", &self.text)
            .field("format_field_key", &self.format_field_key)
            .field("format_select", &self.format_select)
            .finish_non_exhaustive()
    }
}

/// One content-format option declaration.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct LocalizedFormatOptionDefinition {
    /// The option value.
    pub value: String,
    /// The option label key.
    pub label_key: String,
}

/// Configures the content-format select.
#[derive(Clone, Debug, Default)]
pub struct LocalizedFormatSelectConfig {
    /// The select label key.
    pub label_key: String,
    /// The select help-text key.
    pub help_text_key: String,
    /// The option declarations.
    pub options: Vec<LocalizedFormatOptionDefinition>,
}

/// Build the default authoring configuration.
#[must_use]
pub fn new_default_localized_tabs_authoring_config(
    scope: &str,
    widget_key: &str,
    checkbox_name: &str,
) -> LocalizedTabsAuthoringConfig {
    LocalizedTabsAuthoringConfig {
        scope: scope.to_owned(),
        widget_key: widget_key.to_owned(),
        stream_url: String::new(),
        checkbox_name: checkbox_name.to_owned(),
        checkbox_checked: true,
        translation_available: true,
    }
}

/// Build the default media-tools configuration.
#[must_use]
pub fn new_default_localized_tabs_media_tools_config(
    field_key: &str,
    manager: Option<Arc<MediaAttachmentManagerProps>>,
) -> LocalizedTabsMediaToolsConfig {
    LocalizedTabsMediaToolsConfig {
        request: None,
        shortcode_base_url: String::new(),
        field_key: field_key.to_owned(),
        help_text: String::new(),
        help_text_key: MEDIA_SHORTCODE_TOOLBAR_HELP_KEY.to_owned(),
        manager,
    }
}

/// The language-name tab label policy.
#[must_use]
pub fn language_name_tab_label() -> LocalizedTabLabelOptions {
    LocalizedTabLabelOptions {
        mode: LocalizedTabLabelMode::new(LOCALIZED_TAB_LABEL_LANGUAGE_NAME),
        key: String::new(),
    }
}

/// The translated tab label policy.
#[must_use]
pub fn translated_tab_label(key: &str) -> LocalizedTabLabelOptions {
    LocalizedTabLabelOptions {
        mode: LocalizedTabLabelMode::new(LOCALIZED_TAB_LABEL_TRANSLATED),
        key: key.to_owned(),
    }
}

/// The translated-with-language-name tab label policy.
#[must_use]
pub fn translated_language_name_tab_label(key: &str) -> LocalizedTabLabelOptions {
    LocalizedTabLabelOptions {
        mode: LocalizedTabLabelMode::new(LOCALIZED_TAB_LABEL_TRANSLATED_WITH_LANGUAGE_NAME),
        key: key.to_owned(),
    }
}

/// Build the localized tabs props.
#[must_use]
pub fn new_localized_tabs_props(
    request: &Request,
    options: LocalizedTabsPropsOptions,
) -> LocalizedTabsProps {
    let runtime = options.runtime;
    let mut props = LocalizedTabsProps {
        default_value: first_non_blank(&[options.default_value.clone(), runtime.default_value.clone()]),
        aria_label: translate_runtime_key(request, &runtime, options.aria_label_key.as_str()),
        default_badge_text: translate_runtime_key(
            request,
            &runtime,
            options.default_badge_text_key.as_str(),
        ),
        normalize_language: runtime.normalize_language.clone(),
        tabs: options.tabs,
        ..LocalizedTabsProps::default()
    };
    if let Some(authoring) = options.authoring {
        props.authoring = Some(Arc::new(new_localized_tabs_authoring_props(
            request,
            LocalizedTabsAuthoringOptions {
                scope: authoring.scope,
                widget_key: authoring.widget_key,
                stream_url: first_non_blank(&[
                    authoring.stream_url,
                    runtime.authoring_stream_url.clone(),
                ]),
                checkbox_name: authoring.checkbox_name,
                checkbox_checked: authoring.checkbox_checked,
                translation_available: authoring.translation_available,
                text: runtime.translate_key.clone(),
            },
        )));
    }
    if let Some(media_tools) = options.media_tools {
        props.media_tools = Some(Arc::new(new_localized_tabs_media_tools_props(
            request,
            &runtime,
            media_tools,
        )));
    }
    props
}

/// Build the localized text tab set from text drafts.
#[must_use]
pub fn build_localized_text_tab_set(
    request: &Request,
    drafts: &[LocalizedTextDraft],
    options: &LocalizedTextTabSetOptions,
) -> Vec<LocalizedTab> {
    build_localized_text_tabs(request, drafts, &localized_text_tabs_options(request, options))
}

/// Build the localized description tab set from content drafts.
#[must_use]
pub fn build_localized_content_description_tab_set(
    request: &Request,
    drafts: &[LocalizedContentDraft],
    options: &LocalizedTextTabSetOptions,
) -> Vec<LocalizedTab> {
    build_localized_content_description_tabs(
        request,
        drafts,
        &localized_text_tabs_options(request, options),
    )
}

/// Build the localized content tab set from content drafts.
#[must_use]
pub fn build_localized_content_tab_set(
    request: &Request,
    drafts: &[LocalizedContentDraft],
    options: &LocalizedContentTabSetOptions,
) -> Vec<LocalizedTab> {
    build_localized_content_tabs(
        request,
        drafts,
        &LocalizedContentTabsOptions {
            text: localized_text_tabs_options(request, &options.text),
            format_field_name: Some(localized_field_name_func(
                options.format_field_key.as_str(),
                options.text.runtime.normalize_language.clone(),
            )),
            format_field_id: Some(localized_field_id_func(
                options.format_field_key.as_str(),
                options.text.runtime.normalize_language.clone(),
            )),
            normalize_format: options.normalize_format.clone(),
            after_helpers: new_localized_format_select_helper(
                request,
                &options.text.runtime,
                options.format_select.as_ref(),
            ),
        },
    )
}

/// Build a localized field-name function.
#[must_use]
pub fn localized_field_name_func(
    field_key: &str,
    normalize: Option<Normalizer>,
) -> Arc<dyn Fn(&str) -> String + Send + Sync + 'static> {
    let field_key = field_key.to_owned();
    Arc::new(move |lang: &str| {
        crate::localization::localized_field_name(
            field_key.as_str(),
            lang,
            normalize.as_deref(),
        )
    })
}

/// Build a localized field-id function.
#[must_use]
pub fn localized_field_id_func(
    field_key: &str,
    normalize: Option<Normalizer>,
) -> Arc<dyn Fn(&str) -> String + Send + Sync + 'static> {
    let field_key = field_key.trim().to_owned();
    Arc::new(move |lang: &str| {
        if field_key.is_empty() {
            return String::new();
        }
        format!(
            "{field_key}-{}",
            crate::localization::localized_field_suffix(lang, normalize.as_deref())
        )
    })
}

/// Build the format-select helper.
#[must_use]
pub fn new_localized_format_select_helper(
    request: &Request,
    runtime: &LocalizedTabsRuntime,
    config: Option<&LocalizedFormatSelectConfig>,
) -> Option<Arc<dyn Fn(&LocalizedFormatSelectVM) -> Markup + Send + Sync + 'static>> {
    let config = config?;
    let options = localized_format_options(request, runtime, &config.options);
    let label = translate_runtime_key(request, runtime, config.label_key.as_str());
    let help_text = translate_runtime_key(request, runtime, config.help_text_key.as_str());
    Some(Arc::new(move |input: &LocalizedFormatSelectVM| {
        localized_format_select(&LocalizedFormatSelectProps {
            input: input.clone(),
            options: options.clone(),
            label: label.clone(),
            help_text: help_text.clone(),
        })
    }))
}

/// Resolve the localized format options.
#[must_use]
pub fn localized_format_options(
    request: &Request,
    runtime: &LocalizedTabsRuntime,
    definitions: &[LocalizedFormatOptionDefinition],
) -> Vec<LocalizedFormatOption> {
    definitions
        .iter()
        .filter_map(|definition| {
            let value = definition.value.trim();
            if value.is_empty() {
                return None;
            }
            Some(LocalizedFormatOption {
                value: value.to_owned(),
                label: translate_runtime_key(request, runtime, definition.label_key.as_str()),
                disabled: false,
            })
        })
        .collect()
}

fn localized_text_tabs_options(
    request: &Request,
    options: &LocalizedTextTabSetOptions,
) -> LocalizedTextTabsOptions {
    let panel = options
        .panel
        .clone()
        .unwrap_or_else(|| Arc::new(localized_text_area_panel));
    LocalizedTextTabsOptions {
        field_key: options.field_key.clone(),
        widget_key: options.widget_key.clone(),
        rows: options.rows,
        normalize_language: options.runtime.normalize_language.clone(),
        field_name: Some(localized_field_name_func(
            options.field_key.as_str(),
            options.runtime.normalize_language.clone(),
        )),
        field_id: Some(localized_field_id_func(
            options.field_key.as_str(),
            options.runtime.normalize_language.clone(),
        )),
        tab_label: Some(localized_tab_label_func(
            request,
            &options.runtime,
            &options.tab_label,
        )),
        field_label: Some(localized_tab_label_func(
            request,
            &options.runtime,
            &options.field_label,
        )),
        placeholder: Some(localized_placeholder_func(
            request,
            &options.runtime,
            options.placeholder_key.as_str(),
        )),
        panel: Some(panel),
    }
}

fn new_localized_tabs_media_tools_props(
    request: &Request,
    runtime: &LocalizedTabsRuntime,
    config: LocalizedTabsMediaToolsConfig,
) -> LocalizedMediaToolsProps {
    let media_request = config.request.unwrap_or_else(|| request.clone());
    new_localized_media_tools_props(LocalizedMediaToolsOptions {
        request: Some(media_request.clone()),
        shortcode_base_url: first_non_blank(&[
            config.shortcode_base_url,
            runtime.media_shortcode_base_url.clone(),
        ]),
        owner_form_id: InputBytes::default(),
        attachment_id: InputBytes::default(),
        field_key: config.field_key,
        help_text: first_non_blank(&[
            config.help_text,
            translate_runtime_key(&media_request, runtime, config.help_text_key.as_str()),
        ]),
        label: String::new(),
        manager: config.manager,
    })
}

fn localized_tab_label_func(
    _request: &Request,
    runtime: &LocalizedTabsRuntime,
    options: &LocalizedTabLabelOptions,
) -> Arc<dyn Fn(&Request, &str) -> String + Send + Sync + 'static> {
    let runtime = runtime.clone();
    let options = options.clone();
    Arc::new(move |request: &Request, lang: &str| match options.mode.as_str() {
        LOCALIZED_TAB_LABEL_TRANSLATED => translate_runtime_key(request, &runtime, options.key.as_str()),
        LOCALIZED_TAB_LABEL_TRANSLATED_WITH_LANGUAGE_NAME => {
            let label = translate_runtime_key(request, &runtime, options.key.as_str());
            if label.is_empty() {
                return localized_runtime_language_name(request, &runtime, lang);
            }
            format_with_language_name(label.as_str(), localized_runtime_language_name(request, &runtime, lang).as_str())
        }
        _ => localized_runtime_language_name(request, &runtime, lang),
    })
}

fn localized_placeholder_func(
    _request: &Request,
    runtime: &LocalizedTabsRuntime,
    key: &str,
) -> Arc<dyn Fn(&Request) -> String + Send + Sync + 'static> {
    let runtime = runtime.clone();
    let key = key.to_owned();
    Arc::new(move |request: &Request| translate_runtime_key(request, &runtime, key.as_str()))
}

fn localized_runtime_language_name(
    request: &Request,
    runtime: &LocalizedTabsRuntime,
    lang: &str,
) -> String {
    if let Some(language_name) = runtime.language_name.as_ref() {
        let name = language_name(request, lang).trim().to_owned();
        if !name.is_empty() {
            return name;
        }
    }
    lang.trim().to_owned()
}

fn translate_runtime_key(request: &Request, runtime: &LocalizedTabsRuntime, key: &str) -> String {
    let key = key.trim();
    if key.is_empty() {
        return String::new();
    }
    match runtime.translate_key.as_ref() {
        Some(translate) => translate(request, key).trim().to_owned(),
        None => key.to_owned(),
    }
}

/// One localized text-area panel's inputs.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct LocalizedTextAreaPanelVM {
    /// The original draft language.
    pub lang: String,
    /// The submitted field name.
    pub name: String,
    /// The input id.
    pub id: String,
    /// The localized label.
    pub label: String,
    /// The localized placeholder.
    pub placeholder: String,
    /// The value.
    pub value: InputBytes,
    /// Whether the field is required.
    pub required: bool,
    /// The text-area rows.
    pub rows: i64,
    /// The widget key.
    pub widget_key: String,
    /// The localized field key.
    pub field_key: String,
}

/// One content-format select's inputs.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct LocalizedFormatSelectVM {
    /// The normalized language.
    pub lang: String,
    /// The submitted field name.
    pub name: String,
    /// The input id.
    pub id: String,
    /// The selected format.
    pub value: InputBytes,
    /// Whether the field is required.
    pub required: bool,
    /// The widget key.
    pub widget_key: String,
    /// The localized field key.
    pub field_key: String,
}

/// The localized text-tabs builder options.
#[derive(Clone, Default)]
pub struct LocalizedTextTabsOptions {
    /// The localized field key.
    pub field_key: String,
    /// The widget key.
    pub widget_key: String,
    /// The text-area rows.
    pub rows: i64,
    /// The caller's language normalizer.
    pub normalize_language: Option<Normalizer>,
    /// The field-name function.
    pub field_name: Option<Arc<dyn Fn(&str) -> String + Send + Sync + 'static>>,
    /// The field-id function.
    pub field_id: Option<Arc<dyn Fn(&str) -> String + Send + Sync + 'static>>,
    /// The tab-label function.
    pub tab_label: Option<Arc<dyn Fn(&Request, &str) -> String + Send + Sync + 'static>>,
    /// The field-label function.
    pub field_label: Option<Arc<dyn Fn(&Request, &str) -> String + Send + Sync + 'static>>,
    /// The placeholder function.
    pub placeholder: Option<Arc<dyn Fn(&Request) -> String + Send + Sync + 'static>>,
    /// The panel renderer.
    pub panel: Option<Arc<dyn Fn(&LocalizedTextAreaPanelVM) -> Markup + Send + Sync + 'static>>,
}

impl std::fmt::Debug for LocalizedTextTabsOptions {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("LocalizedTextTabsOptions")
            .field("field_key", &self.field_key)
            .field("widget_key", &self.widget_key)
            .field("rows", &self.rows)
            .finish_non_exhaustive()
    }
}

/// The localized content-tabs builder options.
#[derive(Clone, Default)]
pub struct LocalizedContentTabsOptions {
    /// The text tab options.
    pub text: LocalizedTextTabsOptions,
    /// The format field-name function.
    pub format_field_name: Option<Arc<dyn Fn(&str) -> String + Send + Sync + 'static>>,
    /// The format field-id function.
    pub format_field_id: Option<Arc<dyn Fn(&str) -> String + Send + Sync + 'static>>,
    /// The caller's content-format normalizer.
    pub normalize_format: Option<FormatNormalizer>,
    /// The after-helpers renderer.
    pub after_helpers:
        Option<Arc<dyn Fn(&LocalizedFormatSelectVM) -> Markup + Send + Sync + 'static>>,
}

impl std::fmt::Debug for LocalizedContentTabsOptions {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("LocalizedContentTabsOptions")
            .field("text", &self.text)
            .finish_non_exhaustive()
    }
}

/// Build localized text tabs.
#[must_use]
pub fn build_localized_text_tabs(
    request: &Request,
    drafts: &[LocalizedTextDraft],
    options: &LocalizedTextTabsOptions,
) -> Vec<LocalizedTab> {
    build_localized_description_tabs(request, drafts, options, |draft: &LocalizedTextDraft| {
        (draft.lang.clone(), draft.description.clone(), draft.required)
    })
}

/// Build localized description tabs from content drafts.
#[must_use]
pub fn build_localized_content_description_tabs(
    request: &Request,
    drafts: &[LocalizedContentDraft],
    options: &LocalizedTextTabsOptions,
) -> Vec<LocalizedTab> {
    build_localized_description_tabs(request, drafts, options, |draft: &LocalizedContentDraft| {
        (draft.lang.clone(), draft.description.clone(), draft.required)
    })
}

fn build_localized_description_tabs<T>(
    request: &Request,
    drafts: &[T],
    options: &LocalizedTextTabsOptions,
    project: impl Fn(&T) -> (String, InputBytes, bool),
) -> Vec<LocalizedTab> {
    let mut tabs = Vec::with_capacity(drafts.len());
    for value in drafts {
        let (draft_lang, description, required) = project(value);
        let lang = normalize_authoring_language(draft_lang.as_str(), options.normalize_language.as_deref());
        let field_id = localized_text_field_id(lang.as_str(), options);
        let field_label = localized_field_label(request, lang.as_str(), options);
        let label = localized_tab_label(request, lang.as_str(), options);
        let panel = localized_text_panel(
            request,
            draft_lang.as_str(),
            description.as_bytes(),
            required,
            field_id.as_str(),
            field_label.as_str(),
            options,
        );
        tabs.push(LocalizedTab {
            value: lang.clone(),
            label,
            required,
            has_content: !trim_unicode_whitespace(description.as_bytes()).is_empty(),
            error: false,
            error_message: String::new(),
            errors: Vec::new(),
            field_key: options.field_key.trim().to_owned(),
            field_value: description.clone(),
            format_value: InputBytes::default(),
            field_id,
            field_label,
            panel,
            after_helpers: None,
        });
    }
    tabs
}

/// Build localized content tabs from content drafts.
#[must_use]
pub fn build_localized_content_tabs(
    request: &Request,
    drafts: &[LocalizedContentDraft],
    options: &LocalizedContentTabsOptions,
) -> Vec<LocalizedTab> {
    let mut tabs = Vec::with_capacity(drafts.len());
    for draft in drafts {
        let lang = normalize_authoring_language(
            draft.lang.as_str(),
            options.text.normalize_language.as_deref(),
        );
        let field_id = localized_text_field_id(lang.as_str(), &options.text);
        let field_label = localized_field_label(request, lang.as_str(), &options.text);
        let format_value = normalize_localized_format(
            draft.content_format.as_bytes(),
            options.normalize_format.as_deref(),
        );
        let label = localized_tab_label(request, lang.as_str(), &options.text);
        let panel = localized_text_panel(
            request,
            draft.lang.as_str(),
            draft.content.as_bytes(),
            draft.required,
            field_id.as_str(),
            field_label.as_str(),
            &options.text,
        );
        tabs.push(LocalizedTab {
            value: lang.clone(),
            label,
            required: draft.required,
            has_content: !trim_unicode_whitespace(draft.content.as_bytes()).is_empty(),
            error: false,
            error_message: String::new(),
            errors: Vec::new(),
            field_key: options.text.field_key.trim().to_owned(),
            field_value: draft.content.clone(),
            format_value: format_value.clone(),
            field_id,
            field_label,
            panel,
            after_helpers: localized_format_helper(
                draft.lang.as_str(),
                format_value.as_bytes(),
                draft.required,
                options,
            ),
        });
    }
    tabs
}

fn localized_text_panel(
    request: &Request,
    lang: &str,
    value: &[u8],
    required: bool,
    field_id: &str,
    field_label: &str,
    options: &LocalizedTextTabsOptions,
) -> Option<Markup> {
    let panel = options.panel.as_ref()?;
    let name = options
        .field_name
        .as_ref()
        .map(|field_name| field_name(lang))
        .unwrap_or_default();
    let placeholder = options
        .placeholder
        .as_ref()
        .map(|placeholder| placeholder(request))
        .unwrap_or_default();
    Some(panel(&LocalizedTextAreaPanelVM {
        lang: lang.to_owned(),
        name,
        id: field_id.to_owned(),
        label: field_label.to_owned(),
        placeholder,
        value: InputBytes::from(value),
        required,
        rows: options.rows,
        widget_key: options.widget_key.trim().to_owned(),
        field_key: options.field_key.trim().to_owned(),
    }))
}

fn localized_format_helper(
    lang: &str,
    value: &[u8],
    required: bool,
    options: &LocalizedContentTabsOptions,
) -> Option<Markup> {
    let after_helpers = options.after_helpers.as_ref()?;
    let name = options
        .format_field_name
        .as_ref()
        .map(|field_name| field_name(lang))
        .unwrap_or_default();
    let id = options
        .format_field_id
        .as_ref()
        .map(|field_id| field_id(lang))
        .unwrap_or_default();
    Some(after_helpers(&LocalizedFormatSelectVM {
        lang: lang.to_owned(),
        name,
        id,
        value: InputBytes::from(value),
        required,
        widget_key: options.text.widget_key.trim().to_owned(),
        field_key: options.text.field_key.trim().to_owned(),
    }))
}

fn localized_tab_label(
    request: &Request,
    lang: &str,
    options: &LocalizedTextTabsOptions,
) -> String {
    match options.tab_label.as_ref() {
        Some(tab_label) => tab_label(request, lang).trim().to_owned(),
        None => lang.to_owned(),
    }
}

fn localized_field_label(
    request: &Request,
    lang: &str,
    options: &LocalizedTextTabsOptions,
) -> String {
    match options.field_label.as_ref() {
        Some(field_label) => field_label(request, lang).trim().to_owned(),
        None => localized_tab_label(request, lang, options),
    }
}

fn localized_text_field_id(lang: &str, options: &LocalizedTextTabsOptions) -> String {
    match options.field_id.as_ref() {
        Some(field_id) => field_id(lang).trim().to_owned(),
        None => String::new(),
    }
}

fn normalize_localized_format(
    format: &[u8],
    normalize: Option<FormatNormalizerRef<'_>>,
) -> InputBytes {
    match normalize {
        Some(normalize) => {
            let normalized = normalize(format);
            InputBytes::from(trim_unicode_whitespace(normalized.as_bytes()))
        }
        None => InputBytes::from(trim_unicode_whitespace(format)),
    }
}

/// One rendered content-format option.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct LocalizedFormatOption {
    /// The localized label.
    pub label: String,
    /// The option value.
    pub value: String,
    /// Whether the option is disabled.
    pub disabled: bool,
}

/// The content-format select props.
#[derive(Clone, Debug, Default)]
pub struct LocalizedFormatSelectProps {
    /// The select inputs.
    pub input: LocalizedFormatSelectVM,
    /// The options.
    pub options: Vec<LocalizedFormatOption>,
    /// The localized label.
    pub label: String,
    /// The localized help text.
    pub help_text: String,
}

/// Configures the media tools.
#[derive(Clone, Default)]
pub struct LocalizedMediaToolsOptions {
    /// The request; absent is the nil-request contract.
    pub request: Option<Request>,
    /// The media shortcode base URL.
    pub shortcode_base_url: String,
    /// The owner form id.
    pub owner_form_id: InputBytes,
    /// The attachment id.
    pub attachment_id: InputBytes,
    /// The localized field key.
    pub field_key: String,
    /// The help text.
    pub help_text: String,
    /// The label.
    pub label: String,
    /// The attachment manager props.
    pub manager: Option<Arc<MediaAttachmentManagerProps>>,
}

impl std::fmt::Debug for LocalizedMediaToolsOptions {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("LocalizedMediaToolsOptions")
            .field("shortcode_base_url", &self.shortcode_base_url)
            .field("owner_form_id", &self.owner_form_id)
            .field("attachment_id", &self.attachment_id)
            .field("field_key", &self.field_key)
            .field("help_text", &self.help_text)
            .field("label", &self.label)
            .field("manager", &self.manager.is_some())
            .finish_non_exhaustive()
    }
}

/// Build the media-tools props.
#[must_use]
pub fn new_localized_media_tools_props(
    options: LocalizedMediaToolsOptions,
) -> LocalizedMediaToolsProps {
    let manager = options.manager.as_ref();
    let mut owner_form_id = options.owner_form_id.trim().to_owned();
    let mut attachment_id = options.attachment_id.trim().to_owned();
    if let Some(manager) = manager {
        if owner_form_id.is_empty() { owner_form_id = manager.mounts.owner_form_id.trim(); }
        if attachment_id.is_empty() { attachment_id = manager.mounts.section_id.trim(); }
    }
    let mut props = LocalizedMediaToolsProps {
        request: options.request,
        scope: InputBytes::default(),
        dialog_id: InputBytes::default(),
        root_id: InputBytes::default(),
        owner_form_id,
        attachment_id,
        field_key: options.field_key.trim().to_owned(),
        help_text: options.help_text.trim().to_owned(),
        label: options.label.trim().to_owned(),
        open_url: options.shortcode_base_url.trim().to_owned(),
        attachments: Vec::new(),
    };
    if let Some(manager) = manager {
        props.scope = manager.scope.trim().to_owned();
        props.dialog_id = manager.mounts.shortcode_dialog_id.trim().to_owned();
        props.root_id = manager.mounts.shortcode_root_id.trim().to_owned();
        props.attachments = manager.attachments.clone();
    }
    props
}

/// Build the media-insert URL for one tab and field label.
#[must_use]
pub fn localized_media_open_url(
    props: &LocalizedMediaToolsProps,
    tab: &LocalizedTab,
    field_label: &str,
) -> String {
    if props.open_url.is_empty()
        || props.root_id.is_empty()
        || props.dialog_id.is_empty()
        || props.owner_form_id.is_empty()
        || props.attachment_id.is_empty()
        || props.field_key.is_empty()
        || tab.field_id.is_empty()
    {
        return String::new();
    }
    let mut params: Vec<(&str, &[u8])> = vec![
        ("root_id", props.root_id.as_bytes()),
        ("dialog_id", props.dialog_id.as_bytes()),
        ("scope", props.scope.as_bytes()),
        ("owner_form_id", props.owner_form_id.as_bytes()),
        ("attachment_id", props.attachment_id.as_bytes()),
        ("field_key", props.field_key.as_bytes()),
        ("field_id", tab.field_id.as_bytes()),
        ("field_label", field_label.as_bytes()),
    ];
    params.sort_by(|left, right| left.0.cmp(right.0));
    let query = params.into_iter().map(|(name, value)| {
        format!("{}={}", form_urlencoded::byte_serialize(name.trim().as_bytes()).collect::<String>(), form_urlencoded::byte_serialize(crate::validation::trim_unicode_whitespace(value)).collect::<String>())
    }).collect::<Vec<_>>().join("&");
    append_query_string(props.open_url.as_str(), &query)

}

/// Whether the props carry a renderable media-tools mount.
#[must_use]
pub fn localized_tabs_has_media_tools(props: &LocalizedTabsProps) -> bool {
    props.media_tools.as_ref().is_some_and(|media_tools| {
        !media_tools.root_id.trim().is_empty()
            && !media_tools.owner_form_id.trim().is_empty()
            && !media_tools.attachment_id.trim().is_empty()
    })
}

/// The localized editor text-area class.
#[must_use]
pub fn localized_editor_text_area_class() -> String {
    "localized-editor-textarea antialiased".to_owned()
}

fn append_query_string(base: &str, query: &str) -> String {
    let base = base.trim();
    if base.is_empty() {
        return String::new();
    }
    if query.is_empty() {
        return base.to_owned();
    }
    let separator = if base.contains('?') { "&" } else { "?" };
    format!("{base}{separator}{query}")
}

/// One rendered localized field tab.
#[derive(Clone, Debug, Default)]
pub struct LocalizedTab {
    /// The normalized language value.
    pub value: String,
    /// The localized tab label.
    pub label: String,
    /// Whether the language is required.
    pub required: bool,
    /// Whether the draft has content.
    pub has_content: bool,
    /// Whether the tab carries issues.
    pub error: bool,
    /// The tab-level message.
    pub error_message: String,
    /// The tab's localized issues.
    pub errors: Vec<ValidationIssueVM>,
    /// The localized field key.
    pub field_key: String,
    /// The field value.
    pub field_value: InputBytes,
    /// The content format.
    pub format_value: InputBytes,
    /// The field/input id.
    pub field_id: String,
    /// The field label.
    pub field_label: String,
    /// The panel component.
    pub panel: Option<Markup>,
    /// The after-helpers component.
    pub after_helpers: Option<Markup>,
}

/// Configures the localized tabs widget.
#[derive(Clone, Default)]
pub struct LocalizedTabsProps {
    /// The default tab value.
    pub default_value: String,
    /// The localized aria label.
    pub aria_label: String,
    /// The localized default-badge text.
    pub default_badge_text: String,
    /// The panel class.
    pub class: String,
    /// The tab-list class.
    pub list_class: String,
    /// The panel-card class.
    pub panel_card_class: String,
    /// The panel-content class.
    pub panel_content_class: String,
    /// The authoring state.
    pub authoring: Option<Arc<LocalizedTabsAuthoringProps>>,
    /// The media-tools state.
    pub media_tools: Option<Arc<LocalizedMediaToolsProps>>,
    /// The caller's language normalizer.
    pub normalize_language: Option<Normalizer>,
    /// The tabs.
    pub tabs: Vec<LocalizedTab>,
}

impl std::fmt::Debug for LocalizedTabsProps {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("LocalizedTabsProps")
            .field("default_value", &self.default_value)
            .field("aria_label", &self.aria_label)
            .field("tabs", &self.tabs.len())
            .field("has_authoring", &self.authoring.is_some())
            .field("has_media_tools", &self.media_tools.is_some())
            .finish_non_exhaustive()
    }
}

/// The media tools inside localized tabs.
#[derive(Clone, Default)]
pub struct LocalizedMediaToolsProps {
    /// The request.
    pub request: Option<Request>,
    /// The media scope.
    pub scope: InputBytes,
    /// The shortcode dialog id.
    pub dialog_id: InputBytes,
    /// The shortcode root id.
    pub root_id: InputBytes,
    /// The owner form id.
    pub owner_form_id: InputBytes,
    /// The attachment section id.
    pub attachment_id: InputBytes,
    /// The localized field key.
    pub field_key: String,
    /// The help text.
    pub help_text: String,
    /// The label.
    pub label: String,
    /// The media-open URL.
    pub open_url: String,
    /// The presented attachment rows.
    pub attachments: Vec<MediaAttachmentFormItem>,
}

impl std::fmt::Debug for LocalizedMediaToolsProps {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("LocalizedMediaToolsProps")
            .field("scope", &self.scope)
            .field("dialog_id", &self.dialog_id)
            .field("root_id", &self.root_id)
            .field("owner_form_id", &self.owner_form_id)
            .field("attachment_id", &self.attachment_id)
            .field("field_key", &self.field_key)
            .field("help_text", &self.help_text)
            .field("label", &self.label)
            .field("open_url", &self.open_url)
            .field("attachments", &self.attachments.len())
            .finish_non_exhaustive()
    }
}

pub(crate) fn media_insert_action(media: &LocalizedMediaToolsProps) -> String {
    media.open_url.clone()
}
