#![allow(non_snake_case)]

use crate::components::unstyled::iconbox::{IconBox, IconBoxProps};
use crate::components::{ComponentInfo, shared};
use maud::{Markup, html};
use std::collections::BTreeMap;

pub const COMPONENTS: &[ComponentInfo] = &[
    ComponentInfo {
        family: "unstyled",
        module: "producticoncard",
        name: "ProductIconCard",
    },
    ComponentInfo {
        family: "unstyled",
        module: "producticoncard",
        name: "SimpleProductIconCard",
    },
];

#[derive(Clone, Debug)]
pub struct ProductIconCardProps {
    pub icon: Option<Markup>,
    pub emoji: String,
    pub image_url: String,
    pub name: String,
    pub gradient_from: String,
    pub gradient_to: String,
    pub animated: bool,
    pub animation_delay: String,
    pub class: String,
    pub attributes: BTreeMap<String, String>,
}

impl Default for ProductIconCardProps {
    fn default() -> Self {
        Self {
            icon: None,
            emoji: String::new(),
            image_url: String::new(),
            name: String::new(),
            gradient_from: "blue-500".to_string(),
            gradient_to: "purple-600".to_string(),
            animated: false,
            animation_delay: String::new(),
            class: String::new(),
            attributes: BTreeMap::new(),
        }
    }
}

impl ProductIconCardProps {
    #[must_use]
    pub fn with_icon(mut self, icon: Markup) -> Self {
        self.icon = Some(icon);
        self
    }

    #[must_use]
    pub fn with_emoji(mut self, emoji: impl Into<String>) -> Self {
        self.emoji = emoji.into();
        self
    }

    #[must_use]
    pub fn with_image_url(mut self, url: impl Into<String>) -> Self {
        self.image_url = url.into();
        self
    }

    #[must_use]
    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self
    }

    #[must_use]
    pub fn with_gradient(mut self, from: impl Into<String>, to: impl Into<String>) -> Self {
        self.gradient_from = from.into();
        self.gradient_to = to.into();
        self
    }

    #[must_use]
    pub fn with_animation(mut self, delay: impl Into<String>) -> Self {
        self.animated = true;
        self.animation_delay = delay.into();
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
        shared::build_classes(&[
            "flex flex-col items-center gap-2 p-4 rounded-2xl",
            if self.animated { "animate-float" } else { "" },
            self.class.as_str(),
        ])
    }

    #[must_use]
    pub fn build_styles(&self) -> String {
        if self.animated && !self.animation_delay.is_empty() {
            format!("animation-delay: {};", self.animation_delay)
        } else {
            String::new()
        }
    }

    #[must_use]
    pub fn build_attributes(&self) -> BTreeMap<String, String> {
        self.attributes.clone()
    }
}

#[must_use]
pub fn ProductIconCard(props: &ProductIconCardProps) -> Markup {
    let attrs = props.build_attributes();
    let style = props.build_styles();
    html! {
        div
            class=(props.build_classes())
            style=[shared::optional_attr(&style)]
            id=[shared::attr_value(&attrs, "id")]
            title=[shared::attr_value(&attrs, "title")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
        {
            @if !props.emoji.is_empty() {
                (IconBox(
                    &IconBoxProps::default()
                        .with_emoji(&props.emoji)
                        .with_size(shared::SIZE_MD)
                        .with_shape("rounded")
                        .with_gradient(&props.gradient_from, &props.gradient_to),
                    html! {},
                ))
            } @else if let Some(icon) = &props.icon {
                (IconBox(
                    &IconBoxProps::default()
                        .with_icon(icon.clone())
                        .with_size(shared::SIZE_MD)
                        .with_shape("rounded")
                        .with_gradient(&props.gradient_from, &props.gradient_to),
                    html! {},
                ))
            }
            @if !props.name.is_empty() {
                span class="text-sm font-medium" { (&props.name) }
            }
        }
    }
}

#[must_use]
pub fn SimpleProductIconCard(emoji: impl Into<String>, name: impl Into<String>) -> Markup {
    ProductIconCard(
        &ProductIconCardProps::default()
            .with_emoji(emoji)
            .with_name(name),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_go_defaults() {
        let props = ProductIconCardProps::default();
        assert_eq!(props.gradient_from, "blue-500");
        assert_eq!(props.gradient_to, "purple-600");
        assert!(!props.animated);
    }

    #[test]
    fn helpers_cover_animation_styles_attrs_and_builders() {
        let props = ProductIconCardProps::default()
            .with_emoji("!")
            .with_image_url("/image.png")
            .with_name("Name")
            .with_gradient("a", "b")
            .with_animation("0.25s")
            .with_class("extra")
            .with_attribute("title", "Product");
        assert_eq!(
            props.build_classes(),
            "flex flex-col items-center gap-2 p-4 rounded-2xl animate-float extra"
        );
        assert_eq!(props.build_styles(), "animation-delay: 0.25s;");
        assert_eq!(
            props.build_attributes().get("title").map(String::as_str),
            Some("Product")
        );
        assert_eq!(props.image_url, "/image.png");
        assert_eq!(props.gradient_from, "a");
    }

    #[test]
    fn product_icon_card_renders_emoji_icon_name_attrs_and_convenience() {
        let emoji = ProductIconCard(
            &ProductIconCardProps::default()
                .with_emoji("<>")
                .with_name("<Name>")
                .with_attribute("data-testid", "product-icon"),
        )
        .into_string();
        assert!(emoji.starts_with("<div class=\"flex flex-col"));
        assert!(emoji.contains("data-testid=\"product-icon\""));
        assert!(emoji.contains("from-blue-500 to-purple-600"));
        assert!(emoji.contains("<span class=\"text-2xl\">&lt;&gt;</span>"));
        assert!(emoji.contains("<span class=\"text-sm font-medium\">&lt;Name&gt;</span>"));

        let icon = ProductIconCard(
            &ProductIconCardProps::default().with_icon(html! { svg class="raw" {} }),
        )
        .into_string();
        assert!(icon.contains("<span><svg class=\"raw\"></svg></span>"));

        let simple = SimpleProductIconCard("!", "Simple").into_string();
        assert!(simple.contains("Simple"));
    }
}
