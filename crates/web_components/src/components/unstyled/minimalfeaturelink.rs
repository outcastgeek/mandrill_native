#![allow(non_snake_case)]

use crate::components::{ComponentInfo, shared};
use maud::{Markup, html};
use std::collections::BTreeMap;

pub const COMPONENTS: &[ComponentInfo] = &[
    ComponentInfo {
        family: "unstyled",
        module: "minimalfeaturelink",
        name: "MinimalFeatureLink",
    },
    ComponentInfo {
        family: "unstyled",
        module: "minimalfeaturelink",
        name: "SimpleMinimalFeatureLink",
    },
];

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct MinimalFeatureLinkProps {
    pub title: String,
    pub description: String,
    pub href: String,
    pub icon: String,
    pub class: String,
    pub attributes: BTreeMap<String, String>,
}

impl MinimalFeatureLinkProps {
    #[must_use]
    pub fn with_title(mut self, title: impl Into<String>) -> Self {
        self.title = title.into();
        self
    }

    #[must_use]
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = description.into();
        self
    }

    #[must_use]
    pub fn with_href(mut self, href: impl Into<String>) -> Self {
        self.href = href.into();
        self
    }

    #[must_use]
    pub fn with_icon(mut self, icon: impl Into<String>) -> Self {
        self.icon = icon.into();
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
            "group flex items-start gap-3 py-4 px-3 -mx-2 rounded-md transition-colors duration-150",
            self.class.as_str(),
        ])
    }

    #[must_use]
    pub fn build_attributes(&self) -> BTreeMap<String, String> {
        self.attributes.clone()
    }
}

#[must_use]
pub fn MinimalFeatureLink(props: &MinimalFeatureLinkProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        a
            href=(&props.href)
            class=(props.build_classes())
            id=[shared::attr_value(&attrs, "id")]
            title=[shared::attr_value(&attrs, "title")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
        {
            div class="flex-shrink-0 w-6 h-6 mt-0.5 transition-colors duration-150" {
                span class="text-xl leading-none" { (&props.icon) }
            }
            div class="flex-1 min-w-0" {
                div class="font-medium text-sm leading-snug" { (&props.title) }
                div class="text-xs mt-0.5 leading-relaxed" { (&props.description) }
            }
        }
    }
}

#[must_use]
pub fn SimpleMinimalFeatureLink(
    title: impl Into<String>,
    description: impl Into<String>,
    href: impl Into<String>,
    icon: impl Into<String>,
) -> Markup {
    MinimalFeatureLink(
        &MinimalFeatureLinkProps::default()
            .with_title(title)
            .with_description(description)
            .with_href(href)
            .with_icon(icon),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_props_are_empty_like_go_defaults() {
        let props = MinimalFeatureLinkProps::default();
        assert!(props.title.is_empty());
        assert!(props.description.is_empty());
        assert!(props.href.is_empty());
        assert!(props.icon.is_empty());
    }

    #[test]
    fn helpers_build_classes_and_attrs() {
        let props = MinimalFeatureLinkProps::default()
            .with_title("Title")
            .with_description("Desc")
            .with_href("/x")
            .with_icon("*")
            .with_class("extra")
            .with_attribute("title", "Link");
        assert_eq!(
            props.build_classes(),
            "group flex items-start gap-3 py-4 px-3 -mx-2 rounded-md transition-colors duration-150 extra"
        );
        assert_eq!(
            props.build_attributes().get("title").map(String::as_str),
            Some("Link")
        );
        assert_eq!(props.title, "Title");
    }

    #[test]
    fn minimal_feature_link_renders_exact_structure_and_escapes_values() {
        let html = MinimalFeatureLink(
            &MinimalFeatureLinkProps::default()
                .with_title("<Title>")
                .with_description("<Desc>")
                .with_href("/feature?<x>")
                .with_icon("<>")
                .with_attribute("data-testid", "feature"),
        )
        .into_string();

        assert!(html.starts_with("<a href=\"/feature?&lt;x&gt;\""));
        assert!(html.contains("data-testid=\"feature\""));
        assert!(
            html.contains("class=\"flex-shrink-0 w-6 h-6 mt-0.5 transition-colors duration-150\"")
        );
        assert!(html.contains("<span class=\"text-xl leading-none\">&lt;&gt;</span>"));
        assert!(
            html.contains("<div class=\"font-medium text-sm leading-snug\">&lt;Title&gt;</div>")
        );
        assert!(html.contains("<div class=\"text-xs mt-0.5 leading-relaxed\">&lt;Desc&gt;</div>"));

        let simple = SimpleMinimalFeatureLink("T", "D", "/h", "!").into_string();
        assert!(simple.contains("href=\"/h\""));
        assert!(simple.contains(">T</div>"));
    }
}
