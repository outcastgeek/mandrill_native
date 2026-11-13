#![allow(non_snake_case)]
#![allow(clippy::needless_pass_by_value)]

use crate::components::ComponentInfo;
use crate::components::shared;
use crate::components::unstyled::text as base;
use maud::{Markup, html};

pub use base::TextProps;

pub const COMPONENTS: &[ComponentInfo] = &[
    ComponentInfo {
        family: "neomorphism",
        module: "text",
        name: "Text",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "text",
        name: "Paragraph",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "text",
        name: "Span",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "text",
        name: "MutedText",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "text",
        name: "SmallText",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "text",
        name: "LeadText",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "text",
        name: "TruncatedText",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "text",
        name: "ClampedText",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "text",
        name: "NeomorphicDescription",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "text",
        name: "NeomorphicCaption",
    },
];

#[must_use]
fn addNeomorphClasses(props: &TextProps) -> TextProps {
    let mut props = props.clone();
    props.class = shared::build_classes(&[props.class.as_str(), "antialiased"]);
    props
}

#[must_use]
pub fn Text(props: &TextProps, children: Markup) -> Markup {
    let neo_props = addNeomorphClasses(props);
    base::Text(&neo_props, children)
}

#[must_use]
pub fn Paragraph(
    size: &'static str,
    variant: &'static str,
    class: impl Into<String>,
    children: Markup,
) -> Markup {
    let class = class.into();
    base::Paragraph(
        size,
        variant,
        shared::build_classes(&["mb-4", class.as_str()]),
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
    base::Span(size, variant, class, children)
}

#[must_use]
pub fn MutedText(size: &'static str, class: impl Into<String>, children: Markup) -> Markup {
    base::MutedText(size, class, children)
}

#[must_use]
pub fn SmallText(variant: &'static str, class: impl Into<String>, children: Markup) -> Markup {
    base::SmallText(variant, class, children)
}

#[must_use]
pub fn LeadText(class: impl Into<String>, children: Markup) -> Markup {
    let class = class.into();
    base::LeadText(
        shared::build_classes(&["mb-6 text-muted-foreground", class.as_str()]),
        children,
    )
}

#[must_use]
pub fn TruncatedText(size: &'static str, class: impl Into<String>, children: Markup) -> Markup {
    base::TruncatedText(size, class, children)
}

#[must_use]
pub fn ClampedText(
    lines: i32,
    size: &'static str,
    class: impl Into<String>,
    children: Markup,
) -> Markup {
    base::ClampedText(lines, size, class, children)
}

#[must_use]
pub fn NeomorphicDescription(text: &str) -> Markup {
    Paragraph("md", "muted", "leading-relaxed", html! { (text) })
}

#[must_use]
pub fn NeomorphicCaption(text: &str) -> Markup {
    SmallText("muted", "italic", html! { (text) })
}

#[cfg(test)]
mod tests {
    use super::*;
    use maud::html;
    fn assert_no_placeholder_markers(html: &str) {
        assert!(
            !html.contains("data-family="),
            "placeholder family marker leaked: {html}"
        );
        assert!(
            !html.contains("data-module="),
            "placeholder module marker leaked: {html}"
        );
        assert!(
            !html.contains("data-component=\""),
            "placeholder component marker leaked: {html}"
        );
    }

    #[test]
    fn registry_matches_neomorphism_source_components() {
        assert_eq!(COMPONENTS.len(), 10);
        assert!(
            COMPONENTS
                .iter()
                .all(|component| component.family == "neomorphism" && component.module == "text")
        );
        assert!(COMPONENTS.iter().any(|component| component.name == "Text"));
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "Paragraph")
        );
        assert!(COMPONENTS.iter().any(|component| component.name == "Span"));
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "MutedText")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "SmallText")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "LeadText")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "TruncatedText")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "ClampedText")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "NeomorphicDescription")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "NeomorphicCaption")
        );
    }

    #[test]
    fn wrapped_components_render_unstyled_html_with_neomorphism_classes() {
        let html = Text(&TextProps::default(), html! { "child" }).into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.starts_with("<"), "expected rendered element: {html}");
        assert!(html.contains("antialiased"), "missing antialiased: {html}");
    }

    #[test]
    fn neomorphism_only_components_render_html_without_generated_stub_markers() {
        let html = Span("sm", "primary", "inline-extra", html! { "span child" }).into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.contains("<span class=\"text text-sm font-normal text-primary text-left leading-normal inline-extra\">span child</span>"));

        let html = MutedText("lg", "muted-extra", html! { "muted child" }).into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.contains("<p class=\"text text-lg font-normal text-muted-foreground text-left leading-normal muted-extra\">muted child</p>"));

        let html = LeadText("lead-extra", html! { "lead child" }).into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.contains("text text-lg font-normal text-foreground text-left leading-relaxed mb-6 text-muted-foreground lead-extra"));
        assert!(html.contains(">lead child</p>"));

        let html = TruncatedText("xs", "truncate-extra", html! { "truncate child" }).into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.contains("text text-xs font-normal text-foreground text-left leading-normal truncate truncate-extra"));
        assert!(html.contains(">truncate child</p>"));

        let html = ClampedText(4, "xl", "clamp-extra", html! { "clamp child" }).into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.contains("text text-xl font-normal text-foreground text-left leading-normal line-clamp-4 clamp-extra"));
        assert!(html.contains(">clamp child</p>"));

        let html = NeomorphicDescription("sample").into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.contains("<p class=\"text text-base font-normal text-muted-foreground text-left leading-normal mb-4 leading-relaxed\">sample</p>"));
        let html = NeomorphicCaption("sample").into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.contains("<small"));
        assert!(html.contains("italic"));
        assert!(html.contains(">sample</small>"));
    }
}
