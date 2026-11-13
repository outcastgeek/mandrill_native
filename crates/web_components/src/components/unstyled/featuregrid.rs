#![allow(non_snake_case)]

use crate::components::{ComponentInfo, shared};
use maud::{Markup, html};
use std::collections::BTreeMap;

pub const COMPONENTS: &[ComponentInfo] = &[
    ComponentInfo {
        family: "unstyled",
        module: "featuregrid",
        name: "FeatureGrid",
    },
    ComponentInfo {
        family: "unstyled",
        module: "featuregrid",
        name: "SimpleFeatureGrid",
    },
    ComponentInfo {
        family: "unstyled",
        module: "featuregrid",
        name: "TwoColumnGrid",
    },
    ComponentInfo {
        family: "unstyled",
        module: "featuregrid",
        name: "FourColumnGrid",
    },
    ComponentInfo {
        family: "unstyled",
        module: "featuregrid",
        name: "FeatureItem",
    },
    ComponentInfo {
        family: "unstyled",
        module: "featuregrid",
        name: "FeatureCard",
    },
    ComponentInfo {
        family: "unstyled",
        module: "featuregrid",
        name: "featureCardContent",
    },
];

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FeatureGridProps {
    pub columns: i32,
    pub columns_md: i32,
    pub columns_sm: i32,
    pub gap: String,
    pub class: String,
    pub item_class: String,
    pub attributes: BTreeMap<String, String>,
}

impl Default for FeatureGridProps {
    fn default() -> Self {
        Self {
            columns: 3,
            columns_md: 2,
            columns_sm: 1,
            gap: "8".to_string(),
            class: String::new(),
            item_class: String::new(),
            attributes: BTreeMap::new(),
        }
    }
}

impl FeatureGridProps {
    #[must_use]
    pub const fn with_columns(mut self, sm: i32, md: i32, lg: i32) -> Self {
        self.columns_sm = sm;
        self.columns_md = md;
        self.columns = lg;
        self
    }

    #[must_use]
    pub fn with_gap(mut self, value: impl Into<String>) -> Self {
        self.gap = value.into();
        self
    }

    #[must_use]
    pub fn with_class(mut self, value: impl Into<String>) -> Self {
        self.class = value.into();
        self
    }

    #[must_use]
    pub fn with_item_class(mut self, value: impl Into<String>) -> Self {
        self.item_class = value.into();
        self
    }

    #[must_use]
    pub fn with_attribute(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.attributes.insert(name.into(), value.into());
        self
    }

    #[must_use]
    pub fn build_classes(&self) -> String {
        let grid_classes = format!(
            "grid grid-cols-{} md:grid-cols-{} lg:grid-cols-{} gap-{}",
            intToString(self.columns_sm),
            intToString(self.columns_md),
            intToString(self.columns),
            self.gap
        );
        shared::build_classes(&[grid_classes.as_str(), self.class.as_str()])
    }

    #[must_use]
    pub fn build_attributes(&self) -> BTreeMap<String, String> {
        self.attributes.clone()
    }
}

#[must_use]
pub const fn intToString(value: i32) -> &'static str {
    match value {
        1 => "1",
        2 => "2",
        3 => "3",
        4 => "4",
        5 => "5",
        6 => "6",
        _ => "3",
    }
}

#[must_use]
pub fn FeatureGrid(props: &FeatureGridProps, children: Markup) -> Markup {
    let attrs = props.build_attributes();
    html! {
        div
            class=(props.build_classes())
            id=[shared::attr_value(&attrs, "id")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
        {
            (children)
        }
    }
}

#[must_use]
pub fn SimpleFeatureGrid(children: Markup) -> Markup {
    FeatureGrid(&FeatureGridProps::default(), children)
}

#[must_use]
pub fn TwoColumnGrid(children: Markup) -> Markup {
    FeatureGrid(&FeatureGridProps::default().with_columns(1, 2, 2), children)
}

#[must_use]
pub fn FourColumnGrid(children: Markup) -> Markup {
    FeatureGrid(&FeatureGridProps::default().with_columns(1, 2, 4), children)
}

#[must_use]
pub fn FeatureItem(
    icon: &str,
    title: &str,
    description: &str,
    class: &str,
    children: Markup,
) -> Markup {
    html! {
        div class=(format!("flex flex-col gap-3 p-6 rounded-xl {class}")) {
            @if !icon.is_empty() {
                div class="w-12 h-12 rounded-lg bg-primary/10 flex items-center justify-center" {
                    span class="text-2xl" { (icon) }
                }
            }
            @if !title.is_empty() {
                h3 class="font-semibold text-lg" { (title) }
            }
            @if !description.is_empty() {
                p class="text-sm text-muted-foreground leading-relaxed" { (description) }
            }
            (children)
        }
    }
}

#[must_use]
pub fn FeatureCard(icon: &str, title: &str, description: &str, href: &str, class: &str) -> Markup {
    if href.is_empty() {
        html! {
            div class=(format!("p-6 rounded-xl border border-border {class}")) {
                (featureCardContent(icon, title, description))
            }
        }
    } else {
        html! {
            a href=(href) class=(format!("block p-6 rounded-xl border border-border hover:border-primary/50 hover:bg-muted/30 transition-all duration-150 {class}")) {
                (featureCardContent(icon, title, description))
            }
        }
    }
}

#[must_use]
pub fn featureCardContent(icon: &str, title: &str, description: &str) -> Markup {
    html! {
        @if !icon.is_empty() {
            div class="w-10 h-10 rounded-lg bg-primary/10 flex items-center justify-center mb-4" {
                span class="text-xl" { (icon) }
            }
        }
        @if !title.is_empty() {
            h3 class="font-semibold mb-2" { (title) }
        }
        @if !description.is_empty() {
            p class="text-sm text-muted-foreground" { (description) }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn helpers_cover_defaults_columns_gap_attrs_and_int_conversion() {
        let props = FeatureGridProps::default();
        assert_eq!(props.columns, 3);
        assert_eq!(props.columns_md, 2);
        assert_eq!(props.columns_sm, 1);
        assert_eq!(props.gap, "8");
        assert_eq!(
            props.build_classes(),
            "grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-8"
        );
        assert_eq!(
            FeatureGridProps::default()
                .with_columns(2, 3, 6)
                .with_gap("4")
                .with_class("items-start")
                .with_item_class("card")
                .build_classes(),
            "grid grid-cols-2 md:grid-cols-3 lg:grid-cols-6 gap-4 items-start"
        );
        assert_eq!(intToString(1), "1");
        assert_eq!(intToString(5), "5");
        assert_eq!(intToString(6), "6");
        assert_eq!(intToString(9), "3");
        let attrs = FeatureGridProps::default()
            .with_attribute("id", "features")
            .with_attribute("data-testid", "feature-grid")
            .build_attributes();
        assert_eq!(attrs.get("id").map(String::as_str), Some("features"));
        assert_eq!(
            attrs.get("data-testid").map(String::as_str),
            Some("feature-grid")
        );
    }

    #[test]
    fn feature_grid_convenience_variants_render_children() {
        let grid = FeatureGrid(
            &FeatureGridProps::default()
                .with_class("mt-8")
                .with_attribute("id", "grid"),
            html! { article { "Child" } },
        )
        .into_string();
        assert_eq!(
            grid,
            "<div class=\"grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-8 mt-8\" id=\"grid\"><article>Child</article></div>"
        );
        assert!(
            SimpleFeatureGrid(html! { "A" })
                .into_string()
                .contains("lg:grid-cols-3")
        );
        assert!(
            TwoColumnGrid(html! { "B" })
                .into_string()
                .contains("lg:grid-cols-2")
        );
        assert!(
            FourColumnGrid(html! { "C" })
                .into_string()
                .contains("lg:grid-cols-4")
        );
    }

    #[test]
    fn feature_item_renders_optional_parts_children_and_escapes_text() {
        let full = FeatureItem(
            "<*>",
            "<Title>",
            "<Description>",
            "bg-card",
            html! { a href="/more" { "More" } },
        )
        .into_string();
        assert!(full.starts_with("<div class=\"flex flex-col gap-3 p-6 rounded-xl bg-card\">"));
        assert!(full.contains("<span class=\"text-2xl\">&lt;*&gt;</span>"));
        assert!(full.contains("<h3 class=\"font-semibold text-lg\">&lt;Title&gt;</h3>"));
        assert!(full.contains(
            "<p class=\"text-sm text-muted-foreground leading-relaxed\">&lt;Description&gt;</p>"
        ));
        assert!(full.contains("<a href=\"/more\">More</a>"));

        let empty = FeatureItem("", "", "", "", html! { span { "Only child" } }).into_string();
        assert_eq!(
            empty,
            "<div class=\"flex flex-col gap-3 p-6 rounded-xl \"><span>Only child</span></div>"
        );
    }

    #[test]
    fn feature_card_renders_link_and_static_card_variations() {
        let link = FeatureCard("<I>", "<Title>", "<Desc>", "/feature", "shadow-sm").into_string();
        assert!(link.starts_with("<a href=\"/feature\" class=\"block p-6 rounded-xl border border-border hover:border-primary/50 hover:bg-muted/30 transition-all duration-150 shadow-sm\">"));
        assert!(link.contains("<span class=\"text-xl\">&lt;I&gt;</span>"));
        assert!(link.contains("<h3 class=\"font-semibold mb-2\">&lt;Title&gt;</h3>"));
        assert!(link.contains("<p class=\"text-sm text-muted-foreground\">&lt;Desc&gt;</p>"));

        let card = FeatureCard("", "Title", "", "", "").into_string();
        assert_eq!(
            card,
            "<div class=\"p-6 rounded-xl border border-border \"><h3 class=\"font-semibold mb-2\">Title</h3></div>"
        );
        assert_eq!(featureCardContent("", "", "").into_string(), "");
    }
}
