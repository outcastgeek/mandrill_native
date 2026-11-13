#![allow(non_snake_case)]

use crate::components::{ComponentInfo, shared};
use maud::{Markup, html};
use std::collections::BTreeMap;

pub const COMPONENTS: &[ComponentInfo] = &[
    ComponentInfo {
        family: "unstyled",
        module: "autocomplete",
        name: "Autocomplete",
    },
    ComponentInfo {
        family: "unstyled",
        module: "autocomplete",
        name: "AutocompleteWithLabel",
    },
    ComponentInfo {
        family: "unstyled",
        module: "autocomplete",
        name: "AutocompleteWithDatastar",
    },
    ComponentInfo {
        family: "unstyled",
        module: "autocomplete",
        name: "AutocompleteWithDatastarAndLabel",
    },
];

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Option {
    pub label: String,
    pub value: String,
    pub description: String,
    pub disabled: bool,
}

impl Option {
    #[must_use]
    pub fn new(label: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            value: value.into(),
            ..Self::default()
        }
    }
    #[must_use]
    pub fn with_description(mut self, value: impl Into<String>) -> Self {
        self.description = value.into();
        self
    }
    #[must_use]
    pub const fn with_disabled(mut self, value: bool) -> Self {
        self.disabled = value;
        self
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AutocompleteProps {
    pub name: String,
    pub id: String,
    pub placeholder: String,
    pub value: String,
    pub datalist_id: String,
    pub options: Vec<Option>,
    pub size: &'static str,
    pub disabled: bool,
    pub required: bool,
    pub error: bool,
    pub error_message: String,
    pub show_error: bool,
    pub full_width: bool,
    pub class: String,
    pub attributes: BTreeMap<String, String>,
    pub aria_label: String,
    pub aria_described_by: String,
    pub aria_invalid: bool,
    pub hidden_field_name: String,
    pub hidden_field_id: String,
}

impl Default for AutocompleteProps {
    fn default() -> Self {
        Self {
            name: String::new(),
            id: String::new(),
            placeholder: String::new(),
            value: String::new(),
            datalist_id: String::new(),
            options: Vec::new(),
            size: shared::SIZE_MD,
            disabled: false,
            required: false,
            error: false,
            error_message: String::new(),
            show_error: true,
            full_width: false,
            class: String::new(),
            attributes: BTreeMap::new(),
            aria_label: String::new(),
            aria_described_by: String::new(),
            aria_invalid: false,
            hidden_field_name: String::new(),
            hidden_field_id: String::new(),
        }
    }
}

impl AutocompleteProps {
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
    pub fn with_datalist_id(mut self, value: impl Into<String>) -> Self {
        self.datalist_id = value.into();
        self
    }
    #[must_use]
    pub fn with_options(mut self, value: Vec<Option>) -> Self {
        self.options = value;
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
    pub fn with_hidden_field(mut self, name: impl Into<String>, id: impl Into<String>) -> Self {
        self.hidden_field_name = name.into();
        self.hidden_field_id = id.into();
        self
    }

    #[must_use]
    pub fn build_classes(&self) -> String {
        shared::build_classes(&[
            "w-full rounded-md border border-input bg-background px-3 py-2 text-sm ring-offset-background placeholder:text-muted-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 disabled:cursor-not-allowed disabled:opacity-50 transition-all duration-150",
            shared::input_size_classes(self.size),
            if self.error {
                "border-destructive focus-visible:ring-destructive"
            } else {
                ""
            },
            if self.full_width { "w-full" } else { "" },
            self.class.as_str(),
        ])
    }

    #[must_use]
    pub fn build_wrapper_classes(&self) -> String {
        shared::build_classes(&[
            "relative inline-flex items-center",
            if self.full_width { "w-full" } else { "" },
        ])
    }

    #[must_use]
    pub fn get_datalist_id(&self) -> String {
        if !self.datalist_id.is_empty() {
            self.datalist_id.clone()
        } else if !self.id.is_empty() {
            format!("{}List", self.id)
        } else {
            format!("{}List", self.name)
        }
    }

    #[must_use]
    pub fn get_hidden_field_id(&self) -> String {
        if !self.hidden_field_id.is_empty() {
            self.hidden_field_id.clone()
        } else if !self.id.is_empty() {
            format!("{}Value", self.id)
        } else {
            format!("{}Value", self.name)
        }
    }

    #[must_use]
    pub fn get_error_id(&self) -> String {
        if self.id.is_empty() {
            String::new()
        } else {
            format!("{}-error", self.id)
        }
    }

    #[must_use]
    pub fn build_attributes(&self) -> BTreeMap<String, String> {
        let mut attrs = self.attributes.clone();
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
}

fn optional(value: &str) -> std::option::Option<&str> {
    if value.is_empty() { None } else { Some(value) }
}

fn attr<'a>(attrs: &'a BTreeMap<String, String>, name: &str) -> std::option::Option<&'a str> {
    shared::attr_value(attrs, name)
}

fn described_by<'a>(
    props: &'a AutocompleteProps,
    attrs: &'a BTreeMap<String, String>,
) -> std::option::Option<String> {
    if props.error && !props.get_error_id().is_empty() {
        Some(props.get_error_id())
    } else {
        attr(attrs, "aria-describedby").map(str::to_string)
    }
}

#[must_use]
pub fn buildOptionsMapSignal(signal_name: &str, options: &[Option]) -> String {
    let entries = options
        .iter()
        .filter(|option| !option.disabled && !option.value.is_empty())
        .map(|option| {
            format!(
                "\"{}\":\"{}\"",
                json_escape(&option.label),
                json_escape(&option.value)
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    format!("{{ {signal_name}: {{{entries}}} }}")
}

fn json_escape(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
}

#[must_use]
pub fn Autocomplete(props: &AutocompleteProps) -> Markup {
    render_autocomplete(props, None)
}

#[must_use]
pub fn AutocompleteWithDatastar(props: &AutocompleteProps, signal_name: &str) -> Markup {
    render_autocomplete(props, Some(signal_name))
}

fn render_autocomplete(
    props: &AutocompleteProps,
    signal_name: std::option::Option<&str>,
) -> Markup {
    let attrs = props.build_attributes();
    let input_expr =
        signal_name.map(|name| format!("${name} = ${name}Map[evt.target.value] || ''"));
    let signal_map =
        signal_name.map(|name| buildOptionsMapSignal(&format!("{name}Map"), &props.options));
    html! {
        div class=(props.build_wrapper_classes()) data-signals=[signal_map] {
            input
                type="text"
                name=[optional(&props.name)]
                id=[optional(&props.id)]
                placeholder=[optional(&props.placeholder)]
                value=[optional(&props.value)]
                list=(props.get_datalist_id())
                class=(props.build_classes())
                disabled[props.disabled]
                required[props.required]
                autocomplete="off"
                aria-label=[attr(&attrs, "aria-label")]
                aria-describedby=[described_by(props, &attrs)]
                aria-invalid=[attr(&attrs, "aria-invalid")]
                data-testid=[attr(&attrs, "data-testid")]
                data-on:input=[input_expr.clone()]
                data-on:change=[input_expr]
            {}
            datalist id=(props.get_datalist_id()) {
                @for option in &props.options {
                    @if !option.disabled {
                        option value=(&option.label) data-value=[if signal_name.is_none() { optional(&option.value) } else { None }] {}
                    }
                }
            }
            @if !props.hidden_field_name.is_empty() {
                input
                    type="hidden"
                    name=(&props.hidden_field_name)
                    id=(props.get_hidden_field_id())
                    required[props.required]
                    data-bind=[signal_name]
                {}
            }
        }
        @if props.error && props.show_error && !props.error_message.is_empty() {
            p id=(props.get_error_id()) class="mt-1 text-sm text-destructive" role="alert" {
                (&props.error_message)
            }
        }
    }
}

#[must_use]
pub fn AutocompleteWithLabel(
    props: &AutocompleteProps,
    label: impl Into<String>,
    helper_text: impl Into<String>,
) -> Markup {
    let label = label.into();
    let helper_text = helper_text.into();
    html! {
        div class="space-y-2" {
            @if !label.is_empty() {
                label for=[optional(&props.id)] class="text-sm font-medium leading-none peer-disabled:cursor-not-allowed peer-disabled:opacity-70" {
                    (&label)
                    @if props.required {
                        span class="text-destructive ml-1" aria-label=(shared::default_strings().input_required_aria_label) {
                            (shared::default_strings().input_required_symbol)
                        }
                    }
                }
            }
            (Autocomplete(props))
            @if !helper_text.is_empty() && !props.error {
                p class="text-sm text-muted-foreground" { (&helper_text) }
            }
        }
    }
}

#[must_use]
pub fn AutocompleteWithDatastarAndLabel(
    props: &AutocompleteProps,
    signal_name: &str,
    label: impl Into<String>,
    helper_text: impl Into<String>,
) -> Markup {
    let label = label.into();
    let helper_text = helper_text.into();
    html! {
        div class="space-y-2" {
            @if !label.is_empty() {
                label for=[optional(&props.id)] class="text-sm font-medium leading-none peer-disabled:cursor-not-allowed peer-disabled:opacity-70" {
                    (&label)
                    @if props.required {
                        span class="text-destructive ml-1" aria-label=(shared::default_strings().input_required_aria_label) {
                            (shared::default_strings().input_required_symbol)
                        }
                    }
                }
            }
            (AutocompleteWithDatastar(props, signal_name))
            @if !helper_text.is_empty() && !props.error {
                p class="text-sm text-muted-foreground" { (&helper_text) }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn options() -> Vec<Option> {
        vec![
            Option::new("<Alpha>", "a").with_description("First"),
            Option::new("Beta", "b").with_disabled(true),
            Option::new("Gamma", ""),
            Option::new("Quote", "a\"b"),
        ]
    }

    #[test]
    fn autocomplete_helpers_cover_defaults_ids_attrs_classes_and_json_signal() {
        let props = AutocompleteProps::default();
        assert_eq!(props.size, shared::SIZE_MD);
        assert!(props.show_error);
        assert_eq!(props.get_datalist_id(), "List");
        assert_eq!(props.get_hidden_field_id(), "Value");
        assert_eq!(props.get_error_id(), "");
        assert!(
            props
                .build_classes()
                .contains("h-10 px-3 py-2 text-sm rounded-md")
        );
        assert_eq!(
            props.build_wrapper_classes(),
            "relative inline-flex items-center"
        );

        let props = AutocompleteProps::default()
            .with_name("city")
            .with_id("city")
            .with_placeholder("City")
            .with_value("Toronto")
            .with_datalist_id("cities")
            .with_options(options())
            .with_size(shared::SIZE_LG)
            .with_disabled(true)
            .with_required(true)
            .with_error(true, "Bad")
            .with_show_error(false)
            .with_full_width(true)
            .with_class("extra")
            .with_aria_label("City")
            .with_aria_described_by("city-help")
            .with_aria_invalid(true)
            .with_hidden_field("city_id", "city-value")
            .with_attribute("data-testid", "autocomplete");
        assert_eq!(props.get_datalist_id(), "cities");
        assert_eq!(props.get_hidden_field_id(), "city-value");
        assert_eq!(props.get_error_id(), "city-error");
        assert!(
            props
                .build_classes()
                .contains("h-12 px-4 py-3 text-base rounded-lg")
        );
        assert!(
            props
                .build_classes()
                .contains("border-destructive focus-visible:ring-destructive")
        );
        assert!(props.build_classes().contains("w-full extra"));
        assert!(props.build_wrapper_classes().contains("w-full"));
        let attrs = props.build_attributes();
        assert_eq!(attr(&attrs, "aria-label"), Some("City"));
        assert_eq!(attr(&attrs, "aria-describedby"), Some("city-help"));
        assert_eq!(attr(&attrs, "aria-invalid"), Some("true"));
        assert_eq!(attr(&attrs, "data-testid"), Some("autocomplete"));

        assert_eq!(
            AutocompleteProps::default()
                .with_id("thing")
                .get_datalist_id(),
            "thingList"
        );
        assert_eq!(
            AutocompleteProps::default()
                .with_name("thing")
                .get_hidden_field_id(),
            "thingValue"
        );
        let signal = buildOptionsMapSignal("selectedMap", &options());
        assert!(signal.contains("\"<Alpha>\":\"a\""));
        assert!(signal.contains("\"Quote\":\"a\\\"b\""));
        assert!(!signal.contains("Beta"));
        assert!(!signal.contains("Gamma"));
    }

    #[test]
    fn native_autocomplete_renders_options_hidden_field_errors_and_label_variations() {
        let props = AutocompleteProps::default()
            .with_name("city")
            .with_id("city")
            .with_placeholder("<City>")
            .with_value("<Alpha>")
            .with_options(options())
            .with_required(true)
            .with_error(true, "<Bad>")
            .with_hidden_field("city_id", "")
            .with_attribute("data-testid", "auto");
        let html = Autocomplete(&props).into_string();
        assert!(html.starts_with("<div class=\"relative inline-flex items-center\">"));
        assert!(html.contains("type=\"text\""));
        assert!(html.contains("name=\"city\""));
        assert!(html.contains("id=\"city\""));
        assert!(html.contains("placeholder=\"&lt;City&gt;\""));
        assert!(html.contains("value=\"&lt;Alpha&gt;\""));
        assert!(html.contains("list=\"cityList\""));
        assert!(html.contains("required"));
        assert!(html.contains("autocomplete=\"off\""));
        assert!(html.contains("aria-describedby=\"city-error\""));
        assert!(html.contains("data-testid=\"auto\""));
        assert!(html.contains("<datalist id=\"cityList\">"));
        assert!(html.contains("<option value=\"&lt;Alpha&gt;\" data-value=\"a\"></option>"));
        assert!(!html.contains("value=\"Beta\""));
        assert!(html.contains("<option value=\"Gamma\"></option>"));
        assert!(
            html.contains("<input type=\"hidden\" name=\"city_id\" id=\"cityValue\" required>")
        );
        assert!(html.contains("<p id=\"city-error\" class=\"mt-1 text-sm text-destructive\" role=\"alert\">&lt;Bad&gt;</p>"));

        let labeled = AutocompleteWithLabel(&props, "<City>", "Hidden").into_string();
        assert!(labeled.contains("<label for=\"city\""));
        assert!(labeled.contains("&lt;City&gt;"));
        assert!(labeled.contains("aria-label=\"required\">*</span>"));
        assert!(!labeled.contains(">Hidden</p>"));

        let helper =
            AutocompleteWithLabel(&AutocompleteProps::default().with_id("ok"), "City", "Help")
                .into_string();
        assert!(helper.contains("<p class=\"text-sm text-muted-foreground\">Help</p>"));
    }

    #[test]
    fn datastar_autocomplete_renders_signal_map_value_only_bind_and_label() {
        let props = AutocompleteProps::default()
            .with_name("city")
            .with_id("city")
            .with_options(options())
            .with_hidden_field("city_id", "city-id")
            .with_required(true);
        let html = AutocompleteWithDatastar(&props, "selectedCity").into_string();
        assert!(html.contains("data-signals=\"{ selectedCityMap: {"));
        assert!(html.contains("data-on:input="));
        assert!(html.contains("data-on:change="));
        assert!(html.contains("$selectedCity = $selectedCityMap[evt.target.value]"));
        assert!(html.contains("<option value=\"&lt;Alpha&gt;\"></option>"));
        assert!(!html.contains("data-value=\"a\""));
        assert!(html.contains("<input type=\"hidden\" name=\"city_id\" id=\"city-id\" required data-bind=\"selectedCity\">"));
        assert!(!html.contains("data-bind:selectedCity"));

        let labeled =
            AutocompleteWithDatastarAndLabel(&props, "selectedCity", "City", "Help").into_string();
        assert!(labeled.starts_with("<div class=\"space-y-2\">"));
        assert!(labeled.contains("<label for=\"city\""));
        assert!(labeled.contains("data-bind=\"selectedCity\""));
        assert!(labeled.contains("<p class=\"text-sm text-muted-foreground\">Help</p>"));
    }
}
