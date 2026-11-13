#![allow(non_snake_case)]

use crate::components::{ComponentInfo, shared};
use maud::{Markup, html};
use std::collections::BTreeMap;

pub const COMPONENTS: &[ComponentInfo] = &[
    ComponentInfo {
        family: "unstyled",
        module: "checkbox",
        name: "Checkbox",
    },
    ComponentInfo {
        family: "unstyled",
        module: "checkbox",
        name: "CheckboxWithLabel",
    },
    ComponentInfo {
        family: "unstyled",
        module: "checkbox",
        name: "CheckboxGroup",
    },
    ComponentInfo {
        family: "unstyled",
        module: "checkbox",
        name: "CheckboxCard",
    },
];

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CheckboxProps {
    pub name: String,
    pub id: String,
    pub value: String,
    pub checked: bool,
    pub indeterminate: bool,
    pub disabled: bool,
    pub required: bool,
    pub size: &'static str,
    pub error: bool,
    pub error_message: String,
    pub show_error: bool,
    pub class: String,
    pub attributes: BTreeMap<String, String>,
    pub aria_label: String,
    pub aria_described_by: String,
    pub aria_invalid: bool,
}

impl Default for CheckboxProps {
    fn default() -> Self {
        Self {
            name: String::new(),
            id: String::new(),
            value: String::new(),
            checked: false,
            indeterminate: false,
            disabled: false,
            required: false,
            size: shared::SIZE_MD,
            error: false,
            error_message: String::new(),
            show_error: true,
            class: String::new(),
            attributes: BTreeMap::new(),
            aria_label: String::new(),
            aria_described_by: String::new(),
            aria_invalid: false,
        }
    }
}

impl CheckboxProps {
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
    pub const fn with_checked(mut self, checked: bool) -> Self {
        self.checked = checked;
        self
    }
    #[must_use]
    pub const fn with_indeterminate(mut self, indeterminate: bool) -> Self {
        self.indeterminate = indeterminate;
        self
    }
    #[must_use]
    pub const fn with_disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }
    #[must_use]
    pub const fn with_required(mut self, required: bool) -> Self {
        self.required = required;
        self
    }
    #[must_use]
    pub const fn with_size(mut self, size: &'static str) -> Self {
        self.size = size;
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
    pub const fn with_show_error(mut self, show: bool) -> Self {
        self.show_error = show;
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
        shared::build_classes(&[
            "rounded border-2 border-border text-primary focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 disabled:cursor-not-allowed disabled:opacity-50 transition-all duration-150 cursor-pointer",
            self.get_size_classes(),
            if self.error {
                "border-destructive focus-visible:ring-destructive"
            } else {
                ""
            },
            self.class.as_str(),
        ])
    }

    #[must_use]
    pub fn get_size_classes(&self) -> &'static str {
        match self.size {
            shared::SIZE_SM => "h-4 w-4",
            shared::SIZE_LG => "h-6 w-6",
            _ => "h-5 w-5",
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
        if self.indeterminate {
            attrs.insert("aria-checked".to_string(), "mixed".to_string());
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
pub fn Checkbox(props: &CheckboxProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        input
            type="checkbox"
            name=[shared::optional_attr(&props.name)]
            id=[shared::optional_attr(&props.id)]
            value=[shared::optional_attr(&props.value)]
            class=(props.build_classes())
            checked[props.checked]
            disabled[props.disabled]
            required[props.required]
            data-indeterminate=[if props.indeterminate { Some("true") } else { None }]
            aria-describedby=[if props.error && !props.get_error_id().is_empty() { Some(props.get_error_id()) } else { shared::attr_value(&attrs, "aria-describedby").map(str::to_string) }]
            aria-label=[shared::attr_value(&attrs, "aria-label")]
            aria-invalid=[shared::attr_value(&attrs, "aria-invalid")]
            aria-checked=[shared::attr_value(&attrs, "aria-checked")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
            data-bind=[shared::attr_value(&attrs, "data-bind")]
            data-attr:disabled=[shared::attr_value(&attrs, "data-attr:disabled")]
        {}
        @if props.error && props.show_error && !props.error_message.is_empty() {
            p id=(props.get_error_id()) class="mt-1 text-sm text-destructive" role="alert" {
                (&props.error_message)
            }
        }
    }
}

#[must_use]
pub fn CheckboxWithLabel(
    props: &CheckboxProps,
    label: impl Into<String>,
    helper_text: impl Into<String>,
) -> Markup {
    let label = label.into();
    let helper_text = helper_text.into();
    html! {
        div class="flex items-start space-x-2" {
            div class="flex items-center h-5" { (Checkbox(props)) }
            div class="flex flex-col" {
                @if !label.is_empty() {
                    label
                        for=[shared::optional_attr(&props.id)]
                        class="text-sm font-medium leading-none peer-disabled:cursor-not-allowed peer-disabled:opacity-70 cursor-pointer"
                    {
                        (&label)
                        @if props.required {
                            span class="text-destructive ml-1" aria-label=(shared::default_strings().input_required_aria_label) {
                                (shared::default_strings().input_required_symbol)
                            }
                        }
                    }
                }
                @if !helper_text.is_empty() && !props.error {
                    p class="text-sm text-muted-foreground mt-1" { (&helper_text) }
                }
            }
        }
    }
}

#[must_use]
pub fn CheckboxGroup(
    legend: impl Into<String>,
    helper_text: impl Into<String>,
    required: bool,
    has_error: bool,
    error_message: impl Into<String>,
    children: Markup,
) -> Markup {
    let legend = legend.into();
    let helper_text = helper_text.into();
    let error_message = error_message.into();
    html! {
        fieldset class="space-y-3" {
            @if !legend.is_empty() {
                legend class="text-sm font-medium leading-none mb-3" {
                    (&legend)
                    @if required {
                        span class="text-destructive ml-1" aria-label=(shared::default_strings().input_required_aria_label) {
                            (shared::default_strings().input_required_symbol)
                        }
                    }
                }
            }
            @if !helper_text.is_empty() && !has_error {
                p class="text-sm text-muted-foreground mb-3" { (&helper_text) }
            }
            div class="space-y-2" { (children) }
            @if has_error && !error_message.is_empty() {
                p class="text-sm text-destructive mt-2" role="alert" { (&error_message) }
            }
        }
    }
}

#[must_use]
pub fn CheckboxCard(
    props: &CheckboxProps,
    title: impl Into<String>,
    description: impl Into<String>,
) -> Markup {
    let title = title.into();
    let description = description.into();
    html! {
        label
            for=[shared::optional_attr(&props.id)]
            class="relative flex cursor-pointer rounded-lg border border-border p-4 hover:bg-accent/50 transition-colors duration-150"
        {
            div class="flex items-start space-x-3 w-full" {
                div class="flex items-center h-5 mt-0.5" { (Checkbox(props)) }
                div class="flex-1" {
                    @if !title.is_empty() {
                        p class="font-medium text-sm" {
                            (&title)
                            @if props.required {
                                span class="text-destructive ml-1" aria-label=(shared::default_strings().input_required_aria_label) {
                                    (shared::default_strings().input_required_symbol)
                                }
                            }
                        }
                    }
                    @if !description.is_empty() {
                        p class="text-sm text-muted-foreground mt-1" { (&description) }
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
    fn defaults_helpers_and_attributes_match_go_behavior() {
        let props = CheckboxProps::default();
        assert_eq!(props.size, shared::SIZE_MD);
        assert!(props.show_error);
        assert_eq!(props.get_size_classes(), "h-5 w-5");
        assert_eq!(
            CheckboxProps::default()
                .with_size(shared::SIZE_SM)
                .get_size_classes(),
            "h-4 w-4"
        );
        assert_eq!(
            CheckboxProps::default()
                .with_size(shared::SIZE_LG)
                .get_size_classes(),
            "h-6 w-6"
        );
        assert!(!CheckboxProps::default().with_show_error(false).show_error);
        assert_eq!(CheckboxProps::default().get_error_id(), "");

        let props = CheckboxProps::default()
            .with_name("accept")
            .with_id("accept")
            .with_value("yes")
            .with_checked(true)
            .with_indeterminate(true)
            .with_disabled(true)
            .with_required(true)
            .with_error(true, "Bad")
            .with_class("extra")
            .with_aria_label("Accept")
            .with_aria_described_by("external")
            .with_attribute("data-bind", "accept");
        assert!(
            props
                .build_classes()
                .contains("border-destructive focus-visible:ring-destructive extra")
        );
        let attrs = props.build_attributes();
        assert_eq!(attrs.get("aria-label").map(String::as_str), Some("Accept"));
        assert_eq!(
            attrs.get("aria-describedby").map(String::as_str),
            Some("external")
        );
        assert_eq!(attrs.get("aria-invalid").map(String::as_str), Some("true"));
        assert_eq!(attrs.get("aria-checked").map(String::as_str), Some("mixed"));
        assert_eq!(attrs.get("data-bind").map(String::as_str), Some("accept"));
        assert_eq!(props.get_error_id(), "accept-error");
    }

    #[test]
    fn checkbox_renders_all_boolean_aria_error_and_datastar_bind_variations() {
        let html = Checkbox(
            &CheckboxProps::default()
                .with_name("accept")
                .with_id("accept")
                .with_value("<yes>")
                .with_checked(true)
                .with_indeterminate(true)
                .with_disabled(true)
                .with_required(true)
                .with_error(true, "<Bad>")
                .with_attribute("data-testid", "checkbox")
                .with_attribute("data-bind", "accept"),
        )
        .into_string();
        assert!(html.starts_with("<input type=\"checkbox\""));
        assert!(html.contains("name=\"accept\""));
        assert!(html.contains("id=\"accept\""));
        assert!(html.contains("value=\"&lt;yes&gt;\""));
        assert!(html.contains("checked"));
        assert!(html.contains("disabled"));
        assert!(html.contains("required"));
        assert!(html.contains("data-indeterminate=\"true\""));
        assert!(html.contains("aria-describedby=\"accept-error\""));
        assert!(html.contains("aria-invalid=\"true\""));
        assert!(html.contains("aria-checked=\"mixed\""));
        assert!(html.contains("data-bind=\"accept\""));
        assert!(html.contains("<p id=\"accept-error\" class=\"mt-1 text-sm text-destructive\" role=\"alert\">&lt;Bad&gt;</p>"));
    }

    #[test]
    fn checkbox_label_group_and_card_render_expected_html() {
        let props = CheckboxProps::default().with_id("a").with_required(true);
        let labeled = CheckboxWithLabel(&props, "<Label>", "Help").into_string();
        assert!(labeled.contains("for=\"a\""));
        assert!(labeled.contains("&lt;Label&gt;"));
        assert!(labeled.contains("aria-label=\"required\">*</span>"));
        assert!(labeled.contains("<p class=\"text-sm text-muted-foreground mt-1\">Help</p>"));

        let group = CheckboxGroup("Legend", "Help", true, true, "<Error>", html! { input {} })
            .into_string();
        assert!(group.starts_with("<fieldset class=\"space-y-3\">"));
        assert!(group.contains("Legend"));
        assert!(!group.contains(">Help</p>"));
        assert!(group.contains("role=\"alert\">&lt;Error&gt;</p>"));

        let card = CheckboxCard(&props, "Title", "Desc").into_string();
        assert!(card.contains("relative flex cursor-pointer rounded-lg border border-border p-4"));
        assert!(card.contains("<p class=\"font-medium text-sm\">Title"));
        assert!(card.contains("<p class=\"text-sm text-muted-foreground mt-1\">Desc</p>"));
    }
}
