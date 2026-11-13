#![allow(non_snake_case)]

use crate::components::{ComponentInfo, shared};
use maud::{Markup, html};
use std::collections::BTreeMap;

pub const COMPONENTS: &[ComponentInfo] = &[
    ComponentInfo {
        family: "unstyled",
        module: "iconbox",
        name: "IconBox",
    },
    ComponentInfo {
        family: "unstyled",
        module: "iconbox",
        name: "SimpleIconBox",
    },
    ComponentInfo {
        family: "unstyled",
        module: "iconbox",
        name: "GradientIconBox",
    },
    ComponentInfo {
        family: "unstyled",
        module: "iconbox",
        name: "AnimatedIconBox",
    },
];

#[derive(Clone, Debug)]
pub struct IconBoxProps {
    pub icon: Option<Markup>,
    pub icon_type: &'static str,
    pub image_url: String,
    pub size: &'static str,
    pub shape: &'static str,
    pub background: &'static str,
    pub gradient_from: String,
    pub gradient_to: String,
    pub animated: bool,
    pub animation: String,
    pub animation_delay: String,
    pub class: String,
    pub attributes: BTreeMap<String, String>,
}

impl Default for IconBoxProps {
    fn default() -> Self {
        Self {
            icon: None,
            icon_type: "svg",
            image_url: String::new(),
            size: shared::SIZE_MD,
            shape: "rounded",
            background: "none",
            gradient_from: String::new(),
            gradient_to: String::new(),
            animated: false,
            animation: String::new(),
            animation_delay: String::new(),
            class: String::new(),
            attributes: BTreeMap::new(),
        }
    }
}

impl IconBoxProps {
    #[must_use]
    pub fn with_icon(mut self, icon: Markup) -> Self {
        self.icon = Some(icon);
        self.icon_type = "svg";
        self
    }

    #[must_use]
    pub fn with_emoji(mut self, emoji: impl Into<String>) -> Self {
        self.icon_type = "emoji";
        self.image_url = emoji.into();
        self
    }

    #[must_use]
    pub fn with_image(mut self, url: impl Into<String>) -> Self {
        self.icon_type = "image";
        self.image_url = url.into();
        self
    }

    #[must_use]
    pub const fn with_size(mut self, size: &'static str) -> Self {
        self.size = size;
        self
    }

    #[must_use]
    pub const fn with_shape(mut self, shape: &'static str) -> Self {
        self.shape = shape;
        self
    }

    #[must_use]
    pub const fn with_background(mut self, background: &'static str) -> Self {
        self.background = background;
        self
    }

    #[must_use]
    pub fn with_gradient(mut self, from: impl Into<String>, to: impl Into<String>) -> Self {
        self.background = "gradient";
        self.gradient_from = from.into();
        self.gradient_to = to.into();
        self
    }

    #[must_use]
    pub fn with_animation(
        mut self,
        animation_type: impl Into<String>,
        delay: impl Into<String>,
    ) -> Self {
        self.animated = true;
        self.animation = animation_type.into();
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
        let size_class = match self.size {
            shared::SIZE_SM => "w-12 h-12",
            shared::SIZE_MD => "w-16 h-16",
            shared::SIZE_LG => "w-20 h-20",
            shared::SIZE_XL => "w-24 h-24",
            _ => "",
        };

        let shape_class = match self.shape {
            "square" => "rounded-none",
            "rounded" => "rounded-2xl",
            "circle" => "rounded-full",
            _ => "",
        };

        let background_class = if self.background == "gradient"
            && !self.gradient_from.is_empty()
            && !self.gradient_to.is_empty()
        {
            format!(
                "bg-gradient-to-br from-{} to-{}",
                self.gradient_from, self.gradient_to
            )
        } else if self.background == "solid" {
            "bg-primary".to_string()
        } else {
            String::new()
        };

        let animation_class = if self.animated && !self.animation.is_empty() {
            format!("animate-{}", self.animation)
        } else {
            String::new()
        };

        shared::build_classes(&[
            "flex items-center justify-center shrink-0",
            size_class,
            shape_class,
            background_class.as_str(),
            animation_class.as_str(),
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
pub fn IconBox(props: &IconBoxProps, children: Markup) -> Markup {
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
            @if props.icon_type == "emoji" && !props.image_url.is_empty() {
                span class="text-2xl" { (&props.image_url) }
            } @else if props.icon_type == "image" && !props.image_url.is_empty() {
                img src=(&props.image_url) alt="icon" class="w-8 h-8" {}
            } @else if let Some(icon) = &props.icon {
                span { (icon) }
            } @else {
                (children)
            }
        }
    }
}

#[must_use]
pub fn SimpleIconBox(icon: Markup) -> Markup {
    IconBox(&IconBoxProps::default().with_icon(icon), html! {})
}

#[must_use]
pub fn GradientIconBox(icon: Markup, from: impl Into<String>, to: impl Into<String>) -> Markup {
    IconBox(
        &IconBoxProps::default()
            .with_icon(icon)
            .with_size(shared::SIZE_MD)
            .with_shape("rounded")
            .with_gradient(from, to),
        html! {},
    )
}

#[must_use]
pub fn AnimatedIconBox(
    icon: Markup,
    animation: impl Into<String>,
    delay: impl Into<String>,
) -> Markup {
    IconBox(
        &IconBoxProps::default()
            .with_icon(icon)
            .with_animation(animation, delay),
        html! {},
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_props_match_go_defaults() {
        let props = IconBoxProps::default();
        assert_eq!(props.size, shared::SIZE_MD);
        assert_eq!(props.shape, "rounded");
        assert_eq!(props.background, "none");
        assert_eq!(props.icon_type, "svg");
    }

    #[test]
    fn helper_methods_cover_sizes_shapes_backgrounds_animation_attrs() {
        for (size, expected) in [
            (shared::SIZE_SM, "w-12 h-12"),
            (shared::SIZE_MD, "w-16 h-16"),
            (shared::SIZE_LG, "w-20 h-20"),
            (shared::SIZE_XL, "w-24 h-24"),
            (shared::SIZE_XS, ""),
        ] {
            assert!(
                IconBoxProps::default()
                    .with_size(size)
                    .build_classes()
                    .contains(expected)
            );
        }
        for (shape, expected) in [
            ("square", "rounded-none"),
            ("rounded", "rounded-2xl"),
            ("circle", "rounded-full"),
            ("bad", ""),
        ] {
            assert!(
                IconBoxProps::default()
                    .with_shape(shape)
                    .build_classes()
                    .contains(expected)
            );
        }

        let gradient = IconBoxProps::default().with_gradient("a", "b");
        assert!(
            gradient
                .build_classes()
                .contains("bg-gradient-to-br from-a to-b")
        );
        assert!(
            IconBoxProps::default()
                .with_background("solid")
                .build_classes()
                .contains("bg-primary")
        );

        let animated = IconBoxProps::default()
            .with_animation("pulse", "0.5s")
            .with_class("extra")
            .with_attribute("title", "Icon");
        assert!(animated.build_classes().contains("animate-pulse extra"));
        assert_eq!(animated.build_styles(), "animation-delay: 0.5s;");
        assert_eq!(
            animated.build_attributes().get("title").map(String::as_str),
            Some("Icon")
        );
    }

    #[test]
    fn iconbox_renders_emoji_image_icon_children_and_convenience_variants() {
        let emoji =
            IconBox(&IconBoxProps::default().with_emoji("<>"), html! { "child" }).into_string();
        assert!(emoji.contains("<span class=\"text-2xl\">&lt;&gt;</span>"));
        assert!(!emoji.contains("child"));

        let image = IconBox(&IconBoxProps::default().with_image("/i.png"), html! {}).into_string();
        assert!(image.contains("<img src=\"/i.png\" alt=\"icon\" class=\"w-8 h-8\">"));

        let icon = IconBox(
            &IconBoxProps::default()
                .with_icon(html! { svg class="x" {} })
                .with_attribute("data-testid", "box"),
            html! { "child" },
        )
        .into_string();
        assert!(icon.contains("data-testid=\"box\""));
        assert!(icon.contains("<span><svg class=\"x\"></svg></span>"));
        assert!(!icon.contains("child"));

        let child = IconBox(&IconBoxProps::default(), html! { "<child>" }).into_string();
        assert!(child.contains("&lt;child&gt;"));

        assert!(
            SimpleIconBox(html! { svg {} })
                .into_string()
                .contains("<span><svg></svg></span>")
        );
        assert!(
            GradientIconBox(html! { svg {} }, "red", "blue")
                .into_string()
                .contains("from-red to-blue")
        );
        assert!(
            AnimatedIconBox(html! { svg {} }, "bounce", "1s")
                .into_string()
                .contains("animation-delay: 1s;")
        );
    }
}
