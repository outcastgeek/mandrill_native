#![allow(non_snake_case)]

use crate::components::{ComponentInfo, shared};
use maud::{Markup, html};
use std::collections::BTreeMap;

pub const SCROLL_ORIENTATION_VERTICAL: &str = "vertical";
pub const SCROLL_ORIENTATION_HORIZONTAL: &str = "horizontal";
pub const SCROLL_ORIENTATION_BOTH: &str = "both";

pub const COMPONENTS: &[ComponentInfo] = &[ComponentInfo {
    family: "unstyled",
    module: "scrollarea",
    name: "ScrollArea",
}];

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScrollAreaProps {
    pub orientation: &'static str,
    pub class: String,
    pub attributes: BTreeMap<String, String>,
}

impl Default for ScrollAreaProps {
    fn default() -> Self {
        Self {
            orientation: SCROLL_ORIENTATION_VERTICAL,
            class: String::new(),
            attributes: BTreeMap::new(),
        }
    }
}

impl ScrollAreaProps {
    #[must_use]
    pub const fn with_orientation(mut self, value: &'static str) -> Self {
        self.orientation = value;
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
        shared::build_classes(&["relative overflow-hidden", self.class.as_str()])
    }

    #[must_use]
    pub fn build_scroll_classes(&self) -> String {
        let scroll_class = match self.orientation {
            SCROLL_ORIENTATION_HORIZONTAL => "overflow-x-auto overflow-y-hidden",
            SCROLL_ORIENTATION_BOTH => "overflow-auto",
            _ => "overflow-y-auto overflow-x-hidden",
        };
        shared::build_classes(&["h-full w-full rounded-[inherit]", scroll_class])
    }

    #[must_use]
    pub fn build_attributes(&self) -> BTreeMap<String, String> {
        self.attributes.clone()
    }
}

#[must_use]
pub fn ScrollArea(props: &ScrollAreaProps, children: Markup) -> Markup {
    let attrs = props.build_attributes();
    html! {
        div
            class=(props.build_classes())
            id=[shared::attr_value(&attrs, "id")]
            role=[shared::attr_value(&attrs, "role")]
            aria-label=[shared::attr_value(&attrs, "aria-label")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
        {
            div class=(props.build_scroll_classes()) {
                (children)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn helpers_cover_classes_orientations_and_attributes() {
        let props = ScrollAreaProps::default();
        assert_eq!(props.orientation, SCROLL_ORIENTATION_VERTICAL);
        assert_eq!(props.build_classes(), "relative overflow-hidden");
        assert_eq!(
            props.build_scroll_classes(),
            "h-full w-full rounded-[inherit] overflow-y-auto overflow-x-hidden"
        );
        assert_eq!(
            ScrollAreaProps::default()
                .with_orientation(SCROLL_ORIENTATION_HORIZONTAL)
                .with_class("max-h-40")
                .build_classes(),
            "relative overflow-hidden max-h-40"
        );
        assert_eq!(
            ScrollAreaProps::default()
                .with_orientation(SCROLL_ORIENTATION_HORIZONTAL)
                .build_scroll_classes(),
            "h-full w-full rounded-[inherit] overflow-x-auto overflow-y-hidden"
        );
        assert_eq!(
            ScrollAreaProps::default()
                .with_orientation(SCROLL_ORIENTATION_BOTH)
                .build_scroll_classes(),
            "h-full w-full rounded-[inherit] overflow-auto"
        );
        let attrs = ScrollAreaProps::default()
            .with_attribute("id", "scroll")
            .with_attribute("data-testid", "scroll-area")
            .build_attributes();
        assert_eq!(attrs.get("id").map(String::as_str), Some("scroll"));
        assert_eq!(
            attrs.get("data-testid").map(String::as_str),
            Some("scroll-area")
        );
    }

    #[test]
    fn scroll_area_renders_outer_inner_attributes_and_escaped_children() {
        let html = ScrollArea(
            &ScrollAreaProps::default()
                .with_orientation(SCROLL_ORIENTATION_BOTH)
                .with_class("h-64")
                .with_attribute("id", "activity")
                .with_attribute("role", "region")
                .with_attribute("aria-label", "Activity")
                .with_attribute("data-testid", "activity-scroll"),
            html! { p { "<Activity>" } },
        )
        .into_string();
        assert!(html.starts_with("<div class=\"relative overflow-hidden h-64\""));
        assert!(html.contains("id=\"activity\""));
        assert!(html.contains("role=\"region\""));
        assert!(html.contains("aria-label=\"Activity\""));
        assert!(html.contains("data-testid=\"activity-scroll\""));
        assert!(html.contains("<div class=\"h-full w-full rounded-[inherit] overflow-auto\">"));
        assert!(html.contains("<p>&lt;Activity&gt;</p>"));
        assert!(html.ends_with("</div></div>"));
    }
}
