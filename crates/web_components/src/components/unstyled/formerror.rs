#![allow(non_snake_case)]

use crate::components::{ComponentInfo, shared};
use maud::{Markup, html};
use std::collections::BTreeMap;

pub const COMPONENTS: &[ComponentInfo] = &[
    ComponentInfo {
        family: "unstyled",
        module: "formerror",
        name: "FormError",
    },
    ComponentInfo {
        family: "unstyled",
        module: "formerror",
        name: "FormErrorWithIcon",
    },
    ComponentInfo {
        family: "unstyled",
        module: "formerror",
        name: "FormErrorList",
    },
    ComponentInfo {
        family: "unstyled",
        module: "formerror",
        name: "InlineError",
    },
];

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FormErrorProps {
    pub message: String,
    pub id: String,
    pub size: &'static str,
    pub show_icon: bool,
    pub class: String,
    pub attributes: BTreeMap<String, String>,
    pub aria_live: &'static str,
}

impl Default for FormErrorProps {
    fn default() -> Self {
        Self {
            message: String::new(),
            id: String::new(),
            size: shared::SIZE_SM,
            show_icon: true,
            class: String::new(),
            attributes: BTreeMap::new(),
            aria_live: "polite",
        }
    }
}

impl FormErrorProps {
    #[must_use]
    pub fn with_message(mut self, message: impl Into<String>) -> Self {
        self.message = message.into();
        self
    }

    #[must_use]
    pub fn with_id(mut self, id: impl Into<String>) -> Self {
        self.id = id.into();
        self
    }

    #[must_use]
    pub fn with_size(mut self, size: &'static str) -> Self {
        self.size = size;
        self
    }

    #[must_use]
    pub const fn with_show_icon(mut self, show_icon: bool) -> Self {
        self.show_icon = show_icon;
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
    pub fn with_aria_live(mut self, aria_live: &'static str) -> Self {
        self.aria_live = aria_live;
        self
    }

    #[must_use]
    pub fn build_classes(&self) -> String {
        let size_class = match self.size {
            shared::SIZE_XS => "text-xs",
            shared::SIZE_SM => "text-sm",
            shared::SIZE_MD => "text-base",
            shared::SIZE_LG => "text-lg",
            _ => "",
        };
        shared::build_classes(&[
            "flex items-start gap-1.5 text-destructive font-medium",
            size_class,
            self.class.as_str(),
        ])
    }

    #[must_use]
    pub fn build_attributes(&self) -> BTreeMap<String, String> {
        let mut attrs = self.attributes.clone();
        attrs.insert("role".to_string(), "alert".to_string());
        if !self.aria_live.is_empty() {
            attrs.insert("aria-live".to_string(), self.aria_live.to_string());
        }
        attrs
    }
}

fn error_icon() -> Markup {
    html! {
        svg class="w-4 h-4 mt-0.5 flex-shrink-0" fill="none" stroke="currentColor" viewBox="0 0 24 24" aria-hidden="true" {
            path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M12 8v4m0 4h.01M21 12a9 9 0 11-18 0 9 9 0 0118 0z" {}
        }
    }
}

fn error_container(props: &FormErrorProps, body: Markup) -> Markup {
    let attrs = props.build_attributes();
    html! {
        div
            id=[shared::optional_attr(&props.id)]
            class=(props.build_classes())
            role=(shared::attr_value(&attrs, "role").unwrap_or("alert"))
            aria-live=[shared::attr_value(&attrs, "aria-live")]
            title=[shared::attr_value(&attrs, "title")]
        {
            (body)
        }
    }
}

#[must_use]
pub fn FormError(props: &FormErrorProps) -> Markup {
    if props.message.is_empty() {
        return html! {};
    }
    error_container(
        props,
        html! {
            @if props.show_icon {
                (error_icon())
            }
            span class="flex-1" { (props.message) }
        },
    )
}

#[must_use]
pub fn FormErrorWithIcon(props: &FormErrorProps, icon: Option<Markup>) -> Markup {
    if props.message.is_empty() {
        return html! {};
    }
    error_container(
        props,
        html! {
            @if let Some(icon) = icon {
                div class="flex-shrink-0 mt-0.5" {
                    (icon)
                }
            }
            span class="flex-1" { (props.message) }
        },
    )
}

#[must_use]
pub fn FormErrorList(errors: &[&str], props: &FormErrorProps) -> Markup {
    if errors.is_empty() {
        return html! {};
    }
    error_container(
        props,
        html! {
            @if props.show_icon {
                (error_icon())
            }
            div class="flex-1" {
                @if errors.len() == 1 {
                    span { (errors[0]) }
                } @else {
                    ul class="list-disc list-inside space-y-1" {
                        @for err in errors {
                            li { (err) }
                        }
                    }
                }
            }
        },
    )
}

#[must_use]
pub fn InlineError(message: impl Into<String>) -> Markup {
    FormError(
        &FormErrorProps::default()
            .with_message(message)
            .with_size(shared::SIZE_SM)
            .with_show_icon(true)
            .with_aria_live("polite"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const SIZES: &[(&str, &str)] = &[
        (shared::SIZE_XS, "text-xs"),
        (shared::SIZE_SM, "text-sm"),
        (shared::SIZE_MD, "text-base"),
        (shared::SIZE_LG, "text-lg"),
        (shared::SIZE_XL, ""),
    ];

    #[test]
    fn default_props_match_go_defaults() {
        let props = FormErrorProps::default();
        assert_eq!(props.size, shared::SIZE_SM);
        assert!(props.show_icon);
        assert_eq!(props.aria_live, "polite");
    }

    #[test]
    fn helper_classes_and_attributes_cover_sizes_aria_and_custom_attributes() {
        for &(size, expected) in SIZES {
            let props = FormErrorProps::default()
                .with_size(size)
                .with_class("custom")
                .with_attribute("title", "Error")
                .with_aria_live("assertive");
            let classes = props.build_classes();
            assert!(classes.contains("flex items-start gap-1.5"));
            if expected.is_empty() {
                assert!(!classes.contains("text-xl"));
            } else {
                assert!(classes.contains(expected));
            }
            assert!(classes.contains("custom"));
            let attrs = props.build_attributes();
            assert_eq!(attrs.get("role"), Some(&"alert".to_string()));
            assert_eq!(attrs.get("aria-live"), Some(&"assertive".to_string()));
            assert_eq!(attrs.get("title"), Some(&"Error".to_string()));
        }
    }

    #[test]
    fn form_error_renders_nothing_for_empty_message_and_full_html_for_message() {
        assert_eq!(FormError(&FormErrorProps::default()).into_string(), "");

        let html = FormError(
            &FormErrorProps::default()
                .with_message("<Invalid>")
                .with_id("email-error")
                .with_attribute("title", "\"Bad\" & field"),
        )
        .into_string();

        assert!(html.starts_with(r#"<div id="email-error" class=""#));
        assert!(
            html.contains(r#"role="alert" aria-live="polite" title="&quot;Bad&quot; &amp; field""#)
        );
        assert!(html.contains(r#"<svg class="w-4 h-4 mt-0.5 flex-shrink-0""#));
        assert!(html.contains(r#"<span class="flex-1">&lt;Invalid&gt;</span>"#));

        let no_icon = FormError(
            &FormErrorProps::default()
                .with_message("Invalid")
                .with_show_icon(false),
        )
        .into_string();
        assert!(!no_icon.contains("<svg "));
    }

    #[test]
    fn form_error_with_icon_renders_optional_custom_icon() {
        let props = FormErrorProps::default().with_message("Invalid");
        assert_eq!(
            FormErrorWithIcon(&FormErrorProps::default(), Some(html! { svg {} })).into_string(),
            ""
        );
        let without = FormErrorWithIcon(&props, None).into_string();
        assert!(!without.contains("flex-shrink-0 mt-0.5"));
        assert!(without.contains(">Invalid</span>"));

        let with =
            FormErrorWithIcon(&props, Some(html! { svg class="custom-icon" {} })).into_string();
        assert!(with.contains(
            r#"<div class="flex-shrink-0 mt-0.5"><svg class="custom-icon"></svg></div>"#
        ));
    }

    #[test]
    fn form_error_list_renders_empty_single_and_multiple_variations() {
        assert_eq!(
            FormErrorList(&[], &FormErrorProps::default()).into_string(),
            ""
        );

        let single = FormErrorList(&["One"], &FormErrorProps::default()).into_string();
        assert!(single.contains(r#"<span>One</span>"#));
        assert!(!single.contains("<ul "));

        let multiple = FormErrorList(&["One", "Two"], &FormErrorProps::default()).into_string();
        assert!(multiple.contains(r#"<ul class="list-disc list-inside space-y-1">"#));
        assert!(multiple.contains("<li>One</li>"));
        assert!(multiple.contains("<li>Two</li>"));
    }

    #[test]
    fn inline_error_renders_small_polite_error_with_icon() {
        let html = InlineError("Inline").into_string();
        assert!(html.contains("text-sm"));
        assert!(html.contains(r#"aria-live="polite""#));
        assert!(html.contains("<svg "));
        assert!(html.contains(">Inline</span>"));
    }
}
