#![allow(non_snake_case)]

use crate::components::{ComponentInfo, shared};
use maud::{Markup, html};
use std::collections::BTreeMap;

pub const COMPONENTS: &[ComponentInfo] = &[
    ComponentInfo {
        family: "unstyled",
        module: "uploadarea",
        name: "UploadArea",
    },
    ComponentInfo {
        family: "unstyled",
        module: "uploadarea",
        name: "SimpleUploadArea",
    },
    ComponentInfo {
        family: "unstyled",
        module: "uploadarea",
        name: "ImageUploadArea",
    },
    ComponentInfo {
        family: "unstyled",
        module: "uploadarea",
        name: "DocumentUploadArea",
    },
];

#[derive(Clone, Debug, Default)]
pub struct UploadAreaProps {
    pub id: String,
    pub name: String,
    pub accept: String,
    pub multiple: bool,
    pub max_size: String,
    pub title: String,
    pub title_content: Option<Markup>,
    pub description: String,
    pub icon: String,
    pub icon_content: Option<Markup>,
    pub disabled: bool,
    pub is_drag_active: bool,
    pub hide_description: bool,
    pub hide_hints: bool,
    pub class: String,
    pub attributes: BTreeMap<String, String>,
}

impl UploadAreaProps {
    #[must_use]
    pub fn new() -> Self {
        Self {
            icon_content: Some(upload_icon("folder", "text-muted-foreground")),
            ..Self::default()
        }
    }
    #[must_use]
    pub fn with_id(mut self, value: impl Into<String>) -> Self {
        self.id = value.into();
        self
    }
    #[must_use]
    pub fn with_name(mut self, value: impl Into<String>) -> Self {
        self.name = value.into();
        self
    }
    #[must_use]
    pub fn with_accept(mut self, value: impl Into<String>) -> Self {
        self.accept = value.into();
        self
    }
    #[must_use]
    pub const fn with_multiple(mut self, value: bool) -> Self {
        self.multiple = value;
        self
    }
    #[must_use]
    pub fn with_max_size(mut self, value: impl Into<String>) -> Self {
        self.max_size = value.into();
        self
    }
    #[must_use]
    pub fn with_title(mut self, value: impl Into<String>) -> Self {
        self.title = value.into();
        self
    }
    #[must_use]
    pub fn with_title_content(mut self, value: Markup) -> Self {
        self.title_content = Some(value);
        self
    }
    #[must_use]
    pub fn with_description(mut self, value: impl Into<String>) -> Self {
        self.description = value.into();
        self
    }
    #[must_use]
    pub fn with_icon(mut self, value: impl Into<String>) -> Self {
        self.icon = value.into();
        self.icon_content = None;
        self
    }
    #[must_use]
    pub fn with_icon_content(mut self, value: Markup) -> Self {
        self.icon_content = Some(value);
        self.icon.clear();
        self
    }
    #[must_use]
    pub const fn with_disabled(mut self, value: bool) -> Self {
        self.disabled = value;
        self
    }
    #[must_use]
    pub const fn with_drag_active(mut self, value: bool) -> Self {
        self.is_drag_active = value;
        self
    }
    #[must_use]
    pub const fn with_hide_description(mut self, value: bool) -> Self {
        self.hide_description = value;
        self
    }
    #[must_use]
    pub const fn with_hide_hints(mut self, value: bool) -> Self {
        self.hide_hints = value;
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
    pub fn resolve_icon(&self) -> Option<Markup> {
        if let Some(content) = self.icon_content.clone() {
            Some(content)
        } else {
            icon_from_symbol(self.icon.trim())
        }
    }

    #[must_use]
    pub fn build_classes(&self) -> String {
        let state = if self.is_drag_active {
            "border-primary bg-primary/5"
        } else if self.disabled {
            "border-muted bg-muted/50 cursor-not-allowed opacity-60"
        } else {
            "border-border hover:border-primary/50 hover:bg-muted/30"
        };
        shared::build_classes(&[
            "flex flex-col items-center justify-center p-8 border-2 border-dashed rounded-xl transition-colors duration-150 cursor-pointer",
            state,
            self.class.as_str(),
        ])
    }

    #[must_use]
    pub fn build_attributes(&self) -> BTreeMap<String, String> {
        self.attributes.clone()
    }
}

fn optional(value: &str) -> Option<&str> {
    if value.is_empty() { None } else { Some(value) }
}

#[must_use]
pub fn upload_icon(name: &str, class: &str) -> Markup {
    html! {
        svg class=(shared::build_classes(&["h-6 w-6", class])) data-icon=(name) xmlns="http://www.w3.org/2000/svg" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="1.8" {
            path stroke-linecap="round" stroke-linejoin="round" d="M12 16V4m0 0 4 4m-4-4-4 4M4 16.5V19a1 1 0 0 0 1 1h14a1 1 0 0 0 1-1v-2.5" {}
        }
    }
}

#[must_use]
pub fn icon_from_symbol(symbol: &str) -> Option<Markup> {
    match symbol {
        "" => None,
        "folder" | "Folder" => Some(upload_icon("folder", "text-muted-foreground")),
        "image" | "Image" => Some(upload_icon("image", "text-muted-foreground")),
        "file" | "document" | "FileText" => Some(upload_icon("file-text", "text-muted-foreground")),
        _ => None,
    }
}

#[must_use]
pub fn resolveUploadTitle(props: &UploadAreaProps) -> String {
    let title = props.title.trim();
    if title.is_empty() {
        "Upload files".to_string()
    } else {
        title.to_string()
    }
}

#[must_use]
pub fn resolveUploadDescription(props: &UploadAreaProps) -> String {
    let description = props.description.trim();
    if description.is_empty() {
        "Drag and drop files here, or click to browse.".to_string()
    } else {
        description.to_string()
    }
}

#[must_use]
pub fn UploadArea(props: &UploadAreaProps, children: Markup) -> Markup {
    let attrs = props.build_attributes();
    html! {
        label
            class=(props.build_classes())
            for=[optional(&props.id)]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
            data-on:dragover=[shared::attr_value(&attrs, "data-on:dragover")]
            data-on:drop=[shared::attr_value(&attrs, "data-on:drop")]
        {
            input
                type="file"
                class="sr-only"
                id=[optional(&props.id)]
                name=[optional(&props.name)]
                accept=[optional(&props.accept)]
                multiple[props.multiple]
                disabled[props.disabled]
            {}
            div class="flex flex-col items-center gap-3 text-center sm:gap-4" {
                @if let Some(icon) = props.resolve_icon() {
                    div class="flex h-10 w-10 items-center justify-center rounded-full bg-muted/50 sm:h-12 sm:w-12" {
                        (icon)
                    }
                } @else if !props.icon.is_empty() {
                    div class="flex h-10 w-10 items-center justify-center rounded-full bg-muted/50 sm:h-12 sm:w-12" {
                        span class="text-xl sm:text-2xl" { (&props.icon) }
                    }
                }
                div class="max-w-full space-y-1" {
                    @if let Some(title) = props.title_content.clone() {
                        (title)
                    } @else {
                        @let title = resolveUploadTitle(props);
                        @if !title.is_empty() {
                            p class="text-sm font-medium sm:text-base" { (title) }
                        }
                    }
                    @if !props.hide_description {
                        @let description = resolveUploadDescription(props);
                        @if !description.is_empty() {
                            p class="text-xs leading-relaxed text-muted-foreground sm:text-sm" { (description) }
                        }
                    }
                }
                @if !props.hide_hints {
                    div class="flex max-w-full flex-wrap justify-center gap-2 text-[11px] text-muted-foreground sm:text-xs" {
                        @if !props.accept.is_empty() {
                            span class="max-w-full break-all rounded bg-muted px-2 py-1 leading-relaxed whitespace-normal" { (&props.accept) }
                        }
                        @if !props.max_size.is_empty() {
                            span class="rounded bg-muted px-2 py-1 leading-relaxed whitespace-nowrap" { "Max " (&props.max_size) }
                        }
                    }
                }
            }
            (children)
        }
    }
}

#[must_use]
pub fn SimpleUploadArea(name: &str, accept: &str, multiple: bool, class: &str) -> Markup {
    UploadArea(
        &UploadAreaProps::new()
            .with_name(name)
            .with_accept(accept)
            .with_multiple(multiple)
            .with_class(class),
        html! {},
    )
}

#[must_use]
pub fn ImageUploadArea(name: &str, multiple: bool, class: &str) -> Markup {
    UploadArea(
        &UploadAreaProps::new()
            .with_name(name)
            .with_accept("image/*")
            .with_multiple(multiple)
            .with_icon_content(upload_icon("image", "text-muted-foreground"))
            .with_title("Upload images")
            .with_description("Drag images here, or click to browse.")
            .with_max_size("10MB")
            .with_class(class),
        html! {},
    )
}

#[must_use]
pub fn DocumentUploadArea(name: &str, multiple: bool, class: &str) -> Markup {
    UploadArea(
        &UploadAreaProps::new()
            .with_name(name)
            .with_accept(
                ".pdf,.docx,.doc,.pptx,.xlsx,.html,.htm,.txt,.md,.markdown,.text,.csv,.rtf,.epub",
            )
            .with_multiple(multiple)
            .with_icon_content(upload_icon("file-text", "text-muted-foreground"))
            .with_title("Upload documents")
            .with_description("Drag documents here, or click to browse.")
            .with_max_size("200MB")
            .with_class(class),
        html! {},
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upload_helpers_cover_defaults_icons_classes_attrs_titles_and_description() {
        let props = UploadAreaProps::new();
        assert!(props.resolve_icon().is_some());
        assert!(
            props
                .build_classes()
                .contains("border-border hover:border-primary/50")
        );
        assert_eq!(resolveUploadTitle(&props), "Upload files");
        assert_eq!(
            resolveUploadDescription(&props),
            "Drag and drop files here, or click to browse."
        );
        assert_eq!(props.build_attributes().len(), 0);
        assert!(
            upload_icon("folder", "extra")
                .into_string()
                .contains("data-icon=\"folder\"")
        );
        assert!(icon_from_symbol("folder").is_some());
        assert!(icon_from_symbol("Image").is_some());
        assert!(icon_from_symbol("FileText").is_some());
        assert!(icon_from_symbol("").is_none());
        assert!(icon_from_symbol("unknown").is_none());

        let active = UploadAreaProps::new()
            .with_drag_active(true)
            .with_class("extra");
        assert!(
            active
                .build_classes()
                .contains("border-primary bg-primary/5 extra")
        );
        let disabled = UploadAreaProps::new().with_disabled(true);
        assert!(
            disabled
                .build_classes()
                .contains("cursor-not-allowed opacity-60")
        );

        let custom = UploadAreaProps::new()
            .with_id("upload")
            .with_name("files")
            .with_accept("image/*")
            .with_multiple(true)
            .with_max_size("5MB")
            .with_title(" Title ")
            .with_title_content(html! { h4 { "Custom title" } })
            .with_description(" Description ")
            .with_icon("★")
            .with_hide_description(true)
            .with_hide_hints(true)
            .with_attribute("data-testid", "upload");
        assert!(custom.resolve_icon().is_none());
        assert_eq!(resolveUploadTitle(&custom), "Title");
        assert_eq!(resolveUploadDescription(&custom), "Description");
        assert_eq!(
            custom
                .build_attributes()
                .get("data-testid")
                .map(String::as_str),
            Some("upload")
        );
    }

    #[test]
    fn upload_area_renders_input_icon_text_hints_children_and_datastar_attrs() {
        let props = UploadAreaProps::new()
            .with_id("upload")
            .with_name("files")
            .with_accept("<image/*>")
            .with_multiple(true)
            .with_max_size("5MB")
            .with_title("<Upload>")
            .with_description("<Drop>")
            .with_attribute("data-testid", "upload")
            .with_attribute("data-on:drop", "$files = evt.dataTransfer.files");
        let html = UploadArea(&props, html! { small { "Child" } }).into_string();
        assert!(html.starts_with("<label class=\"flex flex-col items-center justify-center"));
        assert!(html.contains("for=\"upload\""));
        assert!(html.contains("data-testid=\"upload\""));
        assert!(html.contains("data-on:drop=\"$files = evt.dataTransfer.files\""));
        assert!(html.contains("<input type=\"file\" class=\"sr-only\" id=\"upload\" name=\"files\" accept=\"&lt;image/*&gt;\" multiple>"));
        assert!(html.contains("data-icon=\"folder\""));
        assert!(html.contains("<p class=\"text-sm font-medium sm:text-base\">&lt;Upload&gt;</p>"));
        assert!(html.contains(
            "<p class=\"text-xs leading-relaxed text-muted-foreground sm:text-sm\">&lt;Drop&gt;</p>"
        ));
        assert!(html.contains("<span class=\"max-w-full break-all rounded bg-muted px-2 py-1 leading-relaxed whitespace-normal\">&lt;image/*&gt;</span>"));
        assert!(html.contains("<span class=\"rounded bg-muted px-2 py-1 leading-relaxed whitespace-nowrap\">Max 5MB</span>"));
        assert!(html.contains("<small>Child</small>"));

        let emoji = UploadArea(
            &UploadAreaProps::new()
                .with_icon("★")
                .with_hide_description(true)
                .with_hide_hints(true),
            html! {},
        )
        .into_string();
        assert!(emoji.contains("<span class=\"text-xl sm:text-2xl\">★</span>"));
        assert!(!emoji.contains("Drag and drop"));
        assert!(!emoji.contains("Max "));
    }

    #[test]
    fn convenience_upload_areas_render_expected_variations() {
        let simple = SimpleUploadArea("files", ".pdf", true, "simple").into_string();
        assert!(simple.contains("name=\"files\""));
        assert!(simple.contains("accept=\".pdf\""));
        assert!(simple.contains("multiple"));
        assert!(simple.contains("simple"));

        let image = ImageUploadArea("image", false, "image-class").into_string();
        assert!(image.contains("accept=\"image/*\""));
        assert!(image.contains("data-icon=\"image\""));
        assert!(image.contains("Upload images"));
        assert!(image.contains("Max 10MB"));
        assert!(image.contains("image-class"));

        let document = DocumentUploadArea("docs", true, "doc-class").into_string();
        assert!(document.contains("name=\"docs\""));
        assert!(document.contains(".pdf,.docx,.doc"));
        assert!(document.contains("data-icon=\"file-text\""));
        assert!(document.contains("Upload documents"));
        assert!(document.contains("Max 200MB"));
        assert!(document.contains("doc-class"));
    }
}
