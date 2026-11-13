#![allow(non_snake_case)]

use crate::components::unstyled::durationbadge::{self, DurationBadgeProps};
use crate::components::{ComponentInfo, shared};
use maud::{Markup, html};
use std::collections::BTreeMap;

pub const COMPONENTS: &[ComponentInfo] = &[
    ComponentInfo {
        family: "unstyled",
        module: "democard",
        name: "DemoCard",
    },
    ComponentInfo {
        family: "unstyled",
        module: "democard",
        name: "SimpleDemoCard",
    },
];

#[derive(Clone, Debug)]
pub struct DemoCardProps {
    pub title: String,
    pub description: String,
    pub duration: String,
    pub icon: Option<Markup>,
    pub thumbnail_url: String,
    pub active: bool,
    pub on_click: String,
    pub class: String,
    pub attributes: BTreeMap<String, String>,
}

impl Default for DemoCardProps {
    fn default() -> Self {
        Self {
            title: String::new(),
            description: String::new(),
            duration: String::new(),
            icon: None,
            thumbnail_url: String::new(),
            active: false,
            on_click: String::new(),
            class: String::new(),
            attributes: BTreeMap::new(),
        }
    }
}

impl DemoCardProps {
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
    pub fn with_duration(mut self, value: impl Into<String>) -> Self {
        self.duration = value.into();
        self
    }

    #[must_use]
    pub fn with_icon(mut self, icon: Markup) -> Self {
        self.icon = Some(icon);
        self
    }

    #[must_use]
    pub fn with_thumbnail(mut self, value: impl Into<String>) -> Self {
        self.thumbnail_url = value.into();
        self
    }

    #[must_use]
    pub const fn with_active(mut self, value: bool) -> Self {
        self.active = value;
        self
    }

    #[must_use]
    pub fn with_on_click(mut self, value: impl Into<String>) -> Self {
        self.on_click = value.into();
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
            "p-4 cursor-pointer transition-all duration-150 rounded-lg",
            if self.active {
                "ring-2 ring-primary"
            } else {
                ""
            },
            self.class.as_str(),
        ])
    }

    #[must_use]
    pub fn build_attributes(&self) -> BTreeMap<String, String> {
        let mut attrs = self.attributes.clone();
        if !self.on_click.is_empty() {
            attrs.insert("data-on:click".to_string(), self.on_click.clone());
        }
        attrs
    }
}

fn duration_badge(duration: &str) -> Markup {
    durationbadge::DurationBadge(&DurationBadgeProps::default().with_duration(duration))
}

#[must_use]
pub fn DemoCard(props: &DemoCardProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        div
            class=(props.build_classes())
            id=[shared::attr_value(&attrs, "id")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
            data-on:click=[shared::attr_value(&attrs, "data-on:click")]
        {
            div class="flex items-start gap-3" {
                @if !props.thumbnail_url.is_empty() {
                    div class="relative w-16 h-16 rounded-lg overflow-hidden flex-shrink-0" {
                        img src=(&props.thumbnail_url) alt=(&props.title) class="w-full h-full object-cover";
                        @if !props.duration.is_empty() {
                            div class="absolute bottom-1 right-1" {
                                (duration_badge(&props.duration))
                            }
                        }
                    }
                } @else if let Some(icon) = &props.icon {
                    div class="w-12 h-12 flex items-center justify-center flex-shrink-0 rounded-lg" {
                        (icon.clone())
                    }
                }
                div class="flex-1 min-w-0" {
                    h4 class="font-semibold" { (&props.title) }
                    @if !props.description.is_empty() {
                        p class="text-sm mt-1 line-clamp-2" {
                            (&props.description)
                        }
                    }
                    @if !props.duration.is_empty() && props.thumbnail_url.is_empty() {
                        div class="mt-2" {
                            (duration_badge(&props.duration))
                        }
                    }
                }
            }
        }
    }
}

#[must_use]
pub fn SimpleDemoCard(title: &str, description: &str) -> Markup {
    DemoCard(
        &DemoCardProps::default()
            .with_title(title)
            .with_description(description),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn helpers_cover_defaults_active_classes_and_click_attrs() {
        let props = DemoCardProps::default();
        assert!(!props.active);
        assert_eq!(
            props.build_classes(),
            "p-4 cursor-pointer transition-all duration-150 rounded-lg"
        );
        assert_eq!(
            DemoCardProps::default()
                .with_active(true)
                .with_class("bg-card")
                .build_classes(),
            "p-4 cursor-pointer transition-all duration-150 rounded-lg ring-2 ring-primary bg-card"
        );
        let attrs = DemoCardProps::default()
            .with_attribute("id", "demo")
            .with_on_click("$demo = 1")
            .build_attributes();
        assert_eq!(attrs.get("id").map(String::as_str), Some("demo"));
        assert_eq!(
            attrs.get("data-on:click").map(String::as_str),
            Some("$demo = 1")
        );
    }

    #[test]
    fn demo_card_renders_thumbnail_duration_content_attrs_and_escapes() {
        let html = DemoCard(
            &DemoCardProps::default()
                .with_title("<Demo>")
                .with_description("<Description>")
                .with_duration("<3:45>")
                .with_thumbnail("/thumb.jpg")
                .with_active(true)
                .with_on_click("$selected = 2")
                .with_attribute("id", "demo-card")
                .with_attribute("data-testid", "demo"),
        )
        .into_string();
        assert!(html.starts_with("<div class=\"p-4 cursor-pointer transition-all duration-150 rounded-lg ring-2 ring-primary\""));
        assert!(html.contains("id=\"demo-card\""));
        assert!(html.contains("data-testid=\"demo\""));
        assert!(html.contains("data-on:click=\"$selected = 2\""));
        assert!(html.contains(
            "<img src=\"/thumb.jpg\" alt=\"&lt;Demo&gt;\" class=\"w-full h-full object-cover\">"
        ));
        assert!(html.contains(
            "<div class=\"absolute bottom-1 right-1\"><span class=\"inline-flex items-center gap-1"
        ));
        assert!(html.contains("<h4 class=\"font-semibold\">&lt;Demo&gt;</h4>"));
        assert!(html.contains("<p class=\"text-sm mt-1 line-clamp-2\">&lt;Description&gt;</p>"));
        assert!(!html.contains("<div class=\"mt-2\">"));
    }

    #[test]
    fn demo_card_renders_icon_duration_without_thumbnail_and_simple_variant() {
        let html = DemoCard(
            &DemoCardProps::default()
                .with_title("Demo")
                .with_duration("1:23")
                .with_icon(html! { svg data-testid="play" {} }),
        )
        .into_string();
        assert!(html.contains("<div class=\"w-12 h-12 flex items-center justify-center flex-shrink-0 rounded-lg\"><svg data-testid=\"play\"></svg></div>"));
        assert!(html.contains("<div class=\"mt-2\"><span class=\"inline-flex items-center gap-1"));
        assert!(html.contains("<span>1:23</span>"));
        assert!(!html.contains("<p class=\"text-sm mt-1 line-clamp-2\""));

        let simple = SimpleDemoCard("Title", "Description").into_string();
        assert!(simple.contains("<h4 class=\"font-semibold\">Title</h4>"));
        assert!(simple.contains("<p class=\"text-sm mt-1 line-clamp-2\">Description</p>"));
        assert!(!simple.contains("<img "));
        assert!(!simple.contains("stat-icon"));
    }
}
