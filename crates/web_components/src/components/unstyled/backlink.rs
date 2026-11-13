#![allow(non_snake_case)]

use crate::components::{ComponentInfo, shared};
use maud::{Markup, html};
use std::collections::BTreeMap;

pub const ICON_ARROW_LEFT: &str = "arrow-left";
pub const ICON_CHEVRON_LEFT: &str = "chevron-left";

pub const COMPONENTS: &[ComponentInfo] = &[
    ComponentInfo {
        family: "unstyled",
        module: "backlink",
        name: "BackLink",
    },
    ComponentInfo {
        family: "unstyled",
        module: "backlink",
        name: "backLinkRoot",
    },
];

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BackLinkProps {
    pub href: String,
    pub label: String,
    pub variant: &'static str,
    pub size: &'static str,
    pub icon: &'static str,
    pub class: String,
    pub attributes: BTreeMap<String, String>,
    pub aria_label: String,
}

impl Default for BackLinkProps {
    fn default() -> Self {
        Self {
            href: String::new(),
            label: String::new(),
            variant: shared::VARIANT_LINK,
            size: shared::SIZE_SM,
            icon: ICON_ARROW_LEFT,
            class: String::new(),
            attributes: BTreeMap::new(),
            aria_label: String::new(),
        }
    }
}

impl BackLinkProps {
    #[must_use]
    pub fn with_href(mut self, value: impl Into<String>) -> Self {
        self.href = value.into();
        self
    }

    #[must_use]
    pub fn with_label(mut self, value: impl Into<String>) -> Self {
        self.label = value.into();
        self
    }

    #[must_use]
    pub const fn with_variant(mut self, value: &'static str) -> Self {
        self.variant = value;
        self
    }

    #[must_use]
    pub const fn with_size(mut self, value: &'static str) -> Self {
        self.size = value;
        self
    }

    #[must_use]
    pub const fn with_icon(mut self, value: &'static str) -> Self {
        self.icon = value;
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
    pub fn normalize(&self) -> Self {
        let mut normalized = self.clone();
        if normalized.variant.is_empty() {
            normalized.variant = shared::VARIANT_LINK;
        }
        if normalized.size.is_empty() {
            normalized.size = shared::SIZE_SM;
        }
        if normalized.icon.is_empty() {
            normalized.icon = ICON_ARROW_LEFT;
        }
        normalized
    }

    #[must_use]
    pub fn build_class(&self) -> String {
        shared::build_classes(&["w-fit", self.class.as_str()])
    }

    #[must_use]
    pub fn aria_label(&self) -> String {
        if self.aria_label.trim().is_empty() {
            self.label.clone()
        } else {
            self.aria_label.clone()
        }
    }

    #[must_use]
    pub fn button_classes(&self) -> String {
        shared::build_classes(&[
            "inline-flex items-center justify-center gap-2 whitespace-nowrap font-medium transition-all duration-150 ease-out cursor-pointer select-none focus-visible:outline-2 focus-visible:outline-ring focus-visible:outline-offset-2 disabled:pointer-events-none disabled:opacity-50 active:scale-[0.98]",
            shared::button_variant_classes(self.variant),
            shared::button_size_classes(self.size),
            self.build_class().as_str(),
        ])
    }
}

#[must_use]
pub fn decorativeIconAttrs() -> BTreeMap<String, String> {
    BTreeMap::from([
        ("aria-hidden".to_string(), "true".to_string()),
        ("focusable".to_string(), "false".to_string()),
    ])
}

fn icon(props: &BackLinkProps) -> Markup {
    let decorative_attrs = decorativeIconAttrs();
    if props.icon == ICON_CHEVRON_LEFT {
        html! {
            svg
                xmlns="http://www.w3.org/2000/svg"
                width="16"
                height="16"
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                stroke-width="2"
                stroke-linecap="round"
                stroke-linejoin="round"
                class="shrink-0"
                aria-hidden=[shared::attr_value(&decorative_attrs, "aria-hidden")]
                focusable=[shared::attr_value(&decorative_attrs, "focusable")]
            {
                path d="m15 18-6-6 6-6" {}
            }
        }
    } else {
        html! {
            svg
                xmlns="http://www.w3.org/2000/svg"
                width="16"
                height="16"
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                stroke-width="2"
                stroke-linecap="round"
                stroke-linejoin="round"
                class="shrink-0"
                aria-hidden=[shared::attr_value(&decorative_attrs, "aria-hidden")]
                focusable=[shared::attr_value(&decorative_attrs, "focusable")]
            {
                path d="m12 19-7-7 7-7" {}
                path d="M19 12H5" {}
            }
        }
    }
}

#[must_use]
pub fn BackLink(props: &BackLinkProps) -> Markup {
    backLinkRoot(&props.normalize())
}

#[must_use]
pub fn backLinkRoot(props: &BackLinkProps) -> Markup {
    let attrs = props.attributes.clone();
    if props.href.is_empty() || props.label.is_empty() {
        return html! {};
    }
    html! {
        a
            href=(&props.href)
            class=(props.button_classes())
            aria-label=(props.aria_label())
            id=[shared::attr_value(&attrs, "id")]
            title=[shared::attr_value(&attrs, "title")]
            data-test=[shared::attr_value(&attrs, "data-test")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
            data-on:click=[shared::attr_value(&attrs, "data-on:click")]
        {
            (icon(props))
            span { (&props.label) }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn helpers_normalize_classes_aria_and_decorative_icon_attrs() {
        let props = BackLinkProps {
            variant: "",
            size: "",
            icon: "",
            ..BackLinkProps::default()
        }
        .with_label("Back");
        let normalized = props.normalize();
        assert_eq!(normalized.variant, shared::VARIANT_LINK);
        assert_eq!(normalized.size, shared::SIZE_SM);
        assert_eq!(normalized.icon, ICON_ARROW_LEFT);
        assert_eq!(normalized.build_class(), "w-fit");
        assert!(normalized.button_classes().contains("w-fit"));
        assert!(
            normalized
                .button_classes()
                .contains(shared::button_size_classes(shared::SIZE_SM))
        );
        assert_eq!(normalized.aria_label(), "Back");
        assert_eq!(
            normalized.with_aria_label("Return").aria_label(),
            "Return".to_string()
        );
        let attrs = decorativeIconAttrs();
        assert_eq!(attrs.get("aria-hidden").map(String::as_str), Some("true"));
        assert_eq!(attrs.get("focusable").map(String::as_str), Some("false"));
    }

    #[test]
    fn backlink_renders_arrow_icon_label_attrs_and_escapes_text() {
        let html = BackLink(
            &BackLinkProps::default()
                .with_href("/en-us/dashboard")
                .with_label("<Back>")
                .with_attribute("data-test", "dashboard-back")
                .with_attribute("data-on:click", "$open = false"),
        )
        .into_string();
        assert!(html.starts_with("<a "));
        assert!(html.contains("href=\"/en-us/dashboard\""));
        assert!(html.contains("aria-label=\"&lt;Back&gt;\""));
        assert!(html.contains("data-test=\"dashboard-back\""));
        assert!(html.contains("data-on:click=\"$open = false\""));
        assert!(html.contains("<svg "));
        assert!(html.contains("aria-hidden=\"true\""));
        assert!(html.contains("focusable=\"false\""));
        assert!(html.contains("<path d=\"m12 19-7-7 7-7\"></path>"));
        assert!(html.contains("<path d=\"M19 12H5\"></path>"));
        assert!(html.contains("<span>&lt;Back&gt;</span>"));
    }

    #[test]
    fn backlink_renders_chevron_variant_and_empty_state() {
        let html = BackLink(
            &BackLinkProps::default()
                .with_href("/sessions")
                .with_label("Back to Sessions")
                .with_variant(shared::VARIANT_OUTLINE)
                .with_size(shared::SIZE_LG)
                .with_icon(ICON_CHEVRON_LEFT)
                .with_class("custom")
                .with_aria_label("Return to sessions")
                .with_attribute("id", "session-back")
                .with_attribute("data-testid", "back-link"),
        )
        .into_string();
        assert!(html.contains("border border-border"));
        assert!(html.contains(shared::button_size_classes(shared::SIZE_LG)));
        assert!(html.contains("w-fit custom"));
        assert!(html.contains("aria-label=\"Return to sessions\""));
        assert!(html.contains("id=\"session-back\""));
        assert!(html.contains("data-testid=\"back-link\""));
        assert!(html.contains("<path d=\"m15 18-6-6 6-6\"></path>"));

        assert_eq!(
            BackLink(&BackLinkProps::default().with_label("Missing href")).into_string(),
            ""
        );
        assert_eq!(
            BackLink(&BackLinkProps::default().with_href("/missing-label")).into_string(),
            ""
        );
    }
}
