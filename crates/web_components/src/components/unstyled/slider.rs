#![allow(non_snake_case)]

use crate::components::{ComponentInfo, shared};
use maud::{Markup, html};
use std::collections::BTreeMap;

pub const COMPONENTS: &[ComponentInfo] = &[
    ComponentInfo {
        family: "unstyled",
        module: "slider",
        name: "Slider",
    },
    ComponentInfo {
        family: "unstyled",
        module: "slider",
        name: "SliderWithLabel",
    },
];

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SliderProps {
    pub id: String,
    pub name: String,
    pub value: i32,
    pub min: i32,
    pub max: i32,
    pub step: i32,
    pub disabled: bool,
    pub show_value: bool,
    pub class: String,
    pub aria_label: String,
    pub attributes: BTreeMap<String, String>,
}

impl Default for SliderProps {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            value: 50,
            min: 0,
            max: 100,
            step: 1,
            disabled: false,
            show_value: false,
            class: String::new(),
            aria_label: String::new(),
            attributes: BTreeMap::new(),
        }
    }
}

impl SliderProps {
    #[must_use]
    pub fn with_id(mut self, value: impl Into<String>) -> Self {
        self.id = value.into();
        self
    }
    #[must_use]
    pub fn with_name(mut self, value: impl Into<String>) -> Self {
        self.name = value.into();
        self
    }
    #[must_use]
    pub const fn with_value(mut self, value: i32) -> Self {
        self.value = value;
        self
    }
    #[must_use]
    pub const fn with_min(mut self, value: i32) -> Self {
        self.min = value;
        self
    }
    #[must_use]
    pub const fn with_max(mut self, value: i32) -> Self {
        self.max = value;
        self
    }
    #[must_use]
    pub const fn with_step(mut self, value: i32) -> Self {
        self.step = value;
        self
    }
    #[must_use]
    pub const fn with_disabled(mut self, value: bool) -> Self {
        self.disabled = value;
        self
    }
    #[must_use]
    pub const fn with_show_value(mut self, value: bool) -> Self {
        self.show_value = value;
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
    pub fn build_container_classes(&self) -> String {
        shared::build_classes(&[
            "relative flex w-full touch-none select-none items-center",
            self.class.as_str(),
        ])
    }

    #[must_use]
    pub const fn build_input_classes(&self) -> &'static str {
        "w-full h-2 bg-secondary rounded-lg appearance-none cursor-pointer accent-primary disabled:opacity-50 disabled:cursor-not-allowed"
    }

    #[must_use]
    pub fn build_attributes(&self) -> BTreeMap<String, String> {
        let mut attrs = self.attributes.clone();
        attrs.insert("min".to_string(), self.min.to_string());
        attrs.insert("max".to_string(), self.max.to_string());
        attrs.insert("step".to_string(), self.step.to_string());
        attrs.insert("value".to_string(), self.value.to_string());
        if !self.id.is_empty() {
            attrs.insert("id".to_string(), self.id.clone());
        }
        if !self.name.is_empty() {
            attrs.insert("name".to_string(), self.name.clone());
        }
        if self.disabled {
            attrs.insert("disabled".to_string(), "true".to_string());
        }
        if !self.aria_label.is_empty() {
            attrs.insert("aria-label".to_string(), self.aria_label.clone());
        }
        attrs
    }

    #[must_use]
    pub fn value_string(&self) -> String {
        self.value.to_string()
    }
}

#[must_use]
pub fn Slider(props: &SliderProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        div class=(props.build_container_classes()) {
            input
                type="range"
                class=(props.build_input_classes())
                min=[shared::attr_value(&attrs, "min")]
                max=[shared::attr_value(&attrs, "max")]
                step=[shared::attr_value(&attrs, "step")]
                value=[shared::attr_value(&attrs, "value")]
                id=[shared::attr_value(&attrs, "id")]
                name=[shared::attr_value(&attrs, "name")]
                disabled[props.disabled]
                aria-label=[shared::attr_value(&attrs, "aria-label")]
                data-testid=[shared::attr_value(&attrs, "data-testid")]
                data-on:input=[shared::attr_value(&attrs, "data-on:input")]
            {}
            @if props.show_value {
                span class="ml-4 text-sm font-medium tabular-nums" { (props.value_string()) }
            }
        }
    }
}

#[must_use]
pub fn SliderWithLabel(props: &SliderProps, label: impl Into<String>) -> Markup {
    let label = label.into();
    html! {
        div class="space-y-2" {
            @if !label.is_empty() {
                label
                    for=[shared::optional_attr(&props.id)]
                    class="text-sm font-medium leading-none"
                { (&label) }
            }
            (Slider(props))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_and_helpers_match_go_behavior() {
        let props = SliderProps::default();
        assert_eq!(props.value, 50);
        assert_eq!(props.min, 0);
        assert_eq!(props.max, 100);
        assert_eq!(props.step, 1);
        assert_eq!(
            props.build_container_classes(),
            "relative flex w-full touch-none select-none items-center"
        );
        assert_eq!(props.value_string(), "50");

        let props = SliderProps::default()
            .with_id("volume")
            .with_name("volume")
            .with_value(75)
            .with_min(10)
            .with_max(90)
            .with_step(5)
            .with_disabled(true)
            .with_show_value(true)
            .with_class("extra")
            .with_aria_label("Volume")
            .with_attribute("data-on:input", "$volume = evt.target.value");
        assert_eq!(
            props.build_container_classes(),
            "relative flex w-full touch-none select-none items-center extra"
        );
        let attrs = props.build_attributes();
        assert_eq!(attrs.get("min").map(String::as_str), Some("10"));
        assert_eq!(attrs.get("max").map(String::as_str), Some("90"));
        assert_eq!(attrs.get("step").map(String::as_str), Some("5"));
        assert_eq!(attrs.get("value").map(String::as_str), Some("75"));
        assert_eq!(attrs.get("id").map(String::as_str), Some("volume"));
        assert_eq!(attrs.get("name").map(String::as_str), Some("volume"));
        assert_eq!(attrs.get("disabled").map(String::as_str), Some("true"));
        assert_eq!(attrs.get("aria-label").map(String::as_str), Some("Volume"));
    }

    #[test]
    fn slider_renders_input_value_datastar_and_label_variations() {
        let props = SliderProps::default()
            .with_id("volume")
            .with_name("volume")
            .with_value(75)
            .with_show_value(true)
            .with_disabled(true)
            .with_attribute("data-testid", "slider")
            .with_attribute("data-on:input", "$volume = evt.target.value");
        let html = Slider(&props).into_string();
        assert!(html.starts_with(
            "<div class=\"relative flex w-full touch-none select-none items-center\">"
        ));
        assert!(html.contains("<input type=\"range\""));
        assert!(html.contains("id=\"volume\""));
        assert!(html.contains("name=\"volume\""));
        assert!(html.contains("value=\"75\""));
        assert!(html.contains("disabled"));
        assert!(html.contains("data-testid=\"slider\""));
        assert!(html.contains("data-on:input=\"$volume = evt.target.value\""));
        assert!(html.contains("<span class=\"ml-4 text-sm font-medium tabular-nums\">75</span>"));

        let labeled = SliderWithLabel(&props, "<Volume>").into_string();
        assert!(labeled.starts_with("<div class=\"space-y-2\">"));
        assert!(labeled.contains("<label for=\"volume\""));
        assert!(labeled.contains("&lt;Volume&gt;"));
    }
}
