#![allow(non_snake_case)]
#![allow(clippy::needless_pass_by_value)]

use crate::components::ComponentInfo;
use crate::components::shared;
use crate::components::unstyled::code as base;
use maud::{Markup, html};

pub use base::{CodeBlock, CodeBlockWithFileName, CodeProps, CodeSnippet, InlineCode};

pub const COMPONENTS: &[ComponentInfo] = &[
    ComponentInfo {
        family: "neomorphism",
        module: "code",
        name: "Code",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "code",
        name: "InlineCode",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "code",
        name: "CodeBlock",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "code",
        name: "CodeBlockWithFileName",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "code",
        name: "CodeSnippet",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "code",
        name: "NeomorphicCodeBlock",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "code",
        name: "TerminalCodeBlock",
    },
];

#[must_use]
fn addNeomorphClasses(props: &CodeProps) -> CodeProps {
    let mut props = props.clone();
    let neomorph_class = if props.inline {
        "neomorph-concave-sm"
    } else {
        "neomorph-concave shadow-inner"
    };
    props.class = shared::build_classes(&[neomorph_class, props.class.as_str()]);
    props
}

#[must_use]
pub fn Code(props: &CodeProps, children: Markup) -> Markup {
    let neo_props = addNeomorphClasses(props);
    base::Code(&neo_props, children)
}

#[must_use]
pub fn NeomorphicCodeBlock(
    language: &str,
    fileName: &str,
    showCopy: bool,
    children: Markup,
) -> Markup {
    html! {
        div class="neomorph-card rounded-xl overflow-hidden" {
            @if !fileName.is_empty() {
                div class="neomorph-flat px-4 py-3 border-b border-border/50" {
                    div class="flex items-center justify-between" {
                        div class="flex items-center gap-2" {
                            div class="flex gap-1.5" {
                                div class="w-3 h-3 rounded-full bg-red-500/70" {}
                                div class="w-3 h-3 rounded-full bg-yellow-500/70" {}
                                div class="w-3 h-3 rounded-full bg-green-500/70" {}
                            }
                            span class="text-xs font-medium text-muted-foreground ml-2" { (fileName) }
                        }
                        @if showCopy {
                            button class="neomorph-button-sm text-xs px-3 py-1.5 rounded-md transition-all duration-150" data-copy-target="code-content" aria-label="Copy code" { "Copy" }
                        }
                    }
                }
            }
            (CodeBlock(language, showCopy && fileName.is_empty(), false, "", children))
        }
    }
}

#[must_use]
pub fn TerminalCodeBlock(showCopy: bool, children: Markup) -> Markup {
    html! {
        div class="neomorph-card rounded-xl overflow-hidden bg-gray-900" {
            div class="neomorph-flat px-4 py-3 bg-gray-800 border-b border-gray-700" {
                div class="flex items-center justify-between" {
                    div class="flex gap-1.5" {
                        div class="w-3 h-3 rounded-full bg-red-500" {}
                        div class="w-3 h-3 rounded-full bg-yellow-500" {}
                        div class="w-3 h-3 rounded-full bg-green-500" {}
                    }
                    @if showCopy { button class="text-xs px-3 py-1.5 rounded-md bg-muted hover:bg-muted/80 text-muted-foreground transition-all duration-150" data-copy-target="code-content" aria-label="Copy code" { "Copy" } }
                }
            }
            pre class="p-4 overflow-x-auto bg-gray-900" { code id="code-content" class="font-mono text-sm text-green-400" { (children) } }
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
                .all(|component| component.family == "neomorphism" && component.module == "code")
        );
        assert!(COMPONENTS.iter().any(|component| component.name == "Code"));
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "InlineCode")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "CodeBlock")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "CodeBlockWithFileName")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "CodeSnippet")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "NeomorphicCodeBlock")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "TerminalCodeBlock")
        );
    }

    #[test]
    fn neomorphism_only_components_render_html_without_generated_stub_markers() {
        let html = NeomorphicCodeBlock("rust", "sample.rs", true, html! { "code" }).into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.starts_with("<div class=\"neomorph-card rounded-xl overflow-hidden\""));
        assert!(html.contains("<div class=\"neomorph-flat px-4 py-3 border-b border-border/50\">"));
        assert!(html.contains(">sample.rs</span>"));
        assert!(html.contains("data-copy-target=\"code-content\""));
        assert!(html.contains("class=\"language-rust\""));
        assert!(html.contains(">code</code>"));
        let html = TerminalCodeBlock(true, html! { "cmd" }).into_string();
        assert_no_placeholder_markers(&html);
        assert!(
            html.starts_with("<div class=\"neomorph-card rounded-xl overflow-hidden bg-gray-900\"")
        );
        assert!(html.contains(
            "<div class=\"neomorph-flat px-4 py-3 bg-gray-800 border-b border-gray-700\">"
        ));
        assert!(html.contains("data-copy-target=\"code-content\""));
        assert!(html.contains(
            "<code id=\"code-content\" class=\"font-mono text-sm text-green-400\">cmd</code>"
        ));
    }

    #[test]
    fn code_helper_adds_inline_and_block_neomorph_classes() {
        let inline = addNeomorphClasses(&CodeProps::default().with_class("custom"));
        assert_eq!(inline.class, "neomorph-concave-sm custom");

        let block =
            addNeomorphClasses(&CodeProps::default().with_inline(false).with_class("custom"));
        assert_eq!(block.class, "neomorph-concave shadow-inner custom");

        let rendered = Code(
            &CodeProps::default().with_class("custom"),
            html! { "let x = 1;" },
        )
        .into_string();
        assert!(rendered.contains("neomorph-concave-sm"));
        assert!(rendered.contains("custom"));
        assert!(rendered.contains("let x = 1;"));
    }
}
