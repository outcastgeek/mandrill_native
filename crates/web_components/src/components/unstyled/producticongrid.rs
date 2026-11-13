#![allow(non_snake_case)]

use crate::components::unstyled::producticoncard::{ProductIconCard, ProductIconCardProps};
use crate::components::{ComponentInfo, shared};
use maud::{Markup, html};
use std::collections::BTreeMap;

pub const COMPONENTS: &[ComponentInfo] = &[
    ComponentInfo {
        family: "unstyled",
        module: "producticongrid",
        name: "ProductIconGrid",
    },
    ComponentInfo {
        family: "unstyled",
        module: "producticongrid",
        name: "SimpleProductIconGrid",
    },
];

#[derive(Clone, Debug)]
pub struct ProductIconGridProps {
    pub products: Vec<ProductIconCardProps>,
    pub columns: i32,
    pub columns_md: i32,
    pub columns_sm: i32,
    pub gap: String,
    pub class: String,
    pub attributes: BTreeMap<String, String>,
}

impl Default for ProductIconGridProps {
    fn default() -> Self {
        Self {
            products: Vec::new(),
            columns: 4,
            columns_md: 3,
            columns_sm: 2,
            gap: "4".to_string(),
            class: String::new(),
            attributes: BTreeMap::new(),
        }
    }
}

impl ProductIconGridProps {
    #[must_use]
    pub fn with_products(mut self, products: Vec<ProductIconCardProps>) -> Self {
        self.products = products;
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
    pub fn with_gap(mut self, gap: impl Into<String>) -> Self {
        self.gap = gap.into();
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
        let grid_classes = format!(
            "grid grid-cols-{} md:grid-cols-{} lg:grid-cols-{} gap-{}",
            int_to_string(self.columns_sm),
            int_to_string(self.columns_md),
            int_to_string(self.columns),
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
pub fn int_to_string(i: i32) -> &'static str {
    match i {
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
pub fn ProductIconGrid(props: &ProductIconGridProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        div
            class=(props.build_classes())
            id=[shared::attr_value(&attrs, "id")]
            title=[shared::attr_value(&attrs, "title")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
        {
            @for product in &props.products {
                (ProductIconCard(product))
            }
        }
    }
}

#[must_use]
pub fn SimpleProductIconGrid(products: Vec<ProductIconCardProps>) -> Markup {
    ProductIconGrid(&ProductIconGridProps::default().with_products(products))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_and_int_to_string_match_go_defaults() {
        let props = ProductIconGridProps::default();
        assert_eq!(props.columns, 4);
        assert_eq!(props.columns_md, 3);
        assert_eq!(props.columns_sm, 2);
        assert_eq!(props.gap, "4");
        for (value, expected) in [
            (1, "1"),
            (2, "2"),
            (3, "3"),
            (4, "4"),
            (5, "5"),
            (6, "6"),
            (0, "4"),
        ] {
            assert_eq!(int_to_string(value), expected);
        }
    }

    #[test]
    fn helpers_build_classes_attrs_and_builders() {
        let props = ProductIconGridProps::default()
            .with_columns(1, 2, 6)
            .with_gap("8")
            .with_class("extra")
            .with_attribute("title", "Products");
        assert_eq!(
            props.build_classes(),
            "grid grid-cols-1 md:grid-cols-2 lg:grid-cols-6 gap-8 extra"
        );
        assert_eq!(
            props.build_attributes().get("title").map(String::as_str),
            Some("Products")
        );
    }

    #[test]
    fn product_icon_grid_renders_cards_and_simple_variant() {
        let products = vec![
            ProductIconCardProps::default()
                .with_emoji("A")
                .with_name("Alpha"),
            ProductIconCardProps::default()
                .with_emoji("B")
                .with_name("Beta"),
        ];
        let html = ProductIconGrid(
            &ProductIconGridProps::default()
                .with_products(products)
                .with_attribute("data-testid", "products"),
        )
        .into_string();
        assert!(
            html.starts_with("<div class=\"grid grid-cols-2 md:grid-cols-3 lg:grid-cols-4 gap-4\"")
        );
        assert!(html.contains("data-testid=\"products\""));
        assert_eq!(
            html.matches("flex flex-col items-center gap-2 p-4 rounded-2xl")
                .count(),
            2
        );
        assert!(html.contains("Alpha"));
        assert!(html.contains("Beta"));

        assert!(
            SimpleProductIconGrid(vec![
                ProductIconCardProps::default()
                    .with_emoji("C")
                    .with_name("Gamma")
            ])
            .into_string()
            .contains("Gamma")
        );
    }
}
