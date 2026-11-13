//! Maud rendering for the localized tabs widget, its alerts and helper panels.
//!
//! Every element, class and data attribute follows the widget's declared contract:
//! the tabs root and tab list carry the declared class and Datastar defaults; each
//! tab trigger carries the tab value, click action, selected/state/tabindex
//! attributes and the tab/panel relationship when a field id exists; each panel
//! carries the panel relationship and the helper stack (authoring panel, media
//! insert helper, after-helpers, error message).
//!
//! The authoring and media state is read from the props; the four localized alert
//! titles and the required/translate-now labels come from the caller's translation
//! port, with the key itself as the absent-port fallback.

use effect_server::Request;
use maud::{Markup, html};

use web_components::components::shared;
use web_components::components::unstyled::alert::AlertProps;
use web_components::components::unstyled::button::ButtonProps;
use web_components::components::unstyled::checkbox::CheckboxProps;
use web_components::components::unstyled::icons::{AlertTriangle, IconProps};
use web_components::components::unstyled::input::{INPUT_TYPE_TEXT, InputProps, label_classes};
use web_components::components::unstyled::select::SelectProps;
use web_components::components::unstyled::tabs::{
    TabListProps, TabsProps, TAB_LIST_VARIANT_UNDERLINE, buildTabClickExpr, buildTabSelectedExpr,
    buildTabStateExpr, buildTabTabIndexExpr,
};

use crate::localization::TranslateKeyFunc;
use crate::queries::first_non_blank;
use crate::routing::translate;
use crate::validation::VALIDATION_SUMMARY_TITLE_KEY;

use super::authoring::{
    localized_authoring_prepare_run_action_for, localized_authoring_run_action_for,
    localized_authoring_toggle_action_for, localized_tab_field_label,
    localized_tab_indicator_class_expr, localized_tab_indicator_state_expr,
    localized_tab_indicator_title_expr, localized_tab_panel_attributes,
    localized_tab_required_value, localized_tab_trigger_attributes, localized_tabs_default_value,
    localized_tabs_has_authoring, localized_tabs_root_attributes,
    localized_translation_field_format_field_name, localized_translation_field_name_field_name,
    localized_translation_field_value_field_name, localized_translation_scope_field_name,
    localized_translation_source_language_field_name,
    localized_translation_target_language_field_name, localized_transport_disabled_expr_for,
    localized_transport_inactive_expr_for, quoted_js,
};
use super::{
    LocalizedFormatSelectProps, LocalizedMediaToolsProps, LocalizedTab, LocalizedTabsProps,
    LocalizedTextAreaPanelVM, localized_editor_text_area_class, localized_tabs_has_media_tools,
    media_insert_action,
};

fn presentation_text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// Render the localized tabs widget.
#[must_use]
pub fn localized_tabs(
    request: &Request,
    translate_key: Option<&TranslateKeyFunc<Request>>,
    props: &LocalizedTabsProps,
) -> Markup {
    let root_attributes = localized_tabs_root_attributes(props);
    let tabs_props = TabsProps::default()
        .with_default_value(localized_tabs_default_value(props))
        .with_class(shared::build_classes(&[
            "w-full overflow-hidden rounded-[1rem] border border-border bg-card shadow-sm focus-within:border-primary focus-within:ring-1 focus-within:ring-primary transition-shadow",
            props.class.as_str(),
        ]));
    let tabs_attributes = tabs_props.build_attributes();
    let list_props = TabListProps::default()
        .with_variant(TAB_LIST_VARIANT_UNDERLINE)
        .with_aria_label(props.aria_label.as_str())
        .with_class(shared::build_classes(&[
            "flex items-center gap-1 border-b border-border bg-muted/15 px-2 py-1.5 overflow-x-auto [scrollbar-width:none] [&::-webkit-scrollbar]:hidden",
            props.list_class.as_str(),
        ]));
    let trigger_base_class = "group relative inline-flex items-center justify-center whitespace-nowrap rounded-md px-3 py-1.5 text-xs font-semibold uppercase tracking-wider text-muted-foreground ring-offset-background transition-all hover:bg-muted hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 disabled:pointer-events-none disabled:opacity-50 data-[state=active]:bg-background data-[state=active]:text-foreground data-[state=active]:shadow-sm data-[state=active]:ring-1 data-[state=active]:ring-border/50";
    let aria_label = if props.aria_label.is_empty() {
        translate(translate_key, request, "component.tabs.aria_label")
    } else { props.aria_label.clone() };
    html! {
        div
            data-localized-widget=[root_attributes.get("data-localized-widget").map(String::as_str)]
            data-localized-widget-key=[root_attributes.get("data-localized-widget-key").map(String::as_str)]
            data-translation-url=[root_attributes.get("data-translation-url").map(String::as_str)]
            data-translation-scope=[root_attributes.get("data-translation-scope").map(String::as_str)]
            data-status-idle=[root_attributes.get("data-status-idle").map(String::as_str)]
            data-status-off=[root_attributes.get("data-status-off").map(String::as_str)]
            data-status-working=[root_attributes.get("data-status-working").map(String::as_str)]
            data-status-done=[root_attributes.get("data-status-done").map(String::as_str)]
            data-status-error=[root_attributes.get("data-status-error").map(String::as_str)]
            data-status-unavailable=[root_attributes.get("data-status-unavailable").map(String::as_str)]
            data-translation-available=[root_attributes.get("data-translation-available").map(String::as_str)]
            data-tab-status-missing=[root_attributes.get("data-tab-status-missing").map(String::as_str)]
            data-tab-status-complete=[root_attributes.get("data-tab-status-complete").map(String::as_str)]
            data-tab-status-empty=[root_attributes.get("data-tab-status-empty").map(String::as_str)]
            data-attr:data-localized-auto-translate=[root_attributes.get("data-attr:data-localized-auto-translate").map(String::as_str)]
            data-attr:data-localized-source-language=[root_attributes.get("data-attr:data-localized-source-language").map(String::as_str)]
            data-attr:data-localized-translating=[root_attributes.get("data-attr:data-localized-translating").map(String::as_str)]
            data-attr:data-localized-translation-available=[root_attributes.get("data-attr:data-localized-translation-available").map(String::as_str)]
            data-attr:aria-busy=[root_attributes.get("data-attr:aria-busy").map(String::as_str)]
        {
            div
            class=(tabs_props.build_classes())
            data-component="tabs"
            data-default-value=(tabs_attributes.get("data-default-value").map_or("", String::as_str))
            data-signals=(tabs_attributes.get("data-signals").map_or("", String::as_str))
            data-orientation=(tabs_attributes.get("data-orientation").map_or("horizontal", String::as_str))
            {
            div role="tablist" class=(list_props.build_classes()) aria-label=(aria_label) {
                @for tab in &props.tabs {
                    @let trigger_attributes = localized_tab_trigger_attributes(tab);
                    button
                        type="button"
                        role="tab"
                        class=(shared::build_classes(&[trigger_base_class]))
                        data-value=(tab.value)
                        data-on:click=(buildTabClickExpr(tab.value.as_str()))
                        data-attr:data-state=(buildTabStateExpr(tab.value.as_str()))
                        data-attr:aria-selected=(buildTabSelectedExpr(tab.value.as_str()))
                        data-attr:tabindex=(buildTabTabIndexExpr(tab.value.as_str()))
                        id=[trigger_attributes.get("id").map(String::as_str)]
                        aria-controls=[trigger_attributes.get("aria-controls").map(String::as_str)]
                    {
                        span class="flex items-center gap-1.5" {
                            @let indicator_class = localized_tab_indicator_class_expr(props, tab);
                            @let indicator_title = localized_tab_indicator_title_expr(props, tab);
                            @let indicator_state = localized_tab_indicator_state_expr(props, tab);
                            @if tab.error {
                                span
                                    class="h-1.5 w-1.5 rounded-full flex-shrink-0 bg-destructive shadow-sm"
                                    data-localized-tab-indicator="true"
                                    data-localized-lang=(tab.value)
                                    data-localized-tab-required=(localized_tab_required_value(tab.required))
                                    data-localized-tab-state="error"
                                    title=(first_non_blank(&[tab.error_message.clone(), translate(translate_key, request, VALIDATION_SUMMARY_TITLE_KEY)]))
                                {}
                            } @else {
                                span
                                    class="h-1.5 w-1.5 rounded-full flex-shrink-0"
                                    data-localized-tab-indicator="true"
                                    data-localized-lang=(tab.value)
                                    data-localized-tab-required=(localized_tab_required_value(tab.required))
                                    data-localized-tab-state=(initial_tab_state(tab))
                                    data-attr:class=(indicator_class)
                                    data-attr:title=(indicator_title)
                                    data-attr:data-localized-tab-state=(indicator_state)
                                {}
                            }
                            span { (tab.value.to_uppercase()) }
                            @if tab.required {
                                span class="text-[10px] text-destructive/80 font-bold ml-0.5" title=(translate(translate_key, request, "localized_tabs.tab_required")) { "*" }
                            }
                        }
                    }
                }
                div class="ml-auto px-2 flex items-center text-xs text-muted-foreground/50 font-medium tracking-wide uppercase pointer-events-none select-none hidden sm:block" {
                    (props.aria_label)
                }
            }
            div class="relative bg-background" {
                @for tab in &props.tabs {
                    @let panel_attributes = localized_tab_panel_attributes(tab);
                    div
                        role="tabpanel"
                        class="mt-0 focus-visible:outline-none"
                        tabindex="0"
                        data-value=(tab.value)
                        data-show=(buildTabSelectedExpr(tab.value.as_str()))
                        data-attr:data-state=(buildTabStateExpr(tab.value.as_str()))
                        style="display: none;"
                        id=[panel_attributes.get("id").map(String::as_str)]
                        aria-labelledby=[panel_attributes.get("aria-labelledby").map(String::as_str)]
                    {
                        div class=(shared::build_classes(&[
                            "px-4 pb-0 pt-4 sm:px-6 sm:pt-5",
                            props.panel_content_class.as_str(),
                        ])) {
                            @if let Some(panel) = tab.panel.as_ref() {
                                (panel)
                            }
                            (localized_tab_error_message(tab))
                            @if localized_tabs_has_helper_panels(props) {
                                (localized_editor_helper_panels(request, translate_key, props, tab))
                            }
                            @if let Some(after_helpers) = tab.after_helpers.as_ref() {
                                (after_helpers)
                            }
                        }
                    }
                }
            }
        }
        @if localized_tabs_has_authoring(props) {
            (localized_translation_transport_form(props))
        }
        }
    }
}

fn initial_tab_state(tab: &LocalizedTab) -> &'static str {
    if tab.error {
        "error"
    } else if tab.required && !tab.has_content {
        "missing"
    } else if tab.has_content {
        "complete"
    } else {
        "empty"
    }
}

fn localized_tabs_has_helper_panels(props: &LocalizedTabsProps) -> bool {
    localized_tabs_has_authoring(props) || localized_tabs_has_media_tools(props)
}

fn localized_editor_helper_panels(
    request: &Request,
    translate_key: Option<&TranslateKeyFunc<Request>>,
    props: &LocalizedTabsProps,
    tab: &LocalizedTab,
) -> Markup {
    html! {
        div class="localized-helper-stack mt-4 border-t border-border" {
            @if localized_tabs_has_authoring(props) {
                (localized_auto_translate_helper(request, translate_key, props, tab))
            }
            @if localized_tabs_has_media_tools(props) {
                @if let Some(media_tools) = props.media_tools.as_deref() {
                    (localized_media_insert_helper(media_tools, tab, localized_tab_field_label(tab).as_str()))
                }
            }
        }
    }
}

fn localized_auto_translate_helper(
    request: &Request,
    translate_key: Option<&TranslateKeyFunc<Request>>,
    props: &LocalizedTabsProps,
    tab: &LocalizedTab,
) -> Markup {
    let Some(authoring) = props.authoring.as_deref() else {
        return Markup::default();
    };
    let signals = super::authoring::localized_authoring_signals_for_widget(authoring.widget_key.as_str());
    let checkbox_props = CheckboxProps {
        name: authoring.checkbox_name.clone(),
        id: format!("{}-{}", authoring.checkbox_id, tab.value),
        value: "true".to_owned(),
        checked: authoring.checkbox_checked,
        class: "mt-0.5 h-4 w-4 rounded border-border text-primary focus:ring-2 focus:ring-primary/25 focus:ring-offset-0".to_owned(),
        ..CheckboxProps::default()
    };
    let button_props = ButtonProps {
        variant: shared::VARIANT_OUTLINE,
        size: shared::SIZE_SM,
        button_type: "button",
        disabled: !authoring.translation_available,
        class: "border-border bg-background text-foreground hover:border-primary/35 hover:bg-primary/5 hover:text-primary disabled:cursor-not-allowed disabled:opacity-45".to_owned(),
        ..ButtonProps::default()
    };
    html! {
        div class="localized-helper-panel localized-helper-panel--translate" {
            div class="localized-helper-row" {
                div class="localized-helper-copy" {
                    div class="flex items-start space-x-2" {
                        div class="flex items-center h-5" {
                            input
                                type="checkbox"
                                name=(checkbox_props.name)
                                id=(checkbox_props.id)
                                value=(checkbox_props.value)
                                class=(checkbox_props.build_classes())
                                checked[checkbox_props.checked]
                                disabled[checkbox_props.disabled]
                                data-auto-translate-toggle="true"
                                data-bind=(signals.auto_translate)
                                data-on:change=(localized_authoring_toggle_action_for(authoring.widget_key.as_str()))
                            {}
                        }
                        div class="space-y-1 leading-none" {
                            label class="text-sm font-medium leading-none" for=(checkbox_props.id) { (authoring.auto_translate_label) }
                            @if !authoring.auto_translate_help.is_empty() {
                                p class="text-sm text-muted-foreground" { (authoring.auto_translate_help) }
                            }
                        }
                    }
                }
                div class="localized-helper-actions" {
                    span
                        class="localized-helper-status"
                        data-auto-translate-status="true"
                        data-text={ "$" (signals.status_text) }
                    { (super::authoring::localized_tabs_initial_status(props)) }
                    button
                        type="button"
                        class=(button_props.build_classes())
                        disabled[button_props.disabled]
                        data-auto-translate-run="true"
                        data-attr:disabled={ "$" (signals.translating) " || !$" (signals.translation_available) " ? 'true' : null" }
                        data-on:click=(localized_authoring_prepare_run_action_for(props))
                        r#"data-on:click__delay.1ms"#=(localized_authoring_run_action_for(props))
                    {
                        (translate(translate_key, request, "localized_tabs.translate_now"))
                    }
                }
            }
        }
    }
}

/// Render the localized tab error message.
#[must_use]
pub fn localized_tab_error_message(tab: &LocalizedTab) -> Markup {
    if !tab.error || tab.error_message.trim().is_empty() {
        return Markup::default();
    }
    html! {
        p id={ (tab.field_id) "-error" } class="mt-2 text-xs leading-relaxed text-destructive" role="alert" {
            (tab.error_message)
        }
    }
}

/// Render the live submit error alert.
#[must_use]
pub fn localized_live_submit_error_alert(
    request: &Request,
    translate_key: Option<&TranslateKeyFunc<Request>>,
    class: &str,
) -> Markup {
    let props = AlertProps {
        variant: shared::VARIANT_DESTRUCTIVE,
        class: shared::build_classes(&[
            "shadow-xl border-2 border-destructive bg-destructive-muted text-foreground ring-2 ring-destructive/35",
            class,
        ]),
        role: "alert",
        aria_live: "assertive",
        ..AlertProps::default()
    };
    html! {
        div
            class=(props.build_classes())
            role="alert"
            aria-live="assertive"
            data-localized-submit-error="true"
            data-show="$locSubmitErrorOpen"
            style="display:none"
        {
            div class="flex items-start gap-3" {
                div class="mt-0.5 flex h-9 w-9 shrink-0 items-center justify-center rounded-full border border-destructive bg-destructive text-destructive-foreground shadow-sm" {
                    (AlertTriangle(&IconProps::default().with_class("h-4 w-4")))
                }
                div class="min-w-0 flex-1" {
                    h3 class="text-sm font-semibold text-destructive" {
                        (translate(translate_key, request, "localized_tabs.form_error_title"))
                    }
                    p class="mt-1 text-sm leading-6 text-foreground" {
                        span data-localized-submit-error-text="true" data-text="$locSubmitErrorText" {}
                    }
                }
            }
        }
    }
}

/// Render the live submit blocked alert.
#[must_use]
pub fn localized_live_submit_blocked_alert(
    request: &Request,
    translate_key: Option<&TranslateKeyFunc<Request>>,
    class: &str,
) -> Markup {
    let props = AlertProps {
        variant: shared::VARIANT_WARNING,
        class: shared::build_classes(&[
            "shadow-lg border-2 border-warning bg-warning-muted text-foreground ring-1 ring-warning/35",
            class,
        ]),
        role: "status",
        aria_live: "polite",
        ..AlertProps::default()
    };
    let blocked_text_expr = "(() => { const root = el.closest('[data-localized-authoring=\"true\"]'); const form = el.closest('form'); if (!(root instanceof HTMLElement) || !(form instanceof HTMLFormElement)) return ''; const widgets = Array.from(form.querySelectorAll('[data-localized-widget=\"true\"]')); const translating = widgets.some((widget) => widget instanceof HTMLElement && String(widget.dataset.localizedTranslating || '') === 'true'); return translating ? String(root.dataset.submitTranslatingText || '') : String(root.dataset.submitBlockedText || ''); })()";
    html! {
        div
            class=(props.build_classes())
            role="status"
            aria-live="polite"
            data-localized-submit-blocked-alert="true"
            data-show="$locSubmitBlocked && !$locSubmitErrorOpen"
            style="display:none"
        {
            div class="flex items-start gap-3" {
                div class="mt-0.5 flex h-9 w-9 shrink-0 items-center justify-center rounded-full border border-warning bg-warning text-warning-foreground shadow-sm" {
                    (AlertTriangle(&IconProps::default().with_class("h-4 w-4")))
                }
                div class="min-w-0 flex-1" {
                    h3 class="text-sm font-semibold text-warning-foreground" {
                        (translate(translate_key, request, "localized_tabs.form_blocked_title"))
                    }
                    p class="mt-1 text-sm leading-6 text-foreground" {
                        span data-localized-submit-blocked-text="true" data-text=(blocked_text_expr) {}
                    }
                }
            }
        }
    }
}

/// Render both live submit alerts.
#[must_use]
pub fn localized_live_submit_alerts(
    request: &Request,
    translate_key: Option<&TranslateKeyFunc<Request>>,
    class: &str,
) -> Markup {
    html! {
        (localized_live_submit_blocked_alert(request, translate_key, class))
        (localized_live_submit_error_alert(request, translate_key, class))
    }
}

/// Render the localized form error alert.
#[must_use]
pub fn localized_form_error_alert(
    request: &Request,
    translate_key: Option<&TranslateKeyFunc<Request>>,
    message: &str,
) -> Markup {
    let props = AlertProps {
        variant: shared::VARIANT_DESTRUCTIVE,
        class: super::authoring::localized_form_error_alert_class(),
        role: "alert",
        aria_live: "assertive",
        ..AlertProps::default()
    };
    html! {
        div class=(props.build_classes()) role="alert" aria-live="assertive" {
            div class="flex items-start gap-3" {
                div class="mt-0.5 flex h-10 w-10 shrink-0 items-center justify-center rounded-full border border-destructive bg-destructive text-destructive-foreground shadow-sm" {
                    (AlertTriangle(&IconProps::default().with_class("h-4 w-4")))
                }
                div class="min-w-0 flex-1" {
                    h2 class="text-base font-semibold text-destructive" {
                        (translate(translate_key, request, "localized_tabs.form_error_title"))
                    }
                    p class="mt-1 text-sm leading-6 text-foreground" { (message) }
                }
            }
        }
    }
}

/// Render one localized text-area panel.
#[must_use]
pub fn localized_text_area_panel(input: &LocalizedTextAreaPanelVM) -> Markup {
    let props = InputProps {
        input_type: INPUT_TYPE_TEXT,
        name: input.name.clone(),
        id: input.id.clone(),
        placeholder: input.placeholder.clone(),
        value: presentation_text(input.value.as_bytes()),
        size: shared::SIZE_MD,
        required: input.required,
        class: localized_editor_text_area_class(),
        ..InputProps::default()
    };
    let field_attributes = super::authoring::localized_field_attributes(
        input.widget_key.as_str(),
        input.lang.as_str(),
        input.field_key.as_str(),
        None,
    );
    let rows = if input.rows > 0 { input.rows } else { 0 };
    html! {
        div class="space-y-4" {
            div class="space-y-1.5" {
                @if !input.label.is_empty() {
                    label for=(input.id) class=(label_classes(&props)) {
                        (input.label)
                        @if input.required {
                            span class="text-destructive ml-1" aria-label="required" { "*" }
                        }
                    }
                }
                div class=(props.build_wrapper_classes()) {
                    textarea
                        name=(input.name)
                        id=(input.id)
                        placeholder=(input.placeholder)
                        class=(props.build_textarea_classes())
                        disabled[props.disabled]
                        required[input.required]
                        rows=(rows.to_string())
                        data-content-editor="true"
                        data-content-editor-format="markdown"
                        data-localized-field=[field_attributes.get("data-localized-field").map(String::as_str)]
                        data-localized-widget-key=[field_attributes.get("data-localized-widget-key").map(String::as_str)]
                        data-localized-lang=[field_attributes.get("data-localized-lang").map(String::as_str)]
                        data-localized-field-key=[field_attributes.get("data-localized-field-key").map(String::as_str)]
                        data-bind=[field_attributes.get("data-bind").map(String::as_str)]
                        data-on:focusin=[field_attributes.get("data-on:focusin").map(String::as_str)]
                        data-on:input=[field_attributes.get("data-on:input").map(String::as_str)]
                        r#"data-on:input__debounce.500ms"#=[field_attributes.get("data-on:input__debounce.500ms").map(String::as_str)]
                    { (props.value) }
                }
            }
        }
    }
}

/// Render the content-format select.
#[must_use]
pub fn localized_format_select(props: &LocalizedFormatSelectProps) -> Markup {
    let select_props = SelectProps {
        name: props.input.name.clone(),
        id: props.input.id.clone(),
        value: presentation_text(props.input.value.as_bytes()),
        required: props.input.required,
        size: shared::SIZE_MD,
        options: props
            .options
            .iter()
            .map(|option| web_components::components::unstyled::select::SelectOption {
                label: option.label.clone(),
                value: option.value.clone(),
                disabled: option.disabled,
                selected: false,
                attributes: std::collections::BTreeMap::new(),
            })
            .collect(),
        ..SelectProps::default()
    };
    let format_attributes = super::authoring::localized_format_attributes(
        props.input.widget_key.as_str(),
        props.input.lang.as_str(),
        props.input.field_key.as_str(),
        None,
    );
    html! {
        div class="space-y-2" {
            @if !props.label.is_empty() {
                label for=(select_props.id) class="text-sm font-medium leading-none peer-disabled:cursor-not-allowed peer-disabled:opacity-70" {
                    (props.label)
                    @if select_props.required {
                        span class="text-destructive ml-1" aria-label="required" { "*" }
                    }
                }
            }
            div class=(select_props.build_wrapper_classes()) {
                select
                    class=(select_props.build_classes())
                    id=(select_props.id)
                    name=(select_props.name)
                    required[select_props.required]
                    data-localized-format=[format_attributes.get("data-localized-format").map(String::as_str)]
                    data-localized-widget-key=[format_attributes.get("data-localized-widget-key").map(String::as_str)]
                    data-localized-lang=[format_attributes.get("data-localized-lang").map(String::as_str)]
                    data-localized-field-key=[format_attributes.get("data-localized-field-key").map(String::as_str)]
                    data-bind=[format_attributes.get("data-bind").map(String::as_str)]
                    data-on:focusin=[format_attributes.get("data-on:focusin").map(String::as_str)]
                    data-on:change=[format_attributes.get("data-on:change").map(String::as_str)]
                {
                    @for option in &props.options {
                        option value=(option.value) selected[props.input.value.as_bytes() == option.value.as_bytes()] disabled[option.disabled] { (option.label) }
                    }
                }
            }
            @if !props.help_text.is_empty() {
                p class="text-sm text-muted-foreground" { (props.help_text) }
            }
        }
    }
}

/// Render the media insert helper for a tab.
#[must_use]
pub fn localized_media_insert_helper(
    media: &LocalizedMediaToolsProps,
    _tab: &LocalizedTab,
    label: &str,
) -> Markup {
    if media.open_url.is_empty() {
        return Markup::default();
    }
    html! {
        button
            type="button"
            class="inline-flex items-center gap-1 rounded-md border border-border px-2 py-1 text-xs text-muted-foreground"
            data-on:click=(media_insert_action(media))
        {
            (label)
        }
    }
}

fn localized_translation_transport_form(props: &LocalizedTabsProps) -> Markup {
    let Some(authoring) = props.authoring.as_deref() else {
        return Markup::default();
    };
    let signals = super::authoring::localized_authoring_signals_for_widget(authoring.widget_key.as_str());
    html! {
        div class="hidden" aria-hidden="true" data-localized-translation-transport="true" {
            input
                type="hidden"
                name=(localized_translation_scope_field_name())
                value=(authoring.scope)
                data-attr:disabled=(localized_transport_disabled_expr_for(authoring.widget_key.as_str()))
            {}
            input
                type="hidden"
                name=(crate::forms::localized_tabs::LOCALIZED_TRANSLATION_WIDGET_KEY_FIELD_NAME)
                value=(signals.widget_key)
                data-attr:disabled=(localized_transport_disabled_expr_for(authoring.widget_key.as_str()))
            {}
            input
                type="hidden"
                name=(localized_translation_source_language_field_name())
                data-bind=(signals.source_language)
                data-attr:disabled=(localized_transport_disabled_expr_for(authoring.widget_key.as_str()))
            {}
            @for tab in &props.tabs {
                @if !tab.field_key.trim().is_empty() {
                    @let lang = super::authoring::localized_tabs_language(props, tab.value.as_str());
                    input
                        type="hidden"
                        name=(localized_translation_target_language_field_name())
                        value=(lang)
                        data-attr:disabled=(format!(
                            "{} || ${} === {} ? 'true' : null",
                            localized_transport_inactive_expr_for(authoring.widget_key.as_str()),
                            signals.source_language,
                            quoted_js(lang.as_str())
                        ))
                    {}
                    input
                        type="hidden"
                        name=(localized_translation_field_name_field_name())
                        value=(tab.field_key.trim())
                        data-attr:disabled=(format!(
                            "{} || ${} !== {} ? 'true' : null",
                            localized_transport_inactive_expr_for(authoring.widget_key.as_str()),
                            signals.source_language,
                            quoted_js(lang.as_str())
                        ))
                    {}
                    input
                        type="hidden"
                        name=(localized_translation_field_value_field_name())
                        data-bind=(super::authoring::localized_field_signal_name(
                            tab.value.as_str(),
                            tab.field_key.as_str(),
                            props.normalize_language.as_deref()
                        ))
                        data-attr:disabled=(format!(
                            "{} || ${} !== {} ? 'true' : null",
                            localized_transport_inactive_expr_for(authoring.widget_key.as_str()),
                            signals.source_language,
                            quoted_js(lang.as_str())
                        ))
                    {}
                    input
                        type="hidden"
                        name=(localized_translation_field_format_field_name())
                        data-bind=(super::authoring::localized_format_signal_name(
                            tab.value.as_str(),
                            tab.field_key.as_str(),
                            props.normalize_language.as_deref()
                        ))
                        data-attr:disabled=(format!(
                            "{} || ${} !== {} ? 'true' : null",
                            localized_transport_inactive_expr_for(authoring.widget_key.as_str()),
                            signals.source_language,
                            quoted_js(lang.as_str())
                        ))
                    {}
                }
            }
        }
    }
}
