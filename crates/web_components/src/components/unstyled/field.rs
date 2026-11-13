#![allow(non_snake_case)]

use crate::components::{ComponentInfo, shared};
use maud::{Markup, html};
use std::collections::BTreeMap;

pub const COMPONENTS: &[ComponentInfo] = &[ComponentInfo {
    family: "unstyled",
    module: "field",
    name: "Field",
}];

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct FieldProps {
    pub label: String,
    pub id: String,
    pub helper_text: String,
    pub error_message: String,
    pub error: bool,
    pub required: bool,
    pub optional: bool,
    pub class: String,
    pub attributes: BTreeMap<String, String>,
}

impl FieldProps {
    #[must_use]
    pub fn with_label(mut self, label: impl Into<String>) -> Self {
        self.label = label.into();
        self
    }

    #[must_use]
    pub fn with_id(mut self, id: impl Into<String>) -> Self {
        self.id = id.into();
        self
    }

    #[must_use]
    pub fn with_helper_text(mut self, helper_text: impl Into<String>) -> Self {
        self.helper_text = helper_text.into();
        self
    }

    #[must_use]
    pub fn with_error(mut self, error: bool, message: impl Into<String>) -> Self {
        self.error = error;
        self.error_message = message.into();
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
    pub fn build_classes(&self) -> String {
        shared::build_classes(&["space-y-2", self.class.as_str()])
    }

    #[must_use]
    pub fn build_label_classes(&self) -> String {
        shared::build_classes(&[
            "text-sm font-medium leading-none peer-disabled:cursor-not-allowed peer-disabled:opacity-70",
            if self.error { "text-destructive" } else { "" },
        ])
    }

    #[must_use]
    pub const fn build_helper_classes(&self) -> &'static str {
        "text-sm text-muted-foreground"
    }

    #[must_use]
    pub const fn build_error_classes(&self) -> &'static str {
        "text-sm font-medium text-destructive"
    }

    #[must_use]
    pub fn helper_id(&self) -> Option<String> {
        if self.id.is_empty() {
            None
        } else {
            Some(format!("{}-helper", self.id))
        }
    }

    #[must_use]
    pub fn error_id(&self) -> Option<String> {
        if self.id.is_empty() {
            None
        } else {
            Some(format!("{}-error", self.id))
        }
    }
}

#[must_use]
pub fn Field(props: &FieldProps, children: Markup) -> Markup {
    html! {
        div
            class=(props.build_classes())
            id=[shared::attr_value(&props.attributes, "id")]
            title=[shared::attr_value(&props.attributes, "title")]
            data-testid=[shared::attr_value(&props.attributes, "data-testid")]
        {
            @if !props.label.is_empty() {
                label
                    for=[shared::optional_attr(&props.id)]
                    class=(props.build_label_classes())
                {
                    (&props.label)
                    @if props.required {
                        span class="text-destructive ml-1" aria-label=(shared::default_strings().input_required_aria_label) {
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
            (children)
            @if !props.helper_text.is_empty() && !props.error {
                p
                    id=[props.helper_id()]
                    class=(props.build_helper_classes())
                {
                    (&props.helper_text)
                }
            }
            @if props.error && !props.error_message.is_empty() {
                p
                    id=[props.error_id()]
                    class=(props.build_error_classes())
                    role="alert"
                {
                    (&props.error_message)
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_and_helpers_match_go_behavior() {
        let props = FieldProps::default();
        assert!(!props.required);
        assert!(!props.optional);
        assert!(!props.error);
        assert_eq!(props.build_classes(), "space-y-2");
        assert_eq!(
            props.build_label_classes(),
            "text-sm font-medium leading-none peer-disabled:cursor-not-allowed peer-disabled:opacity-70"
        );
        assert_eq!(
            props.build_helper_classes(),
            "text-sm text-muted-foreground"
        );
        assert_eq!(
            props.build_error_classes(),
            "text-sm font-medium text-destructive"
        );
        assert_eq!(props.helper_id(), None);
        assert_eq!(props.error_id(), None);
        let identified = FieldProps::default().with_id("email");
        assert_eq!(identified.helper_id().as_deref(), Some("email-helper"));
        assert_eq!(identified.error_id().as_deref(), Some("email-error"));
        assert!(
            FieldProps::default()
                .with_error(true, "Bad")
                .build_label_classes()
                .contains("text-destructive")
        );
    }

    #[test]
    fn builders_and_attributes_are_preserved() {
        let props = FieldProps::default()
            .with_label("Label")
            .with_id("field")
            .with_helper_text("Help")
            .with_required(true)
            .with_optional(true)
            .with_class("extra")
            .with_attribute("data-testid", "field");
        assert_eq!(props.label, "Label");
        assert_eq!(props.id, "field");
        assert_eq!(props.helper_text, "Help");
        assert!(props.required);
        assert!(props.optional);
        assert_eq!(props.build_classes(), "space-y-2 extra");
        assert_eq!(
            props.attributes.get("data-testid").map(String::as_str),
            Some("field")
        );
    }

    #[test]
    fn field_renders_label_required_optional_helper_error_and_children() {
        let required = Field(
            &FieldProps::default()
                .with_label("<Label>")
                .with_id("email")
                .with_helper_text("<Help>")
                .with_required(true)
                .with_attribute("data-testid", "field"),
            html! { input type="email" {} },
        )
        .into_string();
        assert!(required.contains("data-testid=\"field\""));
        assert!(required.contains("<label for=\"email\""));
        assert!(required.contains("&lt;Label&gt;"));
        assert!(required.contains("aria-label=\"required\">*</span>"));
        assert!(required.contains("<input type=\"email\">"));
        assert!(required.contains("id=\"email-helper\""));
        assert!(required.contains("&lt;Help&gt;"));

        let optional = Field(
            &FieldProps::default()
                .with_label("Label")
                .with_optional(true),
            html! {},
        )
        .into_string();
        assert!(optional.contains("(optional)"));

        let error = Field(
            &FieldProps::default()
                .with_label("Label")
                .with_id("name")
                .with_helper_text("Hidden help")
                .with_error(true, "<Error>"),
            html! {},
        )
        .into_string();
        assert!(error.contains("text-destructive"));
        assert!(!error.contains("Hidden help"));
        assert!(error.contains("id=\"name-error\""));
        assert!(error.contains("role=\"alert\""));
        assert!(error.contains("&lt;Error&gt;"));
    }
}
