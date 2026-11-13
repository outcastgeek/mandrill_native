#![allow(non_snake_case)]
#![allow(clippy::needless_pass_by_value)]

use crate::components::ComponentInfo;
use crate::components::shared;
use maud::{Markup, html};
use std::collections::BTreeMap;

pub const COMPONENTS: &[ComponentInfo] = &[
    ComponentInfo {
        family: "neomorphism",
        module: "componentpreview",
        name: "ComponentPreview",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "componentpreview",
        name: "SimpleComponentPreview",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "componentpreview",
        name: "ComponentPreviewWithTitle",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "componentpreview",
        name: "ComponentPreviewWithDescription",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "componentpreview",
        name: "previewIcon",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "componentpreview",
        name: "codeIcon",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "componentpreview",
        name: "copyIcon",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "componentpreview",
        name: "checkIcon",
    },
];

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ComponentPreviewProps {
    pub title: String,
    pub description: String,
    pub code: String,
    pub language: String,
    pub file_name: String,
    pub default_tab: String,
    pub show_copy: bool,
    pub show_line_numbers: bool,
    pub preview_min_height: String,
    pub preview_padding: String,
    pub preview_background: String,
    pub preview_centered: bool,
    pub class: String,
    pub attributes: BTreeMap<String, String>,
}

impl Default for ComponentPreviewProps {
    fn default() -> Self {
        Self {
            title: String::new(),
            description: String::new(),
            code: String::new(),
            language: "go".to_string(),
            file_name: String::new(),
            default_tab: "preview".to_string(),
            show_copy: true,
            show_line_numbers: false,
            preview_min_height: "100px".to_string(),
            preview_padding: "p-6".to_string(),
            preview_background: "bg-muted/30".to_string(),
            preview_centered: true,
            class: String::new(),
            attributes: BTreeMap::new(),
        }
    }
}

impl ComponentPreviewProps {
    #[must_use]
    pub fn with_title(mut self, value: impl Into<String>) -> Self {
        self.title = value.into();
        self
    }
    #[must_use]
    pub fn with_description(mut self, value: impl Into<String>) -> Self {
        self.description = value.into();
        self
    }
    #[must_use]
    pub fn with_code(mut self, value: impl Into<String>) -> Self {
        self.code = value.into();
        self
    }
    #[must_use]
    pub fn with_language(mut self, value: impl Into<String>) -> Self {
        self.language = value.into();
        self
    }
    #[must_use]
    pub fn with_file_name(mut self, value: impl Into<String>) -> Self {
        self.file_name = value.into();
        self
    }
    #[must_use]
    pub fn with_default_tab(mut self, value: impl Into<String>) -> Self {
        self.default_tab = value.into();
        self
    }
    #[must_use]
    pub const fn with_show_copy(mut self, value: bool) -> Self {
        self.show_copy = value;
        self
    }
    #[must_use]
    pub const fn with_show_line_numbers(mut self, value: bool) -> Self {
        self.show_line_numbers = value;
        self
    }
    #[must_use]
    pub fn with_preview_min_height(mut self, value: impl Into<String>) -> Self {
        self.preview_min_height = value.into();
        self
    }
    #[must_use]
    pub fn with_preview_padding(mut self, value: impl Into<String>) -> Self {
        self.preview_padding = value.into();
        self
    }
    #[must_use]
    pub fn with_preview_background(mut self, value: impl Into<String>) -> Self {
        self.preview_background = value.into();
        self
    }
    #[must_use]
    pub const fn with_preview_centered(mut self, value: bool) -> Self {
        self.preview_centered = value;
        self
    }
    #[must_use]
    pub fn with_class(mut self, value: impl Into<String>) -> Self {
        self.class = value.into();
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
            "component-preview neomorph-card rounded-xl overflow-hidden",
            self.class.as_str(),
        ])
    }
    #[must_use]
    pub fn build_preview_classes(&self) -> String {
        shared::build_classes(&[
            self.preview_padding.as_str(),
            self.preview_background.as_str(),
            if self.preview_centered {
                "flex items-center justify-center"
            } else {
                ""
            },
        ])
    }
    #[must_use]
    pub fn get_preview_min_height_style(&self) -> &str {
        if self.preview_min_height.is_empty() {
            "100px"
        } else {
            self.preview_min_height.as_str()
        }
    }
    #[must_use]
    pub fn get_default_tab(&self) -> &str {
        if self.default_tab.is_empty() {
            "preview"
        } else {
            self.default_tab.as_str()
        }
    }
    #[must_use]
    pub fn get_language(&self) -> &str {
        if self.language.is_empty() {
            "go"
        } else {
            self.language.as_str()
        }
    }
    #[must_use]
    pub fn get_code_block_id(&self) -> String {
        if self.title.is_empty() {
            "code-preview".to_string()
        } else {
            format!("code-{}", SanitizeID(&self.title))
        }
    }
}

#[must_use]
pub fn SanitizeID(value: &str) -> String {
    value
        .replace(' ', "-")
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
        .collect::<String>()
        .to_ascii_lowercase()
}

#[must_use]
pub fn ComponentPreview(props: &ComponentPreviewProps, children: Markup) -> Markup {
    let code_block_id = props.get_code_block_id();
    let code_class = format!(
        "language-{} font-mono text-foreground/90",
        props.get_language()
    );
    let min_height = format!("min-height: {}", props.get_preview_min_height_style());
    html! {
        div class=(props.build_classes()) data-component="component-preview" id=[shared::attr_value(&props.attributes, "id")] data-testid=[shared::attr_value(&props.attributes, "data-testid")] {
            @if !props.title.is_empty() || !props.description.is_empty() {
                div class="px-4 py-3 border-b border-border/50" {
                    @if !props.title.is_empty() { h3 class="font-semibold text-foreground" { (&props.title) } }
                    @if !props.description.is_empty() { p class="text-sm text-muted-foreground mt-1" { (&props.description) } }
                }
            }
            div class="w-full" data-signals=(format!("{{ previewTab: '{}' }}", props.get_default_tab())) {
                div class="flex items-center justify-between px-4 py-2 border-b border-border/50 bg-muted/30" {
                    div role="tablist" aria-label="Component preview tabs" class="bg-transparent p-0 gap-1 flex" {
                        button type="button" role="tab" class="px-3 py-1.5 text-sm rounded-md data-[state=active]:bg-background data-[state=active]:shadow-sm" data-on:click="$previewTab = 'preview'" data-class:active="$previewTab === 'preview'" { span class="flex items-center gap-1.5" { (previewIcon()) "Preview" } }
                        button type="button" role="tab" class="px-3 py-1.5 text-sm rounded-md data-[state=active]:bg-background data-[state=active]:shadow-sm" data-on:click="$previewTab = 'code'" data-class:active="$previewTab === 'code'" { span class="flex items-center gap-1.5" { (codeIcon()) "Code" } }
                    }
                    @if props.show_copy { button class="copy-code-btn text-xs px-3 py-1.5 rounded-md bg-muted hover:bg-muted/80 transition-colors duration-150 flex items-center gap-1.5" data-copy-target=(code_block_id.as_str()) aria-label="Copy code" { (copyIcon()) span class="copy-text" { "Copy" } } }
                }
                div class="mt-0" data-show="$previewTab === 'preview'" { div class=(props.build_preview_classes()) style=(min_height) { (children) } }
                div class="mt-0" data-show="$previewTab === 'code'" { div class="relative" {
                    @if !props.file_name.is_empty() { div class="px-4 py-2 bg-muted/50 border-b border-border/30 flex items-center gap-2" { div class="flex gap-1.5" { div class="w-3 h-3 rounded-full bg-red-500/70" {} div class="w-3 h-3 rounded-full bg-yellow-500/70" {} div class="w-3 h-3 rounded-full bg-green-500/70" {} } span class="text-xs font-medium text-muted-foreground ml-2" { (&props.file_name) } } }
                    pre class="p-4 overflow-x-auto bg-muted/50 text-sm max-h-[400px]" { code id=(code_block_id) class=(code_class) { (&props.code) } }
                } }
            }
        }
    }
}

#[must_use]
pub fn SimpleComponentPreview(code: &str, children: Markup) -> Markup {
    ComponentPreview(
        &ComponentPreviewProps::default()
            .with_code(code)
            .with_show_copy(true),
        children,
    )
}

#[must_use]
pub fn ComponentPreviewWithTitle(title: &str, code: &str, children: Markup) -> Markup {
    ComponentPreview(
        &ComponentPreviewProps::default()
            .with_title(title)
            .with_code(code)
            .with_show_copy(true),
        children,
    )
}

#[must_use]
pub fn ComponentPreviewWithDescription(
    title: &str,
    description: &str,
    code: &str,
    children: Markup,
) -> Markup {
    ComponentPreview(
        &ComponentPreviewProps::default()
            .with_title(title)
            .with_description(description)
            .with_code(code)
            .with_show_copy(true),
        children,
    )
}

#[must_use]
pub fn previewIcon() -> Markup {
    html! { svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24" { path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M15 12a3 3 0 11-6 0 3 3 0 016 0z" {} path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M2.458 12C3.732 7.943 7.523 5 12 5c4.478 0 8.268 2.943 9.542 7-1.274 4.057-5.064 7-9.542 7-4.477 0-8.268-2.943-9.542-7z" {} } }
}

#[must_use]
pub fn codeIcon() -> Markup {
    html! { svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24" { path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M10 20l4-16m4 4l4 4-4 4M6 16l-4-4 4-4" {} } }
}

#[must_use]
pub fn copyIcon() -> Markup {
    html! { svg class="w-4 h-4 copy-icon" fill="none" stroke="currentColor" viewBox="0 0 24 24" { path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M8 16H6a2 2 0 01-2-2V6a2 2 0 012-2h8a2 2 0 012 2v2m-6 12h8a2 2 0 002-2v-8a2 2 0 00-2-2h-8a2 2 0 00-2 2v8a2 2 0 002 2z" {} } }
}

#[must_use]
pub fn checkIcon() -> Markup {
    html! { svg class="w-4 h-4 check-icon hidden" fill="none" stroke="currentColor" viewBox="0 0 24 24" { path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M5 13l4 4L19 7" {} } }
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
    }

    #[test]
    fn registry_matches_neomorphism_source_components() {
        assert_eq!(COMPONENTS.len(), 8);
        assert!(
            COMPONENTS
                .iter()
                .all(|component| component.family == "neomorphism"
                    && component.module == "componentpreview")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "ComponentPreview")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "SimpleComponentPreview")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "ComponentPreviewWithTitle")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "ComponentPreviewWithDescription")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "previewIcon")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "codeIcon")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "copyIcon")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "checkIcon")
        );
    }

    #[test]
    fn props_helpers_cover_builders_fallbacks_and_conditional_html() {
        let props = ComponentPreviewProps::default()
            .with_title("Fancy Thing!")
            .with_description("Detailed preview")
            .with_code("let answer = 42;")
            .with_language("rust")
            .with_file_name("demo.rs")
            .with_default_tab("code")
            .with_show_copy(false)
            .with_show_line_numbers(true)
            .with_preview_min_height("240px")
            .with_preview_padding("p-2")
            .with_preview_background("bg-card")
            .with_preview_centered(false)
            .with_class("outer")
            .with_attribute("id", "preview-id")
            .with_attribute("data-testid", "preview-test");
        assert_eq!(
            props.build_classes(),
            "component-preview neomorph-card rounded-xl overflow-hidden outer"
        );
        assert_eq!(props.build_preview_classes(), "p-2 bg-card");
        assert_eq!(props.get_preview_min_height_style(), "240px");
        assert_eq!(props.get_default_tab(), "code");
        assert_eq!(props.get_language(), "rust");
        assert_eq!(props.get_code_block_id(), "code-fancy-thing");
        assert!(props.show_line_numbers);

        let html =
            ComponentPreview(&props, html! { button type="button" { "Demo" } }).into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.contains("id=\"preview-id\""));
        assert!(html.contains("data-testid=\"preview-test\""));
        assert!(html.contains("data-signals=\"{ previewTab: 'code' }\""));
        assert!(html.contains("style=\"min-height: 240px\""));
        assert!(html.contains("<div class=\"p-2 bg-card\""));
        assert!(html.contains("demo.rs"));
        assert!(html.contains("language-rust font-mono text-foreground/90"));
        assert!(html.contains("let answer = 42;"));
        assert!(!html.contains("copy-code-btn"));
        assert!(!html.contains("flex items-center justify-center\"><button"));

        let fallback = ComponentPreviewProps::default()
            .with_description("Only description")
            .with_language("")
            .with_default_tab("")
            .with_preview_min_height("")
            .with_preview_centered(false)
            .with_show_copy(false);
        assert_eq!(fallback.get_preview_min_height_style(), "100px");
        assert_eq!(fallback.get_default_tab(), "preview");
        assert_eq!(fallback.get_language(), "go");
        assert_eq!(fallback.get_code_block_id(), "code-preview");
        assert_eq!(fallback.build_preview_classes(), "p-6 bg-muted/30");

        let html = ComponentPreview(&fallback, html! { "fallback preview" }).into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.contains("Only description"));
        assert!(!html.contains("<h3"));
        assert!(html.contains("data-signals=\"{ previewTab: 'preview' }\""));
        assert!(html.contains("style=\"min-height: 100px\""));
        assert!(html.contains("<div class=\"p-6 bg-muted/30\""));
        assert!(!html.contains("copy-code-btn"));
        assert!(html.contains("language-go font-mono text-foreground/90"));
    }

    #[test]
    fn neomorphism_only_components_render_html_without_generated_stub_markers() {
        let html = ComponentPreview(
            &ComponentPreviewProps::default()
                .with_title("Title")
                .with_description("Desc")
                .with_file_name("demo.rs")
                .with_code("code"),
            html! { "preview" },
        )
        .into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.contains("component-preview neomorph-card rounded-xl overflow-hidden"));
        assert!(html.contains("data-component=\"component-preview\""));
        assert!(html.contains("<h3 class=\"font-semibold text-foreground\">Title</h3>"));
        assert!(html.contains("data-on:click=\"$previewTab = 'preview'\""));
        assert!(html.contains("id=\"code-title\""));
        assert!(html.contains("preview"));

        let html = SimpleComponentPreview("code", html! { "preview" }).into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.contains("id=\"code-preview\""));
        assert!(html.contains("code"));
        let html = ComponentPreviewWithTitle("Title", "code", html! { "preview" }).into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.contains("<h3 class=\"font-semibold text-foreground\">Title</h3>"));
        let html = ComponentPreviewWithDescription("Title", "Desc", "code", html! { "preview" })
            .into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.contains("Desc"));
        let html = previewIcon().into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.contains("<svg class=\"w-4 h-4\""));
        assert!(html.contains("M15 12a3 3"));
        let html = codeIcon().into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.contains("M10 20l4-16"));
        let html = copyIcon().into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.contains("copy-icon"));
        let html = checkIcon().into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.contains("check-icon hidden"));
        assert_eq!(SanitizeID("Demo Component!"), "demo-component");
    }
}
