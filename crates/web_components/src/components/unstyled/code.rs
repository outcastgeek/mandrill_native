#![allow(non_snake_case)]

use crate::components::{ComponentInfo, shared};
use maud::{Markup, html};
use std::collections::BTreeMap;

pub const COMPONENTS: &[ComponentInfo] = &[
    ComponentInfo {
        family: "unstyled",
        module: "code",
        name: "Code",
    },
    ComponentInfo {
        family: "unstyled",
        module: "code",
        name: "InlineCode",
    },
    ComponentInfo {
        family: "unstyled",
        module: "code",
        name: "CodeBlock",
    },
    ComponentInfo {
        family: "unstyled",
        module: "code",
        name: "CodeBlockWithFileName",
    },
    ComponentInfo {
        family: "unstyled",
        module: "code",
        name: "CodeSnippet",
    },
];

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CodeProps {
    pub inline: bool,
    pub language: String,
    pub show_copy: bool,
    pub show_lines: bool,
    pub start_line: i32,
    pub highlight_lines: String,
    pub file_name: String,
    pub class: String,
    pub attributes: BTreeMap<String, String>,
    pub size: &'static str,
}

impl Default for CodeProps {
    fn default() -> Self {
        Self {
            inline: true,
            language: String::new(),
            show_copy: false,
            show_lines: false,
            start_line: 1,
            highlight_lines: String::new(),
            file_name: String::new(),
            class: String::new(),
            attributes: BTreeMap::new(),
            size: shared::SIZE_SM,
        }
    }
}

impl CodeProps {
    #[must_use]
    pub const fn with_inline(mut self, inline: bool) -> Self {
        self.inline = inline;
        self
    }

    #[must_use]
    pub fn with_language(mut self, language: impl Into<String>) -> Self {
        self.language = language.into();
        self
    }

    #[must_use]
    pub const fn with_show_copy(mut self, show_copy: bool) -> Self {
        self.show_copy = show_copy;
        self
    }

    #[must_use]
    pub const fn with_show_lines(mut self, show_lines: bool) -> Self {
        self.show_lines = show_lines;
        self
    }

    #[must_use]
    pub const fn with_start_line(mut self, start_line: i32) -> Self {
        self.start_line = start_line;
        self
    }

    #[must_use]
    pub fn with_highlight_lines(mut self, lines: impl Into<String>) -> Self {
        self.highlight_lines = lines.into();
        self
    }

    #[must_use]
    pub fn with_file_name(mut self, file_name: impl Into<String>) -> Self {
        self.file_name = file_name.into();
        self
    }

    #[must_use]
    pub fn with_class(mut self, class: impl Into<String>) -> Self {
        self.class = class.into();
        self
    }

    #[must_use]
    pub const fn with_size(mut self, size: &'static str) -> Self {
        self.size = size;
        self
    }

    #[must_use]
    pub fn with_attribute(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.attributes.insert(name.into(), value.into());
        self
    }

    #[must_use]
    pub fn build_classes(&self) -> String {
        if self.inline {
            self.build_inline_classes()
        } else {
            self.build_block_classes()
        }
    }

    #[must_use]
    pub fn build_inline_classes(&self) -> String {
        let size_class = match self.size {
            shared::SIZE_XS => "text-xs",
            shared::SIZE_SM => "text-sm",
            shared::SIZE_MD => "text-base",
            shared::SIZE_LG => "text-lg",
            _ => "text-sm",
        };

        shared::build_classes(&[
            "font-mono bg-muted px-1.5 py-0.5 rounded",
            size_class,
            self.class.as_str(),
        ])
    }

    #[must_use]
    pub fn build_block_classes(&self) -> String {
        shared::build_classes(&[
            "font-mono text-sm bg-muted rounded-lg overflow-x-auto",
            self.class.as_str(),
        ])
    }

    #[must_use]
    pub fn build_pre_classes(&self) -> &'static str {
        if self.show_lines {
            "overflow-x-auto p-4 line-numbers"
        } else {
            "overflow-x-auto p-4"
        }
    }

    #[must_use]
    pub fn build_attributes(&self) -> BTreeMap<String, String> {
        let mut attrs = self.attributes.clone();

        if !self.inline && !self.language.is_empty() {
            attrs.insert("data-language".to_string(), self.language.clone());
        }

        if !self.highlight_lines.is_empty() {
            attrs.insert(
                "data-highlight-lines".to_string(),
                self.highlight_lines.clone(),
            );
        }

        if self.start_line > 1 {
            attrs.insert("data-start".to_string(), self.start_line.to_string());
        }

        attrs
    }

    #[must_use]
    pub fn get_language_class(&self) -> String {
        if self.language.is_empty() {
            String::new()
        } else {
            format!("language-{}", self.language)
        }
    }
}

fn copy_button(class: &'static str) -> Markup {
    html! {
        button
            class=(class)
            data-copy-target="code-content"
            aria-label=(shared::default_strings().code_copy_aria_label)
        {
            svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24" {
                path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M8 16H6a2 2 0 01-2-2V6a2 2 0 012-2h8a2 2 0 012 2v2m-6 12h8a2 2 0 002-2v-8a2 2 0 00-2-2h-8a2 2 0 00-2 2v8a2 2 0 002 2z" {}
            }
        }
    }
}

#[must_use]
pub fn Code(props: &CodeProps, children: Markup) -> Markup {
    let attrs = props.build_attributes();
    if props.inline {
        html! {
            code
                class=(props.build_classes())
                id=[shared::attr_value(&attrs, "id")]
                title=[shared::attr_value(&attrs, "title")]
                role=[shared::attr_value(&attrs, "role")]
                aria-label=[shared::attr_value(&attrs, "aria-label")]
                data-testid=[shared::attr_value(&attrs, "data-testid")]
                data-highlight-lines=[shared::attr_value(&attrs, "data-highlight-lines")]
                data-start=[shared::attr_value(&attrs, "data-start")]
            {
                (children)
            }
        }
    } else {
        let language_class = props.get_language_class();
        html! {
            div class="code-block-wrapper relative" {
                @if !props.file_name.is_empty() {
                    div class="code-block-header flex items-center justify-between px-4 py-2 bg-muted/50 border-b border-border rounded-t-lg" {
                        span class="text-xs font-medium text-muted-foreground" { (&props.file_name) }
                        @if props.show_copy {
                            (copy_button("copy-button text-xs px-2 py-1 rounded hover:bg-muted transition-colors duration-150"))
                        }
                    }
                } @else if props.show_copy {
                    (copy_button("copy-button absolute top-2 right-2 text-xs px-2 py-1 rounded bg-muted hover:bg-muted/80 transition-colors duration-150 z-10"))
                }
                pre class=(props.build_pre_classes()) {
                    code
                        id="code-content"
                        class=[if language_class.is_empty() { None } else { Some(language_class.as_str()) }]
                        data-language=[shared::attr_value(&attrs, "data-language")]
                        data-highlight-lines=[shared::attr_value(&attrs, "data-highlight-lines")]
                        data-start=[shared::attr_value(&attrs, "data-start")]
                        title=[shared::attr_value(&attrs, "title")]
                        aria-label=[shared::attr_value(&attrs, "aria-label")]
                    {
                        (children)
                    }
                }
            }
        }
    }
}

#[must_use]
pub fn InlineCode(
    language: impl Into<String>,
    class: impl Into<String>,
    children: Markup,
) -> Markup {
    Code(
        &CodeProps::default()
            .with_inline(true)
            .with_language(language)
            .with_class(class),
        children,
    )
}

#[must_use]
pub fn CodeBlock(
    language: impl Into<String>,
    show_copy: bool,
    show_lines: bool,
    class: impl Into<String>,
    children: Markup,
) -> Markup {
    Code(
        &CodeProps::default()
            .with_inline(false)
            .with_language(language)
            .with_show_copy(show_copy)
            .with_show_lines(show_lines)
            .with_class(class),
        children,
    )
}

#[must_use]
pub fn CodeBlockWithFileName(
    language: impl Into<String>,
    file_name: impl Into<String>,
    show_copy: bool,
    children: Markup,
) -> Markup {
    Code(
        &CodeProps::default()
            .with_inline(false)
            .with_language(language)
            .with_file_name(file_name)
            .with_show_copy(show_copy),
        children,
    )
}

#[must_use]
pub fn CodeSnippet(language: impl Into<String>, children: Markup) -> Markup {
    CodeBlock(language, true, false, "", children)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_props_match_go_defaults() {
        let props = CodeProps::default();
        assert!(props.inline);
        assert_eq!(props.size, shared::SIZE_SM);
        assert!(!props.show_copy);
        assert!(!props.show_lines);
        assert_eq!(props.start_line, 1);
    }

    #[test]
    fn helper_methods_cover_inline_block_classes_attributes_and_language() {
        for (size, expected) in [
            (shared::SIZE_XS, "text-xs"),
            (shared::SIZE_SM, "text-sm"),
            (shared::SIZE_MD, "text-base"),
            (shared::SIZE_LG, "text-lg"),
            (shared::SIZE_XL, "text-sm"),
        ] {
            assert!(
                CodeProps::default()
                    .with_size(size)
                    .build_inline_classes()
                    .contains(expected)
            );
        }

        let block = CodeProps::default()
            .with_inline(false)
            .with_language("rust")
            .with_show_lines(true)
            .with_start_line(7)
            .with_highlight_lines("1,3-5")
            .with_class("extra")
            .with_attribute("title", "Snippet");

        assert_eq!(
            block.build_block_classes(),
            "font-mono text-sm bg-muted rounded-lg overflow-x-auto extra"
        );
        assert_eq!(
            block.build_classes(),
            "font-mono text-sm bg-muted rounded-lg overflow-x-auto extra"
        );
        assert_eq!(
            block.build_pre_classes(),
            "overflow-x-auto p-4 line-numbers"
        );
        assert_eq!(block.get_language_class(), "language-rust");
        assert_eq!(CodeProps::default().get_language_class(), "");
        let attrs = block.build_attributes();
        assert_eq!(attrs.get("data-language").map(String::as_str), Some("rust"));
        assert_eq!(attrs.get("data-start").map(String::as_str), Some("7"));
        assert_eq!(
            attrs.get("data-highlight-lines").map(String::as_str),
            Some("1,3-5")
        );
        assert_eq!(attrs.get("title").map(String::as_str), Some("Snippet"));
    }

    #[test]
    fn inline_code_renders_actual_html_and_escapes_children_and_attrs() {
        let html = Code(
            &CodeProps::default()
                .with_size(shared::SIZE_MD)
                .with_class("custom")
                .with_attribute("title", "<unsafe>")
                .with_attribute("data-testid", "inline-code"),
            html! { "<tag>" },
        )
        .into_string();

        assert!(html.starts_with("<code "));
        assert!(html.contains("font-mono bg-muted px-1.5 py-0.5 rounded text-base custom"));
        assert!(html.contains("title=\"&lt;unsafe&gt;\""));
        assert!(html.contains("data-testid=\"inline-code\""));
        assert!(html.contains("&lt;tag&gt;"));
    }

    #[test]
    fn block_code_renders_copy_button_file_header_lines_and_data_attributes() {
        let html = Code(
            &CodeProps::default()
                .with_inline(false)
                .with_language("go")
                .with_file_name("main.go")
                .with_show_copy(true)
                .with_show_lines(true)
                .with_start_line(3)
                .with_highlight_lines("3-4"),
            html! { "fmt.Println(\"hi\")" },
        )
        .into_string();

        assert!(html.contains("<div class=\"code-block-wrapper relative\">"));
        assert!(html.contains("main.go"));
        assert!(html.contains("aria-label=\"Copy code\""));
        assert!(html.contains("data-copy-target=\"code-content\""));
        assert!(html.contains("<pre class=\"overflow-x-auto p-4 line-numbers\">"));
        assert!(html.contains("id=\"code-content\""));
        assert!(html.contains("class=\"language-go\""));
        assert!(html.contains("data-language=\"go\""));
        assert!(html.contains("data-start=\"3\""));
        assert!(html.contains("data-highlight-lines=\"3-4\""));
        assert!(html.contains("fmt.Println(&quot;hi&quot;)"));
    }

    #[test]
    fn block_code_without_file_uses_absolute_copy_button_and_convenience_wrappers_work() {
        let html = CodeBlock("rs", true, false, "custom", html! { "let x = 1;" }).into_string();
        assert!(html.contains("absolute top-2 right-2"));
        assert!(html.contains("class=\"language-rs\""));
        assert!(html.contains("let x = 1;"));

        assert!(
            InlineCode("", "tiny", html! { "x" })
                .into_string()
                .contains("tiny")
        );
        assert!(
            CodeBlockWithFileName("ts", "app.ts", false, html! { "const x = 1;" })
                .into_string()
                .contains("app.ts")
        );
        assert!(
            CodeSnippet("html", html! { "<div>" })
                .into_string()
                .contains("aria-label=\"Copy code\"")
        );
    }
}
