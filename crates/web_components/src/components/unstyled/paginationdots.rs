#![allow(non_snake_case)]

use crate::components::{ComponentInfo, shared};
use maud::{Markup, html};
use std::collections::BTreeMap;

pub const COMPONENTS: &[ComponentInfo] = &[
    ComponentInfo {
        family: "unstyled",
        module: "paginationdots",
        name: "PaginationDots",
    },
    ComponentInfo {
        family: "unstyled",
        module: "paginationdots",
        name: "SimplePaginationDots",
    },
];

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PaginationDotsProps {
    pub total: i32,
    pub current: i32,
    pub state_var: String,
    pub size: &'static str,
    pub class: String,
    pub attributes: BTreeMap<String, String>,
}

impl Default for PaginationDotsProps {
    fn default() -> Self {
        Self {
            total: 0,
            current: 0,
            state_var: "current".to_string(),
            size: shared::SIZE_MD,
            class: String::new(),
            attributes: BTreeMap::new(),
        }
    }
}

impl PaginationDotsProps {
    #[must_use]
    pub const fn with_total(mut self, value: i32) -> Self {
        self.total = value;
        self
    }

    #[must_use]
    pub const fn with_current(mut self, value: i32) -> Self {
        self.current = value;
        self
    }

    #[must_use]
    pub fn with_state_var(mut self, value: impl Into<String>) -> Self {
        self.state_var = value.into();
        self
    }

    #[must_use]
    pub const fn with_size(mut self, value: &'static str) -> Self {
        self.size = value;
        self
    }

    #[must_use]
    pub fn with_class(mut self, value: impl Into<String>) -> Self {
        self.class = value.into();
        self
    }

    #[must_use]
    pub fn with_attribute(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.attributes.insert(name.into(), value.into());
        self
    }

    #[must_use]
    pub fn build_classes(&self) -> String {
        shared::build_classes(&["flex justify-center items-center", self.class.as_str()])
    }

    #[must_use]
    pub const fn build_dot_classes(&self) -> &'static str {
        "slide-dot"
    }

    #[must_use]
    pub fn build_attributes(&self) -> BTreeMap<String, String> {
        self.attributes.clone()
    }
}

#[must_use]
pub fn ensureSignalPrefix(state_var: &str) -> String {
    if state_var.starts_with('$') {
        state_var.to_string()
    } else {
        format!("${state_var}")
    }
}

#[must_use]
pub fn PaginationDots(props: &PaginationDotsProps) -> Markup {
    let attrs = props.build_attributes();
    let signal = ensureSignalPrefix(&props.state_var);
    html! {
        div
            class=(props.build_classes())
            role="tablist"
            aria-label=(shared::default_strings().pagination_aria_label)
            id=[shared::attr_value(&attrs, "id")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
        {
            @for index in 0..props.total {
                button
                    type="button"
                    data-on:click=(format!("{signal} = {index}"))
                    data-class:slide-dot-active=(format!("{signal} === {index}"))
                    data-attr:aria-selected=(format!("{signal} === {index}"))
                    class=(props.build_dot_classes())
                    role="tab"
                    aria-label=(format!("Go to slide {}", index + 1))
                {}
            }
        }
    }
}

#[must_use]
pub fn SimplePaginationDots(total: i32, state_var: &str) -> Markup {
    PaginationDots(
        &PaginationDotsProps::default()
            .with_total(total)
            .with_state_var(state_var),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn helpers_cover_defaults_classes_attrs_and_signal_prefix() {
        let props = PaginationDotsProps::default();
        assert_eq!(props.total, 0);
        assert_eq!(props.current, 0);
        assert_eq!(props.state_var, "current");
        assert_eq!(props.size, shared::SIZE_MD);
        assert_eq!(props.build_classes(), "flex justify-center items-center");
        assert_eq!(props.build_dot_classes(), "slide-dot");
        assert_eq!(ensureSignalPrefix("current"), "$current");
        assert_eq!(ensureSignalPrefix("$current"), "$current");
        let attrs = PaginationDotsProps::default()
            .with_attribute("id", "dots")
            .with_attribute("data-testid", "slide-dots")
            .build_attributes();
        assert_eq!(attrs.get("id").map(String::as_str), Some("dots"));
        assert_eq!(
            attrs.get("data-testid").map(String::as_str),
            Some("slide-dots")
        );
    }

    #[test]
    fn pagination_dots_render_datastar_attributes_for_each_dot() {
        let rendered = PaginationDots(
            &PaginationDotsProps::default()
                .with_total(3)
                .with_current(1)
                .with_state_var("carouselCurrent")
                .with_size(shared::SIZE_LG)
                .with_class("gap-1")
                .with_attribute("id", "carousel-tabs")
                .with_attribute("data-testid", "dots"),
        )
        .into_string();
        assert!(rendered.starts_with("<div class=\"flex justify-center items-center gap-1\""));
        assert!(rendered.contains("role=\"tablist\""));
        assert!(rendered.contains("aria-label=\"Pagination\""));
        assert!(rendered.contains("id=\"carousel-tabs\""));
        assert!(rendered.contains("data-testid=\"dots\""));
        assert_eq!(rendered.matches("<button ").count(), 3);
        for index in 0..3 {
            assert!(rendered.contains(&format!("data-on:click=\"$carouselCurrent = {index}\"")));
            assert!(rendered.contains(&format!(
                "data-class:slide-dot-active=\"$carouselCurrent === {index}\""
            )));
            assert!(rendered.contains(&format!(
                "data-attr:aria-selected=\"$carouselCurrent === {index}\""
            )));
            assert!(rendered.contains(&format!("aria-label=\"Go to slide {}\"", index + 1)));
        }
        assert_eq!(rendered.matches("class=\"slide-dot\"").count(), 3);
    }

    #[test]
    fn simple_pagination_dots_and_empty_totals_render_correctly() {
        let simple = SimplePaginationDots(2, "$active").into_string();
        assert_eq!(simple.matches("<button ").count(), 2);
        assert!(simple.contains("data-on:click=\"$active = 0\""));
        assert!(simple.contains("data-on:click=\"$active = 1\""));

        let empty = PaginationDots(&PaginationDotsProps::default().with_total(0)).into_string();
        assert!(empty.starts_with("<div class=\"flex justify-center items-center\""));
        assert!(!empty.contains("<button "));
        let negative = PaginationDots(&PaginationDotsProps::default().with_total(-1)).into_string();
        assert!(!negative.contains("<button "));
    }
}
