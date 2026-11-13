#![allow(non_snake_case)]

use crate::components::{ComponentInfo, shared};
use maud::{Markup, html};
use std::collections::BTreeMap;

pub const COMPONENTS: &[ComponentInfo] = &[
    ComponentInfo {
        family: "unstyled",
        module: "formfield",
        name: "FormField",
    },
    ComponentInfo {
        family: "unstyled",
        module: "formfield",
        name: "FormFieldWithIcon",
    },
    ComponentInfo {
        family: "unstyled",
        module: "formfield",
        name: "HorizontalFormField",
    },
];

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FormFieldProps {
    pub label: String,
    pub for_id: String,
    pub helper_text: String,
    pub error: String,
    pub required: bool,
    pub optional: bool,
    pub disabled: bool,
    pub layout: &'static str,
    pub class: String,
    pub attributes: BTreeMap<String, String>,
    pub aria_described_by: String,
}

impl Default for FormFieldProps {
    fn default() -> Self {
        Self {
            label: String::new(),
            for_id: String::new(),
            helper_text: String::new(),
            error: String::new(),
            required: false,
            optional: false,
            disabled: false,
            layout: "vertical",
            class: String::new(),
            attributes: BTreeMap::new(),
            aria_described_by: String::new(),
        }
    }
}

impl FormFieldProps {
    #[must_use]
    pub fn with_label(mut self, label: impl Into<String>) -> Self {
        self.label = label.into();
        self
    }

    #[must_use]
    pub fn with_for(mut self, for_id: impl Into<String>) -> Self {
        self.for_id = for_id.into();
        self
    }

    #[must_use]
    pub fn with_helper_text(mut self, text: impl Into<String>) -> Self {
        self.helper_text = text.into();
        self
    }

    #[must_use]
    pub fn with_error(mut self, error: impl Into<String>) -> Self {
        self.error = error.into();
        self
    }

    #[must_use]
    pub const fn with_required(mut self, required: bool) -> Self {
        self.required = required;
        self
    }

    #[must_use]
    pub const fn with_optional(mut self, optional: bool) -> Self {
        self.optional = optional;
        self
    }

    #[must_use]
    pub const fn with_disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    #[must_use]
    pub const fn with_layout(mut self, layout: &'static str) -> Self {
        self.layout = layout;
        self
    }

    #[must_use]
    pub fn with_class(mut self, class: impl Into<String>) -> Self {
        self.class = class.into();
        self
    }

    #[must_use]
    pub fn with_attribute(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.attributes.insert(name.into(), value.into());
        self
    }

    #[must_use]
    pub fn with_aria_described_by(mut self, described_by: impl Into<String>) -> Self {
        self.aria_described_by = described_by.into();
        self
    }

    #[must_use]
    pub fn has_error(&self) -> bool {
        !self.error.is_empty()
    }

    #[must_use]
    pub fn build_classes(&self) -> String {
        let layout_class = if self.layout == "horizontal" {
            "flex flex-row items-start gap-4"
        } else {
            "flex flex-col gap-1.5"
        };
        shared::build_classes(&[
            "form-field",
            layout_class,
            if self.has_error() {
                "form-field-error"
            } else {
                ""
            },
            if self.disabled {
                "opacity-50 pointer-events-none"
            } else {
                ""
            },
            self.class.as_str(),
        ])
    }

    #[must_use]
    pub fn build_label_classes(&self) -> String {
        shared::build_classes(&[
            "text-sm font-medium",
            if self.has_error() {
                "text-destructive"
            } else {
                ""
            },
        ])
    }

    #[must_use]
    pub const fn build_helper_classes(&self) -> &'static str {
        "text-xs text-muted-foreground"
    }

    #[must_use]
    pub const fn build_error_classes(&self) -> &'static str {
        "text-xs text-destructive"
    }

    #[must_use]
    pub fn generate_described_by(&self) -> String {
        let mut parts = Vec::new();
        if !self.helper_text.is_empty() && !self.for_id.is_empty() {
            parts.push(format!("{}-helper", self.for_id));
        }
        if self.has_error() && !self.for_id.is_empty() {
            parts.push(format!("{}-error", self.for_id));
        }
        if !self.aria_described_by.is_empty() {
            parts.push(self.aria_described_by.clone());
        }
        parts.join(" ")
    }

    #[must_use]
    pub fn helper_id(&self) -> Option<String> {
        if self.for_id.is_empty() {
            None
        } else {
            Some(format!("{}-helper", self.for_id))
        }
    }

    #[must_use]
    pub fn error_id(&self) -> Option<String> {
        if self.for_id.is_empty() {
            None
        } else {
            Some(format!("{}-error", self.for_id))
        }
    }
}

fn label(props: &FormFieldProps) -> Markup {
    html! {
        @if !props.label.is_empty() {
            label
                for=[if props.for_id.is_empty() { None } else { Some(props.for_id.as_str()) }]
                class=(props.build_label_classes())
            {
                (&props.label)
                @if props.required {
                    span class="text-destructive ml-0.5" aria-label=(shared::default_strings().input_required_aria_label) {
                        (shared::default_strings().input_required_symbol)
                    }
                }
                @if props.optional && !props.required {
                    span class="text-muted-foreground ml-1 font-normal" {
                        (shared::default_strings().input_optional_text)
                    }
                }
            }
        }
    }
}

fn helper_or_error(props: &FormFieldProps) -> Markup {
    html! {
        @if !props.helper_text.is_empty() && !props.has_error() {
            p
                id=[props.helper_id()]
                class=(props.build_helper_classes())
            {
                (&props.helper_text)
            }
        }
        @if props.has_error() {
            div class="error-slide is-visible" {
                p
                    id=[props.error_id()]
                    class=(props.build_error_classes())
                    role="alert"
                    aria-live="polite"
                {
                    (&props.error)
                }
            }
        }
    }
}

#[must_use]
pub fn FormField(props: &FormFieldProps, children: Markup) -> Markup {
    html! {
        div class=(props.build_classes()) {
            (label(props))
            div class="form-field-content" {
                (children)
            }
            (helper_or_error(props))
        }
    }
}

#[must_use]
pub fn FormFieldWithIcon(
    props: &FormFieldProps,
    icon: Markup,
    icon_position: &str,
    children: Markup,
) -> Markup {
    html! {
        div class=(props.build_classes()) {
            (label(props))
            div class="form-field-content relative" {
                @if icon_position == "left" {
                    div class="absolute left-3 top-1/2 -translate-y-1/2 pointer-events-none" {
                        (icon.clone())
                    }
                }
                (children)
                @if icon_position == "right" {
                    div class="absolute right-3 top-1/2 -translate-y-1/2 pointer-events-none" {
                        (icon)
                    }
                }
            }
            (helper_or_error(props))
        }
    }
}

#[must_use]
pub fn HorizontalFormField(
    label: impl Into<String>,
    for_id: impl Into<String>,
    helper_text: impl Into<String>,
    error_msg: impl Into<String>,
    required: bool,
    children: Markup,
) -> Markup {
    FormField(
        &FormFieldProps::default()
            .with_label(label)
            .with_for(for_id)
            .with_helper_text(helper_text)
            .with_error(error_msg)
            .with_required(required)
            .with_layout("horizontal"),
        children,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_and_helpers_match_go_behavior() {
        let props = FormFieldProps::default();
        assert_eq!(props.layout, "vertical");
        assert!(!props.required);
        assert!(!props.disabled);
        assert!(!props.optional);
        assert!(!props.has_error());
        assert_eq!(props.build_classes(), "form-field flex flex-col gap-1.5");
        assert_eq!(props.build_label_classes(), "text-sm font-medium");
        assert_eq!(
            props.build_helper_classes(),
            "text-xs text-muted-foreground"
        );
        assert_eq!(props.build_error_classes(), "text-xs text-destructive");
        assert_eq!(props.helper_id(), None);
        assert_eq!(props.error_id(), None);
        let identified = FormFieldProps::default().with_for("email");
        assert_eq!(identified.helper_id().as_deref(), Some("email-helper"));
        assert_eq!(identified.error_id().as_deref(), Some("email-error"));
    }

    #[test]
    fn helpers_cover_error_disabled_horizontal_and_describedby_combinations() {
        let props = FormFieldProps::default()
            .with_for("email")
            .with_helper_text("Help")
            .with_error("Error")
            .with_disabled(true)
            .with_layout("horizontal")
            .with_class("extra")
            .with_aria_described_by("external")
            .with_attribute("title", "Field");
        assert!(props.has_error());
        assert_eq!(
            props.build_classes(),
            "form-field flex flex-row items-start gap-4 form-field-error opacity-50 pointer-events-none extra"
        );
        assert_eq!(
            props.build_label_classes(),
            "text-sm font-medium text-destructive"
        );
        assert_eq!(
            props.generate_described_by(),
            "email-helper email-error external"
        );
        assert_eq!(
            props.attributes.get("title").map(String::as_str),
            Some("Field")
        );

        assert_eq!(
            FormFieldProps::default()
                .with_aria_described_by("external")
                .generate_described_by(),
            "external"
        );
    }

    #[test]
    fn form_field_renders_label_helper_error_required_optional_and_children() {
        let html = FormField(
            &FormFieldProps::default()
                .with_label("<Email>")
                .with_for("email")
                .with_helper_text("<Help>")
                .with_required(true),
            html! { input id="email" {} },
        )
        .into_string();
        assert!(html.starts_with("<div class=\"form-field flex flex-col gap-1.5\">"));
        assert!(html.contains("<label for=\"email\""));
        assert!(html.contains("&lt;Email&gt;"));
        assert!(html.contains("aria-label=\"required\">*</span>"));
        assert!(html.contains("<div class=\"form-field-content\">"));
        assert!(html.contains("<input id=\"email\">"));
        assert!(html.contains("id=\"email-helper\""));
        assert!(html.contains("&lt;Help&gt;"));

        let optional = FormField(
            &FormFieldProps::default()
                .with_label("Name")
                .with_optional(true),
            html! {},
        )
        .into_string();
        assert!(optional.contains("(optional)"));

        let error = FormField(
            &FormFieldProps::default()
                .with_for("name")
                .with_helper_text("Hidden")
                .with_error("<Bad>"),
            html! {},
        )
        .into_string();
        assert!(!error.contains("Hidden"));
        assert!(error.contains("<div class=\"error-slide is-visible\">"));
        assert!(error.contains("id=\"name-error\""));
        assert!(error.contains("role=\"alert\" aria-live=\"polite\""));
        assert!(error.contains("&lt;Bad&gt;"));
    }

    #[test]
    fn form_field_with_icon_and_horizontal_variant_render_expected_html() {
        let left = FormFieldWithIcon(
            &FormFieldProps::default().with_label("Search"),
            html! { svg class="icon" {} },
            "left",
            html! { input {} },
        )
        .into_string();
        assert!(left.contains("form-field-content relative"));
        assert!(left.contains("absolute left-3 top-1/2 -translate-y-1/2 pointer-events-none"));
        assert!(left.contains("<svg class=\"icon\"></svg>"));

        let right = FormFieldWithIcon(
            &FormFieldProps::default(),
            html! { svg class="icon" {} },
            "right",
            html! { input {} },
        )
        .into_string();
        assert!(right.contains("absolute right-3 top-1/2 -translate-y-1/2 pointer-events-none"));

        let horizontal =
            HorizontalFormField("Label", "id", "Help", "", true, html! { input {} }).into_string();
        assert!(horizontal.contains("flex flex-row items-start gap-4"));
        assert!(horizontal.contains("for=\"id\""));
        assert!(horizontal.contains("id=\"id-helper\""));
    }
}
