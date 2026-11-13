#![allow(non_snake_case)]

use crate::components::unstyled::checkmarkitem::{CheckmarkItem, CheckmarkItemProps};
use crate::components::{ComponentInfo, shared};
use maud::{Markup, html};
use std::collections::BTreeMap;

pub const COMPONENTS: &[ComponentInfo] = &[
    ComponentInfo {
        family: "unstyled",
        module: "checkmarklist",
        name: "CheckmarkList",
    },
    ComponentInfo {
        family: "unstyled",
        module: "checkmarklist",
        name: "SimpleCheckmarkList",
    },
];

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CheckmarkListProps {
    pub items: Vec<CheckmarkItemProps>,
    pub spacing: &'static str,
    pub icon_style: &'static str,
    pub icon_color: String,
    pub class: String,
    pub attributes: BTreeMap<String, String>,
}

impl Default for CheckmarkListProps {
    fn default() -> Self {
        Self {
            items: Vec::new(),
            spacing: "normal",
            icon_style: "circle",
            icon_color: "green-600".to_string(),
            class: String::new(),
            attributes: BTreeMap::new(),
        }
    }
}

impl CheckmarkListProps {
    #[must_use]
    pub fn with_items(mut self, items: Vec<CheckmarkItemProps>) -> Self {
        self.items = items;
        self
    }

    #[must_use]
    pub const fn with_spacing(mut self, spacing: &'static str) -> Self {
        self.spacing = spacing;
        self
    }

    #[must_use]
    pub const fn with_icon_style(mut self, icon_style: &'static str) -> Self {
        self.icon_style = icon_style;
        self
    }

    #[must_use]
    pub fn with_icon_color(mut self, icon_color: impl Into<String>) -> Self {
        self.icon_color = icon_color.into();
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
        let spacing_class = match self.spacing {
            "tight" => "space-y-2",
            "normal" => "space-y-4",
            "relaxed" => "space-y-6",
            _ => "",
        };
        shared::build_classes(&[spacing_class, self.class.as_str()])
    }

    #[must_use]
    pub fn build_attributes(&self) -> BTreeMap<String, String> {
        self.attributes.clone()
    }
}

#[must_use]
pub fn CheckmarkList(props: &CheckmarkListProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        ul
            class=(props.build_classes())
            role="list"
            id=[shared::attr_value(&attrs, "id")]
            title=[shared::attr_value(&attrs, "title")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
        {
            @for item in &props.items {
                @if item.icon_color.is_empty() {
                    (CheckmarkItem(&item.clone().with_icon_color(&props.icon_color).with_icon_style(props.icon_style)))
                } @else {
                    (CheckmarkItem(item))
                }
            }
        }
    }
}

#[must_use]
pub fn SimpleCheckmarkList(items: Vec<CheckmarkItemProps>) -> Markup {
    CheckmarkList(&CheckmarkListProps::default().with_items(items))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_go_defaults() {
        let props = CheckmarkListProps::default();
        assert_eq!(props.spacing, "normal");
        assert_eq!(props.icon_style, "circle");
        assert_eq!(props.icon_color, "green-600");
    }

    #[test]
    fn helpers_cover_spacing_class_attrs_and_builders() {
        for (spacing, expected) in [
            ("tight", "space-y-2"),
            ("normal", "space-y-4"),
            ("relaxed", "space-y-6"),
            ("bad", ""),
        ] {
            assert!(
                CheckmarkListProps::default()
                    .with_spacing(spacing)
                    .build_classes()
                    .contains(expected)
            );
        }

        let props = CheckmarkListProps::default()
            .with_icon_style("square")
            .with_icon_color("success")
            .with_class("extra")
            .with_attribute("title", "List");
        assert_eq!(props.build_classes(), "space-y-4 extra");
        assert_eq!(
            props.build_attributes().get("title").map(String::as_str),
            Some("List")
        );
        assert_eq!(props.icon_style, "square");
        assert_eq!(props.icon_color, "success");
    }

    #[test]
    fn checkmark_list_renders_items_default_overrides_and_simple_variant() {
        let items = vec![
            CheckmarkItemProps::default().with_text("Uses own color"),
            CheckmarkItemProps::default()
                .with_text("Uses list color")
                .with_icon_color(""),
        ];
        let html = CheckmarkList(
            &CheckmarkListProps::default()
                .with_items(items)
                .with_spacing("tight")
                .with_icon_style("square")
                .with_icon_color("success")
                .with_attribute("data-testid", "list"),
        )
        .into_string();

        assert!(html.starts_with("<ul class=\"space-y-2\" role=\"list\""));
        assert!(html.contains("data-testid=\"list\""));
        assert_eq!(
            html.matches("<li class=\"flex items-start gap-3 ").count(),
            2
        );
        assert!(html.contains("bg-green-600"));
        assert!(html.contains("bg-success"));
        assert!(html.contains("rounded"));
        assert!(html.contains("Uses own color"));
        assert!(html.contains("Uses list color"));

        let simple = SimpleCheckmarkList(vec![CheckmarkItemProps::default().with_text("<Simple>")])
            .into_string();
        assert!(simple.contains("&lt;Simple&gt;"));
    }
}
