#![allow(non_snake_case)]

use crate::components::{ComponentInfo, shared};
use maud::{Markup, html};
use std::collections::BTreeMap;

pub const COMPONENTS: &[ComponentInfo] = &[ComponentInfo {
    family: "unstyled",
    module: "divider",
    name: "Divider",
}];

pub const DIVIDER_HORIZONTAL: &str = "horizontal";
pub const DIVIDER_VERTICAL: &str = "vertical";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DividerProps {
    pub orientation: &'static str,
    pub class: String,
    pub attributes: BTreeMap<String, String>,
}

impl Default for DividerProps {
    fn default() -> Self {
        Self {
            orientation: DIVIDER_HORIZONTAL,
            class: String::new(),
            attributes: BTreeMap::new(),
        }
    }
}

impl DividerProps {
    #[must_use]
    pub fn with_orientation(mut self, orientation: &'static str) -> Self {
        self.orientation = orientation;
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
        let orientation_classes = if self.orientation == DIVIDER_VERTICAL {
            "h-full w-[1px]"
        } else {
            "h-[1px] w-full"
        };
        shared::build_classes(&[
            "shrink-0 bg-border",
            orientation_classes,
            self.class.as_str(),
        ])
    }

    #[must_use]
    pub fn build_attributes(&self) -> BTreeMap<String, String> {
        let mut attrs = self.attributes.clone();
        attrs.insert("role".to_string(), "separator".to_string());
        attrs.insert("aria-orientation".to_string(), self.orientation.to_string());
        attrs
    }
}

#[must_use]
pub fn Divider(props: &DividerProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        div
            class=(props.build_classes())
            role="separator"
            aria-orientation=(props.orientation)
            id=[shared::attr_value(&attrs, "id")]
            title=[shared::attr_value(&attrs, "title")]
            data-on:click=[shared::attr_value(&attrs, "data-on:click")]
            data-on:load=[shared::attr_value(&attrs, "data-on:load")]
            data-show=[shared::attr_value(&attrs, "data-show")]
            data-bind=[shared::attr_value(&attrs, "data-bind")]
        {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_props_match_templ_defaults() {
        let props = DividerProps::default();
        assert_eq!(props.orientation, DIVIDER_HORIZONTAL);
        assert!(props.class.is_empty());
        assert!(props.attributes.is_empty());
    }

    #[test]
    fn build_classes_covers_horizontal_vertical_and_custom_class() {
        let horizontal = DividerProps::default().with_class("custom").build_classes();
        let vertical = DividerProps::default()
            .with_orientation(DIVIDER_VERTICAL)
            .with_class("custom")
            .build_classes();

        assert_eq!(horizontal, "shrink-0 bg-border h-[1px] w-full custom");
        assert_eq!(vertical, "shrink-0 bg-border h-full w-[1px] custom");
    }

    #[test]
    fn build_attributes_covers_semantic_and_custom_attributes() {
        let attrs = DividerProps::default()
            .with_orientation(DIVIDER_VERTICAL)
            .with_attribute("data-on:click", "$clicked = true")
            .build_attributes();

        assert_eq!(attrs.get("role"), Some(&"separator".to_string()));
        assert_eq!(attrs.get("aria-orientation"), Some(&"vertical".to_string()));
        assert_eq!(
            attrs.get("data-on:click"),
            Some(&"$clicked = true".to_string())
        );
    }

    #[test]
    fn renders_html_for_all_orientation_variations() {
        for &(orientation, expected_size) in &[
            (DIVIDER_HORIZONTAL, "h-[1px] w-full"),
            (DIVIDER_VERTICAL, "h-full w-[1px]"),
        ] {
            let html = Divider(
                &DividerProps::default()
                    .with_orientation(orientation)
                    .with_attribute("title", r#""Line" & more"#),
            )
            .into_string();

            assert!(html.starts_with("<div "));
            assert!(html.contains(expected_size));
            assert!(html.contains(r#"role="separator""#));
            assert!(html.contains(&format!(r#"aria-orientation="{orientation}""#)));
            assert!(html.contains(r#"title="&quot;Line&quot; &amp; more""#));
            assert!(html.ends_with("></div>"));
        }
    }
}
