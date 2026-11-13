#![allow(non_snake_case)]

use crate::components::unstyled::copybutton::{CopyButton, CopyButtonProps};
use crate::components::unstyled::timeago::{TimeAgo, TimeAgoProps};
use crate::components::{ComponentInfo, shared};
use maud::{Markup, Render, html};
use std::collections::BTreeMap;

pub const ROLE_USER: &str = "user";
pub const ROLE_ASSISTANT: &str = "assistant";
pub const ROLE_SYSTEM: &str = "system";
pub const VARIANT_DOTS: &str = "dots";
pub const VARIANT_TEXT: &str = "text";

pub const COMPONENTS: &[ComponentInfo] = &[
    ComponentInfo {
        family: "unstyled",
        module: "chatmessage",
        name: "ChatMessage",
    },
    ComponentInfo {
        family: "unstyled",
        module: "chatmessage",
        name: "ChatInput",
    },
    ComponentInfo {
        family: "unstyled",
        module: "chatmessage",
        name: "TypingIndicator",
    },
    ComponentInfo {
        family: "unstyled",
        module: "chatmessage",
        name: "TypingDots",
    },
    ComponentInfo {
        family: "unstyled",
        module: "chatmessage",
        name: "SimpleChatMessage",
    },
    ComponentInfo {
        family: "unstyled",
        module: "chatmessage",
        name: "SimpleChatInput",
    },
];

struct RawHtml<'a>(&'a str);

impl Render for RawHtml<'_> {
    fn render_to(&self, buffer: &mut String) {
        buffer.push_str(self.0);
    }
}

#[derive(Clone, Debug, Default)]
pub struct ChatMessageProps {
    pub role: String,
    pub content: String,
    pub html_content: String,
    pub timestamp: String,
    pub timestamp_iso: String,
    pub message_id: String,
    pub avatar: String,
    pub avatar_fallback: String,
    pub avatar_content: Option<Markup>,
    pub max_width: String,
    pub show_timestamp: bool,
    pub show_copy_button: bool,
    pub copy_button_aria_label: String,
    pub class: String,
    pub attributes: BTreeMap<String, String>,
}

impl ChatMessageProps {
    #[must_use]
    pub fn new() -> Self {
        Self {
            role: ROLE_USER.to_string(),
            max_width: "max-w-[85%] md:max-w-[70%]".to_string(),
            show_timestamp: true,
            ..Self::default()
        }
    }
    #[must_use]
    pub fn with_role(mut self, value: impl Into<String>) -> Self {
        self.role = value.into();
        self
    }
    #[must_use]
    pub fn with_content(mut self, value: impl Into<String>) -> Self {
        self.content = value.into();
        self
    }
    #[must_use]
    pub fn with_html_content(mut self, value: impl Into<String>) -> Self {
        self.html_content = value.into();
        self
    }
    #[must_use]
    pub fn with_timestamp(mut self, value: impl Into<String>) -> Self {
        self.timestamp = value.into();
        self
    }
    #[must_use]
    pub fn with_timestamp_iso(mut self, value: impl Into<String>) -> Self {
        self.timestamp_iso = value.into();
        self
    }
    #[must_use]
    pub fn with_message_id(mut self, value: impl Into<String>) -> Self {
        self.message_id = value.into();
        self
    }
    #[must_use]
    pub fn with_avatar(mut self, value: impl Into<String>) -> Self {
        self.avatar = value.into();
        self
    }
    #[must_use]
    pub fn with_avatar_fallback(mut self, value: impl Into<String>) -> Self {
        self.avatar_fallback = value.into();
        self
    }
    #[must_use]
    pub fn with_avatar_content(mut self, value: Markup) -> Self {
        self.avatar_content = Some(value);
        self
    }
    #[must_use]
    pub fn with_max_width(mut self, value: impl Into<String>) -> Self {
        self.max_width = value.into();
        self
    }
    #[must_use]
    pub const fn with_show_timestamp(mut self, value: bool) -> Self {
        self.show_timestamp = value;
        self
    }
    #[must_use]
    pub const fn with_show_copy_button(mut self, value: bool) -> Self {
        self.show_copy_button = value;
        self
    }
    #[must_use]
    pub fn with_copy_button_aria_label(mut self, value: impl Into<String>) -> Self {
        self.copy_button_aria_label = value.into();
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
        if self.is_user() {
            shared::build_classes(&["flex justify-end", self.class.as_str()])
        } else {
            shared::build_classes(&["flex justify-start", self.class.as_str()])
        }
    }
    #[must_use]
    pub fn build_attributes(&self) -> BTreeMap<String, String> {
        self.attributes.clone()
    }
    #[must_use]
    pub fn is_user(&self) -> bool {
        self.role == ROLE_USER
    }
    #[must_use]
    pub fn get_avatar_fallback(&self) -> String {
        if !self.avatar_fallback.is_empty() {
            self.avatar_fallback.clone()
        } else if self.is_user() {
            "U".to_string()
        } else {
            "AI".to_string()
        }
    }
    #[must_use]
    pub fn get_timestamp_display(&self) -> String {
        if self.timestamp.is_empty() {
            "\u{2014}".to_string()
        } else {
            self.timestamp.clone()
        }
    }
    #[must_use]
    pub fn get_copy_button_aria_label(&self) -> String {
        if self.copy_button_aria_label.is_empty() {
            "Copy message".to_string()
        } else {
            self.copy_button_aria_label.clone()
        }
    }
    #[must_use]
    pub fn get_message_content_id(&self) -> String {
        if self.message_id.is_empty() {
            String::new()
        } else {
            format!("msg-{}", self.message_id)
        }
    }
}

impl Default for ChatInputProps {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            placeholder: String::new(),
            value: String::new(),
            disabled: false,
            loading: false,
            submit_label: String::new(),
            class: String::new(),
            attributes: BTreeMap::new(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChatInputProps {
    pub id: String,
    pub name: String,
    pub placeholder: String,
    pub value: String,
    pub disabled: bool,
    pub loading: bool,
    pub submit_label: String,
    pub class: String,
    pub attributes: BTreeMap<String, String>,
}

impl ChatInputProps {
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
    pub fn with_placeholder(mut self, value: impl Into<String>) -> Self {
        self.placeholder = value.into();
        self
    }
    #[must_use]
    pub fn with_value(mut self, value: impl Into<String>) -> Self {
        self.value = value.into();
        self
    }
    #[must_use]
    pub const fn with_disabled(mut self, value: bool) -> Self {
        self.disabled = value;
        self
    }
    #[must_use]
    pub const fn with_loading(mut self, value: bool) -> Self {
        self.loading = value;
        self
    }
    #[must_use]
    pub fn with_submit_label(mut self, value: impl Into<String>) -> Self {
        self.submit_label = value.into();
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
        shared::build_classes(&["flex items-end gap-3", self.class.as_str()])
    }
    #[must_use]
    pub fn build_attributes(&self) -> BTreeMap<String, String> {
        self.attributes.clone()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TypingIndicatorProps {
    pub variant: String,
    pub text: String,
    pub class: String,
    pub attributes: BTreeMap<String, String>,
}

impl Default for TypingIndicatorProps {
    fn default() -> Self {
        Self {
            variant: VARIANT_DOTS.to_string(),
            text: String::new(),
            class: String::new(),
            attributes: BTreeMap::new(),
        }
    }
}

impl TypingIndicatorProps {
    #[must_use]
    pub fn with_variant(mut self, value: impl Into<String>) -> Self {
        self.variant = value.into();
        self
    }
    #[must_use]
    pub fn with_text(mut self, value: impl Into<String>) -> Self {
        self.text = value.into();
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
        shared::build_classes(&["flex items-center gap-2", self.class.as_str()])
    }
    #[must_use]
    pub fn build_attributes(&self) -> BTreeMap<String, String> {
        self.attributes.clone()
    }
}

#[must_use]
pub fn getAvatarAlt(is_user: bool) -> &'static str {
    if is_user {
        shared::default_strings().chat_user_avatar
    } else {
        shared::default_strings().chat_ai_avatar
    }
}
#[must_use]
pub fn getInnerContainerClasses(is_user: bool) -> &'static str {
    if is_user {
        "flex items-start gap-3 w-full flex-row-reverse"
    } else {
        "flex items-start gap-3 w-full"
    }
}
#[must_use]
pub fn getAvatarClasses(is_user: bool) -> &'static str {
    if is_user {
        "flex-shrink-0 w-9 h-9 rounded-full flex items-center justify-center font-bold ring-2 ring-background shadow-sm text-sm bg-primary text-primary-foreground"
    } else {
        "flex-shrink-0 w-9 h-9 rounded-full flex items-center justify-center font-bold ring-2 ring-background shadow-sm text-xs bg-accent text-accent-foreground"
    }
}
#[must_use]
pub fn getColumnClasses(is_user: bool) -> &'static str {
    if is_user {
        "flex flex-col items-end flex-1"
    } else {
        "flex flex-col items-start flex-1"
    }
}
#[must_use]
pub fn getBubbleWrapperClasses(max_width: &str, is_user: bool, html_content: &str) -> String {
    if !is_user && isMermaidOnlyHTML(html_content) {
        return "relative w-full max-w-[min(100%,1100px)]".to_string();
    }
    let width = if max_width.is_empty() {
        "max-w-[85%] md:max-w-[70%]"
    } else {
        max_width
    };
    format!("relative {width}")
}
#[must_use]
pub fn getBubbleClasses(is_user: bool, _extra: &str) -> &'static str {
    if is_user {
        "rounded-2xl rounded-tr-sm px-4 py-2.5 bg-chat-user text-chat-user-text shadow-message overflow-hidden break-words"
    } else {
        "rounded-2xl rounded-tl-sm px-4 py-2.5 bg-chat-ai text-chat-ai-text shadow-message overflow-hidden break-words"
    }
}
#[must_use]
pub fn getContentClasses(_is_user: bool) -> &'static str {
    "text-sm leading-relaxed"
}
#[must_use]
pub fn getRichContentClasses(is_user: bool, html_content: &str) -> String {
    let mut base = getProseClasses(is_user);
    if !is_user && isMermaidOnlyHTML(html_content) {
        base.push_str(" w-full max-w-none");
    }
    base
}
#[must_use]
pub fn getTimestampClasses(is_user: bool) -> &'static str {
    if is_user {
        "text-[11px] mt-1 block text-right"
    } else {
        "text-[11px] mt-1 block text-left"
    }
}
#[must_use]
pub fn typingIndicatorText(props: &TypingIndicatorProps) -> String {
    if props.text.is_empty() {
        shared::default_strings().chat_typing_aria_label.to_string()
    } else {
        props.text.clone()
    }
}
#[must_use]
pub fn chatInputPlaceholder(props: &ChatInputProps) -> String {
    if props.placeholder.is_empty() {
        "Type a message...".to_string()
    } else {
        props.placeholder.clone()
    }
}
#[must_use]
pub fn chatInputSubmitLabel(props: &ChatInputProps) -> String {
    if props.submit_label.is_empty() {
        "Send".to_string()
    } else {
        props.submit_label.clone()
    }
}
#[must_use]
pub fn getCopyButtonColorClasses(is_user: bool) -> &'static str {
    if is_user {
        "text-chat-user-text hover:bg-chat-user-text/10 focus-visible:ring-chat-user-text/30"
    } else {
        "text-chat-ai-text hover:bg-chat-ai-text/10 focus-visible:ring-chat-ai-text/30"
    }
}
#[must_use]
pub fn getProseClasses(is_user: bool) -> String {
    let base =
        "prose prose-sm max-w-none leading-relaxed [&_pre]:overflow-x-auto [&_pre]:max-w-full";
    if is_user {
        format!("{base} prose-invert")
    } else {
        format!("{base} dark:prose-invert")
    }
}

#[must_use]
pub fn isMermaidOnlyHTML(html_content: &str) -> bool {
    let trimmed = html_content.trim();
    let starts_mermaid_pre = trimmed.starts_with("<pre")
        && trimmed.contains("<code")
        && trimmed.contains("language-mermaid")
        && trimmed.ends_with("</pre>");
    let starts_mermaid_widget = trimmed.starts_with("<div")
        && trimmed.contains("data-mermaid-widget")
        && trimmed.ends_with("</div>");
    starts_mermaid_pre || starts_mermaid_widget
}

#[must_use]
pub fn ChatMessage(props: &ChatMessageProps) -> Markup {
    let attrs = props.build_attributes();
    let message_content_id = props.get_message_content_id();
    html! {
        div
            class=(props.build_classes())
            id=[shared::attr_value(&attrs, "id")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
        {
            div class=(getInnerContainerClasses(props.is_user())) {
                div class=(getAvatarClasses(props.is_user())) aria-hidden="true" {
                    @if !props.avatar.is_empty() {
                        img src=(&props.avatar) alt=(getAvatarAlt(props.is_user())) class="w-8 h-8 rounded-full object-cover";
                    } @else if let Some(avatar) = &props.avatar_content {
                        (avatar.clone())
                    } @else {
                        (props.get_avatar_fallback())
                    }
                }
                div class=(getColumnClasses(props.is_user())) {
                    div
                        class=(getBubbleWrapperClasses(&props.max_width, props.is_user(), &props.html_content))
                        data-chat-bubble-wrapper=""
                    {
                        div
                            class=(getBubbleClasses(props.is_user(), ""))
                            data-chat-bubble=""
                            id=[if message_content_id.is_empty() { None } else { Some(message_content_id.as_str()) }]
                        {
                            @if !props.html_content.is_empty() {
                                div
                                    class=(getRichContentClasses(props.is_user(), &props.html_content))
                                    data-rich-content="true"
                                    data-on:load="window.processChatContent?.($el)"
                                {
                                    (RawHtml(&props.html_content))
                                }
                            } @else {
                                div class=(getContentClasses(props.is_user())) {
                                    (&props.content)
                                }
                            }
                        }
                        @if props.show_copy_button && !props.message_id.is_empty() {
                            (CopyButton(&CopyButtonProps::default()
                                .with_target_id(message_content_id.clone())
                                .with_aria_label(props.get_copy_button_aria_label())
                                .with_class(getCopyButtonColorClasses(props.is_user()))))
                        }
                    }
                    @if props.show_timestamp {
                        (TimeAgo(&TimeAgoProps::default()
                            .with_datetime(&props.timestamp_iso)
                            .with_display(props.get_timestamp_display())
                            .with_class(getTimestampClasses(props.is_user()))))
                    }
                }
            }
        }
    }
}

#[must_use]
pub fn ChatInput(props: &ChatInputProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        div
            class=(props.build_classes())
            id=[shared::attr_value(&attrs, "id")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
        {
            div class="flex-1" {
                textarea
                    id=[shared::optional_attr(&props.id)]
                    name=[shared::optional_attr(&props.name)]
                    class="w-full resize-none rounded-xl border px-4 py-3 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring min-h-[48px] max-h-32"
                    placeholder=(chatInputPlaceholder(props))
                    rows="1"
                    disabled[props.disabled]
                {
                    (&props.value)
                }
            }
            button
                type="submit"
                class="flex items-center gap-2 px-5 py-3 rounded-xl font-medium transition-colors duration-150 bg-primary text-primary-foreground hover:bg-primary/90 disabled:opacity-50"
                disabled[props.disabled || props.loading]
            {
                @if props.loading {
                    span class="animate-spin w-4 h-4 border-2 border-current border-t-transparent rounded-full" {}
                }
                (chatInputSubmitLabel(props))
            }
        }
    }
}

#[must_use]
pub fn TypingIndicator(props: &TypingIndicatorProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        div
            class=(props.build_classes())
            aria-label=(shared::default_strings().chat_typing_aria_label)
            id=[shared::attr_value(&attrs, "id")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
        {
            @if props.variant == VARIANT_DOTS {
                div class="flex items-center gap-1.5 shimmer rounded-full px-3 py-1.5" {
                    span class="typing-dot w-2 h-2 bg-muted-foreground/50 rounded-full" {}
                    span class="typing-dot w-2 h-2 bg-muted-foreground/50 rounded-full" {}
                    span class="typing-dot w-2 h-2 bg-muted-foreground/50 rounded-full" {}
                }
            } @else {
                span class="text-sm text-muted-foreground" {
                    (typingIndicatorText(props))
                }
            }
        }
    }
}

#[must_use]
pub fn TypingDots(class: &str) -> Markup {
    TypingIndicator(
        &TypingIndicatorProps::default()
            .with_variant(VARIANT_DOTS)
            .with_class(class),
    )
}

#[must_use]
pub fn SimpleChatMessage(role: &str, content: &str, timestamp: &str, class: &str) -> Markup {
    ChatMessage(
        &ChatMessageProps::new()
            .with_role(role)
            .with_content(content)
            .with_timestamp(timestamp)
            .with_show_timestamp(true)
            .with_class(class),
    )
}

#[must_use]
pub fn SimpleChatInput(name: &str, placeholder: &str, class: &str) -> Markup {
    ChatInput(
        &ChatInputProps::default()
            .with_name(name)
            .with_placeholder(placeholder)
            .with_class(class),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chat_message_helpers_cover_defaults_builders_classes_content_ids_and_mermaid_detection() {
        let props = ChatMessageProps::new();
        assert_eq!(props.role, ROLE_USER);
        assert_eq!(props.max_width, "max-w-[85%] md:max-w-[70%]");
        assert!(props.show_timestamp);
        assert!(props.is_user());
        assert_eq!(props.build_classes(), "flex justify-end");
        assert_eq!(
            ChatMessageProps::new()
                .with_role(ROLE_ASSISTANT)
                .build_classes(),
            "flex justify-start"
        );
        assert_eq!(
            ChatMessageProps::new().with_max_width("max-w-sm").max_width,
            "max-w-sm"
        );
        assert_eq!(props.get_avatar_fallback(), "U");
        assert_eq!(
            ChatMessageProps::new()
                .with_role(ROLE_ASSISTANT)
                .get_avatar_fallback(),
            "AI"
        );
        assert_eq!(
            ChatMessageProps::new()
                .with_avatar_fallback("ME")
                .get_avatar_fallback(),
            "ME"
        );
        assert_eq!(ChatMessageProps::new().get_timestamp_display(), "\u{2014}");
        assert_eq!(
            ChatMessageProps::new()
                .with_timestamp("Now")
                .get_timestamp_display(),
            "Now"
        );
        assert_eq!(
            ChatMessageProps::new().get_copy_button_aria_label(),
            "Copy message"
        );
        assert_eq!(
            ChatMessageProps::new()
                .with_copy_button_aria_label("Copy this")
                .get_copy_button_aria_label(),
            "Copy this"
        );
        assert_eq!(ChatMessageProps::new().get_message_content_id(), "");
        assert_eq!(
            ChatMessageProps::new()
                .with_message_id("1")
                .get_message_content_id(),
            "msg-1"
        );
        assert_eq!(getAvatarAlt(true), "User avatar");
        assert_eq!(getAvatarAlt(false), "AI avatar");
        assert_eq!(
            getInnerContainerClasses(true),
            "flex items-start gap-3 w-full flex-row-reverse"
        );
        assert_eq!(
            getAvatarClasses(false),
            "flex-shrink-0 w-9 h-9 rounded-full flex items-center justify-center font-bold ring-2 ring-background shadow-sm text-xs bg-accent text-accent-foreground"
        );
        assert_eq!(getColumnClasses(false), "flex flex-col items-start flex-1");
        assert_eq!(
            getBubbleClasses(true, ""),
            "rounded-2xl rounded-tr-sm px-4 py-2.5 bg-chat-user text-chat-user-text shadow-message overflow-hidden break-words"
        );
        assert_eq!(getContentClasses(false), "text-sm leading-relaxed");
        assert_eq!(
            getTimestampClasses(true),
            "text-[11px] mt-1 block text-right"
        );
        assert_eq!(
            getCopyButtonColorClasses(false),
            "text-chat-ai-text hover:bg-chat-ai-text/10 focus-visible:ring-chat-ai-text/30"
        );
        assert_eq!(
            getProseClasses(true),
            "prose prose-sm max-w-none leading-relaxed [&_pre]:overflow-x-auto [&_pre]:max-w-full prose-invert"
        );

        let mermaid = "<pre><code class=\"language-mermaid\">graph TD\nA-->B</code></pre>";
        let widget = "<div data-mermaid-widget><pre><code class=\"language-mermaid\">graph TD</code></pre></div>";
        assert!(isMermaidOnlyHTML(mermaid));
        assert!(isMermaidOnlyHTML(widget));
        assert!(!isMermaidOnlyHTML(
            "<p>Intro</p><pre><code class=\"language-mermaid\">graph TD</code></pre>"
        ));
        assert!(!isMermaidOnlyHTML("<p>Just text.</p>"));
        assert_eq!(
            getBubbleWrapperClasses("", false, mermaid),
            "relative w-full max-w-[min(100%,1100px)]"
        );
        assert_eq!(
            getBubbleWrapperClasses("", true, mermaid),
            "relative max-w-[85%] md:max-w-[70%]"
        );
        assert!(getRichContentClasses(false, mermaid).contains("w-full max-w-none"));
    }

    #[test]
    fn chat_message_renders_plain_text_timestamp_avatar_and_copy_button() {
        let rendered = ChatMessage(
            &ChatMessageProps::new()
                .with_message_id("abc")
                .with_content("<Hello>")
                .with_timestamp("Just now")
                .with_timestamp_iso("2026-01-01T00:00:00Z")
                .with_avatar("/me.png")
                .with_show_copy_button(true)
                .with_attribute("data-testid", "message"),
        )
        .into_string();
        assert!(rendered.starts_with("<div class=\"flex justify-end\" data-testid=\"message\">"));
        assert!(
            rendered.contains("<div class=\"flex items-start gap-3 w-full flex-row-reverse\">")
        );
        assert!(rendered.contains(
            "<img src=\"/me.png\" alt=\"User avatar\" class=\"w-8 h-8 rounded-full object-cover\">"
        ));
        assert!(rendered.contains("data-chat-bubble-wrapper=\"\""));
        assert!(rendered.contains("data-chat-bubble=\"\" id=\"msg-abc\""));
        assert!(rendered.contains("<div class=\"text-sm leading-relaxed\">&lt;Hello&gt;</div>"));
        assert!(rendered.contains("data-on:click=\"navigator.clipboard.writeText(document.getElementById('msg-abc').innerText)"));
        assert!(rendered.contains("aria-label=\"Copy message\""));
        assert!(rendered.contains("<time datetime=\"2026-01-01T00:00:00Z\" data-timeago=\"\" class=\"text-xs text-muted-foreground text-[11px] mt-1 block text-right\">Just now</time>"));
    }

    #[test]
    fn chat_message_renders_rich_content_raw_and_mermaid_wide_layout() {
        let html_content = "<pre><code class=\"language-mermaid\">graph TD\nA-->B</code></pre>";
        let rendered = ChatMessage(
            &ChatMessageProps::new()
                .with_role(ROLE_ASSISTANT)
                .with_message_id("m1")
                .with_html_content(html_content)
                .with_avatar_content(html! { svg data-testid="bot" {} })
                .with_show_copy_button(false)
                .with_show_timestamp(false),
        )
        .into_string();
        assert!(rendered.contains("max-w-[min(100%,1100px)]"));
        assert!(rendered.contains("bg-accent text-accent-foreground\" aria-hidden=\"true\"><svg data-testid=\"bot\"></svg>"));
        assert!(rendered.contains("data-rich-content=\"true\""));
        assert!(rendered.contains("data-on:load=\"window.processChatContent?.($el)\""));
        assert!(rendered.contains("class=\"prose prose-sm max-w-none leading-relaxed"));
        assert!(rendered.contains("w-full max-w-none"));
        assert!(
            rendered.contains("<pre><code class=\"language-mermaid\">graph TD\nA-->B</code></pre>")
        );
        assert!(!rendered.contains("data-timeago"));
        assert!(!rendered.contains("Copy message"));
    }

    #[test]
    fn chat_input_and_typing_helpers_render_all_variations() {
        let input_props = ChatInputProps::default()
            .with_id("chat")
            .with_name("message")
            .with_placeholder("<Say hi>")
            .with_value("<value>")
            .with_disabled(true)
            .with_loading(true)
            .with_submit_label("Go")
            .with_class("bar")
            .with_attribute("data-testid", "chat-input");
        assert_eq!(input_props.build_classes(), "flex items-end gap-3 bar");
        assert_eq!(
            input_props
                .build_attributes()
                .get("data-testid")
                .map(String::as_str),
            Some("chat-input")
        );
        assert_eq!(
            chatInputPlaceholder(&ChatInputProps::default()),
            "Type a message..."
        );
        assert_eq!(chatInputSubmitLabel(&ChatInputProps::default()), "Send");
        let input = ChatInput(&input_props).into_string();
        assert!(
            input
                .starts_with("<div class=\"flex items-end gap-3 bar\" data-testid=\"chat-input\">")
        );
        assert!(input.contains("id=\"chat\" name=\"message\""));
        assert!(input.contains("placeholder=\"&lt;Say hi&gt;\" rows=\"1\" disabled"));
        assert!(input.contains("&lt;value&gt;"));
        assert!(input.contains("<button type=\"submit\""));
        assert!(input.contains("disabled><span class=\"animate-spin"));
        assert!(input.ends_with("Go</button></div>"));

        let typing = TypingIndicator(
            &TypingIndicatorProps::default().with_attribute("data-testid", "typing"),
        )
        .into_string();
        assert!(typing.starts_with("<div class=\"flex items-center gap-2\" aria-label=\"Typing...\" data-testid=\"typing\">"));
        assert_eq!(typing.matches("typing-dot").count(), 3);
        let text = TypingIndicator(
            &TypingIndicatorProps::default()
                .with_variant(VARIANT_TEXT)
                .with_text("<Working>")
                .with_class("hint"),
        )
        .into_string();
        assert!(text.contains("class=\"flex items-center gap-2 hint\""));
        assert!(
            text.contains("<span class=\"text-sm text-muted-foreground\">&lt;Working&gt;</span>")
        );
        assert_eq!(
            typingIndicatorText(&TypingIndicatorProps::default()),
            "Typing..."
        );
        assert!(
            TypingDots("dots")
                .into_string()
                .contains("flex items-center gap-2 dots")
        );
    }

    #[test]
    fn simple_convenience_components_use_defaults() {
        let message = SimpleChatMessage(ROLE_ASSISTANT, "Hi", "Now", "plain").into_string();
        assert!(message.contains("class=\"flex justify-start plain\""));
        assert!(message.contains("<div class=\"text-sm leading-relaxed\">Hi</div>"));
        assert!(message.contains(">Now</span>"));

        let input = SimpleChatInput("body", "Ask", "compact").into_string();
        assert!(input.contains("class=\"flex items-end gap-3 compact\""));
        assert!(input.contains("name=\"body\""));
        assert!(input.contains("placeholder=\"Ask\""));
        assert!(input.contains(">Send</button>"));
    }
}
