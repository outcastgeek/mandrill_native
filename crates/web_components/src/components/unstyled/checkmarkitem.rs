#![allow(non_snake_case)]

use crate::components::{ComponentInfo, shared};
use maud::{Markup, html};
use std::collections::BTreeMap;

pub const COMPONENTS: &[ComponentInfo] = &[
    ComponentInfo {
        family: "unstyled",
        module: "checkmarkitem",
        name: "CheckmarkItem",
    },
    ComponentInfo {
        family: "unstyled",
        module: "checkmarkitem",
        name: "SimpleCheckmarkItem",
    },
];

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CheckmarkItemProps {
    pub text: String,
    pub description: String,
    pub icon_color: String,
    pub icon_size: &'static str,
    pub icon_style: &'static str,
    pub bold_label: bool,
    pub class: String,
    pub attributes: BTreeMap<String, String>,
}

impl Default for CheckmarkItemProps {
    fn default() -> Self {
        Self {
            text: String::new(),
            description: String::new(),
            icon_color: "green-600".to_string(),
            icon_size: shared::SIZE_SM,
            icon_style: "circle",
            bold_label: false,
            class: String::new(),
            attributes: BTreeMap::new(),
        }
    }
}

impl CheckmarkItemProps {
    #[must_use]
    pub fn with_text(mut self, text: impl Into<String>) -> Self {
        self.text = text.into();
        self
    }

    #[must_use]
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = description.into();
        self
    }

    #[must_use]
    pub fn with_icon_color(mut self, color: impl Into<String>) -> Self {
        self.icon_color = color.into();
        self
    }

    #[must_use]
    pub const fn with_icon_style(mut self, style: &'static str) -> Self {
        self.icon_style = style;
        self
    }

    #[must_use]
    pub const fn with_icon_size(mut self, size: &'static str) -> Self {
        self.icon_size = size;
        self
    }

    #[must_use]
    pub const fn with_bold_label(mut self, bold: bool) -> Self {
        self.bold_label = bold;
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
    pub fn build_icon_classes(&self) -> String {
        let shape_class = match self.icon_style {
            "circle" => "rounded-full",
            "square" => "rounded",
            _ => "",
        };
        let color_class = format!("bg-{}", self.icon_color);
        shared::build_classes(&[
            "flex items-center justify-center flex-shrink-0 mt-1",
            "w-6 h-6",
            shape_class,
            color_class.as_str(),
        ])
    }

    #[must_use]
    pub fn build_attributes(&self) -> BTreeMap<String, String> {
        self.attributes.clone()
    }
}

#[must_use]
pub fn CheckmarkItem(props: &CheckmarkItemProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        li
            class=(format!("flex items-start gap-3 {}", props.class))
            id=[shared::attr_value(&attrs, "id")]
            title=[shared::attr_value(&attrs, "title")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
        {
            @if props.icon_style != "none" {
                div class=(props.build_icon_classes()) {
                    svg class="w-4 h-4 text-success-foreground" fill="none" stroke="currentColor" viewBox="0 0 24 24" aria-hidden="true" {
                        path stroke-linecap="round" stroke-linejoin="round" stroke-width="3" d="M5 13l4 4L19 7" {}
                    }
                }
            }
            div class="flex-1" {
                @if props.bold_label {
                    span {
                        strong { (&props.text) }
                        @if !props.description.is_empty() {
                            " " (&props.description)
                        }
                    }
                } @else {
                    span { (&props.text) }
                    @if !props.description.is_empty() {
                        p class="text-sm mt-1" { (&props.description) }
                    }
                }
            }
        }
    }
}

#[must_use]
pub fn SimpleCheckmarkItem(text: impl Into<String>) -> Markup {
    CheckmarkItem(&CheckmarkItemProps::default().with_text(text))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_go_defaults() {
        let props = CheckmarkItemProps::default();
        assert_eq!(props.icon_color, "green-600");
        assert_eq!(props.icon_size, shared::SIZE_SM);
        assert_eq!(props.icon_style, "circle");
        assert!(!props.bold_label);
    }

    #[test]
    fn helpers_cover_icon_styles_colors_attrs_and_builders() {
        assert!(
            CheckmarkItemProps::default()
                .with_icon_style("circle")
                .build_icon_classes()
                .contains("rounded-full")
        );
        assert!(
            CheckmarkItemProps::default()
                .with_icon_style("square")
                .build_icon_classes()
                .contains("rounded")
        );
        assert!(
            !CheckmarkItemProps::default()
                .with_icon_style("none")
                .build_icon_classes()
                .contains("rounded")
        );
        assert!(
            CheckmarkItemProps::default()
                .with_icon_color("success")
                .build_icon_classes()
                .contains("bg-success")
        );

        let props = CheckmarkItemProps::default()
            .with_text("Text")
            .with_description("Description")
            .with_icon_size(shared::SIZE_LG)
            .with_bold_label(true)
            .with_class("extra")
            .with_attribute("title", "Item");
        assert_eq!(props.text, "Text");
        assert_eq!(props.description, "Description");
        assert_eq!(props.icon_size, shared::SIZE_LG);
        assert!(props.bold_label);
        assert_eq!(
            props.build_attributes().get("title").map(String::as_str),
            Some("Item")
        );
    }

    #[test]
    fn checkmark_item_renders_regular_bold_no_icon_and_escaping_variations() {
        let html = CheckmarkItem(
            &CheckmarkItemProps::default()
                .with_text("<Label>")
                .with_description("<Desc>")
                .with_class("custom")
                .with_attribute("data-testid", "item"),
        )
        .into_string();
        assert!(html.starts_with("<li class=\"flex items-start gap-3 custom\""));
        assert!(html.contains("data-testid=\"item\""));
        assert!(html.contains("bg-green-600"));
        assert!(html.contains("<path stroke-linecap=\"round\""));
        assert!(html.contains("<span>&lt;Label&gt;</span>"));
        assert!(html.contains("<p class=\"text-sm mt-1\">&lt;Desc&gt;</p>"));

        let bold = CheckmarkItem(
            &CheckmarkItemProps::default()
                .with_text("Label")
                .with_description("Desc")
                .with_bold_label(true),
        )
        .into_string();
        assert!(bold.contains("<strong>Label</strong> Desc"));
        assert!(!bold.contains("<p class=\"text-sm mt-1\">"));

        let no_icon = CheckmarkItem(
            &CheckmarkItemProps::default()
                .with_icon_style("none")
                .with_text("No icon"),
        )
        .into_string();
        assert!(!no_icon.contains("text-success-foreground"));
        assert!(no_icon.contains("No icon"));

        assert!(
            SimpleCheckmarkItem("Simple")
                .into_string()
                .contains("Simple")
        );
    }
}
