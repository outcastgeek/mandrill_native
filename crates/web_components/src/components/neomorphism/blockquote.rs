#![allow(non_snake_case)]
#![allow(clippy::needless_pass_by_value)]

use crate::components::unstyled::blockquote as base;
use crate::components::{ComponentInfo, shared};
use maud::{Markup, html};

pub use base::{
    BlockquoteProps, BlockquoteWithCite, BlockquoteWithIcon, PullQuote, SimpleBlockquote,
};

pub const COMPONENTS: &[ComponentInfo] = &[
    ComponentInfo {
        family: "neomorphism",
        module: "blockquote",
        name: "Blockquote",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "blockquote",
        name: "SimpleBlockquote",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "blockquote",
        name: "BlockquoteWithIcon",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "blockquote",
        name: "BlockquoteWithCite",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "blockquote",
        name: "PullQuote",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "blockquote",
        name: "NeomorphicTestimonial",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "blockquote",
        name: "NeomorphicQuoteCard",
    },
];

#[must_use]
fn addNeomorphClasses(props: &BlockquoteProps) -> BlockquoteProps {
    let mut props = props.clone();
    let neomorph_class = match props.variant {
        shared::VARIANT_PRIMARY => "neomorph-flat shadow-md",
        "accent" => "neomorph-raised",
        _ => "neomorph-concave",
    };
    props.class = shared::build_classes(&[neomorph_class, props.class.as_str()]);
    props
}

#[must_use]
pub fn Blockquote(props: &BlockquoteProps, children: Markup) -> Markup {
    let neo_props = addNeomorphClasses(props);
    base::Blockquote(&neo_props, children)
}

#[must_use]
pub fn NeomorphicTestimonial(
    author: &str,
    authorTitle: &str,
    avatarUrl: &str,
    children: Markup,
) -> Markup {
    html! {
        div class="neomorph-card rounded-2xl p-6" {
            div class="flex items-start gap-4" {
                @if !avatarUrl.is_empty() {
                    div class="neomorph-concave rounded-full p-1 flex-shrink-0" {
                        img src=(avatarUrl) alt=(author) class="w-12 h-12 rounded-full object-cover" {}
                    }
                }
                div class="flex-1" { (BlockquoteWithIcon(author, authorTitle, "default", children)) }
            }
        }
    }
}

#[must_use]
pub fn NeomorphicQuoteCard(author: &str, source: &str, children: Markup) -> Markup {
    html! {
        div class="neomorph-raised rounded-2xl p-8 text-center" {
            svg class="w-12 h-12 mx-auto mb-6 text-primary/20" fill="currentColor" viewBox="0 0 24 24" aria-hidden="true" {
                path d="M14.017 21v-7.391c0-5.704 3.731-9.57 8.983-10.609l.995 2.151c-2.432.917-3.995 3.638-3.995 5.849h4v10h-9.983zm-14.017 0v-7.391c0-5.704 3.748-9.57 9-10.609l.996 2.151c-2.433.917-3.996 3.638-3.996 5.849h3.983v10h-9.983z" {}
            }
            div class="text-xl italic mb-4 leading-relaxed" { (children) }
            @if !author.is_empty() || !source.is_empty() {
                footer class="text-sm text-muted-foreground" {
                    @if !author.is_empty() { cite class="font-semibold not-italic text-foreground" { (author) } }
                    @if !source.is_empty() {
                        @if !author.is_empty() { span class="mx-2" { "•" } }
                        cite { (source) }
                    }
                }
            }
        }
    }
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
        assert_eq!(COMPONENTS.len(), 7);
        assert!(
            COMPONENTS
                .iter()
                .all(|component| component.family == "neomorphism"
                    && component.module == "blockquote")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "Blockquote")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "SimpleBlockquote")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "BlockquoteWithIcon")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "BlockquoteWithCite")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "PullQuote")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "NeomorphicTestimonial")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "NeomorphicQuoteCard")
        );
    }

    #[test]
    fn neomorphism_only_components_render_html_without_generated_stub_markers() {
        let html =
            NeomorphicTestimonial("sample", "sample", "sample", html! { "quote" }).into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.starts_with("<div class=\"neomorph-card rounded-2xl p-6\""));
        assert!(html.contains("<div class=\"neomorph-concave rounded-full p-1 flex-shrink-0\">"));
        assert!(html.contains(
            "<img src=\"sample\" alt=\"sample\" class=\"w-12 h-12 rounded-full object-cover\">"
        ));
        assert!(html.contains("quote"));
        let html = NeomorphicQuoteCard("sample", "sample", html! { "quote" }).into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.starts_with("<div class=\"neomorph-raised rounded-2xl p-8 text-center\""));
        assert!(html.contains("<svg class=\"w-12 h-12 mx-auto mb-6 text-primary/20\""));
        assert!(html.contains("<div class=\"text-xl italic mb-4 leading-relaxed\">quote</div>"));
        assert!(html.contains("<footer class=\"text-sm text-muted-foreground\">"));
        assert!(
            html.contains("<cite class=\"font-semibold not-italic text-foreground\">sample</cite>")
        );

        let html = NeomorphicQuoteCard("", "", html! { "quote only" }).into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.contains("quote only"));
        assert!(!html.contains("<footer"));

        let html =
            NeomorphicTestimonial("author", "title", "", html! { "no avatar" }).into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.contains("no avatar"));
        assert!(!html.contains("<img"));
    }

    #[test]
    fn blockquote_helper_adds_variant_specific_neomorph_classes() {
        for (variant, expected) in [
            (shared::VARIANT_PRIMARY, "neomorph-flat shadow-md"),
            ("accent", "neomorph-raised"),
            ("default", "neomorph-concave"),
        ] {
            let props = addNeomorphClasses(
                &BlockquoteProps::default()
                    .with_variant(variant)
                    .with_class("custom"),
            );
            assert!(props.class.contains(expected));
            assert!(props.class.contains("custom"));
        }

        let rendered = Blockquote(
            &BlockquoteProps::default()
                .with_variant("accent")
                .with_class("custom"),
            html! { "Quote" },
        )
        .into_string();
        assert!(rendered.contains("neomorph-raised"));
        assert!(rendered.contains("custom"));
        assert!(rendered.contains("Quote"));
    }
}
