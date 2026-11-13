#![allow(non_snake_case)]

use crate::components::{ComponentInfo, shared};
use maud::{Markup, html};
use std::collections::BTreeMap;

pub const COMPONENTS: &[ComponentInfo] = &[
    ComponentInfo {
        family: "unstyled",
        module: "document",
        name: "DocumentListItem",
    },
    ComponentInfo {
        family: "unstyled",
        module: "document",
        name: "documentItemContent",
    },
    ComponentInfo {
        family: "unstyled",
        module: "document",
        name: "DocumentAttachmentsIndicator",
    },
    ComponentInfo {
        family: "unstyled",
        module: "document",
        name: "LibraryPanel",
    },
    ComponentInfo {
        family: "unstyled",
        module: "document",
        name: "UserInfoBlock",
    },
    ComponentInfo {
        family: "unstyled",
        module: "document",
        name: "SimpleDocumentListItem",
    },
    ComponentInfo {
        family: "unstyled",
        module: "document",
        name: "SimpleDocumentAttachmentsIndicator",
    },
    ComponentInfo {
        family: "unstyled",
        module: "document",
        name: "SimpleLibraryPanel",
    },
    ComponentInfo {
        family: "unstyled",
        module: "document",
        name: "SimpleUserInfoBlock",
    },
];

#[derive(Clone, Debug, Default)]
pub struct DocumentListItemProps {
    pub id: String,
    pub title: String,
    pub description: String,
    pub file_type: String,
    pub icon_content: Option<Markup>,
    pub file_size: String,
    pub timestamp: String,
    pub href: String,
    pub is_selected: bool,
    pub class: String,
    pub attributes: BTreeMap<String, String>,
}

impl DocumentListItemProps {
    #[must_use]
    pub fn new() -> Self {
        Self {
            file_type: "document".to_string(),
            ..Self::default()
        }
    }
    #[must_use]
    pub fn with_id(mut self, value: impl Into<String>) -> Self {
        self.id = value.into();
        self
    }
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
    pub fn with_icon_content(mut self, value: Markup) -> Self {
        self.icon_content = Some(value);
        self
    }
    #[must_use]
    pub fn with_file_info(
        mut self,
        file_type: impl Into<String>,
        file_size: impl Into<String>,
    ) -> Self {
        self.file_type = file_type.into();
        self.file_size = file_size.into();
        self
    }
    #[must_use]
    pub fn with_timestamp(mut self, value: impl Into<String>) -> Self {
        self.timestamp = value.into();
        self
    }
    #[must_use]
    pub fn with_href(mut self, value: impl Into<String>) -> Self {
        self.href = value.into();
        self
    }
    #[must_use]
    pub const fn with_selected(mut self, value: bool) -> Self {
        self.is_selected = value;
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
            "flex items-start gap-3 p-4 rounded-lg transition-colors duration-150",
            if self.is_selected {
                "bg-accent"
            } else {
                "hover:bg-muted"
            },
            self.class.as_str(),
        ])
    }

    #[must_use]
    pub fn build_attributes(&self) -> BTreeMap<String, String> {
        self.attributes.clone()
    }

    #[must_use]
    pub fn get_file_icon_content(&self) -> Markup {
        if let Some(content) = self.icon_content.clone() {
            return content;
        }
        let (name, class) = file_icon_name_and_class(&self.file_type);
        file_icon(name, class)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DocumentAttachmentsIndicatorProps {
    pub count: i32,
    pub show_icon: bool,
    pub class: String,
    pub attributes: BTreeMap<String, String>,
}

impl Default for DocumentAttachmentsIndicatorProps {
    fn default() -> Self {
        Self {
            count: 0,
            show_icon: true,
            class: String::new(),
            attributes: BTreeMap::new(),
        }
    }
}

impl DocumentAttachmentsIndicatorProps {
    #[must_use]
    pub const fn with_count(mut self, value: i32) -> Self {
        self.count = value;
        self
    }
    #[must_use]
    pub const fn with_show_icon(mut self, value: bool) -> Self {
        self.show_icon = value;
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
            "inline-flex items-center gap-1.5 text-sm text-muted-foreground",
            self.class.as_str(),
        ])
    }
    #[must_use]
    pub fn build_attributes(&self) -> BTreeMap<String, String> {
        self.attributes.clone()
    }
}

#[derive(Clone, Debug, Default)]
pub struct LibraryPanelProps {
    pub title: String,
    pub documents: Vec<DocumentListItemProps>,
    pub show_upload: bool,
    pub show_search: bool,
    pub empty_text: String,
    pub class: String,
    pub attributes: BTreeMap<String, String>,
}

impl LibraryPanelProps {
    #[must_use]
    pub fn with_title(mut self, value: impl Into<String>) -> Self {
        self.title = value.into();
        self
    }
    #[must_use]
    pub fn with_documents(mut self, value: Vec<DocumentListItemProps>) -> Self {
        self.documents = value;
        self
    }
    #[must_use]
    pub const fn with_upload(mut self, value: bool) -> Self {
        self.show_upload = value;
        self
    }
    #[must_use]
    pub const fn with_search(mut self, value: bool) -> Self {
        self.show_search = value;
        self
    }
    #[must_use]
    pub fn with_empty_text(mut self, value: impl Into<String>) -> Self {
        self.empty_text = value.into();
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
            "flex flex-col rounded-xl border border-border",
            self.class.as_str(),
        ])
    }
    #[must_use]
    pub fn build_attributes(&self) -> BTreeMap<String, String> {
        self.attributes.clone()
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct UserInfoBlockProps {
    pub name: String,
    pub email: String,
    pub avatar_src: String,
    pub avatar_fallback: String,
    pub role: String,
    pub show_status: bool,
    pub is_online: bool,
    pub class: String,
    pub attributes: BTreeMap<String, String>,
}

impl UserInfoBlockProps {
    #[must_use]
    pub fn with_name(mut self, value: impl Into<String>) -> Self {
        self.name = value.into();
        self
    }
    #[must_use]
    pub fn with_email(mut self, value: impl Into<String>) -> Self {
        self.email = value.into();
        self
    }
    #[must_use]
    pub fn with_avatar(mut self, src: impl Into<String>, fallback: impl Into<String>) -> Self {
        self.avatar_src = src.into();
        self.avatar_fallback = fallback.into();
        self
    }
    #[must_use]
    pub fn with_role(mut self, value: impl Into<String>) -> Self {
        self.role = value.into();
        self
    }
    #[must_use]
    pub const fn with_status(mut self, show: bool, is_online: bool) -> Self {
        self.show_status = show;
        self.is_online = is_online;
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
        shared::build_classes(&["flex items-center gap-3", self.class.as_str()])
    }
    #[must_use]
    pub fn build_attributes(&self) -> BTreeMap<String, String> {
        self.attributes.clone()
    }
    #[must_use]
    pub fn get_avatar_fallback(&self) -> String {
        if !self.avatar_fallback.is_empty() {
            self.avatar_fallback.clone()
        } else if let Some(ch) = self.name.chars().next() {
            ch.to_string()
        } else {
            "?".to_string()
        }
    }
}

fn attr<'a>(attrs: &'a BTreeMap<String, String>, name: &str) -> Option<&'a str> {
    shared::attr_value(attrs, name)
}

#[must_use]
pub fn file_icon_name_and_class(file_type: &str) -> (&'static str, &'static str) {
    match file_type {
        "pdf" => ("file-text", "text-destructive"),
        "doc" | "docx" => ("document", "text-info"),
        "xls" | "xlsx" => ("bar-chart", "text-success"),
        "ppt" | "pptx" => ("presentation", "text-warning"),
        "image" | "jpg" | "png" | "gif" => ("image", "text-primary"),
        "video" | "mp4" | "mov" => ("play-filled", "text-primary"),
        "audio" | "mp3" | "wav" => ("activity", "text-accent-foreground"),
        "zip" | "rar" => ("folder", "text-muted-foreground"),
        _ => ("file-text", "text-muted-foreground"),
    }
}

#[must_use]
pub fn file_icon(name: &str, class: &str) -> Markup {
    html! {
        svg class=(shared::build_classes(&["h-6 w-6", class])) data-icon=(name) xmlns="http://www.w3.org/2000/svg" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="1.8" {
            path stroke-linecap="round" stroke-linejoin="round" d="M6 2.75h8.5L20 8.25v13H6z" {}
            path stroke-linecap="round" stroke-linejoin="round" d="M14.5 2.75V8.25H20" {}
        }
    }
}

#[must_use]
pub fn paperclip_icon() -> Markup {
    html! {
        svg class="h-3.5 w-3.5" data-icon="paperclip" xmlns="http://www.w3.org/2000/svg" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="1.8" {
            path stroke-linecap="round" stroke-linejoin="round" d="m21.44 11.05-9.19 9.19a6 6 0 0 1-8.49-8.49l9.19-9.19a4 4 0 0 1 5.66 5.66l-9.2 9.19a2 2 0 1 1-2.83-2.83l8.49-8.48" {}
        }
    }
}

#[must_use]
pub fn toolbar_icon(name: &str) -> Markup {
    html! {
        svg class="h-4 w-4" data-icon=(name) xmlns="http://www.w3.org/2000/svg" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="1.8" {
            path stroke-linecap="round" stroke-linejoin="round" d="M4 12h16M12 4v16" {}
        }
    }
}

#[must_use]
pub fn DocumentListItem(props: &DocumentListItemProps) -> Markup {
    let attrs = props.build_attributes();
    if props.href.is_empty() {
        html! {
            div
                id=[shared::optional_attr(&props.id)]
                class=(props.build_classes())
                data-testid=[attr(&attrs, "data-testid")]
                data-on:click=[attr(&attrs, "data-on:click")]
            {
                (documentItemContent(props))
            }
        }
    } else {
        html! {
            a
                href=(&props.href)
                id=[shared::optional_attr(&props.id)]
                class=(props.build_classes())
                data-testid=[attr(&attrs, "data-testid")]
                data-on:click=[attr(&attrs, "data-on:click")]
            {
                (documentItemContent(props))
            }
        }
    }
}

#[must_use]
pub fn documentItemContent(props: &DocumentListItemProps) -> Markup {
    html! {
        div class="flex-shrink-0 w-10 h-10 rounded-lg bg-muted flex items-center justify-center" {
            (props.get_file_icon_content())
        }
        div class="flex-1 min-w-0" {
            p class="font-medium truncate" { (&props.title) }
            @if !props.description.is_empty() {
                p class="text-sm text-muted-foreground truncate" { (&props.description) }
            }
            div class="flex items-center gap-2 mt-1 text-xs text-muted-foreground" {
                @if !props.file_size.is_empty() {
                    span { (&props.file_size) }
                }
                @if !props.file_size.is_empty() && !props.timestamp.is_empty() {
                    span { "•" }
                }
                @if !props.timestamp.is_empty() {
                    span { (&props.timestamp) }
                }
            }
        }
    }
}

#[must_use]
pub fn getAttachmentText(count: i32) -> String {
    match count {
        0 => "No attachments".to_string(),
        1 => "1 attachment".to_string(),
        _ => format!("{count} attachments"),
    }
}

#[must_use]
pub fn DocumentAttachmentsIndicator(props: &DocumentAttachmentsIndicatorProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        span class=(props.build_classes()) data-testid=[attr(&attrs, "data-testid")] {
            @if props.show_icon {
                (paperclip_icon())
            }
            span { (getAttachmentText(props.count)) }
        }
    }
}

#[must_use]
pub fn libraryPanelTitle(props: &LibraryPanelProps) -> String {
    if props.title.is_empty() {
        "Library".to_string()
    } else {
        props.title.clone()
    }
}

#[must_use]
pub fn libraryPanelEmptyText(props: &LibraryPanelProps) -> String {
    if props.empty_text.is_empty() {
        "No documents".to_string()
    } else {
        props.empty_text.clone()
    }
}

#[must_use]
pub const fn getOnlineStatusColor(is_online: bool) -> &'static str {
    if is_online {
        "bg-success"
    } else {
        "bg-muted-foreground"
    }
}

#[must_use]
pub fn LibraryPanel(props: &LibraryPanelProps, children: Markup) -> Markup {
    let attrs = props.build_attributes();
    html! {
        div class=(props.build_classes()) data-testid=[attr(&attrs, "data-testid")] {
            div class="flex items-center justify-between px-4 py-3 border-b border-border" {
                h3 class="font-semibold" { (libraryPanelTitle(props)) }
                div class="flex items-center gap-2" {
                    @if props.show_search {
                        button type="button" class="p-2 rounded-md hover:bg-muted transition-colors duration-150" { (toolbar_icon("search")) }
                    }
                    @if props.show_upload {
                        button type="button" class="p-2 rounded-md hover:bg-muted transition-colors duration-150" { (toolbar_icon("cloud-upload")) }
                    }
                }
            }
            div class="flex-1 overflow-y-auto" {
                @if props.documents.is_empty() {
                    div class="flex flex-col items-center justify-center py-12 text-muted-foreground" {
                        span class="mb-2" { (file_icon("folder", "h-8 w-8")) }
                        p class="text-sm" { (libraryPanelEmptyText(props)) }
                    }
                } @else {
                    div class="divide-y divide-border" {
                        @for doc in &props.documents {
                            (DocumentListItem(doc))
                        }
                    }
                }
            }
            (children)
        }
    }
}

#[must_use]
pub fn UserInfoBlock(props: &UserInfoBlockProps, children: Markup) -> Markup {
    let attrs = props.build_attributes();
    html! {
        div class=(props.build_classes()) data-testid=[attr(&attrs, "data-testid")] {
            div class="relative flex-shrink-0" {
                @if !props.avatar_src.is_empty() {
                    img src=(&props.avatar_src) alt=(&props.name) class="w-10 h-10 rounded-full object-cover";
                } @else {
                    div class="w-10 h-10 rounded-full bg-primary text-primary-foreground flex items-center justify-center text-sm font-medium" {
                        (props.get_avatar_fallback())
                    }
                }
                @if props.show_status {
                    span class=(shared::build_classes(&["absolute bottom-0 right-0 w-3 h-3 rounded-full border-2 border-background", getOnlineStatusColor(props.is_online)])) {}
                }
            }
            div class="flex-1 min-w-0" {
                p class="font-medium truncate" { (&props.name) }
                @if !props.email.is_empty() {
                    p class="text-sm text-muted-foreground truncate" { (&props.email) }
                } @else if !props.role.is_empty() {
                    p class="text-sm text-muted-foreground truncate" { (&props.role) }
                }
            }
            (children)
        }
    }
}

#[must_use]
pub fn SimpleDocumentListItem(
    title: &str,
    file_type: &str,
    file_size: &str,
    href: &str,
    class: &str,
) -> Markup {
    DocumentListItem(
        &DocumentListItemProps::new()
            .with_title(title)
            .with_file_info(file_type, file_size)
            .with_href(href)
            .with_class(class),
    )
}

#[must_use]
pub fn SimpleDocumentAttachmentsIndicator(count: i32, class: &str) -> Markup {
    DocumentAttachmentsIndicator(
        &DocumentAttachmentsIndicatorProps::default()
            .with_count(count)
            .with_class(class),
    )
}

#[must_use]
pub fn SimpleLibraryPanel(
    title: &str,
    documents: Vec<DocumentListItemProps>,
    class: &str,
) -> Markup {
    LibraryPanel(
        &LibraryPanelProps::default()
            .with_title(title)
            .with_documents(documents)
            .with_class(class),
        html! {},
    )
}

#[must_use]
pub fn SimpleUserInfoBlock(name: &str, email: &str, avatar_src: &str, class: &str) -> Markup {
    UserInfoBlock(
        &UserInfoBlockProps::default()
            .with_name(name)
            .with_email(email)
            .with_avatar(avatar_src, "")
            .with_class(class),
        html! {},
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn document_item_helpers_cover_classes_icons_attrs_and_custom_icon() {
        let props = DocumentListItemProps::new();
        assert_eq!(props.file_type, "document");
        assert!(props.build_classes().contains("hover:bg-muted"));
        assert_eq!(props.build_attributes().len(), 0);
        assert_eq!(
            file_icon_name_and_class("pdf"),
            ("file-text", "text-destructive")
        );
        assert_eq!(file_icon_name_and_class("docx"), ("document", "text-info"));
        assert_eq!(
            file_icon_name_and_class("xlsx"),
            ("bar-chart", "text-success")
        );
        assert_eq!(
            file_icon_name_and_class("ppt"),
            ("presentation", "text-warning")
        );
        assert_eq!(file_icon_name_and_class("png"), ("image", "text-primary"));
        assert_eq!(
            file_icon_name_and_class("mp4"),
            ("play-filled", "text-primary")
        );
        assert_eq!(
            file_icon_name_and_class("mp3"),
            ("activity", "text-accent-foreground")
        );
        assert_eq!(
            file_icon_name_and_class("zip"),
            ("folder", "text-muted-foreground")
        );
        assert_eq!(
            file_icon_name_and_class("unknown"),
            ("file-text", "text-muted-foreground")
        );

        let selected = DocumentListItemProps::new()
            .with_id("doc1")
            .with_title("Title")
            .with_description("Description")
            .with_file_info("pdf", "10 KB")
            .with_timestamp("Today")
            .with_href("/doc")
            .with_selected(true)
            .with_class("extra")
            .with_attribute("data-testid", "doc");
        assert!(selected.build_classes().contains("bg-accent"));
        assert!(selected.build_classes().contains("extra"));
        assert_eq!(
            selected
                .build_attributes()
                .get("data-testid")
                .map(String::as_str),
            Some("doc")
        );
        assert!(
            selected
                .get_file_icon_content()
                .into_string()
                .contains("data-icon=\"file-text\"")
        );

        let custom = DocumentListItemProps::new().with_icon_content(html! { strong { "Custom" } });
        assert_eq!(
            custom.get_file_icon_content().into_string(),
            "<strong>Custom</strong>"
        );
    }

    #[test]
    fn attachments_library_and_user_helpers_cover_all_branches() {
        assert_eq!(getAttachmentText(0), "No attachments");
        assert_eq!(getAttachmentText(1), "1 attachment");
        assert_eq!(getAttachmentText(4), "4 attachments");
        assert_eq!(getOnlineStatusColor(true), "bg-success");
        assert_eq!(getOnlineStatusColor(false), "bg-muted-foreground");

        let indicator = DocumentAttachmentsIndicatorProps::default()
            .with_count(2)
            .with_show_icon(false)
            .with_class("extra")
            .with_attribute("data-testid", "attachments");
        assert!(indicator.build_classes().contains("extra"));
        assert_eq!(
            indicator
                .build_attributes()
                .get("data-testid")
                .map(String::as_str),
            Some("attachments")
        );

        let panel = LibraryPanelProps::default()
            .with_title("Docs")
            .with_empty_text("Empty")
            .with_upload(true)
            .with_search(true)
            .with_class("panel")
            .with_attribute("data-testid", "library");
        assert_eq!(libraryPanelTitle(&panel), "Docs");
        assert_eq!(libraryPanelEmptyText(&panel), "Empty");
        assert!(panel.build_classes().contains("panel"));
        assert_eq!(LibraryPanelProps::default().build_attributes().len(), 0);
        assert_eq!(libraryPanelTitle(&LibraryPanelProps::default()), "Library");
        assert_eq!(
            libraryPanelEmptyText(&LibraryPanelProps::default()),
            "No documents"
        );

        let user = UserInfoBlockProps::default()
            .with_name("Alice")
            .with_avatar("", "")
            .with_role("Admin");
        assert_eq!(user.get_avatar_fallback(), "A");
        assert!(user.build_classes().contains("flex items-center"));
        assert_eq!(UserInfoBlockProps::default().get_avatar_fallback(), "?");
        assert_eq!(
            UserInfoBlockProps::default()
                .with_avatar("", "ZZ")
                .get_avatar_fallback(),
            "ZZ"
        );
    }

    #[test]
    fn document_components_render_expected_html_variations() {
        let props = DocumentListItemProps::new()
            .with_id("doc1")
            .with_title("<Title>")
            .with_description("<Desc>")
            .with_file_info("pdf", "12 KB")
            .with_timestamp("Today")
            .with_href("/docs/1")
            .with_selected(true)
            .with_attribute("data-testid", "doc");
        let html = DocumentListItem(&props).into_string();
        assert!(html.starts_with("<a href=\"/docs/1\" id=\"doc1\" class=\"flex items-start"));
        assert!(html.contains("bg-accent"));
        assert!(html.contains("data-testid=\"doc\""));
        assert!(html.contains("data-icon=\"file-text\""));
        assert!(html.contains("<p class=\"font-medium truncate\">&lt;Title&gt;</p>"));
        assert!(
            html.contains("<p class=\"text-sm text-muted-foreground truncate\">&lt;Desc&gt;</p>")
        );
        assert!(html.contains("<span>12 KB</span><span>•</span><span>Today</span>"));

        let plain =
            DocumentListItem(&DocumentListItemProps::new().with_title("Plain")).into_string();
        assert!(plain.starts_with("<div class=\"flex items-start"));
        assert!(plain.contains(">Plain</p>"));

        let attachments = DocumentAttachmentsIndicator(
            &DocumentAttachmentsIndicatorProps::default().with_count(1),
        )
        .into_string();
        assert!(attachments.contains("data-icon=\"paperclip\""));
        assert!(attachments.contains("<span>1 attachment</span>"));

        let no_icon = DocumentAttachmentsIndicator(
            &DocumentAttachmentsIndicatorProps::default()
                .with_count(0)
                .with_show_icon(false),
        )
        .into_string();
        assert!(!no_icon.contains("paperclip"));
        assert!(no_icon.contains("No attachments"));
    }

    #[test]
    fn library_and_user_components_render_documents_empty_actions_status_and_children() {
        let doc = DocumentListItemProps::new()
            .with_title("Doc")
            .with_file_info("doc", "1 KB");
        let panel = LibraryPanel(
            &LibraryPanelProps::default()
                .with_title("Library")
                .with_documents(vec![doc])
                .with_search(true)
                .with_upload(true)
                .with_class("extra")
                .with_attribute("data-testid", "library"),
            html! { footer { "Child" } },
        )
        .into_string();
        assert!(
            panel.starts_with("<div class=\"flex flex-col rounded-xl border border-border extra\"")
        );
        assert!(panel.contains("data-testid=\"library\""));
        assert!(panel.contains("<h3 class=\"font-semibold\">Library</h3>"));
        assert!(panel.contains("data-icon=\"search\""));
        assert!(panel.contains("data-icon=\"cloud-upload\""));
        assert!(panel.contains("divide-y divide-border"));
        assert!(panel.contains("<footer>Child</footer>"));

        let empty = LibraryPanel(
            &LibraryPanelProps::default().with_empty_text("No files"),
            html! {},
        )
        .into_string();
        assert!(empty.contains("data-icon=\"folder\""));
        assert!(empty.contains("<p class=\"text-sm\">No files</p>"));

        let user = UserInfoBlock(
            &UserInfoBlockProps::default()
                .with_name("<Alice>")
                .with_email("a@example.com")
                .with_avatar("/a.png", "")
                .with_status(true, true)
                .with_attribute("data-testid", "user"),
            html! { button { "Action" } },
        )
        .into_string();
        assert!(user.contains("data-testid=\"user\""));
        assert!(user.contains("<img src=\"/a.png\" alt=\"&lt;Alice&gt;\""));
        assert!(user.contains("bg-success"));
        assert!(
            user.contains("<p class=\"text-sm text-muted-foreground truncate\">a@example.com</p>")
        );
        assert!(user.contains("<button>Action</button>"));

        let role_user = UserInfoBlock(
            &UserInfoBlockProps::default()
                .with_name("Bob")
                .with_role("Admin")
                .with_status(true, false),
            html! {},
        )
        .into_string();
        assert!(role_user.contains(">B</div>"));
        assert!(role_user.contains("bg-muted-foreground"));
        assert!(role_user.contains(">Admin</p>"));

        assert!(
            SimpleDocumentListItem("Simple", "pdf", "2 KB", "/s", "simple")
                .into_string()
                .contains("simple")
        );
        assert!(
            SimpleDocumentAttachmentsIndicator(3, "count")
                .into_string()
                .contains("3 attachments")
        );
        assert!(
            SimpleLibraryPanel("Simple Library", Vec::new(), "panel")
                .into_string()
                .contains("Simple Library")
        );
        assert!(
            SimpleUserInfoBlock("Name", "email", "/avatar.png", "user")
                .into_string()
                .contains("/avatar.png")
        );
    }
}
