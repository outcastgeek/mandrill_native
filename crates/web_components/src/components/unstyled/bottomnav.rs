#![allow(non_snake_case)]

use crate::components::{ComponentInfo, shared};
use maud::{Markup, html};
use std::collections::BTreeMap;

pub const COMPONENTS: &[ComponentInfo] = &[
    ComponentInfo {
        family: "unstyled",
        module: "bottomnav",
        name: "BottomNavItem",
    },
    ComponentInfo {
        family: "unstyled",
        module: "bottomnav",
        name: "BottomNav",
    },
    ComponentInfo {
        family: "unstyled",
        module: "bottomnav",
        name: "SimpleBottomNav",
    },
    ComponentInfo {
        family: "unstyled",
        module: "bottomnav",
        name: "SimpleBottomNavItem",
    },
];

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct BottomNavItemProps {
    pub href: String,
    pub label: String,
    pub icon: String,
    pub is_active: bool,
    pub badge: String,
    pub class: String,
    pub attributes: BTreeMap<String, String>,
}

impl BottomNavItemProps {
    #[must_use]
    pub fn with_href(mut self, value: impl Into<String>) -> Self {
        self.href = value.into();
        self
    }

    #[must_use]
    pub fn with_label(mut self, value: impl Into<String>) -> Self {
        self.label = value.into();
        self
    }

    #[must_use]
    pub fn with_icon(mut self, value: impl Into<String>) -> Self {
        self.icon = value.into();
        self
    }

    #[must_use]
    pub const fn with_active(mut self, value: bool) -> Self {
        self.is_active = value;
        self
    }

    #[must_use]
    pub fn with_badge(mut self, value: impl Into<String>) -> Self {
        self.badge = value.into();
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
            "flex flex-col items-center justify-center gap-1 py-2 px-3 text-xs transition-colors duration-150",
            if self.is_active {
                "text-primary"
            } else {
                "text-muted-foreground hover:text-foreground"
            },
            self.class.as_str(),
        ])
    }

    #[must_use]
    pub fn build_attributes(&self) -> BTreeMap<String, String> {
        self.attributes.clone()
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct BottomNavProps {
    pub items: Vec<BottomNavItemProps>,
    pub class: String,
    pub attributes: BTreeMap<String, String>,
}

impl BottomNavProps {
    #[must_use]
    pub fn with_items(mut self, value: Vec<BottomNavItemProps>) -> Self {
        self.items = value;
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
            "fixed bottom-0 left-0 right-0 bg-background border-t border-border safe-area-inset-bottom",
            self.class.as_str(),
        ])
    }

    #[must_use]
    pub fn build_attributes(&self) -> BTreeMap<String, String> {
        self.attributes.clone()
    }
}

#[must_use]
pub fn BottomNavItem(props: &BottomNavItemProps, children: Markup) -> Markup {
    let attrs = props.build_attributes();
    html! {
        a
            href=(&props.href)
            class=(props.build_classes())
            aria-current=[if props.is_active { Some("page") } else { None }]
            id=[shared::attr_value(&attrs, "id")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
        {
            div class="relative" {
                @if !props.icon.is_empty() {
                    span class="text-lg" { (&props.icon) }
                } @else {
                    (children)
                }
                @if !props.badge.is_empty() {
                    span class="absolute -top-1 -right-1 min-w-[16px] h-4 px-1 text-[10px] font-bold bg-destructive text-destructive-foreground rounded-full flex items-center justify-center" {
                        (&props.badge)
                    }
                }
            }
            @if !props.label.is_empty() {
                span { (&props.label) }
            }
        }
    }
}

#[must_use]
pub fn BottomNav(props: &BottomNavProps, children: Markup) -> Markup {
    let attrs = props.build_attributes();
    html! {
        nav
            class=(props.build_classes())
            role="navigation"
            aria-label=(shared::default_strings().bottom_nav_aria_label)
            id=[shared::attr_value(&attrs, "id")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
        {
            div class="flex items-center justify-around max-w-lg mx-auto" {
                @for item in &props.items {
                    (BottomNavItem(item, html! {}))
                }
                (children)
            }
        }
    }
}

#[must_use]
pub fn SimpleBottomNav(items: Vec<BottomNavItemProps>, class: &str) -> Markup {
    BottomNav(
        &BottomNavProps::default()
            .with_items(items)
            .with_class(class),
        html! {},
    )
}

#[must_use]
pub fn SimpleBottomNavItem(icon: &str, label: &str, href: &str, is_active: bool) -> Markup {
    BottomNavItem(
        &BottomNavItemProps::default()
            .with_icon(icon)
            .with_label(label)
            .with_href(href)
            .with_active(is_active),
        html! {},
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn helpers_cover_item_and_nav_classes_attrs() {
        assert_eq!(
            BottomNavItemProps::default().build_classes(),
            "flex flex-col items-center justify-center gap-1 py-2 px-3 text-xs transition-colors duration-150 text-muted-foreground hover:text-foreground"
        );
        assert_eq!(
            BottomNavItemProps::default()
                .with_active(true)
                .with_class("custom")
                .build_classes(),
            "flex flex-col items-center justify-center gap-1 py-2 px-3 text-xs transition-colors duration-150 text-primary custom"
        );
        let item_attrs = BottomNavItemProps::default()
            .with_attribute("id", "home")
            .build_attributes();
        assert_eq!(item_attrs.get("id").map(String::as_str), Some("home"));
        assert_eq!(
            BottomNavProps::default().build_classes(),
            "fixed bottom-0 left-0 right-0 bg-background border-t border-border safe-area-inset-bottom"
        );
        let nav_attrs = BottomNavProps::default()
            .with_class("z-50")
            .with_attribute("data-testid", "bottom-nav")
            .build_attributes();
        assert_eq!(
            nav_attrs.get("data-testid").map(String::as_str),
            Some("bottom-nav")
        );
    }

    #[test]
    fn bottom_nav_item_renders_active_badge_icon_child_and_escaped_text() {
        let active = BottomNavItem(
            &BottomNavItemProps::default()
                .with_href("/home")
                .with_label("<Home>")
                .with_icon("<H>")
                .with_badge("<3>")
                .with_active(true)
                .with_attribute("data-testid", "home-item"),
            html! { "Ignored" },
        )
        .into_string();
        assert!(active.starts_with("<a href=\"/home\""));
        assert!(active.contains("text-primary"));
        assert!(active.contains("aria-current=\"page\""));
        assert!(active.contains("data-testid=\"home-item\""));
        assert!(active.contains("<span class=\"text-lg\">&lt;H&gt;</span>"));
        assert!(active.contains(">&lt;3&gt;</span>"));
        assert!(active.contains("<span>&lt;Home&gt;</span>"));
        assert!(!active.contains("Ignored"));

        let child = BottomNavItem(
            &BottomNavItemProps::default().with_href("/custom"),
            html! { svg data-testid="custom-icon" {} },
        )
        .into_string();
        assert!(
            child.contains("<div class=\"relative\"><svg data-testid=\"custom-icon\"></svg></div>")
        );
        assert!(!child.contains("aria-current"));
    }

    #[test]
    fn bottom_nav_and_simple_variants_render_expected_html() {
        let item = BottomNavItemProps::default()
            .with_href("/search")
            .with_icon("S")
            .with_label("Search");
        let nav = BottomNav(
            &BottomNavProps::default()
                .with_items(vec![item])
                .with_class("z-50")
                .with_attribute("id", "mobile-nav"),
            html! { a href="/extra" { "Extra" } },
        )
        .into_string();
        assert!(nav.starts_with("<nav class=\"fixed bottom-0 left-0 right-0 bg-background border-t border-border safe-area-inset-bottom z-50\""));
        assert!(nav.contains("role=\"navigation\""));
        assert!(nav.contains("aria-label=\"Bottom navigation\""));
        assert!(nav.contains("id=\"mobile-nav\""));
        assert!(nav.contains("<div class=\"flex items-center justify-around max-w-lg mx-auto\">"));
        assert!(nav.contains("<span>Search</span>"));
        assert!(nav.contains("<a href=\"/extra\">Extra</a>"));

        let simple_nav = SimpleBottomNav(
            vec![
                BottomNavItemProps::default()
                    .with_href("/")
                    .with_label("Home"),
            ],
            "fixed",
        )
        .into_string();
        assert!(simple_nav.contains("safe-area-inset-bottom fixed"));
        assert!(simple_nav.contains(">Home</span>"));

        let simple_item = SimpleBottomNavItem("H", "Home", "/", true).into_string();
        assert!(simple_item.contains("href=\"/\""));
        assert!(simple_item.contains("aria-current=\"page\""));
        assert!(simple_item.contains("<span class=\"text-lg\">H</span>"));
    }
}
