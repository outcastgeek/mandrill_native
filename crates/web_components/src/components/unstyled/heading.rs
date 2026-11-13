#![allow(non_snake_case)]

use crate::components::{ComponentInfo, shared};
use maud::{Markup, html};
use std::collections::BTreeMap;

pub const COMPONENTS: &[ComponentInfo] = &[
    ComponentInfo {
        family: "unstyled",
        module: "heading",
        name: "Heading",
    },
    ComponentInfo {
        family: "unstyled",
        module: "heading",
        name: "H1",
    },
    ComponentInfo {
        family: "unstyled",
        module: "heading",
        name: "H2",
    },
    ComponentInfo {
        family: "unstyled",
        module: "heading",
        name: "H3",
    },
    ComponentInfo {
        family: "unstyled",
        module: "heading",
        name: "H4",
    },
    ComponentInfo {
        family: "unstyled",
        module: "heading",
        name: "H5",
    },
    ComponentInfo {
        family: "unstyled",
        module: "heading",
        name: "H6",
    },
    ComponentInfo {
        family: "unstyled",
        module: "heading",
        name: "PageTitle",
    },
    ComponentInfo {
        family: "unstyled",
        module: "heading",
        name: "SectionTitle",
    },
    ComponentInfo {
        family: "unstyled",
        module: "heading",
        name: "SubsectionTitle",
    },
];

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HeadingProps {
    pub level: i32,
    pub size: &'static str,
    pub weight: &'static str,
    pub gradient: bool,
    pub align: &'static str,
    pub class: String,
    pub attributes: BTreeMap<String, String>,
    pub aria_label: String,
}

impl Default for HeadingProps {
    fn default() -> Self {
        Self {
            level: 1,
            size: "2xl",
            weight: shared::TEXT_WEIGHT_BOLD,
            gradient: false,
            align: "left",
            class: String::new(),
            attributes: BTreeMap::new(),
            aria_label: String::new(),
        }
    }
}

impl HeadingProps {
    #[must_use]
    pub fn with_level(mut self, level: i32) -> Self {
        self.level = level.clamp(1, 6);
        self
    }

    #[must_use]
    pub fn with_size(mut self, size: &'static str) -> Self {
        self.size = size;
        self
    }

    #[must_use]
    pub fn with_weight(mut self, weight: &'static str) -> Self {
        self.weight = weight;
        self
    }

    #[must_use]
    pub const fn with_gradient(mut self, gradient: bool) -> Self {
        self.gradient = gradient;
        self
    }

    #[must_use]
    pub fn with_align(mut self, align: &'static str) -> Self {
        self.align = align;
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
    pub fn with_aria_label(mut self, aria_label: impl Into<String>) -> Self {
        self.aria_label = aria_label.into();
        self
    }

    #[must_use]
    pub fn get_tag(&self) -> &'static str {
        match self.level {
            1 => "h1",
            2 => "h2",
            3 => "h3",
            4 => "h4",
            5 => "h5",
            6 => "h6",
            _ => "h1",
        }
    }

    #[must_use]
    pub fn build_classes(&self) -> String {
        let size_class = match self.size {
            shared::SIZE_XS => "text-xs",
            shared::SIZE_SM => "text-sm",
            shared::SIZE_MD => "text-base",
            shared::SIZE_LG => "text-lg",
            shared::SIZE_XL => "text-xl",
            "2xl" => "text-2xl",
            "3xl" => "text-3xl",
            "4xl" => "text-4xl",
            "5xl" => "text-5xl",
            "6xl" => "text-6xl",
            "7xl" => "text-7xl",
            "8xl" => "text-8xl",
            "9xl" => "text-9xl",
            _ => "text-2xl",
        };
        let weight_class = match self.weight {
            shared::TEXT_WEIGHT_NORMAL => "font-normal",
            shared::TEXT_WEIGHT_MEDIUM => "font-medium",
            shared::TEXT_WEIGHT_SEMIBOLD => "font-semibold",
            shared::TEXT_WEIGHT_BOLD => "font-bold",
            "extrabold" => "font-extrabold",
            "black" => "font-black",
            _ => "font-bold",
        };
        let align_class = match self.align {
            "left" => "text-left",
            "center" => "text-center",
            "right" => "text-right",
            "justify" => "text-justify",
            _ => "",
        };
        shared::build_classes(&[
            "heading",
            size_class,
            weight_class,
            align_class,
            if self.gradient { "text-gradient" } else { "" },
            self.class.as_str(),
        ])
    }

    #[must_use]
    pub fn build_attributes(&self) -> BTreeMap<String, String> {
        let mut attrs = self.attributes.clone();
        if !self.aria_label.is_empty() {
            attrs.insert("aria-label".to_string(), self.aria_label.clone());
        }
        attrs
    }
}

fn render_heading_tag(tag: &str, props: &HeadingProps, children: Markup) -> Markup {
    let attrs = props.build_attributes();
    let class = props.build_classes();
    match tag {
        "h2" => {
            html! { h2 class=(class) id=[shared::attr_value(&attrs, "id")] title=[shared::attr_value(&attrs, "title")] aria-label=[shared::attr_value(&attrs, "aria-label")] data-show=[shared::attr_value(&attrs, "data-show")] { (children) } }
        }
        "h3" => {
            html! { h3 class=(class) id=[shared::attr_value(&attrs, "id")] title=[shared::attr_value(&attrs, "title")] aria-label=[shared::attr_value(&attrs, "aria-label")] data-show=[shared::attr_value(&attrs, "data-show")] { (children) } }
        }
        "h4" => {
            html! { h4 class=(class) id=[shared::attr_value(&attrs, "id")] title=[shared::attr_value(&attrs, "title")] aria-label=[shared::attr_value(&attrs, "aria-label")] data-show=[shared::attr_value(&attrs, "data-show")] { (children) } }
        }
        "h5" => {
            html! { h5 class=(class) id=[shared::attr_value(&attrs, "id")] title=[shared::attr_value(&attrs, "title")] aria-label=[shared::attr_value(&attrs, "aria-label")] data-show=[shared::attr_value(&attrs, "data-show")] { (children) } }
        }
        "h6" => {
            html! { h6 class=(class) id=[shared::attr_value(&attrs, "id")] title=[shared::attr_value(&attrs, "title")] aria-label=[shared::attr_value(&attrs, "aria-label")] data-show=[shared::attr_value(&attrs, "data-show")] { (children) } }
        }
        _ => {
            html! { h1 class=(class) id=[shared::attr_value(&attrs, "id")] title=[shared::attr_value(&attrs, "title")] aria-label=[shared::attr_value(&attrs, "aria-label")] data-show=[shared::attr_value(&attrs, "data-show")] { (children) } }
        }
    }
}

#[must_use]
pub fn Heading(props: &HeadingProps, children: Markup) -> Markup {
    render_heading_tag(props.get_tag(), props, children)
}

#[must_use]
pub fn H1(
    size: &'static str,
    gradient: bool,
    class: impl Into<String>,
    children: Markup,
) -> Markup {
    Heading(
        &HeadingProps::default()
            .with_level(1)
            .with_size(size)
            .with_weight(shared::TEXT_WEIGHT_BOLD)
            .with_gradient(gradient)
            .with_class(class),
        children,
    )
}

#[must_use]
pub fn H2(
    size: &'static str,
    gradient: bool,
    class: impl Into<String>,
    children: Markup,
) -> Markup {
    Heading(
        &HeadingProps::default()
            .with_level(2)
            .with_size(size)
            .with_weight(shared::TEXT_WEIGHT_BOLD)
            .with_gradient(gradient)
            .with_class(class),
        children,
    )
}

#[must_use]
pub fn H3(
    size: &'static str,
    gradient: bool,
    class: impl Into<String>,
    children: Markup,
) -> Markup {
    Heading(
        &HeadingProps::default()
            .with_level(3)
            .with_size(size)
            .with_weight(shared::TEXT_WEIGHT_SEMIBOLD)
            .with_gradient(gradient)
            .with_class(class),
        children,
    )
}

#[must_use]
pub fn H4(
    size: &'static str,
    gradient: bool,
    class: impl Into<String>,
    children: Markup,
) -> Markup {
    Heading(
        &HeadingProps::default()
            .with_level(4)
            .with_size(size)
            .with_weight(shared::TEXT_WEIGHT_SEMIBOLD)
            .with_gradient(gradient)
            .with_class(class),
        children,
    )
}

#[must_use]
pub fn H5(
    size: &'static str,
    gradient: bool,
    class: impl Into<String>,
    children: Markup,
) -> Markup {
    Heading(
        &HeadingProps::default()
            .with_level(5)
            .with_size(size)
            .with_weight(shared::TEXT_WEIGHT_MEDIUM)
            .with_gradient(gradient)
            .with_class(class),
        children,
    )
}

#[must_use]
pub fn H6(
    size: &'static str,
    gradient: bool,
    class: impl Into<String>,
    children: Markup,
) -> Markup {
    Heading(
        &HeadingProps::default()
            .with_level(6)
            .with_size(size)
            .with_weight(shared::TEXT_WEIGHT_MEDIUM)
            .with_gradient(gradient)
            .with_class(class),
        children,
    )
}

#[must_use]
pub fn PageTitle(gradient: bool, class: impl Into<String>, children: Markup) -> Markup {
    H1("4xl", gradient, class, children)
}

#[must_use]
pub fn SectionTitle(gradient: bool, class: impl Into<String>, children: Markup) -> Markup {
    H2("3xl", gradient, class, children)
}

#[must_use]
pub fn SubsectionTitle(gradient: bool, class: impl Into<String>, children: Markup) -> Markup {
    H3("2xl", gradient, class, children)
}

#[cfg(test)]
mod tests {
    use super::*;

    const LEVELS: &[(i32, &str)] = &[
        (0, "h1"),
        (1, "h1"),
        (2, "h2"),
        (3, "h3"),
        (4, "h4"),
        (5, "h5"),
        (6, "h6"),
        (7, "h6"),
    ];
    const SIZES: &[(&str, &str)] = &[
        (shared::SIZE_XS, "text-xs"),
        (shared::SIZE_SM, "text-sm"),
        (shared::SIZE_MD, "text-base"),
        (shared::SIZE_LG, "text-lg"),
        (shared::SIZE_XL, "text-xl"),
        ("2xl", "text-2xl"),
        ("3xl", "text-3xl"),
        ("4xl", "text-4xl"),
        ("5xl", "text-5xl"),
        ("6xl", "text-6xl"),
        ("7xl", "text-7xl"),
        ("8xl", "text-8xl"),
        ("9xl", "text-9xl"),
        ("unknown", "text-2xl"),
    ];
    const WEIGHTS: &[(&str, &str)] = &[
        (shared::TEXT_WEIGHT_NORMAL, "font-normal"),
        (shared::TEXT_WEIGHT_MEDIUM, "font-medium"),
        (shared::TEXT_WEIGHT_SEMIBOLD, "font-semibold"),
        (shared::TEXT_WEIGHT_BOLD, "font-bold"),
        ("extrabold", "font-extrabold"),
        ("black", "font-black"),
        ("unknown", "font-bold"),
    ];
    const ALIGNS: &[(&str, &str)] = &[
        ("left", "text-left"),
        ("center", "text-center"),
        ("right", "text-right"),
        ("justify", "text-justify"),
        ("unknown", ""),
    ];

    #[test]
    fn default_props_match_go_defaults() {
        let props = HeadingProps::default();
        assert_eq!(props.level, 1);
        assert_eq!(props.size, "2xl");
        assert_eq!(props.weight, shared::TEXT_WEIGHT_BOLD);
        assert_eq!(props.align, "left");
        assert!(!props.gradient);
    }

    #[test]
    fn get_tag_clamps_builder_levels_and_maps_all_tags() {
        for &(input, expected) in LEVELS {
            assert_eq!(
                HeadingProps::default().with_level(input).get_tag(),
                expected
            );
        }
        assert_eq!(
            HeadingProps {
                level: 9,
                ..HeadingProps::default()
            }
            .get_tag(),
            "h1"
        );
    }

    #[test]
    fn build_classes_covers_all_sizes_weights_aligns_and_gradient_state() {
        for &(size, size_class) in SIZES {
            for &(weight, weight_class) in WEIGHTS {
                for &(align, align_class) in ALIGNS {
                    for gradient in [false, true] {
                        let classes = HeadingProps::default()
                            .with_size(size)
                            .with_weight(weight)
                            .with_align(align)
                            .with_gradient(gradient)
                            .with_class("custom")
                            .build_classes();

                        assert!(classes.contains("heading"));
                        assert!(classes.contains(size_class));
                        assert!(classes.contains(weight_class));
                        if align_class.is_empty() {
                            assert!(!classes.contains("text-unknown"));
                        } else {
                            assert!(classes.contains(align_class));
                        }
                        assert_eq!(classes.contains("text-gradient"), gradient);
                        assert!(classes.contains("custom"));
                    }
                }
            }
        }
    }

    #[test]
    fn build_attributes_adds_aria_label_and_preserves_custom_attributes() {
        let attrs = HeadingProps::default()
            .with_aria_label("Visible label")
            .with_attribute("data-show", "$visible")
            .build_attributes();

        assert_eq!(attrs.get("aria-label"), Some(&"Visible label".to_string()));
        assert_eq!(attrs.get("data-show"), Some(&"$visible".to_string()));
    }

    #[test]
    fn heading_renders_every_tag_with_escaped_attributes_and_children() {
        for level in 1..=6 {
            let props = HeadingProps::default()
                .with_level(level)
                .with_attribute("title", "\"Title\" & more")
                .with_attribute("data-show", "$ready")
                .with_aria_label("Label");
            let html = Heading(&props, html! { "<Head>" }).into_string();
            let tag = format!("h{level}");

            assert!(html.starts_with(&format!("<{tag} ")));
            assert!(html.contains(r#"title="&quot;Title&quot; &amp; more""#));
            assert!(html.contains(r#"data-show="$ready""#));
            assert!(html.contains(r#"aria-label="Label""#));
            assert!(html.ends_with(&format!(">&lt;Head&gt;</{tag}>")));
        }
    }

    #[test]
    fn convenience_headings_render_expected_levels_sizes_weights_and_gradient() {
        let cases = [
            (
                H1("4xl", true, "c1", html! { "H1" }).into_string(),
                "h1",
                "text-4xl",
                "font-bold",
                "text-gradient",
            ),
            (
                H2("3xl", false, "c2", html! { "H2" }).into_string(),
                "h2",
                "text-3xl",
                "font-bold",
                "",
            ),
            (
                H3("2xl", true, "c3", html! { "H3" }).into_string(),
                "h3",
                "text-2xl",
                "font-semibold",
                "text-gradient",
            ),
            (
                H4(shared::SIZE_XL, false, "c4", html! { "H4" }).into_string(),
                "h4",
                "text-xl",
                "font-semibold",
                "",
            ),
            (
                H5(shared::SIZE_LG, true, "c5", html! { "H5" }).into_string(),
                "h5",
                "text-lg",
                "font-medium",
                "text-gradient",
            ),
            (
                H6(shared::SIZE_MD, false, "c6", html! { "H6" }).into_string(),
                "h6",
                "text-base",
                "font-medium",
                "",
            ),
            (
                PageTitle(true, "page", html! { "Page" }).into_string(),
                "h1",
                "text-4xl",
                "font-bold",
                "text-gradient",
            ),
            (
                SectionTitle(false, "section", html! { "Section" }).into_string(),
                "h2",
                "text-3xl",
                "font-bold",
                "",
            ),
            (
                SubsectionTitle(true, "sub", html! { "Sub" }).into_string(),
                "h3",
                "text-2xl",
                "font-semibold",
                "text-gradient",
            ),
        ];

        for (html, tag, size, weight, gradient) in cases {
            assert!(html.starts_with(&format!("<{tag} ")));
            assert!(html.contains(size));
            assert!(html.contains(weight));
            if gradient.is_empty() {
                assert!(!html.contains("text-gradient"));
            } else {
                assert!(html.contains(gradient));
            }
        }
    }
}
