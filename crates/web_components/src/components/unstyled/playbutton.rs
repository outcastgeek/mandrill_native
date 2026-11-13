#![allow(non_snake_case)]

use crate::components::{ComponentInfo, shared};
use maud::{Markup, html};
use std::collections::BTreeMap;

pub const COMPONENTS: &[ComponentInfo] = &[
    ComponentInfo {
        family: "unstyled",
        module: "playbutton",
        name: "PlayButton",
    },
    ComponentInfo {
        family: "unstyled",
        module: "playbutton",
        name: "SimplePlayButton",
    },
];

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PlayButtonProps {
    pub size: &'static str,
    pub color: String,
    pub class: String,
    pub on_click: String,
    pub attributes: BTreeMap<String, String>,
}

impl Default for PlayButtonProps {
    fn default() -> Self {
        Self {
            size: shared::SIZE_LG,
            color: "blue-600".to_string(),
            class: String::new(),
            on_click: String::new(),
            attributes: BTreeMap::new(),
        }
    }
}

impl PlayButtonProps {
    #[must_use]
    pub const fn with_size(mut self, size: &'static str) -> Self {
        self.size = size;
        self
    }

    #[must_use]
    pub fn with_color(mut self, color: impl Into<String>) -> Self {
        self.color = color.into();
        self
    }

    #[must_use]
    pub fn with_on_click(mut self, on_click: impl Into<String>) -> Self {
        self.on_click = on_click.into();
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
    pub fn build_button_classes(&self) -> String {
        let size_class = match self.size {
            shared::SIZE_SM => "w-12 h-12",
            shared::SIZE_MD => "w-16 h-16",
            shared::SIZE_LG => "w-20 h-20",
            shared::SIZE_XL => "w-24 h-24",
            _ => "",
        };
        shared::build_classes(&[
            "rounded-full flex items-center justify-center transition-transform cursor-pointer hover:scale-110",
            size_class,
            self.class.as_str(),
        ])
    }

    #[must_use]
    pub fn build_icon_classes(&self) -> String {
        let size_class = match self.size {
            shared::SIZE_SM => "w-6 h-6",
            shared::SIZE_MD => "w-8 h-8",
            shared::SIZE_LG => "w-10 h-10",
            shared::SIZE_XL => "w-12 h-12",
            _ => "",
        };
        shared::build_classes(&[size_class, format!("text-{}", self.color).as_str()])
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
pub fn PlayButton(props: &PlayButtonProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        button
            type="button"
            class=(props.build_button_classes())
            aria-label=(shared::default_strings().play_video_aria_label)
            data-on:click=[shared::attr_value(&attrs, "data-on:click")]
            id=[shared::attr_value(&attrs, "id")]
            title=[shared::attr_value(&attrs, "title")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
        {
            svg class=(props.build_icon_classes()) fill="currentColor" viewBox="0 0 20 20" aria-hidden="true" {
                path d="M6.3 2.841A1.5 1.5 0 004 4.11V15.89a1.5 1.5 0 002.3 1.269l9.344-5.89a1.5 1.5 0 000-2.538L6.3 2.84z" {}
            }
        }
    }
}

#[must_use]
pub fn SimplePlayButton(on_click: impl Into<String>) -> Markup {
    PlayButton(&PlayButtonProps::default().with_on_click(on_click))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_props_match_go_defaults() {
        let props = PlayButtonProps::default();
        assert_eq!(props.size, shared::SIZE_LG);
        assert_eq!(props.color, "blue-600");
    }

    #[test]
    fn helper_methods_cover_sizes_icon_color_attrs_and_datastar_click() {
        for (size, button, icon) in [
            (shared::SIZE_SM, "w-12 h-12", "w-6 h-6"),
            (shared::SIZE_MD, "w-16 h-16", "w-8 h-8"),
            (shared::SIZE_LG, "w-20 h-20", "w-10 h-10"),
            (shared::SIZE_XL, "w-24 h-24", "w-12 h-12"),
            (shared::SIZE_XS, "", ""),
        ] {
            let props = PlayButtonProps::default().with_size(size);
            assert!(props.build_button_classes().contains(button));
            assert!(props.build_icon_classes().contains(icon));
        }

        let props = PlayButtonProps::default()
            .with_color("primary")
            .with_class("extra")
            .with_on_click("$playing = true")
            .with_attribute("title", "Play");
        assert!(props.build_button_classes().contains("extra"));
        assert_eq!(props.build_icon_classes(), "w-10 h-10 text-primary");
        let attrs = props.build_attributes();
        assert_eq!(
            attrs.get("data-on:click").map(String::as_str),
            Some("$playing = true")
        );
        assert_eq!(attrs.get("title").map(String::as_str), Some("Play"));
    }

    #[test]
    fn play_button_renders_accessible_button_svg_and_datastar_colon_attribute() {
        let html = PlayButton(
            &PlayButtonProps::default()
                .with_size(shared::SIZE_SM)
                .with_on_click("$video = 'play'")
                .with_attribute("data-testid", "play"),
        )
        .into_string();

        assert!(html.starts_with("<button type=\"button\""));
        assert!(html.contains("aria-label=\"Play video\""));
        assert!(html.contains("data-on:click=\"$video = 'play'\""));
        assert!(html.contains("data-testid=\"play\""));
        assert!(html.contains("class=\"w-6 h-6 text-blue-600\""));
        assert!(html.contains("<path d=\"M6.3 2.841"));

        let simple = SimplePlayButton("$play = true").into_string();
        assert!(simple.contains("data-on:click=\"$play = true\""));
    }
}
