#![allow(non_snake_case)]

use crate::components::{ComponentInfo, shared};
use maud::{Markup, html};
use std::collections::BTreeMap;

pub const COMPONENTS: &[ComponentInfo] = &[
    ComponentInfo {
        family: "unstyled",
        module: "label",
        name: "Label",
    },
    ComponentInfo {
        family: "unstyled",
        module: "label",
        name: "LabelWithHelperText",
    },
    ComponentInfo {
        family: "unstyled",
        module: "label",
        name: "FormField",
    },
];

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LabelProps {
    pub for_id: String,
    pub required: bool,
    pub optional: bool,
    pub size: &'static str,
    pub class: String,
    pub attributes: BTreeMap<String, String>,
    pub aria_label: String,
}

impl Default for LabelProps {
    fn default() -> Self {
        Self {
            for_id: String::new(),
            required: false,
            optional: false,
            size: shared::SIZE_MD,
            class: String::new(),
            attributes: BTreeMap::new(),
            aria_label: String::new(),
        }
    }
}

impl LabelProps {
    #[must_use]
    pub fn with_for(mut self, for_id: impl Into<String>) -> Self {
        self.for_id = for_id.into();
        self
    }

    #[must_use]
    pub const fn with_required(mut self, required: bool) -> Self {
        self.required = required;
        if required {
            self.optional = false;
        }
        self
    }

    #[must_use]
    pub const fn with_optional(mut self, optional: bool) -> Self {
        self.optional = optional;
        if optional {
            self.required = false;
        }
        self
    }

    #[must_use]
    pub fn with_size(mut self, size: &'static str) -> Self {
        self.size = size;
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
    pub fn with_aria_label(mut self, aria_label: impl Into<String>) -> Self {
        self.aria_label = aria_label.into();
        self
    }

    #[must_use]
    pub fn size_classes(&self) -> &'static str {
        match self.size {
            shared::SIZE_SM => "text-xs",
            shared::SIZE_LG => "text-base",
            _ => "text-sm",
        }
    }

    #[must_use]
    pub fn build_classes(&self) -> String {
        shared::build_classes(&[
            "font-medium leading-none peer-disabled:cursor-not-allowed peer-disabled:opacity-70",
            self.size_classes(),
            self.class.as_str(),
        ])
    }

    #[must_use]
    pub fn build_attributes(&self) -> BTreeMap<String, String> {
        let mut attrs = self.attributes.clone();
        if !self.aria_label.is_empty() {
            attrs.insert("aria-label".to_string(), self.aria_label.clone());
        }
        attrs
    }
}

#[must_use]
pub fn Label(props: &LabelProps, children: Markup) -> Markup {
    let attrs = props.build_attributes();
    html! {
        label
            for=[if props.for_id.is_empty() { None } else { Some(props.for_id.as_str()) }]
            class=(props.build_classes())
            id=[shared::attr_value(&attrs, "id")]
            title=[shared::attr_value(&attrs, "title")]
            aria-label=[shared::attr_value(&attrs, "aria-label")]
            data-show=[shared::attr_value(&attrs, "data-show")]
        {
            (children)
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
}

#[must_use]
pub fn LabelWithHelperText(props: &LabelProps, helper_text: &str, children: Markup) -> Markup {
    html! {
        div class="space-y-1" {
            (Label(props, children))
            @if !helper_text.is_empty() {
                p class="text-xs text-muted-foreground font-normal" {
                    (helper_text)
                }
            }
        }
    }
}

#[must_use]
pub fn FormField(
    props: &LabelProps,
    helper_text: &str,
    error_message: &str,
    show_error: bool,
    children: Markup,
) -> Markup {
    html! {
        div class="space-y-2" {
            @if !helper_text.is_empty() {
                (LabelWithHelperText(props, helper_text, children.clone()))
            } @else {
                (Label(props, children.clone()))
            }
            div class="field-content" {
                (children)
            }
            @if show_error && !error_message.is_empty() {
                p class="text-sm text-destructive" role="alert" {
                    (error_message)
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SIZES: &[(&str, &str)] = &[
        (shared::SIZE_SM, "text-xs"),
        (shared::SIZE_MD, "text-sm"),
        (shared::SIZE_LG, "text-base"),
        (shared::SIZE_XL, "text-sm"),
    ];

    #[test]
    fn default_props_match_go_defaults() {
        let props = LabelProps::default();
        assert_eq!(props.size, shared::SIZE_MD);
        assert!(!props.required);
        assert!(!props.optional);
        assert!(props.for_id.is_empty());
    }

    #[test]
    fn required_and_optional_builders_are_mutually_exclusive() {
        let required = LabelProps::default()
            .with_optional(true)
            .with_required(true);
        assert!(required.required);
        assert!(!required.optional);

        let optional = LabelProps::default()
            .with_required(true)
            .with_optional(true);
        assert!(optional.optional);
        assert!(!optional.required);
    }

    #[test]
    fn helper_classes_and_attributes_cover_sizes_and_aria() {
        for &(size, expected) in SIZES {
            let props = LabelProps::default()
                .with_size(size)
                .with_class("custom")
                .with_aria_label("Name")
                .with_attribute("data-show", "$visible");
            assert_eq!(props.size_classes(), expected);
            assert!(props.build_classes().contains(expected));
            assert!(props.build_classes().contains("custom"));
            let attrs = props.build_attributes();
            assert_eq!(attrs.get("aria-label"), Some(&"Name".to_string()));
            assert_eq!(attrs.get("data-show"), Some(&"$visible".to_string()));
        }
    }

    #[test]
    fn label_renders_for_required_optional_and_escaped_children() {
        let required = Label(
            &LabelProps::default()
                .with_for("email")
                .with_required(true)
                .with_attribute("title", "\"Email\" & label"),
            html! { "<Email>" },
        )
        .into_string();
        assert!(required.starts_with(r#"<label for="email" class=""#));
        assert!(required.contains(r#"title="&quot;Email&quot; &amp; label""#));
        assert!(required.contains("&lt;Email&gt;"));
        assert!(
            required
                .contains(r#"<span class="text-destructive ml-1" aria-label="required">*</span>"#)
        );
        assert!(!required.contains("(optional)"));

        let optional =
            Label(&LabelProps::default().with_optional(true), html! { "Name" }).into_string();
        assert!(
            optional.contains(
                r#"<span class="text-muted-foreground ml-1 font-normal">(optional)</span>"#
            )
        );
        assert!(!optional.contains("text-destructive ml-1"));
    }

    #[test]
    fn label_with_helper_text_renders_helper_only_when_present() {
        let with_helper =
            LabelWithHelperText(&LabelProps::default(), "Helpful", html! { "Label" }).into_string();
        assert!(with_helper.starts_with(r#"<div class="space-y-1"><label "#));
        assert!(
            with_helper
                .contains(r#"<p class="text-xs text-muted-foreground font-normal">Helpful</p>"#)
        );

        let without_helper =
            LabelWithHelperText(&LabelProps::default(), "", html! { "Label" }).into_string();
        assert!(!without_helper.contains("<p "));
    }

    #[test]
    fn form_field_renders_helper_error_and_repeated_children_combinations() {
        for helper_text in ["", "Help"] {
            for show_error in [false, true] {
                let html = FormField(
                    &LabelProps::default(),
                    helper_text,
                    "Invalid",
                    show_error,
                    html! { "Field" },
                )
                .into_string();

                assert!(html.starts_with(r#"<div class="space-y-2">"#));
                assert_eq!(html.matches("Field").count(), 2);
                assert_eq!(html.contains("Help"), !helper_text.is_empty());
                assert_eq!(
                    html.contains(
                        r#"<p class="text-sm text-destructive" role="alert">Invalid</p>"#
                    ),
                    show_error
                );
            }
        }
    }
}
