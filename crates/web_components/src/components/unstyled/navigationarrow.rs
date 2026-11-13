#![allow(non_snake_case)]

use crate::components::{ComponentInfo, shared};
use maud::{Markup, html};
use std::collections::BTreeMap;

pub const COMPONENTS: &[ComponentInfo] = &[
    ComponentInfo {
        family: "unstyled",
        module: "navigationarrow",
        name: "NavigationArrow",
    },
    ComponentInfo {
        family: "unstyled",
        module: "navigationarrow",
        name: "LeftArrow",
    },
    ComponentInfo {
        family: "unstyled",
        module: "navigationarrow",
        name: "RightArrow",
    },
    ComponentInfo {
        family: "unstyled",
        module: "navigationarrow",
        name: "UpArrow",
    },
    ComponentInfo {
        family: "unstyled",
        module: "navigationarrow",
        name: "DownArrow",
    },
];

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NavigationArrowProps {
    pub direction: &'static str,
    pub size: &'static str,
    pub on_click: String,
    pub disabled: bool,
    pub class: String,
    pub attributes: BTreeMap<String, String>,
}

impl Default for NavigationArrowProps {
    fn default() -> Self {
        Self {
            direction: shared::POSITION_RIGHT,
            size: shared::SIZE_MD,
            on_click: String::new(),
            disabled: false,
            class: String::new(),
            attributes: BTreeMap::new(),
        }
    }
}

impl NavigationArrowProps {
    #[must_use]
    pub const fn with_direction(mut self, direction: &'static str) -> Self {
        self.direction = direction;
        self
    }

    #[must_use]
    pub const fn with_size(mut self, size: &'static str) -> Self {
        self.size = size;
        self
    }

    #[must_use]
    pub fn with_on_click(mut self, on_click: impl Into<String>) -> Self {
        self.on_click = on_click.into();
        self
    }

    #[must_use]
    pub const fn with_disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
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
        let size_class = match self.size {
            shared::SIZE_SM => "p-2",
            shared::SIZE_MD => "p-3",
            shared::SIZE_LG => "p-4",
            _ => "",
        };

        shared::build_classes(&[
            "rounded-full flex items-center justify-center transition-transform cursor-pointer disabled:opacity-50 disabled:cursor-not-allowed",
            size_class,
            self.class.as_str(),
        ])
    }

    #[must_use]
    pub fn get_aria_label(&self) -> &'static str {
        match self.direction {
            shared::POSITION_LEFT => shared::default_strings().nav_arrow_previous,
            shared::POSITION_RIGHT => shared::default_strings().nav_arrow_next,
            shared::POSITION_TOP | "up" => shared::default_strings().nav_arrow_up,
            shared::POSITION_BOTTOM | "down" => shared::default_strings().nav_arrow_down,
            _ => "",
        }
    }

    #[must_use]
    pub fn get_svg_path(&self) -> &'static str {
        match self.direction {
            shared::POSITION_LEFT => "M15 19l-7-7 7-7",
            shared::POSITION_RIGHT => "M9 5l7 7-7 7",
            shared::POSITION_TOP | "up" => "M5 15l7-7 7 7",
            shared::POSITION_BOTTOM | "down" => "M19 9l-7 7-7-7",
            _ => "",
        }
    }

    #[must_use]
    pub fn build_attributes(&self) -> BTreeMap<String, String> {
        let mut attrs = self.attributes.clone();
        if !self.on_click.is_empty() {
            attrs.insert("data-on:click".to_string(), self.on_click.clone());
        }
        attrs
    }
}

#[must_use]
pub fn NavigationArrow(props: &NavigationArrowProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        button
            type="button"
            class=(props.build_classes())
            aria-label=(props.get_aria_label())
            disabled[props.disabled]
            data-on:click=[shared::attr_value(&attrs, "data-on:click")]
            id=[shared::attr_value(&attrs, "id")]
            title=[shared::attr_value(&attrs, "title")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
        {
            svg class="w-6 h-6" fill="none" stroke="currentColor" viewBox="0 0 24 24" aria-hidden="true" {
                path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d=(props.get_svg_path()) {}
            }
        }
    }
}

#[must_use]
pub fn LeftArrow(on_click: impl Into<String>) -> Markup {
    NavigationArrow(
        &NavigationArrowProps::default()
            .with_direction(shared::POSITION_LEFT)
            .with_on_click(on_click),
    )
}

#[must_use]
pub fn RightArrow(on_click: impl Into<String>) -> Markup {
    NavigationArrow(
        &NavigationArrowProps::default()
            .with_direction(shared::POSITION_RIGHT)
            .with_on_click(on_click),
    )
}

#[must_use]
pub fn UpArrow(on_click: impl Into<String>) -> Markup {
    NavigationArrow(
        &NavigationArrowProps::default()
            .with_direction("up")
            .with_on_click(on_click),
    )
}

#[must_use]
pub fn DownArrow(on_click: impl Into<String>) -> Markup {
    NavigationArrow(
        &NavigationArrowProps::default()
            .with_direction("down")
            .with_on_click(on_click),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_go_defaults() {
        let props = NavigationArrowProps::default();
        assert_eq!(props.direction, "right");
        assert_eq!(props.size, shared::SIZE_MD);
        assert!(!props.disabled);
    }

    #[test]
    fn helpers_cover_sizes_labels_paths_attrs_and_datastar_click() {
        for (size, expected) in [
            (shared::SIZE_SM, "p-2"),
            (shared::SIZE_MD, "p-3"),
            (shared::SIZE_LG, "p-4"),
            (shared::SIZE_XL, ""),
        ] {
            assert!(
                NavigationArrowProps::default()
                    .with_size(size)
                    .build_classes()
                    .contains(expected)
            );
        }

        for (direction, label, path) in [
            ("left", "Previous", "M15 19l-7-7 7-7"),
            ("right", "Next", "M9 5l7 7-7 7"),
            ("up", "Up", "M5 15l7-7 7 7"),
            ("down", "Down", "M19 9l-7 7-7-7"),
            ("bad", "", ""),
        ] {
            let props = NavigationArrowProps::default().with_direction(direction);
            assert_eq!(props.get_aria_label(), label);
            assert_eq!(props.get_svg_path(), path);
        }

        let attrs = NavigationArrowProps::default()
            .with_on_click("$go = true")
            .with_attribute("title", "Next")
            .build_attributes();
        assert_eq!(
            attrs.get("data-on:click").map(String::as_str),
            Some("$go = true")
        );
        assert_eq!(attrs.get("title").map(String::as_str), Some("Next"));
    }

    #[test]
    fn navigation_arrow_renders_button_svg_disabled_and_colon_attribute() {
        let html = NavigationArrow(
            &NavigationArrowProps::default()
                .with_direction("left")
                .with_disabled(true)
                .with_class("arrow-extra")
                .with_on_click("$slide = 1")
                .with_attribute("data-testid", "prev"),
        )
        .into_string();

        assert!(html.starts_with("<button type=\"button\""));
        assert!(html.contains("aria-label=\"Previous\""));
        assert!(html.contains("disabled"));
        assert!(html.contains("arrow-extra"));
        assert!(html.contains("data-on:click=\"$slide = 1\""));
        assert!(html.contains("data-testid=\"prev\""));
        assert!(html.contains("d=\"M15 19l-7-7 7-7\""));

        assert!(
            LeftArrow("$x = 1")
                .into_string()
                .contains("aria-label=\"Previous\"")
        );
        assert!(
            RightArrow("$x = 1")
                .into_string()
                .contains("aria-label=\"Next\"")
        );
        assert!(
            UpArrow("$x = 1")
                .into_string()
                .contains("aria-label=\"Up\"")
        );
        assert!(
            DownArrow("$x = 1")
                .into_string()
                .contains("aria-label=\"Down\"")
        );
    }
}
