#![allow(non_snake_case)]

use crate::components::{ComponentInfo, shared};
use maud::{Markup, html};
use std::collections::BTreeMap;

pub const CONNECTED: &str = "connected";
pub const CONNECTING: &str = "connecting";
pub const DISCONNECTED: &str = "disconnected";

pub const COMPONENTS: &[ComponentInfo] = &[
    ComponentInfo {
        family: "unstyled",
        module: "activityfeed",
        name: "ActivityItem",
    },
    ComponentInfo {
        family: "unstyled",
        module: "activityfeed",
        name: "activityItemContent",
    },
    ComponentInfo {
        family: "unstyled",
        module: "activityfeed",
        name: "ActivityFeed",
    },
    ComponentInfo {
        family: "unstyled",
        module: "activityfeed",
        name: "SimpleActivityFeed",
    },
];

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ActivityItemProps {
    pub id: String,
    pub username: String,
    pub avatar_initial: String,
    pub avatar_src: String,
    pub action: String,
    pub target: String,
    pub timestamp: String,
    pub href: String,
    pub icon: String,
    pub class: String,
    pub attributes: BTreeMap<String, String>,
}

impl ActivityItemProps {
    #[must_use]
    pub fn with_id(mut self, value: impl Into<String>) -> Self {
        self.id = value.into();
        self
    }
    #[must_use]
    pub fn with_username(mut self, value: impl Into<String>) -> Self {
        self.username = value.into();
        self
    }
    #[must_use]
    pub fn with_avatar_initial(mut self, value: impl Into<String>) -> Self {
        self.avatar_initial = value.into();
        self
    }
    #[must_use]
    pub fn with_avatar_src(mut self, value: impl Into<String>) -> Self {
        self.avatar_src = value.into();
        self
    }
    #[must_use]
    pub fn with_action(mut self, value: impl Into<String>) -> Self {
        self.action = value.into();
        self
    }
    #[must_use]
    pub fn with_target(mut self, value: impl Into<String>) -> Self {
        self.target = value.into();
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
    pub fn with_icon(mut self, value: impl Into<String>) -> Self {
        self.icon = value.into();
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
            "flex items-center gap-3 p-3 rounded-lg hover:bg-muted transition-colors duration-150",
            self.class.as_str(),
        ])
    }

    #[must_use]
    pub fn build_attributes(&self) -> BTreeMap<String, String> {
        self.attributes.clone()
    }

    #[must_use]
    pub fn get_avatar_initial(&self) -> String {
        if !self.avatar_initial.is_empty() {
            self.avatar_initial.clone()
        } else if let Some(ch) = self.username.chars().next() {
            ch.to_string()
        } else {
            "?".to_string()
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ActivityFeedProps {
    pub title: String,
    pub items: Vec<ActivityItemProps>,
    pub online_count: i32,
    pub connection_status: &'static str,
    pub show_typing_indicator: bool,
    pub empty_message: String,
    pub class: String,
    pub attributes: BTreeMap<String, String>,
}

impl Default for ActivityFeedProps {
    fn default() -> Self {
        Self {
            title: String::new(),
            items: Vec::new(),
            online_count: 0,
            connection_status: CONNECTED,
            show_typing_indicator: false,
            empty_message: String::new(),
            class: String::new(),
            attributes: BTreeMap::new(),
        }
    }
}

impl ActivityFeedProps {
    #[must_use]
    pub fn with_title(mut self, value: impl Into<String>) -> Self {
        self.title = value.into();
        self
    }
    #[must_use]
    pub fn with_items(mut self, value: Vec<ActivityItemProps>) -> Self {
        self.items = value;
        self
    }
    #[must_use]
    pub const fn with_online_count(mut self, value: i32) -> Self {
        self.online_count = value;
        self
    }
    #[must_use]
    pub const fn with_connection_status(mut self, value: &'static str) -> Self {
        self.connection_status = value;
        self
    }
    #[must_use]
    pub const fn with_show_typing_indicator(mut self, value: bool) -> Self {
        self.show_typing_indicator = value;
        self
    }
    #[must_use]
    pub fn with_empty_message(mut self, value: impl Into<String>) -> Self {
        self.empty_message = value.into();
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

    #[must_use]
    pub fn get_connection_status_text(&self) -> &'static str {
        match self.connection_status {
            CONNECTED => "Connected",
            CONNECTING => "Connecting...",
            DISCONNECTED => "Disconnected",
            _ => "Unknown status",
        }
    }
}

#[must_use]
pub fn formatOnlineCount(count: i32) -> String {
    if count == 1 {
        "1 online".to_string()
    } else {
        format!("{count} online")
    }
}

#[must_use]
pub fn feedTitle(props: &ActivityFeedProps) -> String {
    if props.title.is_empty() {
        "Activity".to_string()
    } else {
        props.title.clone()
    }
}

#[must_use]
pub fn feedEmptyMessage(props: &ActivityFeedProps) -> String {
    if props.empty_message.is_empty() {
        "Waiting for activity".to_string()
    } else {
        props.empty_message.clone()
    }
}

fn attr<'a>(attrs: &'a BTreeMap<String, String>, name: &str) -> Option<&'a str> {
    shared::attr_value(attrs, name)
}

#[must_use]
pub fn ActivityItem(props: &ActivityItemProps) -> Markup {
    let attrs = props.build_attributes();
    if props.href.is_empty() {
        html! {
            div
                id=[shared::optional_attr(&props.id)]
                class=(props.build_classes())
                data-testid=[attr(&attrs, "data-testid")]
                data-on:click=[attr(&attrs, "data-on:click")]
            {
                (activityItemContent(props))
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
                (activityItemContent(props))
            }
        }
    }
}

#[must_use]
pub fn activityItemContent(props: &ActivityItemProps) -> Markup {
    html! {
        @if !props.icon.is_empty() {
            span class="text-lg flex-shrink-0" { (&props.icon) }
        }
        @if !props.avatar_src.is_empty() {
            img src=(&props.avatar_src) alt=(&props.username) class="w-8 h-8 rounded-full object-cover flex-shrink-0";
        } @else {
            div class="w-8 h-8 rounded-full bg-accent flex items-center justify-center text-accent-foreground text-xs font-medium flex-shrink-0" {
                (props.get_avatar_initial())
            }
        }
        div class="flex-1 min-w-0" {
            p class="text-sm" {
                span class="font-medium" { (&props.username) }
                span class="text-muted-foreground" { " " (&props.action) }
                @if !props.target.is_empty() {
                    span class="font-medium" { " " (&props.target) }
                }
            }
        }
        span class="text-xs text-muted-foreground flex-shrink-0" { (&props.timestamp) }
        @if !props.href.is_empty() {
            svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" class="text-muted-foreground flex-shrink-0" {
                path d="m9 18 6-6-6-6" {}
            }
        }
    }
}

#[must_use]
pub fn ActivityFeed(props: &ActivityFeedProps, children: Markup) -> Markup {
    let attrs = props.build_attributes();
    html! {
        div
            class=(props.build_classes())
            data-testid=[attr(&attrs, "data-testid")]
            data-on:click=[attr(&attrs, "data-on:click")]
        {
            div class="flex items-center justify-between p-4 border-b border-border" {
                h3 class="font-semibold" { (feedTitle(props)) }
                @if props.online_count > 0 {
                    div class="flex items-center gap-2 text-sm text-muted-foreground" {
                        span class="relative flex h-2 w-2" {
                            span class="animate-ping absolute inline-flex h-full w-full rounded-full bg-success opacity-75" {}
                            span class="relative inline-flex rounded-full h-2 w-2 bg-success" {}
                        }
                        span { (formatOnlineCount(props.online_count)) }
                    }
                }
            }
            div class="px-4 py-2 text-sm text-muted-foreground border-b border-border" {
                (props.get_connection_status_text())
            }
            div class="flex-1 overflow-y-auto p-2 space-y-1 min-h-[200px]" {
                @if props.items.is_empty() {
                    div class="flex flex-col items-center justify-center h-full text-muted-foreground py-8" {
                        p class="text-sm" { (feedEmptyMessage(props)) }
                    }
                } @else {
                    @for item in &props.items {
                        (ActivityItem(item))
                    }
                }
            }
            @if props.show_typing_indicator {
                div class="px-4 py-3 border-t border-border" {
                    div class="flex items-center gap-2 text-sm text-muted-foreground" {
                        div class="flex gap-1" {
                            span class="w-2 h-2 bg-muted-foreground rounded-full animate-bounce" style="animation-delay: 0ms" {}
                            span class="w-2 h-2 bg-muted-foreground rounded-full animate-bounce" style="animation-delay: 150ms" {}
                            span class="w-2 h-2 bg-muted-foreground rounded-full animate-bounce" style="animation-delay: 300ms" {}
                        }
                        span { "Someone is typing..." }
                    }
                }
            }
            (children)
        }
    }
}

#[must_use]
pub fn SimpleActivityFeed(title: &str, items: Vec<ActivityItemProps>, class: &str) -> Markup {
    ActivityFeed(
        &ActivityFeedProps::default()
            .with_title(title)
            .with_items(items)
            .with_class(class),
        html! {},
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn item_helpers_cover_defaults_classes_attrs_and_avatar_initials() {
        let props = ActivityItemProps::default();
        assert_eq!(props.get_avatar_initial(), "?");
        assert_eq!(
            props.build_classes(),
            "flex items-center gap-3 p-3 rounded-lg hover:bg-muted transition-colors duration-150"
        );

        let props = ActivityItemProps::default()
            .with_id("a1")
            .with_username("Élodie")
            .with_avatar_initial("E")
            .with_avatar_src("/avatar.png")
            .with_action("created")
            .with_target("a note")
            .with_timestamp("1m")
            .with_href("/activity/1")
            .with_icon("✓")
            .with_class("extra")
            .with_attribute("data-testid", "activity");
        assert_eq!(props.get_avatar_initial(), "E");
        assert!(props.build_classes().ends_with("extra"));
        assert_eq!(
            props
                .build_attributes()
                .get("data-testid")
                .map(String::as_str),
            Some("activity")
        );

        let fallback = ActivityItemProps::default().with_username("Alice");
        assert_eq!(fallback.get_avatar_initial(), "A");
    }

    #[test]
    fn feed_helpers_cover_status_counts_titles_and_attrs() {
        assert_eq!(formatOnlineCount(1), "1 online");
        assert_eq!(formatOnlineCount(3), "3 online");
        let default_props = ActivityFeedProps::default();
        assert_eq!(default_props.connection_status, CONNECTED);
        assert_eq!(default_props.get_connection_status_text(), "Connected");
        assert_eq!(feedTitle(&default_props), "Activity");
        assert_eq!(feedEmptyMessage(&default_props), "Waiting for activity");
        assert_eq!(
            ActivityFeedProps::default()
                .with_connection_status(CONNECTING)
                .get_connection_status_text(),
            "Connecting..."
        );
        assert_eq!(
            ActivityFeedProps::default()
                .with_connection_status(DISCONNECTED)
                .get_connection_status_text(),
            "Disconnected"
        );
        assert_eq!(
            ActivityFeedProps::default()
                .with_connection_status("weird")
                .get_connection_status_text(),
            "Unknown status"
        );

        let props = ActivityFeedProps::default()
            .with_title("Recent")
            .with_empty_message("Nothing")
            .with_online_count(4)
            .with_show_typing_indicator(true)
            .with_class("panel")
            .with_attribute("data-testid", "feed");
        assert!(props.build_classes().contains("panel"));
        assert_eq!(feedTitle(&props), "Recent");
        assert_eq!(feedEmptyMessage(&props), "Nothing");
        assert_eq!(
            props
                .build_attributes()
                .get("data-testid")
                .map(String::as_str),
            Some("feed")
        );
    }

    #[test]
    fn activity_items_render_link_plain_avatar_icon_target_and_escape_content() {
        let linked = ActivityItemProps::default()
            .with_id("a1")
            .with_username("<Alice>")
            .with_avatar_src("/avatar.png")
            .with_action("opened")
            .with_target("<Doc>")
            .with_timestamp("now")
            .with_href("/a")
            .with_icon("★")
            .with_attribute("data-testid", "activity");
        let html = ActivityItem(&linked).into_string();
        assert!(html.starts_with("<a href=\"/a\" id=\"a1\" class=\"flex items-center"));
        assert!(html.contains("data-testid=\"activity\""));
        assert!(html.contains("<span class=\"text-lg flex-shrink-0\">★</span>"));
        assert!(html.contains("<img src=\"/avatar.png\" alt=\"&lt;Alice&gt;\""));
        assert!(html.contains("<span class=\"font-medium\">&lt;Alice&gt;</span>"));
        assert!(html.contains("<span class=\"font-medium\"> &lt;Doc&gt;</span>"));
        assert!(
            html.contains("<span class=\"text-xs text-muted-foreground flex-shrink-0\">now</span>")
        );
        assert!(html.contains("<path d=\"m9 18 6-6-6-6\"></path>"));

        let plain = ActivityItem(
            &ActivityItemProps::default()
                .with_username("Bob")
                .with_action("joined"),
        )
        .into_string();
        assert!(plain.starts_with("<div class=\"flex items-center"));
        assert!(plain.contains(">B</div>"));
        assert!(!plain.contains("<a href="));
    }

    #[test]
    fn activity_feed_renders_items_empty_state_typing_status_and_children() {
        let item = ActivityItemProps::default()
            .with_username("Alice")
            .with_action("joined");
        let props = ActivityFeedProps::default()
            .with_title("Recent")
            .with_items(vec![item])
            .with_online_count(2)
            .with_connection_status(CONNECTING)
            .with_show_typing_indicator(true)
            .with_class("extra")
            .with_attribute("data-testid", "feed");
        let html = ActivityFeed(&props, html! { footer { "Child" } }).into_string();
        assert!(
            html.starts_with("<div class=\"flex flex-col rounded-xl border border-border extra\"")
        );
        assert!(html.contains("data-testid=\"feed\""));
        assert!(html.contains("<h3 class=\"font-semibold\">Recent</h3>"));
        assert!(html.contains("<span>2 online</span>"));
        assert!(html.contains("Connecting..."));
        assert!(html.contains("Someone is typing..."));
        assert!(html.contains("<footer>Child</footer>"));

        let empty = ActivityFeed(
            &ActivityFeedProps::default().with_empty_message("No activity"),
            html! {},
        )
        .into_string();
        assert!(empty.contains("<p class=\"text-sm\">No activity</p>"));

        let simple = SimpleActivityFeed("Simple", Vec::new(), "simple-class").into_string();
        assert!(simple.contains("Simple"));
        assert!(simple.contains("simple-class"));
    }
}
