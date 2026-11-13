#![allow(non_snake_case)]

use crate::components::unstyled::statcard::{self, StatCardProps};
use crate::components::{ComponentInfo, shared};
use maud::{Markup, html};
use std::collections::BTreeMap;

pub const COMPONENTS: &[ComponentInfo] = &[
    ComponentInfo {
        family: "unstyled",
        module: "statsbar",
        name: "StatsBar",
    },
    ComponentInfo {
        family: "unstyled",
        module: "statsbar",
        name: "SimpleStatsBar",
    },
];

#[derive(Clone, Debug)]
pub struct StatsBarProps {
    pub items: Vec<StatCardProps>,
    pub columns: i32,
    pub columns_md: i32,
    pub columns_sm: i32,
    pub gap: String,
    pub variant: String,
    pub class: String,
    pub item_class: String,
    pub attributes: BTreeMap<String, String>,
}

impl Default for StatsBarProps {
    fn default() -> Self {
        Self {
            items: Vec::new(),
            columns: 4,
            columns_md: 2,
            columns_sm: 2,
            gap: "8".to_string(),
            variant: "card".to_string(),
            class: String::new(),
            item_class: String::new(),
            attributes: BTreeMap::new(),
        }
    }
}

impl StatsBarProps {
    #[must_use]
    pub fn with_items(mut self, items: Vec<StatCardProps>) -> Self {
        self.items = items;
        self
    }

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
    pub fn with_variant(mut self, value: impl Into<String>) -> Self {
        self.variant = value.into();
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
        let variant_classes = match self.variant.as_str() {
            "card" => "text-center",
            "separated" => "divide-x divide-border",
            _ => "",
        };
        shared::build_classes(&[grid_classes.as_str(), variant_classes, self.class.as_str()])
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
        _ => "4",
    }
}

#[must_use]
pub fn StatsBar(props: &StatsBarProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        div
            class=(props.build_classes())
            id=[shared::attr_value(&attrs, "id")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
        {
            @for item in &props.items {
                (statcard::StatCard(&item.clone().with_class(props.item_class.clone())))
            }
        }
    }
}

#[must_use]
pub fn SimpleStatsBar(items: Vec<StatCardProps>) -> Markup {
    StatsBar(&StatsBarProps::default().with_items(items))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(value: &str, label: &str) -> StatCardProps {
        StatCardProps::default().with_value(value).with_label(label)
    }

    #[test]
    fn helpers_cover_defaults_columns_variants_attrs_and_ints() {
        let props = StatsBarProps::default();
        assert_eq!(props.columns, 4);
        assert_eq!(props.columns_md, 2);
        assert_eq!(props.columns_sm, 2);
        assert_eq!(props.gap, "8");
        assert_eq!(props.variant, "card");
        assert_eq!(
            props.build_classes(),
            "grid grid-cols-2 md:grid-cols-2 lg:grid-cols-4 gap-8 text-center"
        );
        assert_eq!(
            StatsBarProps::default()
                .with_columns(1, 3, 6)
                .with_gap("4")
                .with_variant("separated")
                .with_class("py-6")
                .build_classes(),
            "grid grid-cols-1 md:grid-cols-3 lg:grid-cols-6 gap-4 divide-x divide-border py-6"
        );
        assert_eq!(
            StatsBarProps::default()
                .with_variant("inline")
                .build_classes(),
            "grid grid-cols-2 md:grid-cols-2 lg:grid-cols-4 gap-8"
        );
        assert_eq!(intToString(1), "1");
        assert_eq!(intToString(5), "5");
        assert_eq!(intToString(6), "6");
        assert_eq!(intToString(9), "4");
        let attrs = StatsBarProps::default()
            .with_attribute("id", "stats")
            .with_attribute("data-testid", "statsbar")
            .build_attributes();
        assert_eq!(attrs.get("id").map(String::as_str), Some("stats"));
        assert_eq!(
            attrs.get("data-testid").map(String::as_str),
            Some("statsbar")
        );
    }

    #[test]
    fn statsbar_renders_items_with_item_class_and_escapes_values() {
        let html = StatsBar(
            &StatsBarProps::default()
                .with_items(vec![item("<10M>", "Users"), item("95%", "<Accuracy>")])
                .with_item_class("px-4")
                .with_attribute("id", "homepage-stats")
                .with_attribute("data-testid", "stats"),
        )
        .into_string();
        assert!(html.starts_with(
            "<div class=\"grid grid-cols-2 md:grid-cols-2 lg:grid-cols-4 gap-8 text-center\""
        ));
        assert!(html.contains("id=\"homepage-stats\""));
        assert!(html.contains("data-testid=\"stats\""));
        assert_eq!(
            html.matches("class=\"stat-card text-center px-4\"").count(),
            2
        );
        assert!(html.contains("&lt;10M&gt;"));
        assert!(html.contains("&lt;Accuracy&gt;"));
        assert!(html.contains(">Users</div>"));
    }

    #[test]
    fn simple_statsbar_and_empty_items_render_expected_html() {
        let simple = SimpleStatsBar(vec![item("24/7", "Support")]).into_string();
        assert!(simple.starts_with(
            "<div class=\"grid grid-cols-2 md:grid-cols-2 lg:grid-cols-4 gap-8 text-center\">"
        ));
        assert!(simple.contains(">24/7</div>"));
        assert!(simple.contains(">Support</div>"));

        let empty = StatsBar(&StatsBarProps::default()).into_string();
        assert_eq!(
            empty,
            "<div class=\"grid grid-cols-2 md:grid-cols-2 lg:grid-cols-4 gap-8 text-center\"></div>"
        );
    }
}
