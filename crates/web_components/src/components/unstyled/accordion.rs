#![allow(non_snake_case)]

use crate::components::{ComponentInfo, shared};
use maud::{Markup, html};
use std::collections::BTreeMap;

pub const ACCORDION_TYPE_SINGLE: &str = "single";
pub const ACCORDION_TYPE_MULTIPLE: &str = "multiple";

pub const COMPONENTS: &[ComponentInfo] = &[
    ComponentInfo {
        family: "unstyled",
        module: "accordion",
        name: "Accordion",
    },
    ComponentInfo {
        family: "unstyled",
        module: "accordion",
        name: "AccordionItem",
    },
    ComponentInfo {
        family: "unstyled",
        module: "accordion",
        name: "AccordionTrigger",
    },
    ComponentInfo {
        family: "unstyled",
        module: "accordion",
        name: "AccordionContent",
    },
];

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AccordionProps {
    pub accordion_type: &'static str,
    pub collapsible: bool,
    pub default_value: String,
    pub class: String,
    pub attributes: BTreeMap<String, String>,
}

impl Default for AccordionProps {
    fn default() -> Self {
        Self {
            accordion_type: ACCORDION_TYPE_SINGLE,
            collapsible: true,
            default_value: String::new(),
            class: String::new(),
            attributes: BTreeMap::new(),
        }
    }
}

impl AccordionProps {
    #[must_use]
    pub const fn with_type(mut self, value: &'static str) -> Self {
        self.accordion_type = value;
        self
    }

    #[must_use]
    pub const fn with_collapsible(mut self, value: bool) -> Self {
        self.collapsible = value;
        self
    }

    #[must_use]
    pub fn with_default_value(mut self, value: impl Into<String>) -> Self {
        self.default_value = value.into();
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
    pub fn build_attributes(&self) -> BTreeMap<String, String> {
        let mut attrs = self.attributes.clone();
        attrs.insert("data-type".to_string(), self.accordion_type.to_string());
        attrs.insert("data-collapsible".to_string(), self.collapsible.to_string());
        if !self.default_value.is_empty() {
            attrs.insert("data-value".to_string(), self.default_value.clone());
        }
        attrs
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct AccordionItemProps {
    pub value: String,
    pub disabled: bool,
    pub class: String,
    pub attributes: BTreeMap<String, String>,
}

impl AccordionItemProps {
    #[must_use]
    pub fn with_value(mut self, value: impl Into<String>) -> Self {
        self.value = value.into();
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
        shared::build_classes(&["border-b accordion-item-accent", self.class.as_str()])
    }

    #[must_use]
    pub fn build_attributes(&self) -> BTreeMap<String, String> {
        let mut attrs = self.attributes.clone();
        if !self.value.is_empty() {
            attrs.insert("data-value".to_string(), self.value.clone());
        }
        attrs.insert("data-state".to_string(), "closed".to_string());
        if self.disabled {
            attrs.insert("data-disabled".to_string(), "true".to_string());
        }
        attrs
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct AccordionTriggerProps {
    pub class: String,
    pub aria_label: String,
    pub attributes: BTreeMap<String, String>,
}

impl AccordionTriggerProps {
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
        shared::build_classes(&[
            "flex flex-1 items-center justify-between py-4 px-3 font-medium transition-all duration-200 accordion-trigger-hover [&[data-state=open]>svg]:rotate-180 focus-visible:outline-2 focus-visible:outline-ring focus-visible:outline-offset-2 focus-visible:rounded-sm",
            self.class.as_str(),
        ])
    }

    #[must_use]
    pub fn build_attributes(&self) -> BTreeMap<String, String> {
        let mut attrs = self.attributes.clone();
        attrs.insert("aria-expanded".to_string(), "false".to_string());
        if !self.aria_label.is_empty() {
            attrs.insert("aria-label".to_string(), self.aria_label.clone());
        }
        attrs
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct AccordionContentProps {
    pub class: String,
    pub attributes: BTreeMap<String, String>,
}

impl AccordionContentProps {
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
        shared::build_classes(&["text-sm", self.class.as_str()])
    }

    #[must_use]
    pub const fn build_grid_classes(&self) -> &'static str {
        "accordion-content-grid"
    }

    #[must_use]
    pub const fn build_inner_classes(&self) -> &'static str {
        "accordion-content-inner"
    }

    #[must_use]
    pub fn build_attributes(&self) -> BTreeMap<String, String> {
        let mut attrs = self.attributes.clone();
        attrs.insert("data-state".to_string(), "closed".to_string());
        attrs.insert("role".to_string(), "region".to_string());
        attrs
    }
}

#[must_use]
pub fn Accordion(props: &AccordionProps, children: Markup) -> Markup {
    let attrs = props.build_attributes();
    html! {
        div
            class=(props.build_classes())
            data-type=[shared::attr_value(&attrs, "data-type")]
            data-collapsible=[shared::attr_value(&attrs, "data-collapsible")]
            data-value=[shared::attr_value(&attrs, "data-value")]
            id=[shared::attr_value(&attrs, "id")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
        {
            (children)
        }
    }
}

#[must_use]
pub fn AccordionItem(props: &AccordionItemProps, children: Markup) -> Markup {
    let attrs = props.build_attributes();
    html! {
        div
            class=(props.build_classes())
            data-value=[shared::attr_value(&attrs, "data-value")]
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
pub fn AccordionTrigger(props: &AccordionTriggerProps, children: Markup) -> Markup {
    let attrs = props.build_attributes();
    html! {
        button
            type="button"
            class=(props.build_classes())
            aria-expanded=[shared::attr_value(&attrs, "aria-expanded")]
            aria-label=[shared::attr_value(&attrs, "aria-label")]
            id=[shared::attr_value(&attrs, "id")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
        {
            (children)
            svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" class="shrink-0 transition-transform duration-200" {
                path d="m6 9 6 6 6-6" {}
            }
        }
    }
}

#[must_use]
pub fn AccordionContent(props: &AccordionContentProps, children: Markup) -> Markup {
    let attrs = props.build_attributes();
    html! {
        div
            data-state=[shared::attr_value(&attrs, "data-state")]
            role=[shared::attr_value(&attrs, "role")]
            id=[shared::attr_value(&attrs, "id")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
        {
            div class=(props.build_grid_classes()) {
                div class=(props.build_inner_classes()) {
                    div class=(format!("{} pb-4 pt-0 px-3", props.build_classes())) {
                        (children)
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
    fn accordion_helpers_cover_defaults_classes_and_attributes() {
        let props = AccordionProps::default();
        assert_eq!(props.accordion_type, ACCORDION_TYPE_SINGLE);
        assert!(props.collapsible);
        assert_eq!(props.build_classes(), "w-full");
        let attrs = props
            .with_type(ACCORDION_TYPE_MULTIPLE)
            .with_collapsible(false)
            .with_default_value("item-1")
            .with_class("space-y-2")
            .with_attribute("id", "faq")
            .build_attributes();
        assert_eq!(attrs.get("data-type").map(String::as_str), Some("multiple"));
        assert_eq!(
            attrs.get("data-collapsible").map(String::as_str),
            Some("false")
        );
        assert_eq!(attrs.get("data-value").map(String::as_str), Some("item-1"));
        assert_eq!(attrs.get("id").map(String::as_str), Some("faq"));
        assert_eq!(
            AccordionProps::default()
                .with_class("space-y-2")
                .build_classes(),
            "w-full space-y-2"
        );
    }

    #[test]
    fn item_trigger_and_content_helpers_cover_all_branches() {
        let item_attrs = AccordionItemProps::default()
            .with_value("item-2")
            .with_disabled(true)
            .with_class("last:border-0")
            .with_attribute("data-testid", "item")
            .build_attributes();
        assert_eq!(
            AccordionItemProps::default()
                .with_class("last:border-0")
                .build_classes(),
            "border-b accordion-item-accent last:border-0"
        );
        assert_eq!(
            item_attrs.get("data-value").map(String::as_str),
            Some("item-2")
        );
        assert_eq!(
            item_attrs.get("data-state").map(String::as_str),
            Some("closed")
        );
        assert_eq!(
            item_attrs.get("data-disabled").map(String::as_str),
            Some("true")
        );
        assert_eq!(
            item_attrs.get("data-testid").map(String::as_str),
            Some("item")
        );

        let trigger_attrs = AccordionTriggerProps::default()
            .with_class("trigger-extra")
            .with_aria_label("Open section")
            .with_attribute("id", "trigger")
            .build_attributes();
        let trigger_classes = AccordionTriggerProps::default()
            .with_class("trigger-extra")
            .build_classes();
        assert!(trigger_classes.contains("accordion-trigger-hover"));
        assert!(trigger_classes.ends_with("trigger-extra"));
        assert_eq!(
            trigger_attrs.get("aria-expanded").map(String::as_str),
            Some("false")
        );
        assert_eq!(
            trigger_attrs.get("aria-label").map(String::as_str),
            Some("Open section")
        );
        assert_eq!(trigger_attrs.get("id").map(String::as_str), Some("trigger"));

        let content = AccordionContentProps::default().with_class("prose");
        assert_eq!(content.build_classes(), "text-sm prose");
        assert_eq!(content.build_grid_classes(), "accordion-content-grid");
        assert_eq!(content.build_inner_classes(), "accordion-content-inner");
        let content_attrs = content.build_attributes();
        assert_eq!(
            content_attrs.get("data-state").map(String::as_str),
            Some("closed")
        );
        assert_eq!(
            content_attrs.get("role").map(String::as_str),
            Some("region")
        );
    }

    #[test]
    fn accordion_parts_render_real_html_and_escape_children() {
        let trigger = AccordionTrigger(
            &AccordionTriggerProps::default().with_aria_label("Open"),
            html! { "<Question>" },
        );
        let content = AccordionContent(
            &AccordionContentProps::default()
                .with_class("text-muted-foreground")
                .with_attribute("id", "answer"),
            html! { p { "<Answer>" } },
        );
        let item = AccordionItem(
            &AccordionItemProps::default()
                .with_value("q1")
                .with_disabled(true),
            html! { (trigger) (content) },
        );
        let rendered = Accordion(
            &AccordionProps::default()
                .with_type(ACCORDION_TYPE_MULTIPLE)
                .with_default_value("q1")
                .with_attribute("data-testid", "accordion"),
            item,
        )
        .into_string();
        assert!(rendered.starts_with("<div class=\"w-full\""));
        assert!(rendered.contains("data-type=\"multiple\""));
        assert!(rendered.contains("data-collapsible=\"true\""));
        assert!(rendered.contains("data-value=\"q1\""));
        assert!(rendered.contains("data-testid=\"accordion\""));
        assert!(rendered.contains("<div class=\"border-b accordion-item-accent\" data-value=\"q1\" data-state=\"closed\" data-disabled=\"true\">"));
        assert!(rendered.contains("<button type=\"button\""));
        assert!(rendered.contains("aria-expanded=\"false\""));
        assert!(rendered.contains("aria-label=\"Open\""));
        assert!(rendered.contains("&lt;Question&gt;"));
        assert!(rendered.contains("<path d=\"m6 9 6 6 6-6\"></path>"));
        assert!(rendered.contains("<div data-state=\"closed\" role=\"region\" id=\"answer\">"));
        assert!(rendered.contains("<div class=\"accordion-content-grid\"><div class=\"accordion-content-inner\"><div class=\"text-sm text-muted-foreground pb-4 pt-0 px-3\"><p>&lt;Answer&gt;</p></div></div></div>"));
    }
}
