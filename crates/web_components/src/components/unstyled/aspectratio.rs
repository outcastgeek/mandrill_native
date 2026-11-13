#![allow(non_snake_case)]

use crate::components::{ComponentInfo, shared};
use maud::{Markup, html};
use std::collections::BTreeMap;

pub const COMPONENTS: &[ComponentInfo] = &[ComponentInfo {
    family: "unstyled",
    module: "aspectratio",
    name: "AspectRatio",
}];

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AspectRatioProps {
    pub ratio: String,
    pub class: String,
    pub attributes: BTreeMap<String, String>,
}

impl Default for AspectRatioProps {
    fn default() -> Self {
        Self {
            ratio: shared::ASPECT_RATIO_16X9.to_string(),
            class: String::new(),
            attributes: BTreeMap::new(),
        }
    }
}

impl AspectRatioProps {
    #[must_use]
    pub fn with_ratio(mut self, ratio: impl Into<String>) -> Self {
        self.ratio = ratio.into();
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
    pub fn ratio_class(&self) -> String {
        match self.ratio.as_str() {
            shared::ASPECT_RATIO_16X9 => "aspect-video".to_string(),
            shared::ASPECT_RATIO_4X3 => "aspect-[4/3]".to_string(),
            shared::ASPECT_RATIO_1X1 => "aspect-square".to_string(),
            shared::ASPECT_RATIO_9X16 => "aspect-[9/16]".to_string(),
            _ => format!("aspect-[{}]", self.ratio),
        }
    }

    #[must_use]
    pub fn build_classes(&self) -> String {
        shared::build_classes(&[
            "relative w-full",
            self.ratio_class().as_str(),
            self.class.as_str(),
        ])
    }

    #[must_use]
    pub fn build_attributes(&self) -> BTreeMap<String, String> {
        self.attributes.clone()
    }
}

#[must_use]
pub fn AspectRatio(props: &AspectRatioProps, children: Markup) -> Markup {
    let attrs = props.build_attributes();
    html! {
        div
            class=(props.build_classes())
            id=[shared::attr_value(&attrs, "id")]
            title=[shared::attr_value(&attrs, "title")]
            data-show=[shared::attr_value(&attrs, "data-show")]
        {
            (children)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATIOS: &[(&str, &str)] = &[
        (shared::ASPECT_RATIO_16X9, "aspect-video"),
        (shared::ASPECT_RATIO_4X3, "aspect-[4/3]"),
        (shared::ASPECT_RATIO_1X1, "aspect-square"),
        (shared::ASPECT_RATIO_9X16, "aspect-[9/16]"),
        ("3/2", "aspect-[3/2]"),
    ];

    #[test]
    fn default_props_match_go_defaults() {
        let props = AspectRatioProps::default();
        assert_eq!(props.ratio, shared::ASPECT_RATIO_16X9);
        assert!(props.class.is_empty());
        assert!(props.attributes.is_empty());
    }

    #[test]
    fn helper_methods_cover_all_ratio_classes_and_attributes() {
        for &(ratio, expected) in RATIOS {
            let props = AspectRatioProps::default()
                .with_ratio(ratio)
                .with_class("custom")
                .with_attribute("data-show", "$ready");
            assert_eq!(props.ratio_class(), expected);
            assert!(props.build_classes().contains("relative w-full"));
            assert!(props.build_classes().contains(expected));
            assert!(props.build_classes().contains("custom"));
            assert_eq!(
                props.build_attributes().get("data-show"),
                Some(&"$ready".to_string())
            );
        }
    }

    #[test]
    fn renders_actual_html_with_escaped_attributes_and_children() {
        let html = AspectRatio(
            &AspectRatioProps::default()
                .with_ratio("4/3")
                .with_attribute("title", "\"Media\" & more"),
            html! { "<Media>" },
        )
        .into_string();

        assert!(html.starts_with(r#"<div class="relative w-full aspect-[4/3]""#));
        assert!(html.contains(r#"title="&quot;Media&quot; &amp; more""#));
        assert!(html.ends_with(">&lt;Media&gt;</div>"));
    }
}
