#![allow(non_snake_case)]

use crate::components::{ComponentInfo, shared};
use maud::{Markup, html};
use std::collections::BTreeMap;

pub const COMPONENTS: &[ComponentInfo] = &[
    ComponentInfo {
        family: "unstyled",
        module: "selectfield",
        name: "Select",
    },
    ComponentInfo {
        family: "unstyled",
        module: "selectfield",
        name: "SelectWithGroups",
    },
    ComponentInfo {
        family: "unstyled",
        module: "selectfield",
        name: "SelectWithLabel",
    },
    ComponentInfo {
        family: "unstyled",
        module: "selectfield",
        name: "SelectWithGroupsAndLabel",
    },
];

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SelectOption {
    pub value: String,
    pub label: String,
    pub selected: bool,
    pub disabled: bool,
}

impl SelectOption {
    #[must_use]
    pub fn new(value: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            value: value.into(),
            label: label.into(),
            ..Self::default()
        }
    }
    #[must_use]
    pub const fn with_selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }
    #[must_use]
    pub const fn with_disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SelectOptionGroup {
    pub label: String,
    pub options: Vec<SelectOption>,
}

impl SelectOptionGroup {
    #[must_use]
    pub fn new(label: impl Into<String>, options: Vec<SelectOption>) -> Self {
        Self {
            label: label.into(),
            options,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SelectProps {
    pub name: String,
    pub id: String,
    pub size: &'static str,
    pub disabled: bool,
    pub required: bool,
    pub multiple: bool,
    pub error: bool,
    pub error_message: String,
    pub show_error: bool,
    pub placeholder: String,
    pub full_width: bool,
    pub class: String,
    pub attributes: BTreeMap<String, String>,
    pub aria_label: String,
    pub aria_described_by: String,
    pub aria_invalid: bool,
    pub autocomplete: String,
}

impl Default for SelectProps {
    fn default() -> Self {
        Self {
            name: String::new(),
            id: String::new(),
            size: shared::SIZE_MD,
            disabled: false,
            required: false,
            multiple: false,
            error: false,
            error_message: String::new(),
            show_error: true,
            placeholder: String::new(),
            full_width: false,
            class: String::new(),
            attributes: BTreeMap::new(),
            aria_label: String::new(),
            aria_described_by: String::new(),
            aria_invalid: false,
            autocomplete: String::new(),
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
    pub const fn with_multiple(mut self, value: bool) -> Self {
        self.multiple = value;
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
    pub fn with_placeholder(mut self, value: impl Into<String>) -> Self {
        self.placeholder = value.into();
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
    pub fn with_autocomplete(mut self, value: impl Into<String>) -> Self {
        self.autocomplete = value.into();
        self
    }

    #[must_use]
    pub fn build_classes(&self) -> String {
        shared::build_classes(&[
            "w-full border-0 bg-transparent focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 disabled:cursor-not-allowed disabled:opacity-50 transition-all duration-150 appearance-none",
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
    pub fn build_attributes(&self) -> BTreeMap<String, String> {
        let mut attrs = self.attributes.clone();
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

fn chevron() -> Markup {
    html! {
        div class="absolute right-3 top-1/2 -translate-y-1/2 pointer-events-none text-muted-foreground" {
            svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" {
                polyline points="6 9 12 15 18 9" {}
            }
        }
    }
}

fn aria_described_by(props: &SelectProps, attrs: &BTreeMap<String, String>) -> Option<String> {
    if props.error && !props.get_error_id().is_empty() {
        Some(props.get_error_id())
    } else {
        shared::attr_value(attrs, "aria-describedby").map(str::to_string)
    }
}

#[must_use]
pub fn Select(props: &SelectProps, options: &[SelectOption]) -> Markup {
    let attrs = props.build_attributes();
    html! {
        div class=(props.build_wrapper_classes()) {
            select
                name=[shared::optional_attr(&props.name)]
                id=[shared::optional_attr(&props.id)]
                class=(props.build_classes())
                disabled[props.disabled]
                required[props.required]
                multiple[props.multiple]
                aria-describedby=[aria_described_by(props, &attrs)]
                aria-label=[shared::attr_value(&attrs, "aria-label")]
                aria-invalid=[shared::attr_value(&attrs, "aria-invalid")]
                autocomplete=[shared::attr_value(&attrs, "autocomplete")]
                data-testid=[shared::attr_value(&attrs, "data-testid")]
                data-on:change=[shared::attr_value(&attrs, "data-on:change")]
            {
                @if !props.placeholder.is_empty() {
                    option value="" disabled selected { (&props.placeholder) }
                }
                @for option in options {
                    option value=(&option.value) selected[option.selected] disabled[option.disabled] {
                        (&option.label)
                    }
                }
            }
            (chevron())
        }
        (error_message(props))
    }
}

#[must_use]
pub fn SelectWithGroups(props: &SelectProps, groups: &[SelectOptionGroup]) -> Markup {
    let attrs = props.build_attributes();
    html! {
        div class=(props.build_wrapper_classes()) {
            select
                name=[shared::optional_attr(&props.name)]
                id=[shared::optional_attr(&props.id)]
                class=(props.build_classes())
                disabled[props.disabled]
                required[props.required]
                multiple[props.multiple]
                aria-describedby=[aria_described_by(props, &attrs)]
                aria-label=[shared::attr_value(&attrs, "aria-label")]
                aria-invalid=[shared::attr_value(&attrs, "aria-invalid")]
                autocomplete=[shared::attr_value(&attrs, "autocomplete")]
            {
                @if !props.placeholder.is_empty() {
                    option value="" disabled selected { (&props.placeholder) }
                }
                @for group in groups {
                    optgroup label=(&group.label) {
                        @for option in &group.options {
                            option value=(&option.value) selected[option.selected] disabled[option.disabled] {
                                (&option.label)
                            }
                        }
                    }
                }
            }
            (chevron())
        }
        (error_message(props))
    }
}

fn error_message(props: &SelectProps) -> Markup {
    html! {
        @if props.error && props.show_error && !props.error_message.is_empty() {
            p id=(props.get_error_id()) class="mt-1 text-sm text-destructive" role="alert" {
                (&props.error_message)
            }
        }
    }
}

#[must_use]
pub fn SelectWithLabel(
    props: &SelectProps,
    options: &[SelectOption],
    label: impl Into<String>,
    helper_text: impl Into<String>,
) -> Markup {
    let label = label.into();
    let helper_text = helper_text.into();
    html! {
        div class="space-y-2" {
            (select_label(props, &label))
            (Select(props, options))
            @if !helper_text.is_empty() && !props.error {
                p class="text-sm text-muted-foreground" { (&helper_text) }
            }
        }
    }
}

#[must_use]
pub fn SelectWithGroupsAndLabel(
    props: &SelectProps,
    groups: &[SelectOptionGroup],
    label: impl Into<String>,
    helper_text: impl Into<String>,
) -> Markup {
    let label = label.into();
    let helper_text = helper_text.into();
    html! {
        div class="space-y-2" {
            (select_label(props, &label))
            (SelectWithGroups(props, groups))
            @if !helper_text.is_empty() && !props.error {
                p class="text-sm text-muted-foreground" { (&helper_text) }
            }
        }
    }
}

fn select_label(props: &SelectProps, label: &str) -> Markup {
    html! {
        @if !label.is_empty() {
            label
                for=[shared::optional_attr(&props.id)]
                class="text-sm font-medium leading-none peer-disabled:cursor-not-allowed peer-disabled:opacity-70"
            {
                (label)
                @if props.required {
                    span class="text-destructive ml-1" aria-label=(shared::default_strings().input_required_aria_label) {
                        (shared::default_strings().input_required_symbol)
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_helpers_and_attrs_match_go_behavior() {
        let props = SelectProps::default();
        assert_eq!(props.size, shared::SIZE_MD);
        assert!(props.show_error);
        assert!(!props.full_width);
        assert_eq!(
            props.build_wrapper_classes(),
            "relative inline-flex items-center"
        );
        assert!(
            props
                .build_classes()
                .contains(shared::input_size_classes(shared::SIZE_MD))
        );
        assert!(!SelectProps::default().with_show_error(false).show_error);
        assert_eq!(SelectProps::default().get_error_id(), "");
        let props = SelectProps::default()
            .with_name("country")
            .with_id("country")
            .with_size(shared::SIZE_LG)
            .with_disabled(true)
            .with_required(true)
            .with_multiple(true)
            .with_error(true, "Bad")
            .with_full_width(true)
            .with_class("extra")
            .with_aria_label("Country")
            .with_aria_described_by("help")
            .with_autocomplete("country")
            .with_attribute("data-on:change", "$country = evt.target.value");
        assert_eq!(
            props.build_wrapper_classes(),
            "relative inline-flex items-center w-full"
        );
        assert!(
            props
                .build_classes()
                .contains("border-destructive focus-visible:ring-destructive w-full extra")
        );
        let attrs = props.build_attributes();
        assert_eq!(
            attrs.get("autocomplete").map(String::as_str),
            Some("country")
        );
        assert_eq!(attrs.get("aria-label").map(String::as_str), Some("Country"));
        assert_eq!(
            attrs.get("aria-describedby").map(String::as_str),
            Some("help")
        );
        assert_eq!(attrs.get("aria-invalid").map(String::as_str), Some("true"));
        assert_eq!(props.get_error_id(), "country-error");
    }

    #[test]
    fn select_renders_options_chevron_error_and_label_variations() {
        let props = SelectProps::default()
            .with_name("country")
            .with_id("country")
            .with_placeholder("<Pick>")
            .with_required(true)
            .with_error(true, "<Bad>")
            .with_attribute("data-testid", "select")
            .with_attribute("data-on:change", "$country = evt.target.value");
        let options = vec![
            SelectOption::new("ca", "<Canada>").with_selected(true),
            SelectOption::new("us", "USA").with_disabled(true),
        ];
        let html = Select(&props, &options).into_string();
        assert!(html.starts_with("<div class=\"relative inline-flex items-center\">"));
        assert!(html.contains("<select "));
        assert!(html.contains("name=\"country\""));
        assert!(html.contains("id=\"country\""));
        assert!(html.contains("required"));
        assert!(html.contains("aria-describedby=\"country-error\""));
        assert!(html.contains("data-on:change=\"$country = evt.target.value\""));
        assert!(html.contains("<option value=\"\" disabled selected>&lt;Pick&gt;</option>"));
        assert!(html.contains("<option value=\"ca\" selected>&lt;Canada&gt;</option>"));
        assert!(html.contains("<option value=\"us\" disabled>USA</option>"));
        assert!(html.contains("<polyline points=\"6 9 12 15 18 9\"></polyline>"));
        assert!(html.contains("<p id=\"country-error\" class=\"mt-1 text-sm text-destructive\" role=\"alert\">&lt;Bad&gt;</p>"));

        let labeled = SelectWithLabel(&props, &options, "<Country>", "Hidden").into_string();
        assert!(labeled.contains("<label for=\"country\""));
        assert!(labeled.contains("&lt;Country&gt;"));
        assert!(labeled.contains("aria-label=\"required\">*</span>"));
        assert!(!labeled.contains(">Hidden</p>"));
    }

    #[test]
    fn select_with_groups_and_label_renders_optgroups_and_helper() {
        let props = SelectProps::default()
            .with_id("state")
            .with_name("state")
            .with_placeholder("Pick");
        let groups = vec![SelectOptionGroup::new(
            "North",
            vec![
                SelectOption::new("on", "Ontario"),
                SelectOption::new("qc", "Quebec").with_disabled(true),
            ],
        )];
        let html = SelectWithGroups(&props, &groups).into_string();
        assert!(html.contains("<optgroup label=\"North\">"));
        assert!(html.contains("<option value=\"on\">Ontario</option>"));
        assert!(html.contains("<option value=\"qc\" disabled>Quebec</option>"));

        let labeled =
            SelectWithGroupsAndLabel(&props, &groups, "State", "Choose one").into_string();
        assert!(labeled.starts_with("<div class=\"space-y-2\">"));
        assert!(labeled.contains("<label for=\"state\""));
        assert!(labeled.contains("<p class=\"text-sm text-muted-foreground\">Choose one</p>"));
    }
}
