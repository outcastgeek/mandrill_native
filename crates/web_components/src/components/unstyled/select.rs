#![allow(non_snake_case)]

use crate::components::{ComponentInfo, shared};
use maud::{Markup, html};
use std::collections::BTreeMap;

pub const COMPONENTS: &[ComponentInfo] = &[
    ComponentInfo {
        family: "unstyled",
        module: "select",
        name: "Select",
    },
    ComponentInfo {
        family: "unstyled",
        module: "select",
        name: "SelectWithLabel",
    },
];

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SelectOption {
    pub label: String,
    pub value: String,
    pub disabled: bool,
    pub selected: bool,
    pub attributes: BTreeMap<String, String>,
}

impl SelectOption {
    #[must_use]
    pub fn new(label: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            value: value.into(),
            ..Self::default()
        }
    }
    #[must_use]
    pub const fn with_disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }
    #[must_use]
    pub const fn with_selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }
    #[must_use]
    pub fn with_attribute(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.attributes.insert(name.into(), value.into());
        self
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SelectProps {
    pub name: String,
    pub id: String,
    pub value: String,
    pub placeholder: String,
    pub placeholder_disabled: bool,
    pub options: Vec<SelectOption>,
    pub size: &'static str,
    pub variant: &'static str,
    pub disabled: bool,
    pub required: bool,
    pub error: bool,
    pub error_message: String,
    pub class: String,
    pub attributes: BTreeMap<String, String>,
    pub aria_label: String,
    pub aria_described_by: String,
}

impl Default for SelectProps {
    fn default() -> Self {
        Self {
            name: String::new(),
            id: String::new(),
            value: String::new(),
            placeholder: String::new(),
            placeholder_disabled: true,
            options: Vec::new(),
            size: shared::SIZE_MD,
            variant: shared::VARIANT_DEFAULT,
            disabled: false,
            required: false,
            error: false,
            error_message: String::new(),
            class: String::new(),
            attributes: BTreeMap::new(),
            aria_label: String::new(),
            aria_described_by: String::new(),
        }
    }
}

impl SelectProps {
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
    pub fn with_value(mut self, value: impl Into<String>) -> Self {
        self.value = value.into();
        self
    }
    #[must_use]
    pub fn with_placeholder(mut self, value: impl Into<String>) -> Self {
        self.placeholder = value.into();
        self
    }
    #[must_use]
    pub const fn with_placeholder_disabled(mut self, value: bool) -> Self {
        self.placeholder_disabled = value;
        self
    }
    #[must_use]
    pub fn with_options(mut self, value: Vec<SelectOption>) -> Self {
        self.options = value;
        self
    }
    #[must_use]
    pub const fn with_size(mut self, value: &'static str) -> Self {
        self.size = value;
        self
    }
    #[must_use]
    pub const fn with_variant(mut self, value: &'static str) -> Self {
        self.variant = value;
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
    pub fn build_classes(&self) -> String {
        let size_class = match self.size {
            shared::SIZE_SM => "h-8 text-xs",
            shared::SIZE_LG => "h-12 text-lg",
            _ => "h-10",
        };
        shared::build_classes(&[
            "flex w-full items-center justify-between rounded-md border border-input bg-background px-3 py-2 text-sm ring-offset-background placeholder:text-muted-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 disabled:cursor-not-allowed disabled:opacity-50 transition-colors duration-150",
            size_class,
            if self.error {
                "border-destructive focus-visible:ring-destructive"
            } else {
                ""
            },
            self.class.as_str(),
        ])
    }

    #[must_use]
    pub const fn build_wrapper_classes(&self) -> &'static str {
        "relative w-full"
    }

    #[must_use]
    pub fn build_attributes(&self) -> BTreeMap<String, String> {
        let mut attrs = self.attributes.clone();
        if !self.id.is_empty() {
            attrs.insert("id".to_string(), self.id.clone());
        }
        if !self.name.is_empty() {
            attrs.insert("name".to_string(), self.name.clone());
        }
        if self.disabled {
            attrs.insert("disabled".to_string(), "disabled".to_string());
        }
        if self.required {
            attrs.insert("required".to_string(), "required".to_string());
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
        if self.error {
            attrs.insert("aria-invalid".to_string(), "true".to_string());
        }
        attrs
    }
}

#[must_use]
pub fn Select(props: &SelectProps) -> Markup {
    let mut attrs = props.build_attributes();
    attrs.remove("disabled");attrs.remove("required");
    crate::html! {
        div class=(props.build_wrapper_classes()) {
            select
                class=(props.build_classes())
                disabled[props.disabled]
                required[props.required]
                ..(&attrs)
            {
                @if !props.placeholder.is_empty() {
                    option value="" disabled[props.placeholder_disabled] selected[props.value.is_empty()] {
                        (&props.placeholder)
                    }
                }
                @for option in &props.options {
                    option
                        value=(&option.value)
                        disabled[option.disabled]
                        selected[option.selected || props.value == option.value]
                ..(&option.attributes)
                    {
                        (&option.label)
                    }
                }
            }
            @if props.error && !props.error_message.is_empty() {
                p class="mt-1 text-sm text-destructive" role="alert" { (&props.error_message) }
            }
        }
    }
}

#[must_use]
pub fn SelectWithLabel(
    props: &SelectProps,
    label: impl Into<String>,
    helper_text: impl Into<String>,
) -> Markup {
    let label = label.into();
    let helper_text = helper_text.into();
    html! {
        div class="space-y-2" {
            @if !label.is_empty() {
                label
                    for=[shared::optional_attr(&props.id)]
                    class="text-sm font-medium leading-none peer-disabled:cursor-not-allowed peer-disabled:opacity-70"
                {
                    (&label)
                    @if props.required {
                        span class="text-destructive ml-1" aria-label=(shared::default_strings().input_required_aria_label) {
                            (shared::default_strings().input_required_symbol)
                        }
                    }
                }
            }
            (Select(props))
            @if !helper_text.is_empty() && !props.error {
                p class="text-sm text-muted-foreground" { (&helper_text) }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_helpers_and_attributes_match_go_behavior() {
        let props = SelectProps::default();
        assert_eq!(props.size, shared::SIZE_MD);
        assert_eq!(props.variant, shared::VARIANT_DEFAULT);
        assert!(props.placeholder_disabled);
        assert_eq!(props.build_wrapper_classes(), "relative w-full");
        assert!(props.build_classes().contains("h-10"));
        assert!(
            SelectProps::default()
                .with_size(shared::SIZE_SM)
                .build_classes()
                .contains("h-8 text-xs")
        );
        assert!(
            SelectProps::default()
                .with_size(shared::SIZE_LG)
                .build_classes()
                .contains("h-12 text-lg")
        );
        assert_eq!(
            SelectProps::default()
                .with_variant(shared::VARIANT_GHOST)
                .variant,
            shared::VARIANT_GHOST
        );
        let option = SelectOption::new("Free", "free")
            .with_selected(true)
            .with_attribute("data-testid", "free");
        assert!(option.selected);
        assert_eq!(
            option.attributes.get("data-testid").map(String::as_str),
            Some("free")
        );

        let props = SelectProps::default()
            .with_name("plan")
            .with_id("plan")
            .with_value("pro")
            .with_placeholder("Choose")
            .with_placeholder_disabled(false)
            .with_options(vec![SelectOption::new("Pro", "pro")])
            .with_disabled(true)
            .with_required(true)
            .with_error(true, "Bad")
            .with_class("extra")
            .with_aria_label("Plan")
            .with_aria_described_by("help")
            .with_attribute("data-on:change", "$plan = evt.target.value");
        assert!(
            props
                .build_classes()
                .contains("border-destructive focus-visible:ring-destructive extra")
        );
        let attrs = props.build_attributes();
        assert_eq!(attrs.get("id").map(String::as_str), Some("plan"));
        assert_eq!(attrs.get("name").map(String::as_str), Some("plan"));
        assert_eq!(attrs.get("disabled").map(String::as_str), Some("disabled"));
        assert_eq!(attrs.get("required").map(String::as_str), Some("required"));
        assert_eq!(attrs.get("aria-label").map(String::as_str), Some("Plan"));
        assert_eq!(
            attrs.get("aria-describedby").map(String::as_str),
            Some("help")
        );
        assert_eq!(attrs.get("aria-invalid").map(String::as_str), Some("true"));
    }

    #[test]
    fn select_renders_placeholder_options_errors_and_label_variations() {
        let props = SelectProps::default()
            .with_id("plan")
            .with_name("plan")
            .with_placeholder("<Choose>")
            .with_value("pro")
            .with_required(true)
            .with_error(true, "<Bad>")
            .with_options(vec![
                SelectOption::new("<Free>", "free").with_disabled(true),
                SelectOption::new("Pro", "pro").with_attribute("data-testid", "pro"),
            ])
            .with_attribute("data-testid", "select")
            .with_attribute("data-on:change", "$plan = evt.target.value");
        let html = Select(&props).into_string();
        assert!(html.starts_with("<div class=\"relative w-full\">"));
        assert!(html.contains("<select class=\""));
        assert!(html.contains("id=\"plan\""));
        assert!(html.contains("name=\"plan\""));
        assert!(html.contains("required"));
        assert!(html.contains("aria-invalid=\"true\""));
        assert!(html.contains("data-on:change=\"$plan = evt.target.value\""));
        assert!(html.contains("<option value=\"\" disabled>&lt;Choose&gt;</option>"));
        assert!(html.contains("<option value=\"free\" disabled>&lt;Free&gt;</option>"));
        assert!(html.contains("<option value=\"pro\" selected data-testid=\"pro\">Pro</option>"));
        assert!(html.contains("role=\"alert\">&lt;Bad&gt;</p>"));

        let labeled = SelectWithLabel(&props, "<Plan>", "Hidden").into_string();
        assert!(labeled.starts_with("<div class=\"space-y-2\">"));
        assert!(labeled.contains("<label for=\"plan\""));
        assert!(labeled.contains("&lt;Plan&gt;"));
        assert!(labeled.contains("aria-label=\"required\">*</span>"));
        assert!(!labeled.contains(">Hidden</p>"));
    }
}
