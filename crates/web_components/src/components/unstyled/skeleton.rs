#![allow(non_snake_case)]

use crate::components::{ComponentInfo, shared};
use maud::{Markup, html};
use std::collections::BTreeMap;

pub const COMPONENTS: &[ComponentInfo] = &[
    ComponentInfo {
        family: "unstyled",
        module: "skeleton",
        name: "Skeleton",
    },
    ComponentInfo {
        family: "unstyled",
        module: "skeleton",
        name: "SkeletonText",
    },
    ComponentInfo {
        family: "unstyled",
        module: "skeleton",
        name: "SkeletonCircle",
    },
    ComponentInfo {
        family: "unstyled",
        module: "skeleton",
        name: "SkeletonRectangle",
    },
    ComponentInfo {
        family: "unstyled",
        module: "skeleton",
        name: "SkeletonCard",
    },
    ComponentInfo {
        family: "unstyled",
        module: "skeleton",
        name: "SkeletonTable",
    },
    ComponentInfo {
        family: "unstyled",
        module: "skeleton",
        name: "SkeletonList",
    },
    ComponentInfo {
        family: "unstyled",
        module: "skeleton",
        name: "SkeletonLine",
    },
    ComponentInfo {
        family: "unstyled",
        module: "skeleton",
        name: "SkeletonStats",
    },
];

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SkeletonProps {
    pub variant: &'static str,
    pub width: String,
    pub height: String,
    pub animated: bool,
    pub lines: i32,
    pub class: String,
    pub attributes: BTreeMap<String, String>,
}

impl Default for SkeletonProps {
    fn default() -> Self {
        Self {
            variant: "rectangle",
            width: "100%".to_string(),
            height: "1rem".to_string(),
            animated: true,
            lines: 1,
            class: String::new(),
            attributes: BTreeMap::new(),
        }
    }
}

impl SkeletonProps {
    #[must_use]
    pub fn with_variant(mut self, variant: &'static str) -> Self {
        self.variant = variant;
        self
    }

    #[must_use]
    pub fn with_width(mut self, width: impl Into<String>) -> Self {
        self.width = width.into();
        self
    }

    #[must_use]
    pub fn with_height(mut self, height: impl Into<String>) -> Self {
        self.height = height.into();
        self
    }

    #[must_use]
    pub const fn with_animated(mut self, animated: bool) -> Self {
        self.animated = animated;
        self
    }

    #[must_use]
    pub const fn with_lines(mut self, lines: i32) -> Self {
        self.lines = lines;
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
        let variant_class = match self.variant {
            "text" => "rounded",
            "circle" => "rounded-full",
            "rectangle" => "rounded-md",
            _ => "",
        };
        shared::build_classes(&[
            "bg-muted",
            variant_class,
            if self.animated { "animate-pulse" } else { "" },
            self.class.as_str(),
        ])
    }

    #[must_use]
    pub fn build_attributes(&self) -> BTreeMap<String, String> {
        let mut attrs = self.attributes.clone();
        attrs.insert("role".to_string(), "status".to_string());
        attrs.insert("aria-label".to_string(), "Loading".to_string());
        attrs.insert("aria-live".to_string(), "polite".to_string());
        attrs
    }

    #[must_use]
    pub fn get_style(&self) -> String {
        let mut style = String::new();
        if self.variant == "circle" {
            if !self.width.is_empty() {
                style = format!("width: {}; height: {};", self.width, self.width);
            } else if !self.height.is_empty() {
                style = format!("width: {}; height: {};", self.height, self.height);
            }
        } else {
            if !self.width.is_empty() {
                style.push_str(&format!("width: {}; ", self.width));
            }
            if !self.height.is_empty() {
                style.push_str(&format!("height: {};", self.height));
            }
        }
        style
    }
}

#[must_use]
pub fn columnsClass(cols: i32) -> &'static str {
    match cols {
        1 => "1",
        2 => "2",
        3 => "3",
        4 => "4",
        _ => "3",
    }
}

fn skeleton_div(props: &SkeletonProps, include_attrs: bool) -> Markup {
    let attrs = props.build_attributes();
    html! {
        div
            class=(props.build_classes())
            style=(props.get_style())
            role=[if include_attrs { shared::attr_value(&attrs, "role") } else { None }]
            aria-label=[if include_attrs { shared::attr_value(&attrs, "aria-label") } else { None }]
            aria-live=[if include_attrs { shared::attr_value(&attrs, "aria-live") } else { None }]
            id=[if include_attrs { shared::attr_value(&attrs, "id") } else { None }]
            title=[if include_attrs { shared::attr_value(&attrs, "title") } else { None }]
        {}
    }
}

#[must_use]
pub fn Skeleton(props: &SkeletonProps) -> Markup {
    if props.variant == "text" && props.lines > 1 {
        let attrs = props.build_attributes();
        html! {
            div
                class="space-y-2"
                role=(shared::attr_value(&attrs, "role").unwrap_or("status"))
                aria-label=(shared::attr_value(&attrs, "aria-label").unwrap_or("Loading"))
                aria-live=(shared::attr_value(&attrs, "aria-live").unwrap_or("polite"))
                id=[shared::attr_value(&attrs, "id")]
                title=[shared::attr_value(&attrs, "title")]
            {
                @for _ in 0..props.lines {
                    (skeleton_div(props, false))
                }
            }
        }
    } else {
        skeleton_div(props, true)
    }
}

#[must_use]
pub fn SkeletonText(lines: i32, animated: bool, class: impl Into<String>) -> Markup {
    Skeleton(
        &SkeletonProps::default()
            .with_variant("text")
            .with_width("100%")
            .with_height("1rem")
            .with_animated(animated)
            .with_lines(lines)
            .with_class(class),
    )
}

#[must_use]
pub fn SkeletonCircle(size: impl Into<String>, animated: bool, class: impl Into<String>) -> Markup {
    let size = size.into();
    Skeleton(
        &SkeletonProps::default()
            .with_variant("circle")
            .with_width(size.clone())
            .with_height(size)
            .with_animated(animated)
            .with_class(class),
    )
}

#[must_use]
pub fn SkeletonRectangle(
    width: impl Into<String>,
    height: impl Into<String>,
    animated: bool,
    class: impl Into<String>,
) -> Markup {
    Skeleton(
        &SkeletonProps::default()
            .with_variant("rectangle")
            .with_width(width)
            .with_height(height)
            .with_animated(animated)
            .with_class(class),
    )
}

#[must_use]
pub fn SkeletonCard(animated: bool, class: impl Into<String>) -> Markup {
    let class = class.into();
    html! {
        div class=(shared::build_classes(&["space-y-4", class.as_str()])) role="status" aria-label=(shared::default_strings().skeleton_loading_card) {
            div class="flex items-center space-x-4" {
                (SkeletonCircle("3rem", animated, ""))
                div class="flex-1 space-y-2" {
                    (SkeletonRectangle("60%", "1rem", animated, ""))
                    (SkeletonRectangle("40%", "0.875rem", animated, ""))
                }
            }
            div class="space-y-2" {
                (SkeletonText(3, animated, ""))
            }
        }
    }
}

#[must_use]
pub fn SkeletonTable(rows: i32, animated: bool, class: impl Into<String>) -> Markup {
    let class = class.into();
    html! {
        div class=(shared::build_classes(&["space-y-2", class.as_str()])) role="status" aria-label=(shared::default_strings().skeleton_loading_table) {
            @for _ in 0..rows {
                div class="flex space-x-4" {
                    (SkeletonRectangle("20%", "2.5rem", animated, ""))
                    (SkeletonRectangle("30%", "2.5rem", animated, ""))
                    (SkeletonRectangle("25%", "2.5rem", animated, ""))
                    (SkeletonRectangle("25%", "2.5rem", animated, ""))
                }
            }
        }
    }
}

#[must_use]
pub fn SkeletonList(items: i32, animated: bool, class: impl Into<String>) -> Markup {
    let class = class.into();
    html! {
        div class=(shared::build_classes(&["space-y-3", class.as_str()])) role="status" aria-label=(shared::default_strings().skeleton_loading_list) {
            @for _ in 0..items {
                div class="flex items-center space-x-3" {
                    (SkeletonCircle("2.5rem", animated, ""))
                    div class="flex-1 space-y-2" {
                        (SkeletonRectangle("70%", "1rem", animated, ""))
                        (SkeletonRectangle("50%", "0.875rem", animated, ""))
                    }
                }
            }
        }
    }
}

#[must_use]
pub fn SkeletonLine(width: impl Into<String>, animated: bool, class: impl Into<String>) -> Markup {
    Skeleton(
        &SkeletonProps::default()
            .with_variant("text")
            .with_width(width)
            .with_height("1rem")
            .with_animated(animated)
            .with_lines(1)
            .with_class(class),
    )
}

#[must_use]
pub fn SkeletonStats(columns: i32, animated: bool, class: impl Into<String>) -> Markup {
    let class = class.into();
    html! {
        div class=(shared::build_classes(&["grid gap-4", &format!("grid-cols-{}", columnsClass(columns)), class.as_str()])) role="status" aria-label=(shared::default_strings().skeleton_loading_stats) {
            @for _ in 0..columns {
                div class="space-y-2 p-4 rounded-lg border" {
                    (SkeletonRectangle("40%", "0.875rem", animated, ""))
                    (SkeletonRectangle("60%", "1.5rem", animated, ""))
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const VARIANTS: &[(&str, &str)] = &[
        ("text", "rounded"),
        ("circle", "rounded-full"),
        ("rectangle", "rounded-md"),
        ("custom", ""),
    ];

    #[test]
    fn default_props_match_go_defaults() {
        let props = SkeletonProps::default();
        assert_eq!(props.variant, "rectangle");
        assert_eq!(props.width, "100%");
        assert_eq!(props.height, "1rem");
        assert!(props.animated);
        assert_eq!(props.lines, 1);
    }

    #[test]
    fn helper_methods_cover_variants_animation_attributes_styles_and_columns() {
        for &(variant, variant_class) in VARIANTS {
            for animated in [false, true] {
                let props = SkeletonProps::default()
                    .with_variant(variant)
                    .with_width("50%")
                    .with_height("2rem")
                    .with_animated(animated)
                    .with_class("custom")
                    .with_attribute("title", "Loading");
                let classes = props.build_classes();
                assert!(classes.contains("bg-muted"));
                if variant_class.is_empty() {
                    assert!(!classes.contains("rounded-full"));
                    assert!(!classes.contains("rounded-md"));
                } else {
                    assert!(classes.contains(variant_class));
                }
                assert_eq!(classes.contains("animate-pulse"), animated);
                assert!(classes.contains("custom"));
                let attrs = props.build_attributes();
                assert_eq!(attrs.get("role"), Some(&"status".to_string()));
                assert_eq!(attrs.get("aria-label"), Some(&"Loading".to_string()));
                assert_eq!(attrs.get("aria-live"), Some(&"polite".to_string()));
                assert_eq!(attrs.get("title"), Some(&"Loading".to_string()));
            }
        }
        assert_eq!(
            SkeletonProps::default()
                .with_width("10rem")
                .with_height("2rem")
                .get_style(),
            "width: 10rem; height: 2rem;"
        );
        assert_eq!(
            SkeletonProps::default()
                .with_variant("circle")
                .with_width("3rem")
                .get_style(),
            "width: 3rem; height: 3rem;"
        );
        assert_eq!(
            SkeletonProps::default()
                .with_variant("circle")
                .with_width("")
                .with_height("4rem")
                .get_style(),
            "width: 4rem; height: 4rem;"
        );
        for cols in 0..=5 {
            let expected = match cols {
                1 => "1",
                2 => "2",
                3 => "3",
                4 => "4",
                _ => "3",
            };
            assert_eq!(columnsClass(cols), expected);
        }
    }

    #[test]
    fn skeleton_renders_single_and_multiline_text_variations() {
        let single =
            Skeleton(&SkeletonProps::default().with_attribute("title", "One")).into_string();
        assert!(single.starts_with(r#"<div class="bg-muted rounded-md animate-pulse" style="width: 100%; height: 1rem;" role="status" aria-label="Loading" aria-live="polite""#));
        assert!(single.contains(r#"title="One""#));

        let multiline = Skeleton(
            &SkeletonProps::default()
                .with_variant("text")
                .with_lines(3)
                .with_width("80%"),
        )
        .into_string();
        assert!(multiline.starts_with(
            r#"<div class="space-y-2" role="status" aria-label="Loading" aria-live="polite">"#
        ));
        assert_eq!(multiline.matches(r#"<div class="bg-muted rounded animate-pulse" style="width: 80%; height: 1rem;"></div>"#).count(), 3);
    }

    #[test]
    fn skeleton_convenience_shapes_render_expected_styles_and_animation() {
        assert!(
            SkeletonText(2, false, "txt")
                .into_string()
                .contains("space-y-2")
        );
        assert!(SkeletonText(2, false, "txt").into_string().contains("txt"));
        assert!(
            !SkeletonText(2, false, "txt")
                .into_string()
                .contains("animate-pulse")
        );
        assert!(
            SkeletonCircle("2rem", true, "circle")
                .into_string()
                .contains("rounded-full animate-pulse circle")
        );
        assert!(
            SkeletonCircle("2rem", true, "circle")
                .into_string()
                .contains(r#"style="width: 2rem; height: 2rem;""#)
        );
        assert!(
            SkeletonRectangle("10rem", "5rem", false, "rect")
                .into_string()
                .contains(r#"style="width: 10rem; height: 5rem;""#)
        );
        assert!(
            SkeletonLine("42%", true, "line")
                .into_string()
                .contains(r#"style="width: 42%; height: 1rem;""#)
        );
    }

    #[test]
    fn composite_skeletons_render_expected_counts_and_labels() {
        let card = SkeletonCard(true, "card").into_string();
        assert!(card.contains(r#"aria-label="Loading card""#));
        assert!(card.contains("card"));
        assert!(card.contains(r#"style="width: 3rem; height: 3rem;""#));

        let table = SkeletonTable(2, false, "table").into_string();
        assert!(table.contains(r#"aria-label="Loading table""#));
        assert_eq!(table.matches(r#"<div class="flex space-x-4">"#).count(), 2);
        assert_eq!(
            table
                .matches(r#"style="width: 20%; height: 2.5rem;""#)
                .count(),
            2
        );

        let list = SkeletonList(3, true, "list").into_string();
        assert!(list.contains(r#"aria-label="Loading list""#));
        assert_eq!(
            list.matches(r#"<div class="flex items-center space-x-3">"#)
                .count(),
            3
        );

        let stats = SkeletonStats(4, true, "stats").into_string();
        assert!(stats.contains(r#"aria-label="Loading statistics""#));
        assert!(stats.contains("grid-cols-4"));
        assert_eq!(
            stats
                .matches(r#"<div class="space-y-2 p-4 rounded-lg border">"#)
                .count(),
            4
        );
    }
}
