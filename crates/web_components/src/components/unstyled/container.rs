#![allow(non_snake_case)]

use crate::components::{ComponentInfo, shared};
use maud::{Markup, html};
use std::collections::BTreeMap;

pub const COMPONENTS: &[ComponentInfo] = &[
    ComponentInfo {
        family: "unstyled",
        module: "container",
        name: "Container",
    },
    ComponentInfo {
        family: "unstyled",
        module: "container",
        name: "Section",
    },
    ComponentInfo {
        family: "unstyled",
        module: "container",
        name: "Article",
    },
    ComponentInfo {
        family: "unstyled",
        module: "container",
        name: "Main",
    },
];

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ContainerProps {
    pub max_width: &'static str,
    pub padding: &'static str,
    pub centered: bool,
    pub full_bleed: bool,
    pub class: String,
    pub attributes: BTreeMap<String, String>,
}

impl Default for ContainerProps {
    fn default() -> Self {
        Self {
            max_width: "xl",
            padding: "md",
            centered: true,
            full_bleed: false,
            class: String::new(),
            attributes: BTreeMap::new(),
        }
    }
}

impl ContainerProps {
    #[must_use]
    pub fn with_max_width(mut self, max_width: &'static str) -> Self {
        self.max_width = max_width;
        self
    }

    #[must_use]
    pub fn with_padding(mut self, padding: &'static str) -> Self {
        self.padding = padding;
        self
    }

    #[must_use]
    pub const fn with_centered(mut self, centered: bool) -> Self {
        self.centered = centered;
        self
    }

    #[must_use]
    pub const fn with_full_bleed(mut self, full_bleed: bool) -> Self {
        self.full_bleed = full_bleed;
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
        let max_width_class = if self.full_bleed {
            ""
        } else {
            match self.max_width {
                "sm" => "max-w-screen-sm",
                "md" => "max-w-screen-md",
                "lg" => "max-w-screen-lg",
                "xl" => "max-w-screen-xl",
                "2xl" => "max-w-screen-2xl",
                "full" => "max-w-full",
                _ => "max-w-screen-xl",
            }
        };
        let padding_class = match self.padding {
            "none" => "",
            "sm" => "px-4 py-2",
            "md" => "px-6 py-3 sm:px-8 sm:py-4",
            "lg" => "px-8 py-4 sm:px-12 sm:py-6",
            _ => "px-6 py-3 sm:px-8 sm:py-4",
        };
        shared::build_classes(&[
            "w-full",
            max_width_class,
            padding_class,
            if self.centered { "mx-auto" } else { "" },
            self.class.as_str(),
        ])
    }

    #[must_use]
    pub fn build_attributes(&self) -> BTreeMap<String, String> {
        self.attributes.clone()
    }
}

fn render_tag(tag: &str, props: &ContainerProps, children: Markup) -> Markup {
    let classes = props.build_classes();
    let attrs = props.build_attributes();

    match tag {
        "section" => html! {
            section
                class=(classes)
                id=[shared::attr_value(&attrs, "id")]
                title=[shared::attr_value(&attrs, "title")]
                role=[shared::attr_value(&attrs, "role")]
                aria-label=[shared::attr_value(&attrs, "aria-label")]
                data-on:click=[shared::attr_value(&attrs, "data-on:click")]
                data-on:load=[shared::attr_value(&attrs, "data-on:load")]
                data-show=[shared::attr_value(&attrs, "data-show")]
                data-bind=[shared::attr_value(&attrs, "data-bind")]
                data-signals=[shared::attr_value(&attrs, "data-signals")]
                data-init=[shared::attr_value(&attrs, "data-init")]
            {
                (children)
            }
        },
        "article" => html! {
            article
                class=(classes)
                id=[shared::attr_value(&attrs, "id")]
                title=[shared::attr_value(&attrs, "title")]
                role=[shared::attr_value(&attrs, "role")]
                aria-label=[shared::attr_value(&attrs, "aria-label")]
                data-on:click=[shared::attr_value(&attrs, "data-on:click")]
                data-on:load=[shared::attr_value(&attrs, "data-on:load")]
                data-show=[shared::attr_value(&attrs, "data-show")]
                data-bind=[shared::attr_value(&attrs, "data-bind")]
                data-signals=[shared::attr_value(&attrs, "data-signals")]
                data-init=[shared::attr_value(&attrs, "data-init")]
            {
                (children)
            }
        },
        "main" => html! {
            main
                class=(classes)
                id=[shared::attr_value(&attrs, "id")]
                title=[shared::attr_value(&attrs, "title")]
                role=[shared::attr_value(&attrs, "role")]
                aria-label=[shared::attr_value(&attrs, "aria-label")]
                data-on:click=[shared::attr_value(&attrs, "data-on:click")]
                data-on:load=[shared::attr_value(&attrs, "data-on:load")]
                data-show=[shared::attr_value(&attrs, "data-show")]
                data-bind=[shared::attr_value(&attrs, "data-bind")]
                data-signals=[shared::attr_value(&attrs, "data-signals")]
                data-init=[shared::attr_value(&attrs, "data-init")]
            {
                (children)
            }
        },
        _ => html! {
            div
                class=(classes)
                id=[shared::attr_value(&attrs, "id")]
                title=[shared::attr_value(&attrs, "title")]
                role=[shared::attr_value(&attrs, "role")]
                aria-label=[shared::attr_value(&attrs, "aria-label")]
                data-on:click=[shared::attr_value(&attrs, "data-on:click")]
                data-on:load=[shared::attr_value(&attrs, "data-on:load")]
                data-show=[shared::attr_value(&attrs, "data-show")]
                data-bind=[shared::attr_value(&attrs, "data-bind")]
                data-signals=[shared::attr_value(&attrs, "data-signals")]
                data-init=[shared::attr_value(&attrs, "data-init")]
            {
                (children)
            }
        },
    }
}

#[must_use]
pub fn Container(props: &ContainerProps, children: Markup) -> Markup {
    render_tag("div", props, children)
}

#[must_use]
pub fn Section(props: &ContainerProps, children: Markup) -> Markup {
    render_tag("section", props, children)
}

#[must_use]
pub fn Article(props: &ContainerProps, children: Markup) -> Markup {
    render_tag("article", props, children)
}

#[must_use]
pub fn Main(props: &ContainerProps, children: Markup) -> Markup {
    render_tag("main", props, children)
}

#[cfg(test)]
mod tests {
    use super::*;

    const WIDTHS: &[&str] = &["sm", "md", "lg", "xl", "2xl", "full", "unknown"];
    const PADDINGS: &[&str] = &["none", "sm", "md", "lg", "unknown"];

    #[test]
    fn default_props_match_templ_defaults() {
        let props = ContainerProps::default();
        assert_eq!(props.max_width, "xl");
        assert_eq!(props.padding, "md");
        assert!(props.centered);
        assert!(!props.full_bleed);
    }

    #[test]
    fn build_classes_covers_width_padding_center_and_full_bleed_combinations() {
        for &width in WIDTHS {
            for &padding in PADDINGS {
                for centered in [false, true] {
                    for full_bleed in [false, true] {
                        let props = ContainerProps::default()
                            .with_max_width(width)
                            .with_padding(padding)
                            .with_centered(centered)
                            .with_full_bleed(full_bleed)
                            .with_class("custom");
                        let classes = props.build_classes();

                        assert!(classes.contains("w-full"));
                        assert!(classes.contains("custom"));
                        assert_eq!(classes.contains("mx-auto"), centered);
                        assert_eq!(classes.contains("max-w-"), !full_bleed);
                        match padding {
                            "none" => assert!(!classes.contains("px-")),
                            "sm" => assert!(classes.contains("px-4 py-2")),
                            "lg" => assert!(classes.contains("px-8 py-4 sm:px-12 sm:py-6")),
                            _ => assert!(classes.contains("px-6 py-3 sm:px-8 sm:py-4")),
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn build_attributes_returns_custom_attributes() {
        let attrs = ContainerProps::default()
            .with_attribute("data-on:click", "$x = true")
            .build_attributes();
        assert_eq!(attrs.get("data-on:click"), Some(&"$x = true".to_string()));
    }

    #[test]
    fn renders_all_semantic_tags_with_actual_attributes() {
        let props = ContainerProps::default()
            .with_attribute("title", r#""Container" & more"#)
            .with_class("custom");

        let cases = [
            ("div", Container(&props, html! { "Body" }).into_string()),
            ("section", Section(&props, html! { "Body" }).into_string()),
            ("article", Article(&props, html! { "Body" }).into_string()),
            ("main", Main(&props, html! { "Body" }).into_string()),
        ];

        for (tag, rendered) in cases {
            assert!(rendered.starts_with(&format!("<{tag} ")));
            assert!(rendered.contains("custom"));
            assert!(rendered.contains(r#"title="&quot;Container&quot; &amp; more""#));
            assert!(rendered.ends_with(&format!("Body</{tag}>")));
        }
    }
}
