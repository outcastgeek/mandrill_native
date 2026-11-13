#![allow(non_snake_case)]

use crate::components::{ComponentInfo, shared};
use maud::{Markup, html};
use std::collections::BTreeMap;

pub const COMPONENTS: &[ComponentInfo] = &[
    ComponentInfo {
        family: "unstyled",
        module: "blockquote",
        name: "Blockquote",
    },
    ComponentInfo {
        family: "unstyled",
        module: "blockquote",
        name: "SimpleBlockquote",
    },
    ComponentInfo {
        family: "unstyled",
        module: "blockquote",
        name: "BlockquoteWithIcon",
    },
    ComponentInfo {
        family: "unstyled",
        module: "blockquote",
        name: "BlockquoteWithCite",
    },
    ComponentInfo {
        family: "unstyled",
        module: "blockquote",
        name: "PullQuote",
    },
];

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BlockquoteProps {
    pub cite: String,
    pub author: String,
    pub author_title: String,
    pub source: String,
    pub variant: &'static str,
    pub size: &'static str,
    pub show_icon: bool,
    pub border_position: &'static str,
    pub class: String,
    pub attributes: BTreeMap<String, String>,
}

impl Default for BlockquoteProps {
    fn default() -> Self {
        Self {
            cite: String::new(),
            author: String::new(),
            author_title: String::new(),
            source: String::new(),
            variant: shared::VARIANT_DEFAULT,
            size: shared::SIZE_MD,
            show_icon: false,
            border_position: "left",
            class: String::new(),
            attributes: BTreeMap::new(),
        }
    }
}

impl BlockquoteProps {
    #[must_use]
    pub fn with_cite(mut self, cite: impl Into<String>) -> Self {
        self.cite = cite.into();
        self
    }

    #[must_use]
    pub fn with_author(mut self, author: impl Into<String>) -> Self {
        self.author = author.into();
        self
    }

    #[must_use]
    pub fn with_author_title(mut self, title: impl Into<String>) -> Self {
        self.author_title = title.into();
        self
    }

    #[must_use]
    pub fn with_source(mut self, source: impl Into<String>) -> Self {
        self.source = source.into();
        self
    }

    #[must_use]
    pub fn with_variant(mut self, variant: &'static str) -> Self {
        self.variant = variant;
        self
    }

    #[must_use]
    pub fn with_size(mut self, size: &'static str) -> Self {
        self.size = size;
        self
    }

    #[must_use]
    pub const fn with_show_icon(mut self, show_icon: bool) -> Self {
        self.show_icon = show_icon;
        self
    }

    #[must_use]
    pub fn with_border_position(mut self, position: &'static str) -> Self {
        self.border_position = position;
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
    pub fn has_citation(&self) -> bool {
        !self.author.is_empty() || !self.source.is_empty()
    }

    #[must_use]
    pub fn border_class(&self, color_class: &str) -> String {
        match self.border_position {
            "left" => format!("border-l-4 {color_class}"),
            "top" => format!("border-t-4 {color_class}"),
            "none" => String::new(),
            _ => format!("border-l-4 {color_class}"),
        }
    }

    #[must_use]
    pub fn build_classes(&self) -> String {
        let size_class = match self.size {
            shared::SIZE_XS => "text-xs p-2",
            shared::SIZE_SM => "text-sm p-3",
            shared::SIZE_MD => "text-base p-4",
            shared::SIZE_LG => "text-lg p-5",
            shared::SIZE_XL => "text-xl p-6",
            _ => "text-base p-4",
        };
        let (variant_class, border_color) = match self.variant {
            shared::VARIANT_PRIMARY => ("bg-primary/5", "border-primary"),
            "accent" => ("bg-accent/10", "border-accent"),
            "muted" => ("bg-muted/50", "border-muted-foreground/30"),
            _ => ("bg-muted/30", "border-muted-foreground/50"),
        };
        let border_class = self.border_class(border_color);
        shared::build_classes(&[
            "blockquote relative",
            size_class,
            variant_class,
            border_class.as_str(),
            "rounded-lg",
            self.class.as_str(),
        ])
    }

    #[must_use]
    pub fn build_quote_classes(&self) -> String {
        shared::build_classes(&[
            "quote-content italic",
            if self.show_icon { "pl-8" } else { "" },
        ])
    }

    #[must_use]
    pub fn build_cite_classes(&self) -> &'static str {
        "mt-3 text-sm text-muted-foreground not-italic"
    }

    #[must_use]
    pub fn build_attributes(&self) -> BTreeMap<String, String> {
        self.attributes.clone()
    }
}

#[must_use]
pub fn Blockquote(props: &BlockquoteProps, children: Markup) -> Markup {
    let attrs = props.build_attributes();
    html! {
        blockquote
            cite=[if props.cite.is_empty() { None } else { Some(props.cite.as_str()) }]
            class=(props.build_classes())
            id=[shared::attr_value(&attrs, "id")]
            title=[shared::attr_value(&attrs, "title")]
            data-show=[shared::attr_value(&attrs, "data-show")]
        {
            @if props.show_icon {
                svg class="absolute top-4 left-4 w-6 h-6 text-muted-foreground/30" fill="currentColor" viewBox="0 0 24 24" aria-hidden="true" {
                    path d="M14.017 21v-7.391c0-5.704 3.731-9.57 8.983-10.609l.995 2.151c-2.432.917-3.995 3.638-3.995 5.849h4v10h-9.983zm-14.017 0v-7.391c0-5.704 3.748-9.57 9-10.609l.996 2.151c-2.433.917-3.996 3.638-3.996 5.849h3.983v10h-9.983z" {}
                }
            }
            div class=(props.build_quote_classes()) {
                (children)
            }
            @if props.has_citation() {
                footer class=(props.build_cite_classes()) {
                    @if !props.author.is_empty() {
                        cite class="font-semibold not-italic" {
                            (props.author)
                        }
                        @if !props.author_title.is_empty() {
                            span class="text-muted-foreground/80" {
                                ", "
                                (props.author_title)
                            }
                        }
                    }
                    @if !props.source.is_empty() {
                        @if !props.author.is_empty() {
                            span class="mx-1" { "\u{2022}" }
                        }
                        cite { (props.source) }
                    }
                }
            }
        }
    }
}

#[must_use]
pub fn SimpleBlockquote(
    variant: &'static str,
    class: impl Into<String>,
    children: Markup,
) -> Markup {
    Blockquote(
        &BlockquoteProps::default()
            .with_variant(variant)
            .with_size(shared::SIZE_MD)
            .with_border_position("left")
            .with_class(class),
        children,
    )
}

#[must_use]
pub fn BlockquoteWithIcon(
    author: impl Into<String>,
    author_title: impl Into<String>,
    variant: &'static str,
    children: Markup,
) -> Markup {
    Blockquote(
        &BlockquoteProps::default()
            .with_author(author)
            .with_author_title(author_title)
            .with_variant(variant)
            .with_show_icon(true)
            .with_size(shared::SIZE_MD),
        children,
    )
}

#[must_use]
pub fn BlockquoteWithCite(
    author: impl Into<String>,
    source: impl Into<String>,
    cite: impl Into<String>,
    variant: &'static str,
    children: Markup,
) -> Markup {
    Blockquote(
        &BlockquoteProps::default()
            .with_author(author)
            .with_source(source)
            .with_cite(cite)
            .with_variant(variant)
            .with_size(shared::SIZE_MD),
        children,
    )
}

#[must_use]
pub fn PullQuote(author: impl Into<String>, class: impl Into<String>, children: Markup) -> Markup {
    Blockquote(
        &BlockquoteProps::default()
            .with_author(author)
            .with_variant(shared::VARIANT_PRIMARY)
            .with_size(shared::SIZE_LG)
            .with_show_icon(true)
            .with_border_position("top")
            .with_class(class),
        children,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const SIZES: &[(&str, &str)] = &[
        (shared::SIZE_XS, "text-xs p-2"),
        (shared::SIZE_SM, "text-sm p-3"),
        (shared::SIZE_MD, "text-base p-4"),
        (shared::SIZE_LG, "text-lg p-5"),
        (shared::SIZE_XL, "text-xl p-6"),
        ("unknown", "text-base p-4"),
    ];
    const VARIANTS: &[(&str, &str, &str)] = &[
        (
            shared::VARIANT_DEFAULT,
            "bg-muted/30",
            "border-muted-foreground/50",
        ),
        (shared::VARIANT_PRIMARY, "bg-primary/5", "border-primary"),
        ("accent", "bg-accent/10", "border-accent"),
        ("muted", "bg-muted/50", "border-muted-foreground/30"),
        ("unknown", "bg-muted/30", "border-muted-foreground/50"),
    ];
    const BORDERS: &[(&str, &str)] = &[
        ("left", "border-l-4"),
        ("top", "border-t-4"),
        ("none", ""),
        ("unknown", "border-l-4"),
    ];

    #[test]
    fn default_props_match_go_defaults() {
        let props = BlockquoteProps::default();
        assert_eq!(props.variant, shared::VARIANT_DEFAULT);
        assert_eq!(props.size, shared::SIZE_MD);
        assert!(!props.show_icon);
        assert_eq!(props.border_position, "left");
    }

    #[test]
    fn helper_methods_cover_citation_border_classes_quote_classes_and_attributes() {
        assert!(!BlockquoteProps::default().has_citation());
        assert!(BlockquoteProps::default().with_author("A").has_citation());
        assert!(BlockquoteProps::default().with_source("S").has_citation());

        for &(position, expected) in BORDERS {
            let props = BlockquoteProps::default().with_border_position(position);
            let border = props.border_class("border-primary");
            if expected.is_empty() {
                assert!(border.is_empty());
            } else {
                assert!(border.contains(expected));
                assert!(border.contains("border-primary"));
            }
        }

        assert_eq!(
            BlockquoteProps::default().build_quote_classes(),
            "quote-content italic"
        );
        assert_eq!(
            BlockquoteProps::default()
                .with_show_icon(true)
                .build_quote_classes(),
            "quote-content italic pl-8"
        );
        assert_eq!(
            BlockquoteProps::default().build_cite_classes(),
            "mt-3 text-sm text-muted-foreground not-italic"
        );
        let attrs = BlockquoteProps::default()
            .with_attribute("data-show", "$visible")
            .build_attributes();
        assert_eq!(attrs.get("data-show"), Some(&"$visible".to_string()));
    }

    #[test]
    fn build_classes_covers_all_sizes_variants_and_border_positions() {
        for &(size, size_class) in SIZES {
            for &(variant, variant_class, border_color) in VARIANTS {
                for &(position, border_class) in BORDERS {
                    let classes = BlockquoteProps::default()
                        .with_size(size)
                        .with_variant(variant)
                        .with_border_position(position)
                        .with_class("custom")
                        .build_classes();

                    assert!(classes.contains("blockquote relative"));
                    assert!(classes.contains(size_class));
                    assert!(classes.contains(variant_class));
                    if border_class.is_empty() {
                        assert!(!classes.contains("border-l-4"));
                        assert!(!classes.contains("border-t-4"));
                    } else {
                        assert!(classes.contains(border_class));
                        assert!(classes.contains(border_color));
                    }
                    assert!(classes.contains("rounded-lg"));
                    assert!(classes.contains("custom"));
                }
            }
        }
    }

    #[test]
    fn blockquote_renders_icon_citations_cite_attribute_and_escaped_values() {
        let html = Blockquote(
            &BlockquoteProps::default()
                .with_cite("/source")
                .with_author("Ada")
                .with_author_title("Engineer")
                .with_source("Notes")
                .with_show_icon(true)
                .with_attribute("title", "\"Quote\" & more")
                .with_attribute("data-show", "$ready"),
            html! { "<Quote>" },
        )
        .into_string();

        assert!(html.starts_with(r#"<blockquote cite="/source" class=""#));
        assert!(html.contains(r#"title="&quot;Quote&quot; &amp; more""#));
        assert!(html.contains(r#"data-show="$ready""#));
        assert!(
            html.contains(r#"<svg class="absolute top-4 left-4 w-6 h-6 text-muted-foreground/30""#)
        );
        assert!(html.contains(r#"<div class="quote-content italic pl-8">&lt;Quote&gt;</div>"#));
        assert!(html.contains(r#"<cite class="font-semibold not-italic">Ada</cite>"#));
        assert!(html.contains(r#"<span class="text-muted-foreground/80">, Engineer</span>"#));
        assert!(html.contains(r#"<span class="mx-1">•</span><cite>Notes</cite>"#));
    }

    #[test]
    fn blockquote_omits_optional_sections_when_empty() {
        let html = Blockquote(&BlockquoteProps::default(), html! { "Quote" }).into_string();
        assert!(!html.contains("<footer"));
        assert!(!html.contains("<svg"));
        assert!(!html.contains(" cite="));
    }

    #[test]
    fn convenience_components_render_expected_variations() {
        let simple = SimpleBlockquote("accent", "simple", html! { "Simple" }).into_string();
        assert!(simple.contains("bg-accent/10"));
        assert!(simple.contains("simple"));

        let with_icon =
            BlockquoteWithIcon("Ada", "Engineer", "muted", html! { "Icon" }).into_string();
        assert!(with_icon.contains("<svg "));
        assert!(with_icon.contains("pl-8"));
        assert!(with_icon.contains("Ada"));
        assert!(with_icon.contains("Engineer"));

        let with_cite = BlockquoteWithCite(
            "Ada",
            "Notes",
            "/cite",
            shared::VARIANT_PRIMARY,
            html! { "Cite" },
        )
        .into_string();
        assert!(with_cite.contains(r#"cite="/cite""#));
        assert!(with_cite.contains("border-primary"));
        assert!(with_cite.contains("Notes"));

        let pull = PullQuote("Ada", "pull", html! { "Pull" }).into_string();
        assert!(pull.contains("text-lg p-5"));
        assert!(pull.contains("border-t-4 border-primary"));
        assert!(pull.contains("pull"));
        assert!(pull.contains("<svg "));
    }
}
