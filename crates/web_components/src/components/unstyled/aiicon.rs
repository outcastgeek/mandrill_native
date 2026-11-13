#![allow(non_snake_case)]

use crate::components::{ComponentInfo, shared};
use maud::{Markup, html};
use std::collections::BTreeMap;

pub const COMPONENTS: &[ComponentInfo] = &[
    ComponentInfo {
        family: "unstyled",
        module: "aiicon",
        name: "AiIcon",
    },
    ComponentInfo {
        family: "unstyled",
        module: "aiicon",
        name: "SimpleAiIcon",
    },
    ComponentInfo {
        family: "unstyled",
        module: "aiicon",
        name: "AnimatedAiIcon",
    },
];

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AiIconProps {
    pub size: &'static str,
    pub animated: bool,
    pub color: String,
    pub class: String,
    pub attrs: BTreeMap<String, String>,
}

impl Default for AiIconProps {
    fn default() -> Self {
        Self {
            size: shared::SIZE_MD,
            animated: false,
            color: "text-primary".to_string(),
            class: String::new(),
            attrs: BTreeMap::new(),
        }
    }
}

impl AiIconProps {
    #[must_use]
    pub const fn with_size(mut self, size: &'static str) -> Self {
        self.size = size;
        self
    }

    #[must_use]
    pub const fn with_animated(mut self, animated: bool) -> Self {
        self.animated = animated;
        self
    }

    #[must_use]
    pub fn with_color(mut self, color: impl Into<String>) -> Self {
        self.color = color.into();
        self
    }

    #[must_use]
    pub fn with_class(mut self, class: impl Into<String>) -> Self {
        self.class = class.into();
        self
    }

    #[must_use]
    pub fn with_attr(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.attrs.insert(name.into(), value.into());
        self
    }

    #[must_use]
    pub fn get_size_classes(&self) -> &'static str {
        match self.size {
            shared::SIZE_XS => "w-3 h-3",
            shared::SIZE_SM => "w-4 h-4",
            shared::SIZE_MD => "w-6 h-6",
            shared::SIZE_LG => "w-8 h-8",
            shared::SIZE_XL => "w-12 h-12",
            _ => "w-6 h-6",
        }
    }

    #[must_use]
    pub fn build_classes(&self) -> String {
        shared::build_classes(&[
            "ai-icon inline-flex items-center justify-center",
            self.color.as_str(),
            if self.animated { "animate-pulse" } else { "" },
            self.class.as_str(),
        ])
    }
}

#[must_use]
pub fn buildAiIconClasses(props: &AiIconProps) -> String {
    props.build_classes()
}

#[must_use]
pub fn AiIcon(props: &AiIconProps) -> Markup {
    html! {
        div
            class=(props.build_classes())
            aria-label=(shared::default_strings().ai_icon_aria_label)
            id=[shared::attr_value(&props.attrs, "id")]
            title=[shared::attr_value(&props.attrs, "title")]
            data-testid=[shared::attr_value(&props.attrs, "data-testid")]
        {
            svg
                xmlns="http://www.w3.org/2000/svg"
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                stroke-width="2"
                stroke-linecap="round"
                stroke-linejoin="round"
                class=(props.get_size_classes())
            {
                path d="M12 2a4 4 0 0 1 4 4v1a3 3 0 0 1 3 3v1a3 3 0 0 1-3 3h-1v4a3 3 0 0 1-6 0v-4H8a3 3 0 0 1-3-3v-1a3 3 0 0 1 3-3V6a4 4 0 0 1 4-4z" {}
                circle cx="9" cy="9" r="1" {}
                circle cx="15" cy="9" r="1" {}
            }
        }
    }
}

#[must_use]
pub fn SimpleAiIcon() -> Markup {
    AiIcon(&AiIconProps::default())
}

#[must_use]
pub fn AnimatedAiIcon() -> Markup {
    AiIcon(&AiIconProps::default().with_animated(true))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_go_defaults() {
        let props = AiIconProps::default();
        assert_eq!(props.size, shared::SIZE_MD);
        assert!(!props.animated);
        assert_eq!(props.color, "text-primary");
    }

    #[test]
    fn helper_methods_cover_sizes_animation_color_class_and_attrs() {
        for (size, expected) in [
            (shared::SIZE_XS, "w-3 h-3"),
            (shared::SIZE_SM, "w-4 h-4"),
            (shared::SIZE_MD, "w-6 h-6"),
            (shared::SIZE_LG, "w-8 h-8"),
            (shared::SIZE_XL, "w-12 h-12"),
            ("bad", "w-6 h-6"),
        ] {
            assert_eq!(
                AiIconProps::default().with_size(size).get_size_classes(),
                expected
            );
        }

        let props = AiIconProps::default()
            .with_animated(true)
            .with_color("text-info")
            .with_class("extra")
            .with_attr("title", "AI");
        assert_eq!(
            props.build_classes(),
            "ai-icon inline-flex items-center justify-center text-info animate-pulse extra"
        );
        assert_eq!(buildAiIconClasses(&props), props.build_classes());
        assert_eq!(props.attrs.get("title").map(String::as_str), Some("AI"));
    }

    #[test]
    fn ai_icon_renders_svg_label_and_optional_attrs() {
        let html = AiIcon(
            &AiIconProps::default()
                .with_size(shared::SIZE_LG)
                .with_animated(true)
                .with_attr("data-testid", "ai"),
        )
        .into_string();

        assert!(html.starts_with("<div "));
        assert!(html.contains("aria-label=\"AI\""));
        assert!(html.contains("animate-pulse"));
        assert!(html.contains("data-testid=\"ai\""));
        assert!(html.contains("<svg xmlns=\"http://www.w3.org/2000/svg\""));
        assert!(html.contains("class=\"w-8 h-8\""));
        assert!(html.contains("<circle cx=\"9\" cy=\"9\" r=\"1\"></circle>"));

        assert!(SimpleAiIcon().into_string().contains("text-primary"));
        assert!(AnimatedAiIcon().into_string().contains("animate-pulse"));
    }
}
