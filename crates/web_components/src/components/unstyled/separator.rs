#![allow(non_snake_case)]

use crate::components::{ComponentInfo, shared};
use maud::{Markup, html};
use std::collections::BTreeMap;

pub const COMPONENTS: &[ComponentInfo] = &[
    ComponentInfo {
        family: "unstyled",
        module: "separator",
        name: "Separator",
    },
    ComponentInfo {
        family: "unstyled",
        module: "separator",
        name: "HorizontalSeparator",
    },
    ComponentInfo {
        family: "unstyled",
        module: "separator",
        name: "VerticalSeparator",
    },
    ComponentInfo {
        family: "unstyled",
        module: "separator",
        name: "DashedSeparator",
    },
    ComponentInfo {
        family: "unstyled",
        module: "separator",
        name: "DottedSeparator",
    },
    ComponentInfo {
        family: "unstyled",
        module: "separator",
        name: "SeparatorWithText",
    },
];

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SeparatorProps {
    pub orientation: &'static str,
    pub style: &'static str,
    pub spacing: &'static str,
    pub has_text: bool,
    pub text: String,
    pub class: String,
    pub attributes: BTreeMap<String, String>,
}

impl Default for SeparatorProps {
    fn default() -> Self {
        Self {
            orientation: "horizontal",
            style: "solid",
            spacing: "md",
            has_text: false,
            text: String::new(),
            class: String::new(),
            attributes: BTreeMap::new(),
        }
    }
}

impl SeparatorProps {
    #[must_use]
    pub fn with_orientation(mut self, orientation: &'static str) -> Self {
        self.orientation = orientation;
        self
    }

    #[must_use]
    pub fn with_style(mut self, style: &'static str) -> Self {
        self.style = style;
        self
    }

    #[must_use]
    pub fn with_spacing(mut self, spacing: &'static str) -> Self {
        self.spacing = spacing;
        self
    }

    #[must_use]
    pub fn with_text(mut self, text: impl Into<String>) -> Self {
        self.has_text = true;
        self.text = text.into();
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
        let base_classes = if self.orientation == "vertical" {
            "inline-block h-full"
        } else {
            "w-full"
        };
        let spacing_class = match (self.orientation, self.spacing) {
            ("vertical", "none") => "mx-0",
            ("vertical", "sm") => "mx-2",
            ("vertical", "md") => "mx-4",
            ("vertical", "lg") => "mx-6",
            ("vertical", _) => "mx-4",
            (_, "none") => "my-0",
            (_, "sm") => "my-2",
            (_, "md") => "my-4",
            (_, "lg") => "my-6",
            (_, _) => "my-4",
        };
        shared::build_classes(&[base_classes, spacing_class, self.class.as_str()])
    }

    #[must_use]
    pub fn build_line_classes(&self) -> String {
        let style_class = match self.style {
            "dashed" => "border-dashed",
            "dotted" => "border-dotted",
            _ => "border-solid",
        };
        let orientation_class = if self.orientation == "vertical" {
            "border-l h-full w-0"
        } else {
            "border-t w-full h-0"
        };
        shared::build_classes(&["border-border", style_class, orientation_class])
    }

    #[must_use]
    pub fn build_text_container_classes(&self) -> &'static str {
        if self.orientation == "vertical" {
            "flex flex-col items-center h-full"
        } else {
            "flex items-center w-full"
        }
    }

    #[must_use]
    pub fn build_text_classes(&self) -> &'static str {
        if self.orientation == "vertical" {
            "px-2 text-sm text-muted-foreground whitespace-nowrap rotate-90"
        } else {
            "px-4 text-sm text-muted-foreground whitespace-nowrap"
        }
    }

    #[must_use]
    pub fn build_attributes(&self) -> BTreeMap<String, String> {
        let mut attrs = self.attributes.clone();
        attrs.insert("role".to_string(), "separator".to_string());
        attrs.insert(
            "aria-orientation".to_string(),
            if self.orientation == "vertical" {
                "vertical".to_string()
            } else {
                "horizontal".to_string()
            },
        );
        attrs
    }
}

#[must_use]
pub fn Separator(props: &SeparatorProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        div
            class=(props.build_classes())
            role="separator"
            aria-orientation=(shared::attr_value(&attrs, "aria-orientation").unwrap_or("horizontal"))
            id=[shared::attr_value(&attrs, "id")]
            title=[shared::attr_value(&attrs, "title")]
            data-on:click=[shared::attr_value(&attrs, "data-on:click")]
            data-on:load=[shared::attr_value(&attrs, "data-on:load")]
            data-show=[shared::attr_value(&attrs, "data-show")]
            data-bind=[shared::attr_value(&attrs, "data-bind")]
        {
            @if props.has_text && !props.text.is_empty() {
                div class=(props.build_text_container_classes()) {
                    div class=(props.build_line_classes()) style="flex: 1;" {}
                    span class=(props.build_text_classes()) { (props.text) }
                    div class=(props.build_line_classes()) style="flex: 1;" {}
                }
            } @else {
                div class=(props.build_line_classes()) {}
            }
        }
    }
}

#[must_use]
pub fn HorizontalSeparator(spacing: &'static str, class: impl Into<String>) -> Markup {
    Separator(
        &SeparatorProps::default()
            .with_orientation("horizontal")
            .with_style("solid")
            .with_spacing(spacing)
            .with_class(class),
    )
}

#[must_use]
pub fn VerticalSeparator(spacing: &'static str, class: impl Into<String>) -> Markup {
    Separator(
        &SeparatorProps::default()
            .with_orientation("vertical")
            .with_style("solid")
            .with_spacing(spacing)
            .with_class(class),
    )
}

#[must_use]
pub fn DashedSeparator(spacing: &'static str, class: impl Into<String>) -> Markup {
    Separator(
        &SeparatorProps::default()
            .with_orientation("horizontal")
            .with_style("dashed")
            .with_spacing(spacing)
            .with_class(class),
    )
}

#[must_use]
pub fn DottedSeparator(spacing: &'static str, class: impl Into<String>) -> Markup {
    Separator(
        &SeparatorProps::default()
            .with_orientation("horizontal")
            .with_style("dotted")
            .with_spacing(spacing)
            .with_class(class),
    )
}

#[must_use]
pub fn SeparatorWithText(
    text: impl Into<String>,
    spacing: &'static str,
    class: impl Into<String>,
) -> Markup {
    Separator(
        &SeparatorProps::default()
            .with_orientation("horizontal")
            .with_style("solid")
            .with_spacing(spacing)
            .with_text(text)
            .with_class(class),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const ORIENTATIONS: &[&str] = &["horizontal", "vertical"];
    const STYLES: &[&str] = &["solid", "dashed", "dotted", "unknown"];
    const SPACINGS: &[&str] = &["none", "sm", "md", "lg", "unknown"];

    #[test]
    fn default_props_match_templ_defaults() {
        let props = SeparatorProps::default();
        assert_eq!(props.orientation, "horizontal");
        assert_eq!(props.style, "solid");
        assert_eq!(props.spacing, "md");
        assert!(!props.has_text);
    }

    #[test]
    fn build_classes_covers_orientation_spacing_and_custom_class() {
        for &orientation in ORIENTATIONS {
            for &spacing in SPACINGS {
                let classes = SeparatorProps::default()
                    .with_orientation(orientation)
                    .with_spacing(spacing)
                    .with_class("custom")
                    .build_classes();

                if orientation == "vertical" {
                    assert!(classes.contains("inline-block h-full"));
                    assert!(classes.contains(match spacing {
                        "none" => "mx-0",
                        "sm" => "mx-2",
                        "lg" => "mx-6",
                        _ => "mx-4",
                    }));
                } else {
                    assert!(classes.contains("w-full"));
                    assert!(classes.contains(match spacing {
                        "none" => "my-0",
                        "sm" => "my-2",
                        "lg" => "my-6",
                        _ => "my-4",
                    }));
                }
                assert!(classes.contains("custom"));
            }
        }
    }

    #[test]
    fn build_line_classes_covers_orientation_and_style_combinations() {
        for &orientation in ORIENTATIONS {
            for &style in STYLES {
                let classes = SeparatorProps::default()
                    .with_orientation(orientation)
                    .with_style(style)
                    .build_line_classes();

                assert!(classes.contains("border-border"));
                assert!(classes.contains(match style {
                    "dashed" => "border-dashed",
                    "dotted" => "border-dotted",
                    _ => "border-solid",
                }));
                assert!(classes.contains(if orientation == "vertical" {
                    "border-l h-full w-0"
                } else {
                    "border-t w-full h-0"
                }));
            }
        }
    }

    #[test]
    fn text_and_attribute_helpers_cover_orientation_variations() {
        for &orientation in ORIENTATIONS {
            let props = SeparatorProps::default()
                .with_orientation(orientation)
                .with_attribute("data-on:click", "$x = true");

            let attrs = props.build_attributes();
            assert_eq!(attrs.get("role"), Some(&"separator".to_string()));
            assert_eq!(
                attrs.get("aria-orientation"),
                Some(&orientation.to_string())
            );
            assert_eq!(attrs.get("data-on:click"), Some(&"$x = true".to_string()));

            if orientation == "vertical" {
                assert_eq!(
                    props.build_text_container_classes(),
                    "flex flex-col items-center h-full"
                );
                assert_eq!(
                    props.build_text_classes(),
                    "px-2 text-sm text-muted-foreground whitespace-nowrap rotate-90"
                );
            } else {
                assert_eq!(
                    props.build_text_container_classes(),
                    "flex items-center w-full"
                );
                assert_eq!(
                    props.build_text_classes(),
                    "px-4 text-sm text-muted-foreground whitespace-nowrap"
                );
            }
        }
    }

    #[test]
    fn renders_separator_html_for_all_finite_variations() {
        for &orientation in ORIENTATIONS {
            for &style in STYLES {
                for has_text in [false, true] {
                    let mut props = SeparatorProps::default()
                        .with_orientation(orientation)
                        .with_style(style)
                        .with_attribute("title", "Separator");
                    if has_text {
                        props = props.with_text("Middle");
                    }
                    let html = Separator(&props).into_string();

                    assert!(html.starts_with("<div "));
                    assert!(html.contains(r#"role="separator""#));
                    assert!(html.contains(&format!(r#"aria-orientation="{orientation}""#)));
                    assert!(html.contains(r#"title="Separator""#));
                    assert!(html.contains(props.build_line_classes().as_str()));
                    assert_eq!(html.contains(">Middle</span>"), has_text);
                    if has_text {
                        assert_eq!(html.matches(r#"style="flex: 1;""#).count(), 2);
                    }
                }
            }
        }
    }

    #[test]
    fn convenience_components_render_expected_html() {
        assert!(
            HorizontalSeparator("sm", "x")
                .into_string()
                .contains("my-2")
        );
        assert!(VerticalSeparator("lg", "x").into_string().contains("mx-6"));
        assert!(
            DashedSeparator("md", "x")
                .into_string()
                .contains("border-dashed")
        );
        assert!(
            DottedSeparator("md", "x")
                .into_string()
                .contains("border-dotted")
        );
        let with_text = SeparatorWithText("Section", "md", "x").into_string();
        assert!(with_text.contains(">Section</span>"));
        assert!(with_text.contains("border-solid"));
    }
}
