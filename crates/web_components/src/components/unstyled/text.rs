#![allow(non_snake_case)]

use crate::components::{ComponentInfo, shared};
use maud::{Markup, Render, html};
use std::collections::BTreeMap;

pub const COMPONENTS: &[ComponentInfo] = &[
    ComponentInfo {
        family: "unstyled",
        module: "text",
        name: "Text",
    },
    ComponentInfo {
        family: "unstyled",
        module: "text",
        name: "Paragraph",
    },
    ComponentInfo {
        family: "unstyled",
        module: "text",
        name: "Span",
    },
    ComponentInfo {
        family: "unstyled",
        module: "text",
        name: "MutedText",
    },
    ComponentInfo {
        family: "unstyled",
        module: "text",
        name: "SmallText",
    },
    ComponentInfo {
        family: "unstyled",
        module: "text",
        name: "LeadText",
    },
    ComponentInfo {
        family: "unstyled",
        module: "text",
        name: "TruncatedText",
    },
    ComponentInfo {
        family: "unstyled",
        module: "text",
        name: "ClampedText",
    },
    ComponentInfo {
        family: "unstyled",
        module: "text",
        name: "InlineCode",
    },
    ComponentInfo {
        family: "unstyled",
        module: "text",
        name: "Blockquote",
    },
    ComponentInfo {
        family: "unstyled",
        module: "text",
        name: "Large",
    },
    ComponentInfo {
        family: "unstyled",
        module: "text",
        name: "Muted",
    },
    ComponentInfo {
        family: "unstyled",
        module: "text",
        name: "Lead",
    },
    ComponentInfo {
        family: "unstyled",
        module: "text",
        name: "RichContent",
    },
    ComponentInfo {
        family: "unstyled",
        module: "text",
        name: "RichContentLarge",
    },
    ComponentInfo {
        family: "unstyled",
        module: "text",
        name: "RichContentInline",
    },
];

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TextProps {
    pub as_element: &'static str,
    pub size: &'static str,
    pub weight: &'static str,
    pub variant: &'static str,
    pub align: &'static str,
    pub leading: &'static str,
    pub truncate: bool,
    pub line_clamp: i32,
    pub class: String,
    pub attributes: BTreeMap<String, String>,
}

impl Default for TextProps {
    fn default() -> Self {
        Self {
            as_element: "p",
            size: shared::SIZE_MD,
            weight: shared::TEXT_WEIGHT_NORMAL,
            variant: shared::VARIANT_DEFAULT,
            align: "left",
            leading: "normal",
            truncate: false,
            line_clamp: 0,
            class: String::new(),
            attributes: BTreeMap::new(),
        }
    }
}

impl TextProps {
    #[must_use]
    pub fn with_as(mut self, as_element: &'static str) -> Self {
        self.as_element = as_element;
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
    pub fn with_variant(mut self, variant: &'static str) -> Self {
        self.variant = variant;
        self
    }

    #[must_use]
    pub fn with_align(mut self, align: &'static str) -> Self {
        self.align = align;
        self
    }

    #[must_use]
    pub fn with_leading(mut self, leading: &'static str) -> Self {
        self.leading = leading;
        self
    }

    #[must_use]
    pub const fn with_truncate(mut self, truncate: bool) -> Self {
        self.truncate = truncate;
        self
    }

    #[must_use]
    pub const fn with_line_clamp(mut self, line_clamp: i32) -> Self {
        self.line_clamp = line_clamp;
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
            "text",
            text_size_class(self.size),
            text_weight_class(self.weight),
            text_variant_class(self.variant),
            text_align_class(self.align),
            text_leading_class(self.leading),
            if self.truncate { "truncate" } else { "" },
            line_clamp_class(self.line_clamp),
            self.class.as_str(),
        ])
    }

    #[must_use]
    pub fn build_attributes(&self) -> BTreeMap<String, String> {
        self.attributes.clone()
    }
}

#[must_use]
pub fn text_size_class(size: &str) -> &'static str {
    match size {
        shared::SIZE_XS => "text-xs",
        shared::SIZE_SM => "text-sm",
        shared::SIZE_MD => "text-base",
        shared::SIZE_LG => "text-lg",
        shared::SIZE_XL => "text-xl",
        _ => "text-base",
    }
}

#[must_use]
pub fn text_weight_class(weight: &str) -> &'static str {
    match weight {
        shared::TEXT_WEIGHT_MEDIUM => "font-medium",
        shared::TEXT_WEIGHT_SEMIBOLD => "font-semibold",
        shared::TEXT_WEIGHT_BOLD => "font-bold",
        _ => "font-normal",
    }
}

#[must_use]
pub fn text_variant_class(variant: &str) -> &'static str {
    match variant {
        shared::VARIANT_DEFAULT => "text-foreground",
        "muted" => "text-muted-foreground",
        shared::VARIANT_PRIMARY => "text-primary",
        shared::VARIANT_DESTRUCTIVE => "text-destructive",
        shared::VARIANT_SUCCESS => "text-success",
        shared::VARIANT_WARNING => "text-warning",
        "inherit" => "",
        _ => "text-foreground",
    }
}

#[must_use]
pub fn text_align_class(align: &str) -> &'static str {
    match align {
        "left" => "text-left",
        "center" => "text-center",
        "right" => "text-right",
        "justify" => "text-justify",
        _ => "",
    }
}

#[must_use]
pub fn text_leading_class(leading: &str) -> &'static str {
    match leading {
        "none" => "leading-none",
        "tight" => "leading-tight",
        "snug" => "leading-snug",
        "normal" => "leading-normal",
        "relaxed" => "leading-relaxed",
        "loose" => "leading-loose",
        _ => "",
    }
}

#[must_use]
pub fn line_clamp_class(lines: i32) -> &'static str {
    match lines {
        1 => "line-clamp-1",
        2 => "line-clamp-2",
        3 => "line-clamp-3",
        4 => "line-clamp-4",
        5 => "line-clamp-5",
        6 => "line-clamp-6",
        line if line > 0 => "line-clamp-3",
        _ => "",
    }
}

fn text_attrs<'a>(attrs: &'a BTreeMap<String, String>, name: &str) -> Option<&'a str> {
    shared::attr_value(attrs, name)
}

#[must_use]
pub fn Text(props: &TextProps, children: Markup) -> Markup {
    let attrs = props.build_attributes();
    let class = props.build_classes();
    match props.as_element {
        "span" => {
            html! { span class=(class) id=[text_attrs(&attrs, "id")] title=[text_attrs(&attrs, "title")] data-show=[text_attrs(&attrs, "data-show")] data-text=[text_attrs(&attrs, "data-text")] { (children) } }
        }
        "div" => {
            html! { div class=(class) id=[text_attrs(&attrs, "id")] title=[text_attrs(&attrs, "title")] data-show=[text_attrs(&attrs, "data-show")] data-text=[text_attrs(&attrs, "data-text")] { (children) } }
        }
        "label" => {
            html! { label class=(class) for=[text_attrs(&attrs, "for")] id=[text_attrs(&attrs, "id")] title=[text_attrs(&attrs, "title")] data-show=[text_attrs(&attrs, "data-show")] data-text=[text_attrs(&attrs, "data-text")] { (children) } }
        }
        "strong" => {
            html! { strong class=(class) id=[text_attrs(&attrs, "id")] title=[text_attrs(&attrs, "title")] data-show=[text_attrs(&attrs, "data-show")] data-text=[text_attrs(&attrs, "data-text")] { (children) } }
        }
        "em" => {
            html! { em class=(class) id=[text_attrs(&attrs, "id")] title=[text_attrs(&attrs, "title")] data-show=[text_attrs(&attrs, "data-show")] data-text=[text_attrs(&attrs, "data-text")] { (children) } }
        }
        "small" => {
            html! { small class=(class) id=[text_attrs(&attrs, "id")] title=[text_attrs(&attrs, "title")] data-show=[text_attrs(&attrs, "data-show")] data-text=[text_attrs(&attrs, "data-text")] { (children) } }
        }
        _ => {
            html! { p class=(class) id=[text_attrs(&attrs, "id")] title=[text_attrs(&attrs, "title")] data-show=[text_attrs(&attrs, "data-show")] data-text=[text_attrs(&attrs, "data-text")] { (children) } }
        }
    }
}

#[must_use]
pub fn Paragraph(
    size: &'static str,
    variant: &'static str,
    class: impl Into<String>,
    children: Markup,
) -> Markup {
    Text(
        &TextProps::default()
            .with_as("p")
            .with_size(size)
            .with_weight(shared::TEXT_WEIGHT_NORMAL)
            .with_variant(variant)
            .with_class(class),
        children,
    )
}

#[must_use]
pub fn Span(
    size: &'static str,
    variant: &'static str,
    class: impl Into<String>,
    children: Markup,
) -> Markup {
    Text(
        &TextProps::default()
            .with_as("span")
            .with_size(size)
            .with_weight(shared::TEXT_WEIGHT_NORMAL)
            .with_variant(variant)
            .with_class(class),
        children,
    )
}

#[must_use]
pub fn MutedText(size: &'static str, class: impl Into<String>, children: Markup) -> Markup {
    Text(
        &TextProps::default()
            .with_as("p")
            .with_size(size)
            .with_variant("muted")
            .with_class(class),
        children,
    )
}

#[must_use]
pub fn SmallText(variant: &'static str, class: impl Into<String>, children: Markup) -> Markup {
    Text(
        &TextProps::default()
            .with_as("small")
            .with_size(shared::SIZE_XS)
            .with_variant(variant)
            .with_class(class),
        children,
    )
}

#[must_use]
pub fn LeadText(class: impl Into<String>, children: Markup) -> Markup {
    Text(
        &TextProps::default()
            .with_as("p")
            .with_size(shared::SIZE_LG)
            .with_weight(shared::TEXT_WEIGHT_NORMAL)
            .with_leading("relaxed")
            .with_class(class),
        children,
    )
}

#[must_use]
pub fn TruncatedText(size: &'static str, class: impl Into<String>, children: Markup) -> Markup {
    Text(
        &TextProps::default()
            .with_as("p")
            .with_size(size)
            .with_truncate(true)
            .with_class(class),
        children,
    )
}

#[must_use]
pub fn ClampedText(
    lines: i32,
    size: &'static str,
    class: impl Into<String>,
    children: Markup,
) -> Markup {
    Text(
        &TextProps::default()
            .with_as("p")
            .with_size(size)
            .with_line_clamp(lines)
            .with_class(class),
        children,
    )
}

#[must_use]
pub fn InlineCode(class: impl Into<String>, children: Markup) -> Markup {
    let class = class.into();
    html! { code class=(shared::build_classes(&["relative rounded bg-muted px-[0.3rem] py-[0.2rem] font-mono text-sm font-semibold", class.as_str()])) { (children) } }
}

#[must_use]
pub fn Blockquote(class: impl Into<String>, children: Markup) -> Markup {
    let class = class.into();
    html! { blockquote class=(shared::build_classes(&["mt-6 border-l-2 border-muted pl-6 italic text-muted-foreground", class.as_str()])) { (children) } }
}

#[must_use]
pub fn Large(class: impl Into<String>, children: Markup) -> Markup {
    let class = class.into();
    html! { div class=(shared::build_classes(&["text-lg font-semibold", class.as_str()])) { (children) } }
}

#[must_use]
pub fn Muted(class: impl Into<String>, children: Markup) -> Markup {
    let class = class.into();
    html! { p class=(shared::build_classes(&["text-sm text-muted-foreground", class.as_str()])) { (children) } }
}

#[must_use]
pub fn Lead(class: impl Into<String>, children: Markup) -> Markup {
    let class = class.into();
    html! { p class=(shared::build_classes(&["text-xl text-muted-foreground", class.as_str()])) { (children) } }
}

struct RawHtml<'a>(&'a str);

impl Render for RawHtml<'_> {
    fn render_to(&self, buffer: &mut String) {
        buffer.push_str(self.0);
    }
}

#[must_use]
pub fn RichContentAttributes() -> BTreeMap<String, String> {
    BTreeMap::from([
        ("data-rich-content".to_string(), "true".to_string()),
        (
            "data-on:load".to_string(),
            "window.processChatContent?.($el)".to_string(),
        ),
    ])
}

#[must_use]
pub fn RichContent(content: &str, class: impl Into<String>) -> Markup {
    let class = class.into();
    let attrs = RichContentAttributes();
    html! {
        div
            class=(shared::build_classes(&["prose prose-sm dark:prose-invert max-w-none rich-content", class.as_str()]))
            data-rich-content=(text_attrs(&attrs, "data-rich-content").unwrap_or("true"))
            data-on:load=(text_attrs(&attrs, "data-on:load").unwrap_or("window.processChatContent?.($el)"))
        {
            (RawHtml(content))
        }
    }
}

#[must_use]
pub fn RichContentLarge(content: &str, class: impl Into<String>) -> Markup {
    let class = class.into();
    let attrs = RichContentAttributes();
    html! {
        div
            class=(shared::build_classes(&["prose prose-lg dark:prose-invert max-w-none rich-content", class.as_str()]))
            data-rich-content=(text_attrs(&attrs, "data-rich-content").unwrap_or("true"))
            data-on:load=(text_attrs(&attrs, "data-on:load").unwrap_or("window.processChatContent?.($el)"))
        {
            (RawHtml(content))
        }
    }
}

#[must_use]
pub fn RichContentInline(content: &str, class: impl Into<String>) -> Markup {
    let class = class.into();
    let attrs = RichContentAttributes();
    html! {
        span
            class=(shared::build_classes(&["prose prose-sm dark:prose-invert max-w-none rich-content [&>p]:inline [&>p]:m-0", class.as_str()]))
            data-rich-content=(text_attrs(&attrs, "data-rich-content").unwrap_or("true"))
            data-on:load=(text_attrs(&attrs, "data-on:load").unwrap_or("window.processChatContent?.($el)"))
        {
            (RawHtml(content))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ELEMENTS: &[(&str, &str)] = &[
        ("p", "p"),
        ("span", "span"),
        ("div", "div"),
        ("label", "label"),
        ("strong", "strong"),
        ("em", "em"),
        ("small", "small"),
        ("unknown", "p"),
    ];
    const SIZES: &[(&str, &str)] = &[
        (shared::SIZE_XS, "text-xs"),
        (shared::SIZE_SM, "text-sm"),
        (shared::SIZE_MD, "text-base"),
        (shared::SIZE_LG, "text-lg"),
        (shared::SIZE_XL, "text-xl"),
        ("unknown", "text-base"),
    ];
    const WEIGHTS: &[(&str, &str)] = &[
        (shared::TEXT_WEIGHT_NORMAL, "font-normal"),
        (shared::TEXT_WEIGHT_MEDIUM, "font-medium"),
        (shared::TEXT_WEIGHT_SEMIBOLD, "font-semibold"),
        (shared::TEXT_WEIGHT_BOLD, "font-bold"),
        ("unknown", "font-normal"),
    ];
    const VARIANTS: &[(&str, &str)] = &[
        (shared::VARIANT_DEFAULT, "text-foreground"),
        ("muted", "text-muted-foreground"),
        (shared::VARIANT_PRIMARY, "text-primary"),
        (shared::VARIANT_DESTRUCTIVE, "text-destructive"),
        (shared::VARIANT_SUCCESS, "text-success"),
        (shared::VARIANT_WARNING, "text-warning"),
        ("inherit", ""),
        ("unknown", "text-foreground"),
    ];
    const ALIGNS: &[(&str, &str)] = &[
        ("left", "text-left"),
        ("center", "text-center"),
        ("right", "text-right"),
        ("justify", "text-justify"),
        ("unknown", ""),
    ];
    const LEADINGS: &[(&str, &str)] = &[
        ("none", "leading-none"),
        ("tight", "leading-tight"),
        ("snug", "leading-snug"),
        ("normal", "leading-normal"),
        ("relaxed", "leading-relaxed"),
        ("loose", "leading-loose"),
        ("unknown", ""),
    ];

    #[test]
    fn default_props_match_go_defaults() {
        let props = TextProps::default();
        assert_eq!(props.as_element, "p");
        assert_eq!(props.size, shared::SIZE_MD);
        assert_eq!(props.weight, shared::TEXT_WEIGHT_NORMAL);
        assert_eq!(props.variant, shared::VARIANT_DEFAULT);
        assert_eq!(props.align, "left");
        assert_eq!(props.leading, "normal");
    }

    #[test]
    fn helper_functions_cover_all_finite_class_variations() {
        for &(input, expected) in SIZES {
            assert_eq!(text_size_class(input), expected);
        }
        for &(input, expected) in WEIGHTS {
            assert_eq!(text_weight_class(input), expected);
        }
        for &(input, expected) in VARIANTS {
            assert_eq!(text_variant_class(input), expected);
        }
        for &(input, expected) in ALIGNS {
            assert_eq!(text_align_class(input), expected);
        }
        for &(input, expected) in LEADINGS {
            assert_eq!(text_leading_class(input), expected);
        }
        for lines in -1..=8 {
            let expected = match lines {
                1 => "line-clamp-1",
                2 => "line-clamp-2",
                3 => "line-clamp-3",
                4 => "line-clamp-4",
                5 => "line-clamp-5",
                6 => "line-clamp-6",
                7 | 8 => "line-clamp-3",
                _ => "",
            };
            assert_eq!(line_clamp_class(lines), expected);
        }
    }

    #[test]
    fn build_classes_covers_combinations_and_flags() {
        for &(size, size_class) in SIZES {
            for &(weight, weight_class) in WEIGHTS {
                for &(variant, variant_class) in VARIANTS {
                    let classes = TextProps::default()
                        .with_size(size)
                        .with_weight(weight)
                        .with_variant(variant)
                        .with_align("center")
                        .with_leading("relaxed")
                        .with_truncate(true)
                        .with_line_clamp(2)
                        .with_class("custom")
                        .build_classes();
                    assert!(classes.contains("text"));
                    assert!(classes.contains(size_class));
                    assert!(classes.contains(weight_class));
                    if variant_class.is_empty() {
                        assert!(!classes.contains("text-inherit"));
                    } else {
                        assert!(classes.contains(variant_class));
                    }
                    assert!(classes.contains("text-center"));
                    assert!(classes.contains("leading-relaxed"));
                    assert!(classes.contains("truncate"));
                    assert!(classes.contains("line-clamp-2"));
                    assert!(classes.contains("custom"));
                }
            }
        }
    }

    #[test]
    fn text_renders_all_supported_elements_and_escapes_normal_children() {
        for &(as_element, tag) in ELEMENTS {
            let html = Text(
                &TextProps::default()
                    .with_as(as_element)
                    .with_attribute("title", "\"Text\" & more")
                    .with_attribute("data-show", "$ready"),
                html! { "<Copy>" },
            )
            .into_string();
            assert!(html.starts_with(&format!("<{tag} ")));
            assert!(html.contains(r#"title="&quot;Text&quot; &amp; more""#));
            assert!(html.contains(r#"data-show="$ready""#));
            assert!(html.ends_with(&format!(">&lt;Copy&gt;</{tag}>")));
        }
    }

    #[test]
    fn convenience_text_components_render_expected_tags_and_classes() {
        assert!(
            Paragraph(shared::SIZE_SM, "muted", "p", html! { "P" })
                .into_string()
                .starts_with(r#"<p class="text text-sm font-normal text-muted-foreground"#)
        );
        assert!(
            Span(shared::SIZE_LG, shared::VARIANT_PRIMARY, "s", html! { "S" })
                .into_string()
                .starts_with(r#"<span class="text text-lg font-normal text-primary"#)
        );
        assert!(
            MutedText(shared::SIZE_MD, "m", html! { "M" })
                .into_string()
                .contains("text-muted-foreground")
        );
        assert!(
            SmallText(shared::VARIANT_WARNING, "small", html! { "Small" })
                .into_string()
                .starts_with(r#"<small class="text text-xs font-normal text-warning"#)
        );
        assert!(
            LeadText("lead", html! { "Lead" })
                .into_string()
                .contains("text-lg font-normal text-foreground text-left leading-relaxed")
        );
        assert!(
            TruncatedText(shared::SIZE_XL, "tr", html! { "Tr" })
                .into_string()
                .contains("truncate")
        );
        assert!(
            ClampedText(5, shared::SIZE_MD, "cl", html! { "Cl" })
                .into_string()
                .contains("line-clamp-5")
        );
    }

    #[test]
    fn legacy_text_helpers_render_exact_base_classes() {
        assert_eq!(
            InlineCode("extra", html! { "code" }).into_string(),
            r#"<code class="relative rounded bg-muted px-[0.3rem] py-[0.2rem] font-mono text-sm font-semibold extra">code</code>"#
        );
        assert_eq!(
            Blockquote("quote", html! { "Quote" }).into_string(),
            r#"<blockquote class="mt-6 border-l-2 border-muted pl-6 italic text-muted-foreground quote">Quote</blockquote>"#
        );
        assert_eq!(
            Large("large", html! { "Large" }).into_string(),
            r#"<div class="text-lg font-semibold large">Large</div>"#
        );
        assert_eq!(
            Muted("muted", html! { "Muted" }).into_string(),
            r#"<p class="text-sm text-muted-foreground muted">Muted</p>"#
        );
        assert_eq!(
            Lead("lead", html! { "Lead" }).into_string(),
            r#"<p class="text-xl text-muted-foreground lead">Lead</p>"#
        );
    }

    #[test]
    fn rich_content_helpers_render_datastar_attributes_and_raw_content() {
        let attrs = RichContentAttributes();
        assert_eq!(attrs.get("data-rich-content"), Some(&"true".to_string()));
        assert_eq!(
            attrs.get("data-on:load"),
            Some(&"window.processChatContent?.($el)".to_string())
        );

        let content = "<p><strong>Safe</strong></p>";
        let rich = RichContent(content, "rich").into_string();
        assert!(rich.starts_with(r#"<div class="prose prose-sm dark:prose-invert max-w-none rich-content rich" data-rich-content="true" data-on:load="window.processChatContent?.($el)">"#));
        assert!(rich.contains(content));
        assert!(!rich.contains("&lt;p&gt;"));

        let large = RichContentLarge(content, "large").into_string();
        assert!(large.contains("prose prose-lg"));
        assert!(large.contains(content));

        let inline = RichContentInline(content, "inline").into_string();
        assert!(inline.starts_with(r#"<span class="prose prose-sm dark:prose-invert max-w-none rich-content [&amp;&gt;p]:inline [&amp;&gt;p]:m-0 inline""#));
        assert!(inline.contains(content));
    }
}
