#![allow(non_snake_case)]

use crate::components::{ComponentInfo, shared};
use maud::{Markup, html};
use std::collections::BTreeMap;

pub const COMPONENTS: &[ComponentInfo] = &[
    ComponentInfo {
        family: "unstyled",
        module: "videocontainer",
        name: "VideoContainer",
    },
    ComponentInfo {
        family: "unstyled",
        module: "videocontainer",
        name: "SimpleVideoContainer",
    },
    ComponentInfo {
        family: "unstyled",
        module: "videocontainer",
        name: "SquareVideoContainer",
    },
];

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VideoContainerProps {
    pub aspect_ratio: String,
    pub container_style: String,
    pub class: String,
    pub container_class: String,
    pub attributes: BTreeMap<String, String>,
}

impl Default for VideoContainerProps {
    fn default() -> Self {
        Self {
            aspect_ratio: "16/9".to_string(),
            container_style: "rounded".to_string(),
            class: String::new(),
            container_class: String::new(),
            attributes: BTreeMap::new(),
        }
    }
}

impl VideoContainerProps {
    #[must_use]
    pub fn with_aspect_ratio(mut self, value: impl Into<String>) -> Self {
        self.aspect_ratio = value.into();
        self
    }
    #[must_use]
    pub fn with_container_style(mut self, value: impl Into<String>) -> Self {
        self.container_style = value.into();
        self
    }
    #[must_use]
    pub fn with_class(mut self, value: impl Into<String>) -> Self {
        self.class = value.into();
        self
    }
    #[must_use]
    pub fn with_container_class(mut self, value: impl Into<String>) -> Self {
        self.container_class = value.into();
        self
    }
    #[must_use]
    pub fn with_attribute(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.attributes.insert(name.into(), value.into());
        self
    }
    #[must_use]
    pub fn build_container_classes(&self) -> String {
        let style = match self.container_style.as_str() {
            "flat" => "",
            "elevated" => "shadow-lg",
            _ => "rounded-2xl",
        };
        let aspect = match self.aspect_ratio.as_str() {
            "4/3" => "aspect-4/3",
            "1/1" => "aspect-square",
            _ => "aspect-video",
        };
        shared::build_classes(&[
            "video-container relative overflow-hidden",
            style,
            aspect,
            self.container_class.as_str(),
            self.class.as_str(),
        ])
    }
    #[must_use]
    pub fn build_attributes(&self) -> BTreeMap<String, String> {
        self.attributes.clone()
    }
}

#[must_use]
pub fn VideoContainer(props: &VideoContainerProps, children: Markup) -> Markup {
    let attrs = props.build_attributes();
    html! {
        div
            class=(props.build_container_classes())
            id=[shared::attr_value(&attrs, "id")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
        {
            (children)
        }
    }
}

#[must_use]
pub fn SimpleVideoContainer(children: Markup) -> Markup {
    VideoContainer(&VideoContainerProps::default(), children)
}

#[must_use]
pub fn SquareVideoContainer(children: Markup) -> Markup {
    VideoContainer(
        &VideoContainerProps::default().with_aspect_ratio("1/1"),
        children,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn helpers_cover_defaults_styles_aspect_ratios_classes_and_attrs() {
        let props = VideoContainerProps::default();
        assert_eq!(props.aspect_ratio, "16/9");
        assert_eq!(props.container_style, "rounded");
        assert_eq!(
            props.build_container_classes(),
            "video-container relative overflow-hidden rounded-2xl aspect-video"
        );
        assert_eq!(
            VideoContainerProps::default()
                .with_container_style("flat")
                .build_container_classes(),
            "video-container relative overflow-hidden aspect-video"
        );
        assert_eq!(
            VideoContainerProps::default()
                .with_container_style("elevated")
                .build_container_classes(),
            "video-container relative overflow-hidden shadow-lg aspect-video"
        );
        assert!(
            VideoContainerProps::default()
                .with_aspect_ratio("4/3")
                .build_container_classes()
                .contains("aspect-4/3")
        );
        assert!(
            VideoContainerProps::default()
                .with_aspect_ratio("1/1")
                .build_container_classes()
                .contains("aspect-square")
        );
        assert!(
            VideoContainerProps::default()
                .with_aspect_ratio("bad")
                .build_container_classes()
                .contains("aspect-video")
        );
        let attrs = VideoContainerProps::default()
            .with_container_class("inner")
            .with_class("outer")
            .with_attribute("data-testid", "video")
            .build_attributes();
        assert_eq!(attrs.get("data-testid").map(String::as_str), Some("video"));
        assert_eq!(
            VideoContainerProps::default()
                .with_container_class("inner")
                .with_class("outer")
                .build_container_classes(),
            "video-container relative overflow-hidden rounded-2xl aspect-video inner outer",
        );
    }

    #[test]
    fn renders_container_and_convenience_variants() {
        let rendered = VideoContainer(
            &VideoContainerProps::default().with_attribute("id", "video"),
            html! { "<video>" },
        )
        .into_string();
        assert_eq!(
            rendered,
            "<div class=\"video-container relative overflow-hidden rounded-2xl aspect-video\" id=\"video\">&lt;video&gt;</div>"
        );
        assert!(
            SimpleVideoContainer(html! { "Body" })
                .into_string()
                .contains("aspect-video")
        );
        assert!(
            SquareVideoContainer(html! { "Body" })
                .into_string()
                .contains("aspect-square")
        );
    }
}
