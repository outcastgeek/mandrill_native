#![allow(non_snake_case)]

use crate::components::{ComponentInfo, shared};
use maud::{Markup, html};
use std::collections::BTreeMap;

pub const COMPONENTS: &[ComponentInfo] = &[
    ComponentInfo {
        family: "unstyled",
        module: "collapsible",
        name: "Collapsible",
    },
    ComponentInfo {
        family: "unstyled",
        module: "collapsible",
        name: "CollapsibleTrigger",
    },
    ComponentInfo {
        family: "unstyled",
        module: "collapsible",
        name: "CollapsibleContent",
    },
];

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CollapsibleProps {
    pub open: bool,
    pub default_open: bool,
    pub disabled: bool,
    pub class: String,
    pub attributes: BTreeMap<String, String>,
}

impl CollapsibleProps {
    #[must_use]
    pub const fn with_open(mut self, value: bool) -> Self {
        self.open = value;
        self
    }

    #[must_use]
    pub const fn with_default_open(mut self, value: bool) -> Self {
        self.default_open = value;
        self
    }

    #[must_use]
    pub const fn with_disabled(mut self, value: bool) -> Self {
        self.disabled = value;
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
    pub fn build_classes(&self) -> String {
        shared::build_classes(&["w-full", self.class.as_str()])
    }

    #[must_use]
    pub const fn is_open(&self) -> bool {
        self.open || self.default_open
    }

    #[must_use]
    pub const fn get_state(&self) -> &'static str {
        if self.is_open() { "open" } else { "closed" }
    }

    #[must_use]
    pub fn build_attributes(&self) -> BTreeMap<String, String> {
        let mut attrs = self.attributes.clone();
        attrs
            .entry("data-state".to_string())
            .or_insert_with(|| self.get_state().to_string());
        if self.disabled {
            attrs.insert("data-disabled".to_string(), "true".to_string());
        }
        attrs
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CollapsibleTriggerProps {
    pub as_child: bool,
    pub class: String,
    pub aria_label: String,
    pub attributes: BTreeMap<String, String>,
}

impl CollapsibleTriggerProps {
    #[must_use]
    pub const fn with_as_child(mut self, value: bool) -> Self {
        self.as_child = value;
        self
    }

    #[must_use]
    pub fn with_class(mut self, value: impl Into<String>) -> Self {
        self.class = value.into();
        self
    }

    #[must_use]
    pub fn with_aria_label(mut self, value: impl Into<String>) -> Self {
        self.aria_label = value.into();
        self
    }

    #[must_use]
    pub fn with_attribute(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.attributes.insert(name.into(), value.into());
        self
    }

    #[must_use]
    pub fn build_classes(&self) -> String {
        if self.as_child {
            shared::build_classes(&[self.class.as_str()])
        } else {
            shared::build_classes(&[
                "flex items-center justify-between w-full",
                self.class.as_str(),
            ])
        }
    }

    #[must_use]
    pub fn build_attributes(&self) -> BTreeMap<String, String> {
        let mut attrs = self.attributes.clone();
        attrs
            .entry("aria-expanded".to_string())
            .or_insert_with(|| "false".to_string());
        if !self.aria_label.is_empty() {
            attrs.insert("aria-label".to_string(), self.aria_label.clone());
        }
        attrs
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CollapsibleContentProps {
    pub class: String,
    pub attributes: BTreeMap<String, String>,
}

impl CollapsibleContentProps {
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
    pub fn build_classes(&self) -> String {
        shared::build_classes(&[
            "overflow-hidden transition-all data-[state=closed]:animate-collapsible-up data-[state=open]:animate-collapsible-down",
            self.class.as_str(),
        ])
    }

    #[must_use]
    pub fn build_attributes(&self) -> BTreeMap<String, String> {
        let mut attrs = self.attributes.clone();
        attrs
            .entry("data-state".to_string())
            .or_insert_with(|| "closed".to_string());
        attrs
    }
}

#[must_use]
pub fn Collapsible(props: &CollapsibleProps, children: Markup) -> Markup {
    let attrs = props.build_attributes();
    html! {
        div
            class=(props.build_classes())
            data-state=[shared::attr_value(&attrs, "data-state")]
            data-disabled=[shared::attr_value(&attrs, "data-disabled")]
            id=[shared::attr_value(&attrs, "id")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
        {
            (children)
        }
    }
}

#[must_use]
pub fn CollapsibleTrigger(props: &CollapsibleTriggerProps, children: Markup) -> Markup {
    let attrs = props.build_attributes();
    html! {
        button
            type="button"
            class=(props.build_classes())
            aria-expanded=[shared::attr_value(&attrs, "aria-expanded")]
            aria-label=[shared::attr_value(&attrs, "aria-label")]
            id=[shared::attr_value(&attrs, "id")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
            aria-controls=[shared::attr_value(&attrs, "aria-controls")]
            data-on:click=[shared::attr_value(&attrs, "data-on:click")]
            data-attr:aria-expanded=[shared::attr_value(&attrs, "data-attr:aria-expanded")]
            data-attr:data-state=[shared::attr_value(&attrs, "data-attr:data-state")]
            data-attr:data-expanded=[shared::attr_value(&attrs, "data-attr:data-expanded")]
        {
            (children)
        }
    }
}

#[must_use]
pub fn CollapsibleContent(props: &CollapsibleContentProps, children: Markup) -> Markup {
    let attrs = props.build_attributes();
    html! {
        div
            class=(props.build_classes())
            data-state=[shared::attr_value(&attrs, "data-state")]
            id=[shared::attr_value(&attrs, "id")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
            data-show=[shared::attr_value(&attrs, "data-show")]
            style=[shared::attr_value(&attrs, "style")]
            data-attr:data-state=[shared::attr_value(&attrs, "data-attr:data-state")]
        {
            (children)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collapsible_helpers_cover_state_classes_and_attributes() {
        let props = CollapsibleProps::default();
        assert!(!props.is_open());
        assert_eq!(props.get_state(), "closed");
        assert_eq!(props.build_classes(), "w-full");
        let open = CollapsibleProps::default()
            .with_default_open(true)
            .with_disabled(true)
            .with_class("space-y-2")
            .with_attribute("id", "details");
        assert!(open.is_open());
        assert_eq!(open.get_state(), "open");
        assert_eq!(open.build_classes(), "w-full space-y-2");
        let attrs = open.build_attributes();
        assert_eq!(attrs.get("data-state").map(String::as_str), Some("open"));
        assert_eq!(attrs.get("data-disabled").map(String::as_str), Some("true"));
        assert_eq!(attrs.get("id").map(String::as_str), Some("details"));
        let overridden = CollapsibleProps::default()
            .with_open(true)
            .with_attribute("data-state", "closed")
            .build_attributes();
        assert_eq!(
            overridden.get("data-state").map(String::as_str),
            Some("closed")
        );
    }

    #[test]
    fn trigger_and_content_helpers_cover_as_child_defaults_and_overrides() {
        assert_eq!(
            CollapsibleTriggerProps::default().build_classes(),
            "flex items-center justify-between w-full"
        );
        assert_eq!(
            CollapsibleTriggerProps::default()
                .with_as_child(true)
                .with_class("custom")
                .build_classes(),
            "custom"
        );
        let trigger_attrs = CollapsibleTriggerProps::default()
            .with_aria_label("Toggle")
            .with_attribute("aria-expanded", "true")
            .build_attributes();
        assert_eq!(
            trigger_attrs.get("aria-expanded").map(String::as_str),
            Some("true")
        );
        assert_eq!(
            trigger_attrs.get("aria-label").map(String::as_str),
            Some("Toggle")
        );

        assert!(
            CollapsibleContentProps::default()
                .build_classes()
                .contains("data-[state=open]:animate-collapsible-down")
        );
        let content_attrs = CollapsibleContentProps::default()
            .with_attribute("data-state", "open")
            .build_attributes();
        assert_eq!(
            content_attrs.get("data-state").map(String::as_str),
            Some("open")
        );
    }

    #[test]
    fn collapsible_parts_render_actual_html_and_escape_children() {
        let trigger = CollapsibleTrigger(
            &CollapsibleTriggerProps::default()
                .with_aria_label("Toggle details")
                .with_attribute("id", "trigger"),
            html! { "<More>" },
        );
        let content = CollapsibleContent(
            &CollapsibleContentProps::default()
                .with_class("pt-2")
                .with_attribute("data-testid", "content"),
            html! { p { "<Hidden>" } },
        );
        let rendered = Collapsible(
            &CollapsibleProps::default()
                .with_open(true)
                .with_disabled(true)
                .with_attribute("data-testid", "collapsible"),
            html! { (trigger) (content) },
        )
        .into_string();
        assert!(
            rendered
                .starts_with("<div class=\"w-full\" data-state=\"open\" data-disabled=\"true\"")
        );
        assert!(rendered.contains("data-testid=\"collapsible\""));
        assert!(rendered.contains("<button type=\"button\" class=\"flex items-center justify-between w-full\" aria-expanded=\"false\" aria-label=\"Toggle details\" id=\"trigger\">&lt;More&gt;</button>"));
        assert!(rendered.contains("<div class=\"overflow-hidden transition-all data-[state=closed]:animate-collapsible-up data-[state=open]:animate-collapsible-down pt-2\" data-state=\"closed\" data-testid=\"content\"><p>&lt;Hidden&gt;</p></div>"));
    }
}
