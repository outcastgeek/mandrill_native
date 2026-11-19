#![allow(non_snake_case)]

use crate::components::unstyled::datepicker::DatePickerProps;
use crate::components::{ComponentInfo, shared};
use maud::{Markup, html};
use std::collections::BTreeMap;

pub const INPUT_TYPE_TEXT: &str = shared::INPUT_TYPE_TEXT;
pub const INPUT_TYPE_EMAIL: &str = shared::INPUT_TYPE_EMAIL;
pub const INPUT_TYPE_PASSWORD: &str = shared::INPUT_TYPE_PASSWORD;
pub const INPUT_TYPE_SEARCH: &str = shared::INPUT_TYPE_SEARCH;
pub const INPUT_TYPE_URL: &str = shared::INPUT_TYPE_URL;
pub const INPUT_TYPE_TEL: &str = shared::INPUT_TYPE_TEL;
pub const INPUT_TYPE_NUMBER: &str = shared::INPUT_TYPE_NUMBER;
pub const INPUT_TYPE_DATE: &str = shared::INPUT_TYPE_DATE;
pub const INPUT_TYPE_TIME: &str = shared::INPUT_TYPE_TIME;
pub const INPUT_TYPE_HIDDEN: &str = shared::INPUT_TYPE_HIDDEN;
pub const INPUT_TYPE_FILE: &str = "file";

pub const COMPONENTS: &[ComponentInfo] = &[
    ComponentInfo { family: "unstyled", module: "input", name: "CommandSearchInput" },
    ComponentInfo {
        family: "unstyled",
        module: "input",
        name: "CommaSeparatedInputWithLabel",
    },
    ComponentInfo {
        family: "unstyled",
        module: "input",
        name: "Input",
    },
    ComponentInfo {
        family: "unstyled",
        module: "input",
        name: "InputWithLabel",
    },
    ComponentInfo {
        family: "unstyled",
        module: "input",
        name: "FloatingInput",
    },
    ComponentInfo {
        family: "unstyled",
        module: "input",
        name: "FloatingInputWithIcon",
    },
    ComponentInfo {
        family: "unstyled",
        module: "input",
        name: "FloatingPasswordInput",
    },
    ComponentInfo {
        family: "unstyled",
        module: "input",
        name: "TextArea",
    },
    ComponentInfo {
        family: "unstyled",
        module: "input",
        name: "TextAreaWithLabel",
    },
    ComponentInfo {
        family: "unstyled",
        module: "input",
        name: "InputWithIcon",
    },
    ComponentInfo {
        family: "unstyled",
        module: "input",
        name: "SearchInput",
    },
    ComponentInfo {
        family: "unstyled",
        module: "input",
        name: "EmailInput",
    },
    ComponentInfo {
        family: "unstyled",
        module: "input",
        name: "PasswordInput",
    },
    ComponentInfo {
        family: "unstyled",
        module: "input",
        name: "NumberInput",
    },
    ComponentInfo {
        family: "unstyled",
        module: "input",
        name: "URLInput",
    },
    ComponentInfo {
        family: "unstyled",
        module: "input",
        name: "TelInput",
    },
];

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InputProps {
    pub input_type: &'static str,
    pub name: String,
    pub id: String,
    pub placeholder: String,
    pub value: String,
    pub size: &'static str,
    pub disabled: bool,
    pub read_only: bool,
    pub required: bool,
    pub error: bool,
    pub success: bool,
    pub error_message: String,
    pub show_error: bool,
    pub full_width: bool,
    pub class: String,
    pub attributes: BTreeMap<String, String>,
    pub aria_label: String,
    pub aria_described_by: String,
    pub aria_invalid: bool,
    pub min: String,
    pub max: String,
    pub step: String,
    pub pattern: String,
    pub autocomplete: String,
}

impl Default for InputProps {
    fn default() -> Self {
        Self {
            input_type: INPUT_TYPE_TEXT,
            name: String::new(),
            id: String::new(),
            placeholder: String::new(),
            value: String::new(),
            size: shared::SIZE_MD,
            disabled: false,
            read_only: false,
            required: false,
            error: false,
            success: false,
            error_message: String::new(),
            show_error: true,
            full_width: false,
            class: String::new(),
            attributes: BTreeMap::new(),
            aria_label: String::new(),
            aria_described_by: String::new(),
            aria_invalid: false,
            min: String::new(),
            max: String::new(),
            step: String::new(),
            pattern: String::new(),
            autocomplete: String::new(),
        }
    }
}

impl InputProps {
    #[must_use]
    pub const fn with_type(mut self, value: &'static str) -> Self {
        self.input_type = value;
        self
    }
    #[must_use]
    pub fn with_name(mut self, value: impl Into<String>) -> Self {
        self.name = value.into();
        self
    }
    #[must_use]
    pub fn with_id(mut self, value: impl Into<String>) -> Self {
        self.id = value.into();
        self
    }
    #[must_use]
    pub fn with_placeholder(mut self, value: impl Into<String>) -> Self {
        self.placeholder = value.into();
        self
    }
    #[must_use]
    pub fn with_value(mut self, value: impl Into<String>) -> Self {
        self.value = value.into();
        self
    }
    #[must_use]
    pub const fn with_size(mut self, value: &'static str) -> Self {
        self.size = value;
        self
    }
    #[must_use]
    pub const fn with_disabled(mut self, value: bool) -> Self {
        self.disabled = value;
        self
    }
    #[must_use]
    pub const fn with_read_only(mut self, value: bool) -> Self {
        self.read_only = value;
        self
    }
    #[must_use]
    pub const fn with_required(mut self, value: bool) -> Self {
        self.required = value;
        self
    }
    #[must_use]
    pub fn with_error(mut self, error: bool, message: impl Into<String>) -> Self {
        self.error = error;
        self.error_message = message.into();
        self.aria_invalid = error;
        self
    }
    #[must_use]
    pub const fn with_success(mut self, value: bool) -> Self {
        self.success = value;
        self
    }
    #[must_use]
    pub const fn with_show_error(mut self, value: bool) -> Self {
        self.show_error = value;
        self
    }
    #[must_use]
    pub const fn with_full_width(mut self, value: bool) -> Self {
        self.full_width = value;
        self
    }
    #[must_use]
    pub fn with_class(mut self, value: impl Into<String>) -> Self {
        self.class = value.into();
        self
    }
    #[must_use]
    pub fn with_attribute(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.attributes.insert(name.into(), value.into());
        self
    }
    #[must_use]
    pub fn with_aria_label(mut self, value: impl Into<String>) -> Self {
        self.aria_label = value.into();
        self
    }
    #[must_use]
    pub fn with_aria_described_by(mut self, value: impl Into<String>) -> Self {
        self.aria_described_by = value.into();
        self
    }
    #[must_use]
    pub const fn with_aria_invalid(mut self, value: bool) -> Self {
        self.aria_invalid = value;
        self
    }
    #[must_use]
    pub fn with_min(mut self, value: impl Into<String>) -> Self {
        self.min = value.into();
        self
    }
    #[must_use]
    pub fn with_max(mut self, value: impl Into<String>) -> Self {
        self.max = value.into();
        self
    }
    #[must_use]
    pub fn with_step(mut self, value: impl Into<String>) -> Self {
        self.step = value.into();
        self
    }
    #[must_use]
    pub fn with_pattern(mut self, value: impl Into<String>) -> Self {
        self.pattern = value.into();
        self
    }
    #[must_use]
    pub fn with_autocomplete(mut self, value: impl Into<String>) -> Self {
        self.autocomplete = value.into();
        self
    }

    #[must_use]
    pub const fn base_classes(&self) -> &'static str {
        "flex w-full border border-input bg-transparent shadow-sm placeholder:text-muted-foreground input-hover-border focus-visible:outline-none focus-visible:border-ring focus-visible:shadow-[0_0_0_2px_var(--ring)] disabled:cursor-not-allowed disabled:opacity-50 transition-[border-color,box-shadow] duration-150"
    }

    #[must_use]
    pub const fn state_classes(&self) -> &'static str {
        if self.error {
            "border-2 border-destructive focus-visible:shadow-[0_0_0_2px_var(--destructive)] animate-shake"
        } else if self.success {
            "border-2 border-success focus-visible:shadow-[0_0_0_2px_var(--success)]"
        } else {
            ""
        }
    }

    #[must_use]
    pub fn build_classes(&self) -> String {
        shared::build_classes(&[
            self.base_classes(),
            shared::input_size_classes(self.size),
            self.state_classes(),
            if self.full_width { "w-full" } else { "" },
            self.class.as_str(),
        ])
    }

    #[must_use]
    pub fn build_wrapper_classes(&self) -> String {
        shared::build_classes(&["relative", if self.full_width { "w-full" } else { "" }])
    }

    #[must_use]
    pub fn build_textarea_classes(&self) -> String {
        shared::build_classes(&[
            self.base_classes(),
            shared::textarea_size_classes(self.size),
            self.state_classes(),
            if self.full_width { "w-full" } else { "" },
            "resize-y min-h-[80px]",
            self.class.as_str(),
        ])
    }

    #[must_use]
    pub fn build_attributes(&self) -> BTreeMap<String, String> {
        let mut attrs = self.attributes.clone();
        if !self.min.is_empty() {
            attrs.insert("min".to_string(), self.min.clone());
        }
        if !self.max.is_empty() {
            attrs.insert("max".to_string(), self.max.clone());
        }
        if !self.step.is_empty() {
            attrs.insert("step".to_string(), self.step.clone());
        }
        if !self.pattern.is_empty() {
            attrs.insert("pattern".to_string(), self.pattern.clone());
        }
        if !self.autocomplete.is_empty() {
            attrs.insert("autocomplete".to_string(), self.autocomplete.clone());
        }
        if !self.aria_label.is_empty() {
            attrs.insert("aria-label".to_string(), self.aria_label.clone());
        }
        if !self.aria_described_by.is_empty() {
            attrs.insert(
                "aria-describedby".to_string(),
                self.aria_described_by.clone(),
            );
        }
        if self.aria_invalid {
            attrs.insert("aria-invalid".to_string(), "true".to_string());
        }
        attrs
    }

    #[must_use]
    pub fn get_error_id(&self) -> String {
        if self.id.is_empty() {
            String::new()
        } else {
            format!("{}-error", self.id)
        }
    }
}

#[must_use]
pub fn DatePickerPropsFromInput(props: &InputProps) -> DatePickerProps {
    let id = if props.id.is_empty() {
        props
            .attributes
            .get("id")
            .cloned()
            .unwrap_or_else(String::new)
    } else {
        props.id.clone()
    };

    DatePickerProps {
        name: props.name.clone(),
        id,
        placeholder: props.placeholder.clone(),
        value: props.value.clone(),
        size: props.size,
        disabled: props.disabled,
        read_only: props.read_only,
        required: props.required,
        error: props.error,
        success: props.success,
        error_message: props.error_message.clone(),
        show_error: true,
        full_width: props.full_width,
        class: props.class.clone(),
        attributes: props.attributes.clone(),
        aria_label: props.aria_label.clone(),
        aria_described_by: props.aria_described_by.clone(),
        aria_invalid: props.aria_invalid,
        min: props.min.clone(),
        max: props.max.clone(),
        ..Default::default()
    }
}

fn optional(value: &str) -> Option<&str> {
    if value.is_empty() { None } else { Some(value) }
}

#[must_use]
pub fn label_classes(props: &InputProps) -> String {
    shared::build_classes(&[
        "text-sm font-medium leading-none peer-disabled:cursor-not-allowed peer-disabled:opacity-70",
        if props.error { "text-destructive" } else { "" },
    ])
}

fn attrs_described_by<'a>(
    props: &'a InputProps,
    attrs: &'a BTreeMap<String, String>,
) -> Option<String> {
    if props.error && !props.get_error_id().is_empty() {
        Some(props.get_error_id())
    } else {
        shared::attr_value(attrs, "aria-describedby").map(str::to_string)
    }
}

#[must_use]
pub fn Input(props: &InputProps) -> Markup {
    let mut attrs = props.build_attributes();
    if let Some(value)=attrs_described_by(props,&attrs){attrs.insert("aria-describedby".into(),value);}
    crate::html! {
        div class=(props.build_wrapper_classes()) {
            input
                type=(props.input_type)
                name=[optional(&props.name)]
                id=[optional(&props.id)]
                placeholder=[optional(&props.placeholder)]
                value=[optional(&props.value)]
                class=(props.build_classes())
                disabled[props.disabled]
                readonly[props.read_only]
                required[props.required]
                ..(&attrs)
            {}
        }
        @if props.error && props.show_error && !props.error_message.is_empty() {
            p id=(props.get_error_id()) class="mt-1.5 text-xs text-destructive" role="alert" { (&props.error_message) }
        }
    }
}

#[must_use]
pub fn InputWithLabel(
    props: &InputProps,
    label: impl Into<String>,
    helper_text: impl Into<String>,
) -> Markup {
    let label = label.into();
    let helper_text = helper_text.into();
    html! {
        div class="space-y-1.5" {
            @if !label.is_empty() {
                label for=[optional(&props.id)] class=(label_classes(props)) {
                    (&label)
                    @if props.required {
                        span class="text-destructive ml-1" aria-label=(shared::default_strings().input_required_aria_label) {
                            (shared::default_strings().input_required_symbol)
                        }
                    }
                }
            }
            (Input(props))
            @if !helper_text.is_empty() && !props.error {
                p class="text-xs text-muted-foreground" { (&helper_text) }
            }
        }
    }
}

#[must_use]
pub fn floatingGroupClasses(props: &InputProps) -> String {
    shared::build_classes(&[
        "floating-input-group",
        if props.error { "is-invalid" } else { "" },
    ])
}

#[must_use]
pub fn floatingInputClasses(props: &InputProps) -> String {
    shared::build_classes(&["floating-input-field", "pl-3", props.class.as_str()])
}

#[must_use]
pub fn floatingInputWithIconClasses(props: &InputProps) -> String {
    shared::build_classes(&["floating-input-field", "pl-11", props.class.as_str()])
}

fn floating_error_and_helper(props: &InputProps, helper_text: &str) -> Markup {
    html! {
        @if !helper_text.is_empty() && !props.error {
            p class="text-xs text-muted-foreground mt-1.5" { (helper_text) }
        }
        @if props.error && props.show_error && !props.error_message.is_empty() {
            p id=(props.get_error_id()) class="mt-1.5 text-xs text-destructive" role="alert" { (&props.error_message) }
        }
    }
}

#[must_use]
pub fn FloatingInput(
    props: &InputProps,
    label: impl Into<String>,
    helper_text: impl Into<String>,
) -> Markup {
    let label = label.into();
    let helper_text = helper_text.into();
    let attrs = props.build_attributes();
    html! {
        div class=(floatingGroupClasses(props)) {
            input
                type=(props.input_type)
                name=[optional(&props.name)]
                id=[optional(&props.id)]
                placeholder=" "
                value=[optional(&props.value)]
                class=(floatingInputClasses(props))
                disabled[props.disabled]
                readonly[props.read_only]
                required[props.required]
                aria-describedby=[attrs_described_by(props, &attrs)]
                min=[shared::attr_value(&attrs, "min")]
                max=[shared::attr_value(&attrs, "max")]
                step=[shared::attr_value(&attrs, "step")]
                pattern=[shared::attr_value(&attrs, "pattern")]
                autocomplete=[shared::attr_value(&attrs, "autocomplete")]
                data-bind=[shared::attr_value(&attrs, "data-bind")]
            {}
            @if !label.is_empty() {
                label for=[optional(&props.id)] class="floating-input-label" {
                    (&label)
                    @if props.required {
                        span class="text-destructive ml-0.5" aria-label=(shared::default_strings().input_required_aria_label) {
                            (shared::default_strings().input_required_symbol)
                        }
                    }
                }
            }
        }
        (floating_error_and_helper(props, &helper_text))
    }
}

#[must_use]
pub fn FloatingInputWithIcon(
    props: &InputProps,
    label: impl Into<String>,
    helper_text: impl Into<String>,
    icon: Markup,
) -> Markup {
    let label = label.into();
    let helper_text = helper_text.into();
    let attrs = props.build_attributes();
    html! {
        div class=(floatingGroupClasses(props)) {
            div class="floating-input-icon" { (icon) }
            input
                type=(props.input_type)
                name=[optional(&props.name)]
                id=[optional(&props.id)]
                placeholder=" "
                value=[optional(&props.value)]
                class=(floatingInputWithIconClasses(props))
                disabled[props.disabled]
                readonly[props.read_only]
                required[props.required]
                aria-describedby=[attrs_described_by(props, &attrs)]
                autocomplete=[shared::attr_value(&attrs, "autocomplete")]
            {}
            @if !label.is_empty() {
                label for=[optional(&props.id)] class="floating-input-label" style="left: 2.75rem;" {
                    (&label)
                    @if props.required {
                        span class="text-destructive ml-0.5" aria-label=(shared::default_strings().input_required_aria_label) {
                            (shared::default_strings().input_required_symbol)
                        }
                    }
                }
            }
        }
        (floating_error_and_helper(props, &helper_text))
    }
}

#[must_use]
pub fn passwordSignalInit(signal_name: &str) -> String {
    format!("{{ {signal_name}: false }}")
}

#[must_use]
pub fn passwordInputAttributes(props: &InputProps, signal_name: &str) -> BTreeMap<String, String> {
    let mut attrs = props.build_attributes();
    attrs.insert(
        "data-attr:type".to_string(),
        format!("${signal_name} ? 'text' : 'password'"),
    );
    attrs
}

#[must_use]
pub fn FloatingPasswordInput(
    props: &InputProps,
    label: impl Into<String>,
    helper_text: impl Into<String>,
    signal_name: &str,
) -> Markup {
    let label = label.into();
    let helper_text = helper_text.into();
    let attrs = passwordInputAttributes(props, signal_name);
    let show_expr = format!("${signal_name}");
    let hide_expr = format!("!${signal_name}");
    let toggle_expr = format!("${signal_name} = !${signal_name}");
    html! {
        div class=(floatingGroupClasses(props)) data-signals=(passwordSignalInit(signal_name)) {
            div class="floating-input-icon" {
                svg xmlns="http://www.w3.org/2000/svg" width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" {
                    rect width="18" height="11" x="3" y="11" rx="2" ry="2" {}
                    path d="M7 11V7a5 5 0 0 1 10 0v4" {}
                }
            }
            input
                name=[optional(&props.name)]
                id=[optional(&props.id)]
                placeholder=" "
                value=[optional(&props.value)]
                class=(shared::build_classes(&[floatingInputWithIconClasses(props).as_str(), "pr-11"]))
                disabled[props.disabled]
                readonly[props.read_only]
                required[props.required]
                aria-describedby=[attrs_described_by(props, &attrs)]
                autocomplete=[shared::attr_value(&attrs, "autocomplete")]
                data-attr:type=[shared::attr_value(&attrs, "data-attr:type")]
            {}
            @if !label.is_empty() {
                label for=[optional(&props.id)] class="floating-input-label" style="left: 2.75rem;" {
                    (&label)
                    @if props.required {
                        span class="text-destructive ml-0.5" aria-label=(shared::default_strings().input_required_aria_label) {
                            (shared::default_strings().input_required_symbol)
                        }
                    }
                }
            }
            button type="button" class="password-toggle" aria-label="Toggle password visibility" data-on:click=(toggle_expr) {
                svg xmlns="http://www.w3.org/2000/svg" width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" data-show=(hide_expr) {
                    path d="M2 12s3-7 10-7 10 7 10 7-3 7-10 7-10-7-10-7Z" {}
                    circle cx="12" cy="12" r="3" {}
                }
                svg xmlns="http://www.w3.org/2000/svg" width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" style="display:none" data-show=(show_expr) {
                    path d="M9.88 9.88a3 3 0 1 0 4.24 4.24" {}
                    path d="M10.73 5.08A10.43 10.43 0 0 1 12 5c7 0 10 7 10 7a13.16 13.16 0 0 1-1.67 2.68" {}
                    path d="M6.61 6.61A13.526 13.526 0 0 0 2 12s3 7 10 7a9.74 9.74 0 0 0 5.39-1.61" {}
                    line x1="2" x2="22" y1="2" y2="22" {}
                }
            }
        }
        (floating_error_and_helper(props, &helper_text))
    }
}

#[must_use]
pub fn TextArea(props: &InputProps, rows: i32) -> Markup {
    let mut attrs = props.build_attributes();
    if let Some(value)=attrs_described_by(props,&attrs){attrs.insert("aria-describedby".into(),value);}
    crate::html! {
        div class=(props.build_wrapper_classes()) {
            textarea
                name=[optional(&props.name)]
                id=[optional(&props.id)]
                placeholder=[optional(&props.placeholder)]
                class=(props.build_textarea_classes())
                disabled[props.disabled]
                readonly[props.read_only]
                required[props.required]
                rows=(rows)
                ..(&attrs)
            { (&props.value) }
        }
        @if props.error && props.show_error && !props.error_message.is_empty() {
            p id=(props.get_error_id()) class="mt-1.5 text-xs text-destructive" role="alert" { (&props.error_message) }
        }
    }
}

#[must_use]
pub fn TextAreaWithLabel(
    props: &InputProps,
    label: impl Into<String>,
    helper_text: impl Into<String>,
    rows: i32,
) -> Markup {
    let label = label.into();
    let helper_text = helper_text.into();
    html! {
        div class="space-y-1.5" {
            @if !label.is_empty() {
                label for=[optional(&props.id)] class=(label_classes(props)) {
                    (&label)
                    @if props.required {
                        span class="text-destructive ml-1" aria-label=(shared::default_strings().input_required_aria_label) {
                            (shared::default_strings().input_required_symbol)
                        }
                    }
                }
            }
            (TextArea(props, rows))
            @if !helper_text.is_empty() && !props.error {
                p class="text-xs text-muted-foreground" { (&helper_text) }
            }
        }
    }
}

#[must_use]
pub fn InputWithIcon(props: &InputProps, icon_position: &str, icon: Markup) -> Markup {
    let attrs = props.build_attributes();
    let icon_left = icon_position == "left";
    html! {
        div class="relative" {
            @if icon_left {
                div class="absolute left-3 top-1/2 -translate-y-1/2 pointer-events-none" { (icon) }
            }
            input
                type=(props.input_type)
                name=[optional(&props.name)]
                id=[optional(&props.id)]
                placeholder=[optional(&props.placeholder)]
                value=[optional(&props.value)]
                class=(shared::build_classes(&[props.build_classes().as_str(), if icon_left { "pl-10" } else { "pr-10" }]))
                disabled[props.disabled]
                readonly[props.read_only]
                required[props.required]
                aria-describedby=[attrs_described_by(props, &attrs)]
                autocomplete=[shared::attr_value(&attrs, "autocomplete")]
            {}
            @if !icon_left {
                div class="absolute right-3 top-1/2 -translate-y-1/2 pointer-events-none" { (icon) }
            }
        }
        @if props.error && props.show_error && !props.error_message.is_empty() {
            p id=(props.get_error_id()) class="mt-1.5 text-xs text-destructive" role="alert" { (&props.error_message) }
        }
    }
}

#[must_use]
pub fn SearchInput(id: &str, name: &str, placeholder: &str, size: &'static str) -> Markup {
    html! {
        div class="relative" {
            div class="absolute left-3 top-1/2 -translate-y-1/2 text-muted-foreground" {
                svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" {
                    circle cx="11" cy="11" r="8" {}
                    path d="m21 21-4.3-4.3" {}
                }
            }
            (Input(&InputProps::default()
                .with_type(INPUT_TYPE_SEARCH)
                .with_id(id)
                .with_name(name)
                .with_placeholder(placeholder)
                .with_size(size)
                .with_class("pl-10")
                .with_autocomplete("off")))
        }
    }
}

#[must_use]
pub fn EmailInput(id: &str, name: &str, placeholder: &str, required: bool) -> Markup {
    InputWithLabel(
        &InputProps::default()
            .with_type(INPUT_TYPE_EMAIL)
            .with_id(id)
            .with_name(name)
            .with_placeholder(placeholder)
            .with_required(required)
            .with_autocomplete("email"),
        shared::default_strings().input_email_label,
        "",
    )
}

#[must_use]
pub fn PasswordInput(id: &str, name: &str, placeholder: &str, required: bool) -> Markup {
    InputWithLabel(
        &InputProps::default()
            .with_type(INPUT_TYPE_PASSWORD)
            .with_id(id)
            .with_name(name)
            .with_placeholder(placeholder)
            .with_required(required)
            .with_autocomplete("current-password"),
        shared::default_strings().input_password_label,
        "",
    )
}

#[must_use]
pub fn NumberInput(
    id: &str,
    name: &str,
    placeholder: &str,
    min: &str,
    max: &str,
    step: &str,
) -> Markup {
    Input(
        &InputProps::default()
            .with_type(INPUT_TYPE_NUMBER)
            .with_id(id)
            .with_name(name)
            .with_placeholder(placeholder)
            .with_min(min)
            .with_max(max)
            .with_step(step),
    )
}

#[must_use]
pub fn URLInput(id: &str, name: &str, placeholder: &str) -> Markup {
    html! {
        div class="relative" {
            div class="absolute left-3 top-1/2 -translate-y-1/2 text-muted-foreground text-sm" {
                (shared::default_strings().input_url_prefix)
            }
            (Input(&InputProps::default()
                .with_type(INPUT_TYPE_URL)
                .with_id(id)
                .with_name(name)
                .with_placeholder(placeholder)
                .with_class("pl-20")
                .with_autocomplete("url")))
        }
    }
}

#[must_use]
pub fn TelInput(id: &str, name: &str, placeholder: &str) -> Markup {
    html! {
        div class="relative" {
            div class="absolute left-3 top-1/2 -translate-y-1/2 text-muted-foreground" {
                svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" {
                    path d="M22 16.92v3a2 2 0 0 1-2.18 2 19.79 19.79 0 0 1-8.63-3.07 19.5 19.5 0 0 1-6-6 19.79 19.79 0 0 1-3.07-8.67A2 2 0 0 1 4.11 2h3a2 2 0 0 1 2 1.72 12.84 12.84 0 0 0 .7 2.81 2 2 0 0 1-.45 2.11L8.09 9.91a16 16 0 0 0 6 6l1.27-1.27a2 2 0 0 1 2.11-.45 12.84 12.84 0 0 0 2.81.7A2 2 0 0 1 22 16.92z" {}
                }
            }
            (Input(&InputProps::default()
                .with_type(INPUT_TYPE_TEL)
                .with_id(id)
                .with_name(name)
                .with_placeholder(placeholder)
                .with_class("pl-10")
                .with_autocomplete("tel")))
        }
    }
}

#[must_use]
pub fn NormalizeCommaSeparatedValues(raw: &str) -> Vec<String> {
    let mut values = Vec::new();
    for part in raw.split(',') {
        let value = part.trim();
        if value.is_empty() || values.iter().any(|existing: &String| existing == value) {
            continue;
        }
        values.push(value.to_string());
    }
    values
}

#[must_use]
pub fn CommaSeparatedValueString(raw: &str) -> String {
    NormalizeCommaSeparatedValues(raw).join(", ")
}

#[must_use]
pub fn CommaSeparatedEntryID(props: &InputProps) -> String {
    format!("{}-entry", commaSeparatedBaseID(props))
}

#[must_use]
pub fn CommaSeparatedHiddenID(props: &InputProps) -> String {
    format!("{}-hidden", commaSeparatedBaseID(props))
}

#[must_use]
pub fn CommaSeparatedContainerClasses(props: &InputProps) -> String {
    shared::build_classes(&[
        props.base_classes(),
        "h-auto min-h-10 cursor-text flex-col items-stretch gap-2 rounded-md px-3 py-2",
        props.state_classes(),
        if props.full_width { "w-full" } else { "" },
        props.class.as_str(),
    ])
}

#[must_use]
pub fn CommaSeparatedEntryClasses(props: &InputProps) -> String {
    let size_class = match props.size {
        shared::SIZE_LG => "text-base",
        shared::SIZE_SM => "text-xs",
        _ => "text-sm",
    };
    shared::build_classes(&[
        "comma-token-input-entry h-7 w-full min-w-0 border-0 bg-transparent p-0 text-foreground placeholder:text-muted-foreground shadow-none outline-none focus:outline-none focus:ring-0",
        size_class,
    ])
}

#[must_use]
pub fn CommaSeparatedValuesSignal(props: &InputProps) -> String {
    format!("{}_values", commaSeparatedSignalStem(props))
}

#[must_use]
pub fn CommaSeparatedDraftSignal(props: &InputProps) -> String {
    format!("{}_draft", commaSeparatedSignalStem(props))
}

#[must_use]
pub fn CommaSeparatedCSVSignal(props: &InputProps) -> String {
    format!("{}_csv", commaSeparatedSignalStem(props))
}

#[must_use]
pub fn CommaSeparatedSignals(props: &InputProps) -> String {
    let values = NormalizeCommaSeparatedValues(&props.value)
        .into_iter()
        .map(|value| format!("\"{}\"", js_string_content(&value)))
        .collect::<Vec<_>>()
        .join(",");
    format!(
        "{{ {}: [{}], {}: '', {}: '{}' }}",
        CommaSeparatedValuesSignal(props),
        values,
        CommaSeparatedDraftSignal(props),
        CommaSeparatedCSVSignal(props),
        js_string_content(&CommaSeparatedValueString(&props.value)),
    )
}

#[must_use]
pub fn CommaSeparatedRootAttributes(props: &InputProps) -> BTreeMap<String, String> {
    let mut attrs = BTreeMap::from([
        ("data-comma-token-input".to_string(), "true".to_string()),
        ("data-signals".to_string(), CommaSeparatedSignals(props)),
        (
            "data-effect".to_string(),
            format!(
                "${} = ({}).join(', ')",
                CommaSeparatedCSVSignal(props),
                commaSeparatedNormalizedValuesExpr(props),
            ),
        ),
    ]);
    if props.disabled || props.read_only {
        attrs.insert("data-comma-token-locked".to_string(), "true".to_string());
    }
    attrs
}

#[must_use]
pub fn CommaSeparatedHiddenAttributes(props: &InputProps) -> BTreeMap<String, String> {
    let mut attrs = props.build_attributes();
    attrs.insert("data-bind".to_string(), CommaSeparatedCSVSignal(props));
    attrs
}

#[must_use]
pub fn CommaSeparatedShellAttributes(props: &InputProps) -> BTreeMap<String, String> {
    BTreeMap::from([
        ("data-comma-token-shell".to_string(), "true".to_string()),
        (
            "data-on:click".to_string(),
            commaSeparatedShellClickExpr(props),
        ),
    ])
}

#[must_use]
pub fn CommaSeparatedListAttributes(props: &InputProps) -> BTreeMap<String, String> {
    BTreeMap::from([
        ("data-comma-token-list".to_string(), "true".to_string()),
        (
            "data-effect".to_string(),
            commaSeparatedRenderEffectExpr(props),
        ),
    ])
}

#[must_use]
pub fn CommaSeparatedEntryAttributes(props: &InputProps) -> BTreeMap<String, String> {
    let mut attrs = BTreeMap::from([
        ("data-comma-token-entry".to_string(), "true".to_string()),
        ("data-bind".to_string(), CommaSeparatedDraftSignal(props)),
        (
            "data-on:keydown".to_string(),
            commaSeparatedEntryKeydownExpr(props),
        ),
        (
            "data-on:input".to_string(),
            commaSeparatedEntryInputExpr(props),
        ),
        (
            "data-on:blur".to_string(),
            commaSeparatedEntryBlurExpr(props),
        ),
        ("autocomplete".to_string(), "off".to_string()),
        ("spellcheck".to_string(), "false".to_string()),
    ]);
    if props.error && !props.get_error_id().is_empty() {
        attrs.insert("aria-describedby".to_string(), props.get_error_id());
    }
    if props.error {
        attrs.insert("aria-invalid".to_string(), "true".to_string());
    }
    attrs
}

#[must_use]
pub fn commaSeparatedBaseID(props: &InputProps) -> String {
    let id = props.id.trim();
    if !id.is_empty() {
        return id.to_string();
    }
    let name = props.name.trim();
    if !name.is_empty() {
        return name.to_string();
    }
    "comma-values".to_string()
}

#[must_use]
pub fn commaSeparatedSignalStem(props: &InputProps) -> String {
    let mut result = String::new();
    for ch in commaSeparatedBaseID(props).chars() {
        if ch.is_ascii_alphanumeric() {
            result.push(ch.to_ascii_lowercase());
        } else {
            result.push('_');
        }
    }
    while result.contains("__") {
        result = result.replace("__", "_");
    }
    let trimmed = result.trim_matches('_').to_string();
    let mut stem = if trimmed.is_empty() {
        "comma_values".to_string()
    } else {
        trimmed
    };
    if stem.as_bytes().first().is_some_and(u8::is_ascii_digit) {
        stem = format!("csv_{stem}");
    }
    stem
}

#[must_use]
pub fn commaSeparatedShellClickExpr(props: &InputProps) -> String {
    let values_signal = format!("${}", CommaSeparatedValuesSignal(props));
    let values_expr = commaSeparatedNormalizedValuesExpr(props);
    format!(
        "if (el.dataset.commaTokenLocked === 'true') {{ return; }}\nconst remove = evt.target instanceof HTMLElement ? evt.target.closest('[data-comma-token-remove=\"true\"]') : null;\nif (remove instanceof HTMLButtonElement) {{\n  evt.preventDefault();\n  const index = Number(remove.dataset.commaTokenIndex || '-1');\n  if (index >= 0) {{\n    {values_signal} = ({values_expr}).filter((_, itemIndex) => itemIndex !== index);\n  }}\n  return;\n}}\nif (!(evt.target instanceof HTMLElement && evt.target.closest('[data-comma-token-entry=\"true\"]'))) {{\n  const entry = el.querySelector('[data-comma-token-entry=\"true\"]');\n  if (entry instanceof HTMLInputElement) {{\n    entry.focus();\n  }}\n}}"
    )
}

#[must_use]
pub fn commaSeparatedEntryKeydownExpr(props: &InputProps) -> String {
    let values_signal = format!("${}", CommaSeparatedValuesSignal(props));
    let draft_signal = format!("${}", CommaSeparatedDraftSignal(props));
    let values_expr = commaSeparatedNormalizedValuesExpr(props);
    let commit = commaSeparatedCommitExpr(props, "evt.target.value");
    format!(
        "if ((evt.key === 'Enter' || evt.key === ',') && !evt.isComposing) {{\n  evt.preventDefault();\n  {commit}\n}} else if (evt.key === 'Backspace' && String({draft_signal} || '').trim() === '' && ({values_expr}).length > 0) {{\n  {values_signal} = ({values_expr}).slice(0, -1);\n}}"
    )
}

#[must_use]
pub fn commaSeparatedEntryInputExpr(props: &InputProps) -> String {
    format!(
        "if (/[,\\n]/.test(String(evt.target.value || ''))) {{ {} }}",
        commaSeparatedCommitExpr(props, "evt.target.value")
    )
}

#[must_use]
pub fn commaSeparatedEntryBlurExpr(props: &InputProps) -> String {
    commaSeparatedCommitExpr(props, "evt.target.value")
}

#[must_use]
pub fn commaSeparatedCommitExpr(props: &InputProps, source_expr: &str) -> String {
    let values_signal = format!("${}", CommaSeparatedValuesSignal(props));
    let draft_signal = format!("${}", CommaSeparatedDraftSignal(props));
    let values_expr = commaSeparatedNormalizedValuesExpr(props);
    format!(
        "const raw = String({source_expr} || '').trim();\nif (raw === '') {{\n  {draft_signal} = '';\n}} else {{\n  const next = raw.split(/[,\\n]/g).map((value) => value.trim()).filter((value) => value !== '');\n  if (next.length === 0) {{\n    {draft_signal} = '';\n  }} else {{\n    const seen = new Set({values_expr});\n    const additions = next.filter((value) => {{\n      if (seen.has(value)) {{\n        return false;\n      }}\n      seen.add(value);\n      return true;\n    }});\n    {values_signal} = [...({values_expr}), ...additions];\n    {draft_signal} = '';\n  }}\n}}"
    )
}

#[must_use]
pub fn commaSeparatedRenderEffectExpr(props: &InputProps) -> String {
    let values_expr = commaSeparatedNormalizedValuesExpr(props);
    format!(
        "const root = el.closest('[data-comma-token-input=\"true\"]');\nconst template = root instanceof HTMLElement ? root.querySelector('template[data-comma-token-template=\"true\"]') : null;\nif (!(root instanceof HTMLElement) || !(template instanceof HTMLTemplateElement)) {{\n  return;\n}}\nel.replaceChildren();\nconst values = {values_expr};\nfor (let index = 0; index < values.length; index += 1) {{\n  const value = values[index];\n  const fragment = template.content.cloneNode(true);\n  const label = fragment.querySelector('[data-comma-token-label=\"true\"]');\n  const remove = fragment.querySelector('[data-comma-token-remove=\"true\"]');\n  if (label instanceof HTMLElement) {{\n    label.textContent = value;\n  }}\n  if (remove instanceof HTMLButtonElement) {{\n    remove.dataset.commaTokenIndex = String(index);\n    remove.dataset.commaTokenValue = value;\n    const templateText = root.dataset.commaTokenRemoveLabelTemplate || 'Remove %s';\n    remove.setAttribute('aria-label', templateText.replace('%s', value));\n  }}\n  el.appendChild(fragment);\n}}\nroot.dataset.commaTokenEmpty = values.length === 0 ? 'true' : 'false';"
    )
}

#[must_use]
pub fn commaSeparatedNormalizedValuesExpr(props: &InputProps) -> String {
    let values_signal = format!("${}", CommaSeparatedValuesSignal(props));
    format!(
        "(Array.isArray({0}) ? {0} : String({0} || '').split(',').map((value) => value.trim()).filter((value) => value !== ''))",
        values_signal
    )
}

fn js_string_content(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('\'', "\\'")
        .replace('"', "\\\"")
}

#[must_use]
pub fn CommaSeparatedInputWithLabel(
    props: &InputProps,
    label: impl Into<String>,
    helper_text: impl Into<String>,
) -> Markup {
    let label = label.into();
    let helper_text = helper_text.into();
    let root_attrs = CommaSeparatedRootAttributes(props);
    let hidden_attrs = CommaSeparatedHiddenAttributes(props);
    let shell_attrs = CommaSeparatedShellAttributes(props);
    let list_attrs = CommaSeparatedListAttributes(props);
    let entry_attrs = CommaSeparatedEntryAttributes(props);
    crate::html! {
        div class="space-y-1.5" {
            @if !label.is_empty() {
                label for=(CommaSeparatedEntryID(props)) class=(label_classes(props)) {
                    (&label)
                    @if props.required {
                        span class="text-destructive ml-1" aria-label=(shared::default_strings().input_required_aria_label) {
                            (shared::default_strings().input_required_symbol)
                        }
                    }
                }
            }
            div
                class="space-y-2"
                data-comma-token-remove-label-template=(shared::default_strings().badge_remove_aria_label)
                ..(&root_attrs)
            {
                input
                    type="hidden"
                    name=[optional(&props.name)]
                    id=(CommaSeparatedHiddenID(props))
                    value=(CommaSeparatedValueString(&props.value))
                ..(&hidden_attrs)
                {}
                div
                    class=(CommaSeparatedContainerClasses(props))
                ..(&shell_attrs)
                {
                    div
                        class="flex flex-wrap items-center gap-2"
                ..(&list_attrs)
                    {}
                    input
                        type="text"
                        id=(CommaSeparatedEntryID(props))
                        class=(CommaSeparatedEntryClasses(props))
                        placeholder=[optional(&props.placeholder)]
                        disabled[props.disabled]
                        readonly[props.read_only]
                        aria-label=(&label)
                ..(&entry_attrs)
                    {}
                }
                template data-comma-token-template="true" {
                    span class="inline-flex items-center rounded-full bg-primary text-primary-foreground px-2 py-0.5 text-xs font-medium gap-1.5 pr-1 max-w-full" data-comma-token-chip-template="true" {
                        span class="truncate max-w-[12rem]" data-comma-token-label="true" {}
                        button type="button" class="inline-flex size-5 items-center justify-center rounded-full bg-primary-foreground/15 text-primary-foreground transition-colors hover:bg-primary-foreground/25 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary-foreground/35" data-comma-token-remove="true" aria-label=(shared::default_strings().badge_remove_aria_label) {
                            "×"
                        }
                    }
                }
            }
            @if !helper_text.is_empty() && !props.error {
                p class="text-xs text-muted-foreground" { (&helper_text) }
            }
            @if props.error && props.show_error && !props.error_message.is_empty() {
                p id=(props.get_error_id()) class="mt-1.5 text-xs text-destructive" role="alert" { (&props.error_message) }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn html_icon() -> Markup {
        html! { span class="icon" { "I" } }
    }

    #[test]
    fn input_props_helpers_cover_defaults_states_attrs_and_labels() {
        let props = InputProps::default();
        assert_eq!(props.input_type, INPUT_TYPE_TEXT);
        assert_eq!(props.size, shared::SIZE_MD);
        assert!(props.show_error);
        assert_eq!(
            props.base_classes(),
            "flex w-full border border-input bg-transparent shadow-sm placeholder:text-muted-foreground input-hover-border focus-visible:outline-none focus-visible:border-ring focus-visible:shadow-[0_0_0_2px_var(--ring)] disabled:cursor-not-allowed disabled:opacity-50 transition-[border-color,box-shadow] duration-150"
        );
        assert_eq!(props.state_classes(), "");
        assert_eq!(props.build_wrapper_classes(), "relative");
        assert!(
            props
                .build_classes()
                .contains("h-10 px-3 py-2 text-sm rounded-md")
        );
        assert!(
            props
                .build_textarea_classes()
                .contains("resize-y min-h-[80px]")
        );
        assert_eq!(props.get_error_id(), "");

        let props = InputProps::default()
            .with_type(INPUT_TYPE_NUMBER)
            .with_name("age")
            .with_id("age")
            .with_placeholder("Age")
            .with_value("42")
            .with_size(shared::SIZE_LG)
            .with_disabled(true)
            .with_read_only(true)
            .with_required(true)
            .with_error(true, "Bad")
            .with_show_error(false)
            .with_full_width(true)
            .with_class("extra")
            .with_aria_label("Age field")
            .with_aria_described_by("age-help")
            .with_aria_invalid(true)
            .with_min("1")
            .with_max("100")
            .with_step("1")
            .with_pattern("[0-9]+")
            .with_autocomplete("off")
            .with_attribute("data-testid", "age-input");
        assert_eq!(props.get_error_id(), "age-error");
        assert_eq!(
            props.state_classes(),
            "border-2 border-destructive focus-visible:shadow-[0_0_0_2px_var(--destructive)] animate-shake"
        );
        assert!(
            props
                .build_classes()
                .contains("h-12 px-4 py-3 text-base rounded-lg")
        );
        assert!(props.build_classes().contains("w-full extra"));
        assert_eq!(
            label_classes(&props),
            "text-sm font-medium leading-none peer-disabled:cursor-not-allowed peer-disabled:opacity-70 text-destructive"
        );
        let attrs = props.build_attributes();
        assert_eq!(shared::attr_value(&attrs, "min"), Some("1"));
        assert_eq!(shared::attr_value(&attrs, "max"), Some("100"));
        assert_eq!(shared::attr_value(&attrs, "step"), Some("1"));
        assert_eq!(shared::attr_value(&attrs, "pattern"), Some("[0-9]+"));
        assert_eq!(shared::attr_value(&attrs, "autocomplete"), Some("off"));
        assert_eq!(shared::attr_value(&attrs, "aria-label"), Some("Age field"));
        assert_eq!(
            shared::attr_value(&attrs, "aria-describedby"),
            Some("age-help")
        );
        assert_eq!(shared::attr_value(&attrs, "aria-invalid"), Some("true"));
        assert_eq!(shared::attr_value(&attrs, "data-testid"), Some("age-input"));

        let date_props = DatePickerPropsFromInput(&props);
        assert_eq!(date_props.name, "age");
        assert_eq!(date_props.id, "age");
        assert_eq!(date_props.placeholder, "Age");
        assert_eq!(date_props.value, "42");
        assert_eq!(date_props.size, shared::SIZE_LG);
        assert!(date_props.disabled);
        assert!(date_props.read_only);
        assert!(date_props.required);
        assert!(date_props.error);
        assert!(date_props.show_error);
        assert!(date_props.full_width);
        assert_eq!(date_props.class, "extra");
        assert_eq!(date_props.aria_label, "Age field");
        assert_eq!(date_props.aria_described_by, "age-help");
        assert!(date_props.aria_invalid);
        assert_eq!(date_props.min, "1");
        assert_eq!(date_props.max, "100");
        assert_eq!(
            date_props.attributes.get("data-testid").map(String::as_str),
            Some("age-input")
        );

        let attr_id_props = DatePickerPropsFromInput(
            &InputProps::default()
                .with_name("birthdate")
                .with_attribute("id", "birthdate-id"),
        );
        assert_eq!(attr_id_props.id, "birthdate-id");
        assert_eq!(attr_id_props.name, "birthdate");

        let success = InputProps::default().with_success(true);
        assert_eq!(
            success.state_classes(),
            "border-2 border-success focus-visible:shadow-[0_0_0_2px_var(--success)]"
        );
    }

    #[test]
    fn input_textarea_and_labeled_variations_render_real_html() {
        let props = InputProps::default()
            .with_type(INPUT_TYPE_EMAIL)
            .with_id("email")
            .with_name("email")
            .with_placeholder("<Email>")
            .with_value("a@example.com")
            .with_required(true)
            .with_error(true, "<Bad>")
            .with_autocomplete("email")
            .with_attribute("data-on:input", "$email = evt.target.value");
        let rendered = Input(&props).into_string();
        assert!(rendered.starts_with("<div class=\"relative\">"));
        assert!(rendered.contains("type=\"email\""));
        assert!(rendered.contains("name=\"email\""));
        assert!(rendered.contains("placeholder=\"&lt;Email&gt;\""));
        assert!(rendered.contains("value=\"a@example.com\""));
        assert!(rendered.contains("required"));
        assert!(rendered.contains("autocomplete=\"email\""));
        assert!(rendered.contains("aria-describedby=\"email-error\""));
        assert!(rendered.contains("data-on:input=\"$email = evt.target.value\""));
        assert!(rendered.contains("<p id=\"email-error\" class=\"mt-1.5 text-xs text-destructive\" role=\"alert\">&lt;Bad&gt;</p>"));

        let labeled = InputWithLabel(&props, "<Email>", "hidden while error").into_string();
        assert!(labeled.contains("<label for=\"email\" class=\"text-sm font-medium"));
        assert!(labeled.contains("&lt;Email&gt;"));
        assert!(labeled.contains("aria-label=\"required\">*</span>"));
        assert!(!labeled.contains("hidden while error"));

        let textarea_props = InputProps::default()
            .with_id("bio")
            .with_name("bio")
            .with_placeholder("Bio")
            .with_value("<hello>")
            .with_required(true)
            .with_full_width(true)
            .with_attribute("data-bind", "bio");
        let textarea = TextAreaWithLabel(&textarea_props, "Bio", "Help", 5).into_string();
        assert!(textarea.contains("<textarea name=\"bio\" id=\"bio\" placeholder=\"Bio\""));
        assert!(textarea.contains(r#"aria-label="required">*"#));
        assert!(textarea.contains("rows=\"5\""));
        assert!(textarea.contains("data-bind=\"bio\""));
        assert!(textarea.contains("&lt;hello&gt;</textarea>"));
        assert!(textarea.contains("<p class=\"text-xs text-muted-foreground\">Help</p>"));

        let textarea_error = TextArea(
            &InputProps::default()
                .with_id("bio-error")
                .with_error(true, "Bad bio"),
            3,
        )
        .into_string();
        assert!(textarea_error.contains(r#"id="bio-error-error""#));
        assert!(textarea_error.contains("Bad bio"));
    }

    #[test]
    fn floating_icon_password_and_convenience_inputs_render_expected_html() {
        let props = InputProps::default()
            .with_id("pwd")
            .with_name("pwd")
            .with_required(true)
            .with_error(true, "Required")
            .with_class("custom")
            .with_autocomplete("current-password");
        assert_eq!(
            floatingGroupClasses(&props),
            "floating-input-group is-invalid"
        );
        assert_eq!(
            floatingInputClasses(&props),
            "floating-input-field pl-3 custom"
        );
        assert_eq!(
            floatingInputWithIconClasses(&props),
            "floating-input-field pl-11 custom"
        );
        assert_eq!(passwordSignalInit("showPwd"), "{ showPwd: false }");
        assert_eq!(
            passwordInputAttributes(&props, "showPwd")
                .get("data-attr:type")
                .map(String::as_str),
            Some("$showPwd ? 'text' : 'password'")
        );

        let floating = FloatingInput(&props, "Password", "").into_string();
        assert!(floating.contains("class=\"floating-input-group is-invalid\""));
        assert!(floating.contains("placeholder=\" \""));
        assert!(floating.contains("class=\"floating-input-label\""));
        assert!(floating.contains("role=\"alert\">Required</p>"));

        let with_icon = FloatingInputWithIcon(&props, "Password", "", html_icon()).into_string();
        assert!(
            with_icon
                .contains("<div class=\"floating-input-icon\"><span class=\"icon\">I</span></div>")
        );
        assert!(with_icon.contains("style=\"left: 2.75rem;\""));

        let password = FloatingPasswordInput(&props, "Password", "", "showPwd").into_string();
        assert!(password.contains("data-signals=\"{ showPwd: false }\""));
        assert!(password.contains("data-attr:type="));
        assert!(password.contains("$showPwd ?"));
        assert!(password.contains("text"));
        assert!(password.contains("password"));
        assert!(password.contains("data-on:click=\"$showPwd = !$showPwd\""));
        assert!(password.contains("data-show=\"!$showPwd\""));
        assert!(password.contains("data-show=\"$showPwd\""));

        assert!(
            SearchInput("q", "q", "Search", shared::SIZE_SM)
                .into_string()
                .contains("type=\"search\"")
        );
        assert!(
            EmailInput("email", "email", "Email", true)
                .into_string()
                .contains("autocomplete=\"email\"")
        );
        assert!(
            PasswordInput("password", "password", "Password", false)
                .into_string()
                .contains("type=\"password\"")
        );
        let number = NumberInput("age", "age", "Age", "1", "10", "1").into_string();
        assert!(number.contains("type=\"number\""));
        assert!(number.contains("min=\"1\""));
        assert!(number.contains("max=\"10\""));
        assert!(number.contains("step=\"1\""));
        assert!(
            URLInput("site", "site", "Site")
                .into_string()
                .contains("https://")
        );
        assert!(
            TelInput("phone", "phone", "Phone")
                .into_string()
                .contains("type=\"tel\"")
        );
    }

    #[test]
    fn icon_input_variations_render_left_right_and_errors() {
        let props = InputProps::default()
            .with_id("search")
            .with_name("search")
            .with_placeholder("Search")
            .with_error(true, "Bad");
        let left = InputWithIcon(&props, "left", html_icon()).into_string();
        assert!(left.contains("absolute left-3 top-1/2"));
        assert!(left.contains("pl-10"));
        assert!(left.contains("id=\"search-error\""));

        let right = InputWithIcon(&props, "right", html_icon()).into_string();
        assert!(right.contains("pr-10"));
        assert!(right.contains("absolute right-3 top-1/2"));
    }

    #[test]
    fn comma_separated_helpers_cover_normalization_signals_attrs_and_expressions() {
        let props = InputProps::default()
            .with_id("Team Tags")
            .with_name("tags")
            .with_value(" alpha, beta, alpha, ,gamma ")
            .with_placeholder("Add tag")
            .with_size(shared::SIZE_SM)
            .with_full_width(true)
            .with_error(true, "Bad")
            .with_read_only(true);
        assert_eq!(
            NormalizeCommaSeparatedValues(" a, b, a,, c "),
            vec!["a", "b", "c"]
        );
        assert_eq!(CommaSeparatedValueString(" a, b, a,, c "), "a, b, c");
        assert_eq!(CommaSeparatedEntryID(&props), "Team Tags-entry");
        assert_eq!(CommaSeparatedHiddenID(&props), "Team Tags-hidden");
        assert_eq!(commaSeparatedBaseID(&props), "Team Tags");
        assert_eq!(commaSeparatedSignalStem(&props), "team_tags");
        assert_eq!(
            commaSeparatedBaseID(&InputProps::default().with_name("Only Name")),
            "Only Name"
        );
        assert_eq!(
            commaSeparatedSignalStem(&InputProps::default().with_id("A---B")),
            "a_b"
        );
        assert_eq!(
            commaSeparatedSignalStem(&InputProps::default().with_id("!!!")),
            "comma_values"
        );
        assert_eq!(CommaSeparatedValuesSignal(&props), "team_tags_values");
        assert_eq!(CommaSeparatedDraftSignal(&props), "team_tags_draft");
        assert_eq!(CommaSeparatedCSVSignal(&props), "team_tags_csv");
        assert!(CommaSeparatedContainerClasses(&props).contains("w-full"));
        assert!(CommaSeparatedContainerClasses(&props).contains("border-destructive"));
        assert!(CommaSeparatedEntryClasses(&props).contains("text-xs"));
        assert!(
            CommaSeparatedSignals(&props)
                .contains("team_tags_values: [\"alpha\",\"beta\",\"gamma\"]")
        );
        assert!(CommaSeparatedSignals(&props).contains("team_tags_csv: 'alpha, beta, gamma'"));
        assert_eq!(
            CommaSeparatedRootAttributes(&props)
                .get("data-comma-token-locked")
                .map(String::as_str),
            Some("true")
        );
        assert_eq!(
            CommaSeparatedHiddenAttributes(&props)
                .get("data-bind")
                .map(String::as_str),
            Some("team_tags_csv")
        );
        assert_eq!(
            CommaSeparatedShellAttributes(&props)
                .get("data-comma-token-shell")
                .map(String::as_str),
            Some("true")
        );
        assert_eq!(
            CommaSeparatedListAttributes(&props)
                .get("data-comma-token-list")
                .map(String::as_str),
            Some("true")
        );
        assert_eq!(
            CommaSeparatedEntryAttributes(&props)
                .get("data-bind")
                .map(String::as_str),
            Some("team_tags_draft")
        );
        assert_eq!(
            CommaSeparatedEntryAttributes(&props)
                .get("aria-invalid")
                .map(String::as_str),
            Some("true")
        );
        assert!(commaSeparatedShellClickExpr(&props).contains("data-comma-token-remove"));
        assert!(commaSeparatedEntryKeydownExpr(&props).contains("evt.key === 'Enter'"));
        assert!(commaSeparatedEntryInputExpr(&props).contains("/[,\\n]/"));
        assert!(
            commaSeparatedEntryBlurExpr(&props).contains("const raw = String(evt.target.value")
        );
        assert!(
            commaSeparatedCommitExpr(&props, "source")
                .contains("const raw = String(source || '').trim();")
        );
        assert!(
            commaSeparatedRenderEffectExpr(&props).contains("template[data-comma-token-template")
        );
        assert!(commaSeparatedNormalizedValuesExpr(&props).contains("$team_tags_values"));

        let numeric = InputProps::default().with_id("123 start");
        assert_eq!(commaSeparatedSignalStem(&numeric), "csv_123_start");
        let fallback = InputProps::default();
        assert_eq!(commaSeparatedSignalStem(&fallback), "comma_values");
    }

    #[test]
    fn comma_separated_component_renders_all_interactive_datastar_parts() {
        let props = InputProps::default()
            .with_name("tags")
            .with_id("tags")
            .with_value("red, blue")
            .with_placeholder("Add")
            .with_required(true);
        let rendered =
            CommaSeparatedInputWithLabel(&props, "Tags", "Comma separated").into_string();
        assert!(rendered.starts_with("<div class=\"space-y-1.5\">"));
        assert!(rendered.contains("<label for=\"tags-entry\""));
        assert!(rendered.contains("aria-label=\"required\">*</span>"));
        assert!(rendered.contains("data-comma-token-input=\"true\""));
        assert!(rendered.contains("data-signals=\"{ tags_values: ["));
        assert!(rendered.contains("data-effect=\"$tags_csv = "));
        assert!(rendered.contains("<input type=\"hidden\" name=\"tags\" id=\"tags-hidden\" value=\"red, blue\" data-bind=\"tags_csv\""));
        assert!(rendered.contains("data-comma-token-shell=\"true\""));
        assert!(rendered.contains("data-on:click=\"if (el.dataset.commaTokenLocked"));
        assert!(rendered.contains("data-comma-token-list=\"true\""));
        assert!(rendered.contains("data-comma-token-entry=\"true\""));
        assert!(rendered.contains("data-bind=\"tags_draft\""));
        assert!(rendered.contains("data-on:keydown=\"if ((evt.key ==="));
        assert!(rendered.contains("Enter"));
        assert!(rendered.contains("<template data-comma-token-template=\"true\">"));
        assert!(rendered.contains("data-comma-token-chip-template=\"true\""));
        assert!(rendered.contains("data-comma-token-label=\"true\""));
        assert!(rendered.contains("data-comma-token-remove=\"true\""));
        assert!(
            rendered.contains("<p class=\"text-xs text-muted-foreground\">Comma separated</p>")
        );

        let error = CommaSeparatedInputWithLabel(
            &InputProps::default()
                .with_id("tags")
                .with_error(true, "Invalid tags"),
            "Tags",
            "Hidden",
        )
        .into_string();
        assert!(error.contains(r#"id="tags-error""#));
        assert!(error.contains("Invalid tags"));
        assert!(!error.contains("Hidden"));
    }
}

/// Reusable command-key search input and shortcut state.
#[derive(Clone,Debug,Default)]
pub struct CommandSearchInputProps{
    pub id:String,pub name:String,pub value:String,pub placeholder:String,pub aria_label:String,pub bind_signal:String,
    pub size:&'static str,pub shortcut_label:String,pub prefix:String,pub enable_global_shortcut:bool,pub full_width:bool,
    pub class:String,pub input_class:String,pub shortcut_class:String,pub attributes:BTreeMap<String,String>,pub input_attributes:BTreeMap<String,String>,
}
pub fn CommandSearchInputID(p:&CommandSearchInputProps)->String{
    if !p.id.trim().is_empty(){p.id.trim().into()}else if !p.name.trim().is_empty(){format!("{}-search",p.name.trim())}else{"command-search".into()}
}
pub fn CommandSearchInputAttributes(p:&CommandSearchInputProps)->BTreeMap<String,String>{
    let mut attrs=p.input_attributes.clone();attrs.insert("aria-keyshortcuts".into(),"Control+K Meta+K".into());
    if !p.bind_signal.trim().is_empty(){attrs.insert("data-bind".into(),p.bind_signal.trim().into());}
    attrs.entry("data-on:keydown".into()).or_insert_with(||"if (evt.key === 'Enter') { evt.preventDefault(); evt.currentTarget.form?.requestSubmit(); }".into());
    if p.enable_global_shortcut{
        let id=CommandSearchInputID(p).replace('\\',"\\\\").replace('\'',"\\'").replace('\n',"\\n").replace('\r',"\\r");
        attrs.insert("data-on:keydown__window".into(),format!("if ((evt.ctrlKey || evt.metaKey) && evt.key && evt.key.toLowerCase() === 'k') {{ evt.preventDefault(); document.getElementById('{id}')?.focus(); }}"));
    }attrs
}
pub fn CommandSearchInput(p:&CommandSearchInputProps)->Markup{
    let attrs=CommandSearchInputAttributes(p);
    let size=if matches!(p.size,shared::SIZE_LG|shared::SIZE_XL){"h-12 text-sm"}else{"h-10 text-sm"};
    let classes=shared::build_classes(&[size,"w-full min-w-0 rounded-xl border border-[var(--control-border,var(--input))] bg-card pl-10 pr-10 font-mono text-foreground caret-primary shadow-sm transition-[border-color,box-shadow,background-color] duration-150 placeholder:text-muted-foreground hover:border-ring focus:border-ring focus:outline-none focus:ring-0 focus:shadow-[0_0_0_2px_var(--ring)] focus-visible:border-ring focus-visible:outline-none focus-visible:ring-0 focus-visible:shadow-[0_0_0_2px_var(--ring)] disabled:cursor-not-allowed disabled:opacity-50 sm:pr-28",&p.input_class]);
    crate::html!{div class=(shared::build_classes(&["group relative min-w-0",if p.full_width{"w-full"}else{"max-w-xl"},&p.class])) ..(&p.attributes) {
        span aria-hidden="true" class="pointer-events-none absolute left-4 top-1/2 -translate-y-1/2 select-none font-mono text-lg font-bold leading-none text-primary" {(if p.prefix.trim().is_empty(){">"}else{p.prefix.trim()})}
        input type="search" name=[optional(&p.name)] id=(CommandSearchInputID(p)) value=[optional(&p.value)] placeholder=[optional(&p.placeholder)] class=(classes) autocomplete="off" aria-label=(if p.aria_label.is_empty(){"Search"}else{&p.aria_label}) ..(&attrs);
        div class="pointer-events-none absolute right-3 top-1/2 -translate-y-1/2 flex items-center gap-1" {
            kbd aria-hidden="true" class=(shared::build_classes(&["hidden max-w-[7.25rem] select-none items-center justify-center rounded-md border border-[var(--control-border,var(--input))] bg-muted px-2 py-0.5 font-mono text-xs leading-5 text-muted-foreground shadow-sm sm:inline-flex",&p.shortcut_class])) {(if p.shortcut_label.trim().is_empty(){"Ctrl/Cmd+K"}else{p.shortcut_label.trim()})}
        }
    }}
}
