#![allow(non_snake_case)]

use crate::components::{ComponentInfo, shared};
use maud::{Markup, html};
use std::collections::BTreeMap;

pub const COMPONENTS: &[ComponentInfo] = &[
    ComponentInfo {
        family: "unstyled",
        module: "switch",
        name: "Switch",
    },
    ComponentInfo {
        family: "unstyled",
        module: "switch",
        name: "SwitchWithLabel",
    },
];

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SwitchProps {
    pub checked: bool,
    pub disabled: bool,
    pub name: String,
    pub id: String,
    pub value: String,
    pub size: &'static str,
    pub variant: &'static str,
    pub class: String,
    pub attributes: BTreeMap<String, String>,
    pub aria_label: String,
    pub aria_described_by: String,
}

impl Default for SwitchProps {
    fn default() -> Self {
        Self {
            checked: false,
            disabled: false,
            name: String::new(),
            id: String::new(),
            value: String::new(),
            size: shared::SIZE_MD,
            variant: shared::VARIANT_PRIMARY,
            class: String::new(),
            attributes: BTreeMap::new(),
            aria_label: String::new(),
            aria_described_by: String::new(),
        }
    }
}

impl SwitchProps {
    #[must_use]
    pub const fn with_checked(mut self, checked: bool) -> Self {
        self.checked = checked;
        self
    }
    #[must_use]
    pub const fn with_disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }
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
    pub const fn with_size(mut self, size: &'static str) -> Self {
        self.size = size;
        self
    }
    #[must_use]
    pub const fn with_variant(mut self, variant: &'static str) -> Self {
        self.variant = variant;
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
        let bg_classes = if self.class.is_empty() {
            "data-[state=checked]:bg-primary data-[state=unchecked]:bg-input"
        } else {
            ""
        };
        shared::build_classes(&[
            "peer inline-flex shrink-0 cursor-pointer items-center rounded-full border-2 border-transparent switch-track-hover transition-colors duration-150 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 focus-visible:ring-offset-background disabled:cursor-not-allowed disabled:opacity-50",
            shared::switch_size_classes(self.size),
            bg_classes,
            self.class.as_str(),
        ])
    }

    #[must_use]
    pub fn build_thumb_classes(&self) -> String {
        let bg_class = if self.class.is_empty() {
            "bg-background"
        } else {
            ""
        };
        shared::build_classes(&[
            "pointer-events-none block rounded-full shadow-lg ring-0 transition-transform duration-150 data-[state=unchecked]:translate-x-0",
            shared::switch_thumb_size_classes(self.size),
            bg_class,
        ])
    }

    #[must_use]
    pub fn build_attributes(&self) -> BTreeMap<String, String> {
        let mut attrs = self.attributes.clone();
        if !self.id.is_empty() {
            attrs.insert("id".to_string(), self.id.clone());
        }
        attrs.insert(
            "aria-checked".to_string(),
            if self.checked { "true" } else { "false" }.to_string(),
        );
        attrs.insert("data-state".to_string(), self.get_state().to_string());
        if self.disabled {
            attrs.insert("disabled".to_string(), "disabled".to_string());
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
        attrs
    }

    #[must_use]
    pub const fn get_state(&self) -> &'static str {
        if self.checked { "checked" } else { "unchecked" }
    }
}

#[must_use]
pub fn Switch(props: &SwitchProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        button
            type="button"
            role="switch"
            class=(props.build_classes())
            id=[shared::attr_value(&attrs, "id")]
            aria-checked=[shared::attr_value(&attrs, "aria-checked")]
            data-state=[shared::attr_value(&attrs, "data-state")]
            disabled[props.disabled]
            aria-label=[shared::attr_value(&attrs, "aria-label")]
            aria-describedby=[shared::attr_value(&attrs, "aria-describedby")]
            name=[shared::optional_attr(&props.name)]
            value=[if props.name.is_empty() { None } else { Some(props.value.as_str()) }]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
        {
            span class=(props.build_thumb_classes()) data-state=(props.get_state()) {}
        }
    }
}

#[must_use]
pub fn SwitchWithLabel(props: &SwitchProps, label: impl Into<String>) -> Markup {
    let label = label.into();
    let switch_props = props.clone().with_aria_label(&label);
    html! {
        div class="flex items-center space-x-2" {
            (Switch(&switch_props))
            label
                for=[shared::optional_attr(&props.id)]
                class="text-sm font-medium leading-none peer-disabled:cursor-not-allowed peer-disabled:opacity-70"
            {
                (&label)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_helpers_and_attributes_match_go_behavior() {
        let props = SwitchProps::default();
        assert_eq!(props.size, shared::SIZE_MD);
        assert_eq!(props.variant, shared::VARIANT_PRIMARY);
        assert_eq!(props.get_state(), "unchecked");
        assert!(
            props
                .build_classes()
                .contains("data-[state=checked]:bg-primary data-[state=unchecked]:bg-input")
        );
        assert!(props.build_thumb_classes().contains("bg-background"));

        let props = SwitchProps::default()
            .with_checked(true)
            .with_disabled(true)
            .with_name("notify")
            .with_id("notify")
            .with_value("yes")
            .with_size(shared::SIZE_LG)
            .with_variant(shared::VARIANT_SECONDARY)
            .with_class("custom")
            .with_aria_label("Notify")
            .with_aria_described_by("help")
            .with_attribute("data-testid", "switch");
        assert_eq!(props.get_state(), "checked");
        assert!(
            !props
                .build_classes()
                .contains("data-[state=checked]:bg-primary")
        );
        assert!(!props.build_thumb_classes().contains("bg-background"));
        let attrs = props.build_attributes();
        assert_eq!(attrs.get("id").map(String::as_str), Some("notify"));
        assert_eq!(attrs.get("aria-checked").map(String::as_str), Some("true"));
        assert_eq!(attrs.get("data-state").map(String::as_str), Some("checked"));
        assert_eq!(attrs.get("disabled").map(String::as_str), Some("disabled"));
        assert_eq!(attrs.get("aria-label").map(String::as_str), Some("Notify"));
        assert_eq!(
            attrs.get("aria-describedby").map(String::as_str),
            Some("help")
        );
    }

    #[test]
    fn switch_renders_button_thumb_state_and_label_variant() {
        let props = SwitchProps::default()
            .with_checked(true)
            .with_disabled(true)
            .with_name("notify")
            .with_id("notify")
            .with_value("yes")
            .with_attribute("data-testid", "switch");
        let html = Switch(&props).into_string();
        assert!(html.starts_with("<button type=\"button\" role=\"switch\""));
        assert!(html.contains("id=\"notify\""));
        assert!(html.contains("aria-checked=\"true\""));
        assert!(html.contains("data-state=\"checked\""));
        assert!(html.contains("disabled"));
        assert!(html.contains("name=\"notify\""));
        assert!(html.contains("value=\"yes\""));
        assert!(html.contains("data-testid=\"switch\""));
        assert!(html.contains("<span class=\"pointer-events-none block rounded-full"));
        assert!(html.contains("data-state=\"checked\"></span>"));

        let labeled = SwitchWithLabel(&props, "<Notify>").into_string();
        assert!(labeled.starts_with("<div class=\"flex items-center space-x-2\">"));
        assert!(labeled.contains("aria-label=\"&lt;Notify&gt;\""));
        assert!(labeled.contains("<label for=\"notify\""));
        assert!(labeled.contains("&lt;Notify&gt;</label>"));
    }
}
