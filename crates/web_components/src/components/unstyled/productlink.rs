#![allow(non_snake_case)]

use crate::components::{ComponentInfo, shared};
use maud::{Markup, Render, html};
use std::collections::BTreeMap;

pub const COMPONENTS: &[ComponentInfo] = &[
    ComponentInfo {
        family: "unstyled",
        module: "productlink",
        name: "ProductLink",
    },
    ComponentInfo {
        family: "unstyled",
        module: "productlink",
        name: "SimpleProductLink",
    },
];

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProductLinkProps {
    pub title: String,
    pub description: String,
    pub href: String,
    pub icon: String,
    pub icon_bg: String,
    pub class: String,
    pub icon_class: String,
    pub attributes: BTreeMap<String, String>,
}

impl Default for ProductLinkProps {
    fn default() -> Self {
        Self {
            title: String::new(),
            description: String::new(),
            href: String::new(),
            icon: String::new(),
            icon_bg: "bg-primary".to_string(),
            class: String::new(),
            icon_class: String::new(),
            attributes: BTreeMap::new(),
        }
    }
}

impl ProductLinkProps {
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
    pub fn with_icon_bg(mut self, bg: impl Into<String>) -> Self {
        self.icon_bg = bg.into();
        self
    }

    #[must_use]
    pub fn with_icon_class(mut self, class: impl Into<String>) -> Self {
        self.icon_class = class.into();
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
            "flex items-start gap-3 p-3 rounded-lg transition-all duration-150",
            self.class.as_str(),
        ])
    }

    #[must_use]
    pub fn build_icon_classes(&self) -> String {
        shared::build_classes(&[
            "flex-shrink-0 w-10 h-10 rounded flex items-center justify-center text-primary-foreground",
            self.icon_bg.as_str(),
            self.icon_class.as_str(),
        ])
    }

    #[must_use]
    pub fn build_attributes(&self) -> BTreeMap<String, String> {
        self.attributes.clone()
    }
}

struct RawHtml<'a>(&'a str);

impl Render for RawHtml<'_> {
    fn render_to(&self, output: &mut String) {
        output.push_str(self.0);
    }
}

#[must_use]
pub fn ProductLink(props: &ProductLinkProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        a
            href=(&props.href)
            class=(props.build_classes())
            id=[shared::attr_value(&attrs, "id")]
            title=[shared::attr_value(&attrs, "title")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
        {
            div class=(props.build_icon_classes()) {
                (RawHtml(&props.icon))
            }
            div class="flex-1" {
                div class="font-semibold text-sm" { (&props.title) }
                div class="text-sm mt-1" { (&props.description) }
            }
        }
    }
}

#[must_use]
pub fn SimpleProductLink(
    title: impl Into<String>,
    description: impl Into<String>,
    href: impl Into<String>,
    icon: impl Into<String>,
) -> Markup {
    ProductLink(
        &ProductLinkProps::default()
            .with_title(title)
            .with_description(description)
            .with_href(href)
            .with_icon(icon)
            .with_icon_bg("bg-primary"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_props_match_go_defaults() {
        assert_eq!(ProductLinkProps::default().icon_bg, "bg-primary");
    }

    #[test]
    fn helpers_build_classes_icon_classes_attrs_and_builders() {
        let props = ProductLinkProps::default()
            .with_title("Title")
            .with_description("Desc")
            .with_href("/x")
            .with_icon("<svg></svg>")
            .with_icon_bg("bg-info")
            .with_icon_class("extra-icon")
            .with_class("extra")
            .with_attribute("title", "Product");
        assert_eq!(
            props.build_classes(),
            "flex items-start gap-3 p-3 rounded-lg transition-all duration-150 extra"
        );
        assert_eq!(
            props.build_icon_classes(),
            "flex-shrink-0 w-10 h-10 rounded flex items-center justify-center text-primary-foreground bg-info extra-icon"
        );
        assert_eq!(
            props.build_attributes().get("title").map(String::as_str),
            Some("Product")
        );
    }

    #[test]
    fn product_link_renders_raw_icon_escaped_text_and_attrs() {
        let html = ProductLink(
            &ProductLinkProps::default()
                .with_title("<Title>")
                .with_description("<Desc>")
                .with_href("/product?<x>")
                .with_icon("<svg class=\"raw\"></svg>")
                .with_attribute("data-testid", "product"),
        )
        .into_string();

        assert!(html.starts_with("<a href=\"/product?&lt;x&gt;\""));
        assert!(html.contains("data-testid=\"product\""));
        assert!(html.contains("<div class=\"flex-shrink-0 w-10 h-10 rounded flex items-center justify-center text-primary-foreground bg-primary\"><svg class=\"raw\"></svg></div>"));
        assert!(html.contains("<div class=\"font-semibold text-sm\">&lt;Title&gt;</div>"));
        assert!(html.contains("<div class=\"text-sm mt-1\">&lt;Desc&gt;</div>"));

        let simple = SimpleProductLink("T", "D", "/h", "<b>x</b>").into_string();
        assert!(simple.contains("<b>x</b>"));
        assert!(simple.contains(">T</div>"));
    }
}
