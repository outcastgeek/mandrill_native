#![allow(non_snake_case)]

use crate::components::{ComponentInfo, shared};
use maud::{Markup, html};
use std::collections::BTreeMap;

pub const COMPONENTS: &[ComponentInfo] = &[
    ComponentInfo {
        family: "unstyled",
        module: "emptystate",
        name: "EmptyState",
    },
    ComponentInfo {
        family: "unstyled",
        module: "emptystate",
        name: "SimpleEmptyState",
    },
    ComponentInfo {
        family: "unstyled",
        module: "emptystate",
        name: "EmptyStateWithAction",
    },
    ComponentInfo {
        family: "unstyled",
        module: "emptystate",
        name: "NoDataEmptyState",
    },
    ComponentInfo {
        family: "unstyled",
        module: "emptystate",
        name: "NoResultsEmptyState",
    },
];

pub const VARIANT_INLINE: &str = "inline";
pub const VARIANT_FULL_PAGE: &str = "fullpage";
pub const VARIANT_CARD: &str = "card";

#[derive(Clone, Debug)]
pub struct EmptyStateProps {
    pub variant: &'static str,
    pub icon: String,
    pub icon_content: Option<Markup>,
    pub icon_class: String,
    pub title: String,
    pub description: String,
    pub action_label: String,
    pub action_href: String,
    pub action_on_click: String,
    pub class: String,
    pub attributes: BTreeMap<String, String>,
}

impl Default for EmptyStateProps {
    fn default() -> Self {
        Self {
            variant: VARIANT_INLINE,
            icon: String::new(),
            icon_content: Some(mail_icon("text-muted-foreground")),
            icon_class: String::new(),
            title: String::new(),
            description: String::new(),
            action_label: String::new(),
            action_href: String::new(),
            action_on_click: String::new(),
            class: String::new(),
            attributes: BTreeMap::new(),
        }
    }
}

impl EmptyStateProps {
    #[must_use]
    pub const fn with_variant(mut self, variant: &'static str) -> Self {
        self.variant = variant;
        self
    }

    #[must_use]
    pub fn with_icon(mut self, icon: impl Into<String>) -> Self {
        self.icon = icon.into();
        self.icon_content = None;
        self
    }

    #[must_use]
    pub fn with_icon_content(mut self, icon: Markup) -> Self {
        self.icon_content = Some(icon);
        self
    }

    #[must_use]
    pub fn with_icon_class(mut self, class: impl Into<String>) -> Self {
        self.icon_class = class.into();
        self
    }

    #[must_use]
    pub fn with_title(mut self, title: impl Into<String>) -> Self {
        self.title = title.into();
        self
    }

    #[must_use]
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = description.into();
        self
    }

    #[must_use]
    pub fn with_action(mut self, label: impl Into<String>, href: impl Into<String>) -> Self {
        self.action_label = label.into();
        self.action_href = href.into();
        self
    }

    #[must_use]
    pub fn with_action_on_click(
        mut self,
        label: impl Into<String>,
        on_click: impl Into<String>,
    ) -> Self {
        self.action_label = label.into();
        self.action_on_click = on_click.into();
        self
    }

    #[must_use]
    pub fn with_class(mut self, class: impl Into<String>) -> Self {
        self.class = class.into();
        self
    }

    #[must_use]
    pub fn with_attribute(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.attributes.insert(name.into(), value.into());
        self
    }

    #[must_use]
    pub fn build_classes(&self) -> String {
        let variant_class = match self.variant {
            VARIANT_FULL_PAGE => "min-h-[400px] py-12 px-6",
            VARIANT_CARD => "p-8 rounded-xl border border-border",
            _ => "py-8 px-4",
        };
        shared::build_classes(&[
            "flex flex-col items-center justify-center text-center",
            variant_class,
            self.class.as_str(),
        ])
    }

    #[must_use]
    pub fn build_attributes(&self) -> BTreeMap<String, String> {
        self.attributes.clone()
    }
}

#[must_use]
pub fn empty_state_icon_class(props: &EmptyStateProps) -> String {
    if props.icon_class.is_empty() {
        "w-16 h-16 rounded-full bg-muted/50 flex items-center justify-center mb-4".to_string()
    } else {
        format!("{} flex items-center justify-center mb-4", props.icon_class)
    }
}

fn mail_icon(class: &str) -> Markup {
    html! {
        svg class=(shared::build_classes(&["w-8 h-8", class])) fill="none" stroke="currentColor" viewBox="0 0 24 24" aria-hidden="true" {
            path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M3 8l7.89 5.26a2 2 0 002.22 0L21 8" {}
            path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M5 19h14a2 2 0 002-2V7a2 2 0 00-2-2H5a2 2 0 00-2 2v10a2 2 0 002 2z" {}
        }
    }
}

fn search_icon(class: &str) -> Markup {
    html! {
        svg class=(shared::build_classes(&["w-8 h-8", class])) fill="none" stroke="currentColor" viewBox="0 0 24 24" aria-hidden="true" {
            path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M21 21l-4.35-4.35" {}
            circle cx="11" cy="11" r="7" stroke-width="2" {}
        }
    }
}

fn bar_chart_icon(class: &str) -> Markup {
    html! {
        svg class=(shared::build_classes(&["w-8 h-8", class])) fill="none" stroke="currentColor" viewBox="0 0 24 24" aria-hidden="true" {
            path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M3 3v18h18" {}
            path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M7 16V8m5 8V5m5 11v-6" {}
        }
    }
}

#[must_use]
pub fn EmptyState(props: &EmptyStateProps, children: Markup) -> Markup {
    let attrs = props.build_attributes();
    html! {
        div
            class=(props.build_classes())
            id=[shared::attr_value(&attrs, "id")]
            title=[shared::attr_value(&attrs, "title")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
        {
            @if let Some(icon) = &props.icon_content {
                div class=(empty_state_icon_class(props)) { (icon) }
            } @else if !props.icon.is_empty() {
                div class=(empty_state_icon_class(props)) {
                    span class="text-3xl" { (&props.icon) }
                }
            }
            @if !props.title.is_empty() {
                h2 class="font-semibold text-lg mb-2 text-foreground" { (&props.title) }
            }
            @if !props.description.is_empty() {
                p class="text-muted-foreground text-sm max-w-sm" { (&props.description) }
            }
            @if !props.action_label.is_empty() {
                div class="mt-4" {
                    @if !props.action_href.is_empty() {
                        a
                            href=(&props.action_href)
                            class="inline-flex items-center gap-2 px-4 py-2 rounded-lg bg-primary text-primary-foreground text-sm font-medium hover:bg-primary/90 transition-colors duration-150"
                        { (&props.action_label) }
                    } @else {
                        button
                            type="button"
                            class="inline-flex items-center gap-2 px-4 py-2 rounded-lg bg-primary text-primary-foreground text-sm font-medium hover:bg-primary/90 transition-colors duration-150"
                            onclick=[if props.action_on_click.is_empty() { None } else { Some(props.action_on_click.as_str()) }]
                        { (&props.action_label) }
                    }
                }
            }
            (children)
        }
    }
}

#[must_use]
pub fn SimpleEmptyState(
    icon: impl Into<String>,
    title: impl Into<String>,
    description: impl Into<String>,
    class: impl Into<String>,
) -> Markup {
    EmptyState(
        &EmptyStateProps::default()
            .with_icon(icon)
            .with_title(title)
            .with_description(description)
            .with_class(class),
        html! {},
    )
}

#[must_use]
pub fn EmptyStateWithAction(
    icon: impl Into<String>,
    title: impl Into<String>,
    description: impl Into<String>,
    action_label: impl Into<String>,
    action_href: impl Into<String>,
    class: impl Into<String>,
) -> Markup {
    EmptyState(
        &EmptyStateProps::default()
            .with_icon(icon)
            .with_title(title)
            .with_description(description)
            .with_action(action_label, action_href)
            .with_class(class),
        html! {},
    )
}

#[must_use]
pub fn NoDataEmptyState(message: impl Into<String>, class: impl Into<String>) -> Markup {
    EmptyState(
        &EmptyStateProps::default()
            .with_icon_content(bar_chart_icon("text-muted-foreground"))
            .with_title(shared::default_strings().empty_state_no_data)
            .with_description(message)
            .with_variant(VARIANT_INLINE)
            .with_class(class),
        html! {},
    )
}

#[must_use]
pub fn NoResultsEmptyState(query: impl Into<String>, class: impl Into<String>) -> Markup {
    let query = query.into();
    let description = if query.is_empty() {
        shared::default_strings().empty_state_no_results.to_string()
    } else {
        shared::default_strings()
            .empty_state_no_results_query
            .replace("%s", &query)
    };
    EmptyState(
        &EmptyStateProps::default()
            .with_icon_content(search_icon("text-muted-foreground"))
            .with_title(shared::default_strings().empty_state_no_results)
            .with_description(description)
            .with_variant(VARIANT_INLINE)
            .with_class(class),
        html! {},
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_props_and_helpers_match_go_defaults() {
        let props = EmptyStateProps::default();
        assert_eq!(props.variant, VARIANT_INLINE);
        assert!(props.icon_content.is_some());
        assert!(props.icon.is_empty());
        assert_eq!(
            props.build_classes(),
            "flex flex-col items-center justify-center text-center py-8 px-4"
        );
        assert_eq!(
            empty_state_icon_class(&props),
            "w-16 h-16 rounded-full bg-muted/50 flex items-center justify-center mb-4"
        );
    }

    #[test]
    fn helper_methods_cover_variants_custom_icon_class_actions_and_attrs() {
        for (variant, expected) in [
            (VARIANT_INLINE, "py-8 px-4"),
            (VARIANT_FULL_PAGE, "min-h-[400px] py-12 px-6"),
            (VARIANT_CARD, "p-8 rounded-xl border border-border"),
            ("bad", "py-8 px-4"),
        ] {
            assert!(
                EmptyStateProps::default()
                    .with_variant(variant)
                    .build_classes()
                    .contains(expected)
            );
        }

        let props = EmptyStateProps::default()
            .with_icon("!")
            .with_icon_class("custom-icon")
            .with_title("Title")
            .with_description("Desc")
            .with_action("Go", "/go")
            .with_action_on_click("Click", "doThing()")
            .with_class("extra")
            .with_attribute("title", "Empty");
        assert_eq!(props.icon, "!");
        assert!(props.icon_content.is_none());
        assert_eq!(
            empty_state_icon_class(&props),
            "custom-icon flex items-center justify-center mb-4"
        );
        assert!(props.build_classes().contains("extra"));
        assert_eq!(
            props.build_attributes().get("title").map(String::as_str),
            Some("Empty")
        );
        assert_eq!(props.action_label, "Click");
        assert_eq!(props.action_on_click, "doThing()");
    }

    #[test]
    fn empty_state_renders_icon_title_description_action_children_and_escaping() {
        let html = EmptyState(
            &EmptyStateProps::default()
                .with_icon("<>")
                .with_title("<Title>")
                .with_description("<Desc>")
                .with_action("Open", "/open?<x>")
                .with_attribute("data-testid", "empty"),
            html! { span { "<child>" } },
        )
        .into_string();

        assert!(html.starts_with("<div class=\"flex flex-col"));
        assert!(html.contains("data-testid=\"empty\""));
        assert!(html.contains("<span class=\"text-3xl\">&lt;&gt;</span>"));
        assert!(html.contains(
            "<h2 class=\"font-semibold text-lg mb-2 text-foreground\">&lt;Title&gt;</h2>"
        ));
        assert!(
            html.contains("<p class=\"text-muted-foreground text-sm max-w-sm\">&lt;Desc&gt;</p>")
        );
        assert!(html.contains("href=\"/open?&lt;x&gt;\""));
        assert!(html.contains(">Open</a>"));
        assert!(html.contains("<span>&lt;child&gt;</span>"));

        let button = EmptyState(
            &EmptyStateProps::default().with_action_on_click("Click", "run('<x>')"),
            html! {},
        )
        .into_string();
        assert!(button.contains("onclick=\"run('&lt;x&gt;')\""));
        assert!(button.contains(">Click</button>"));
    }

    #[test]
    fn convenience_empty_states_render_expected_variants() {
        assert!(
            SimpleEmptyState("!", "Title", "Desc", "extra")
                .into_string()
                .contains("extra")
        );
        assert!(
            EmptyStateWithAction("!", "Title", "Desc", "Act", "/act", "")
                .into_string()
                .contains("href=\"/act\"")
        );

        let no_data = NoDataEmptyState("Nothing here", "extra").into_string();
        assert!(no_data.contains("No Data"));
        assert!(no_data.contains("Nothing here"));
        assert!(no_data.contains("extra"));

        let no_results = NoResultsEmptyState("abc", "").into_string();
        assert!(no_results.contains("No results"));
        assert!(
            no_results.contains("No results found for &quot;abc&quot;. Try adjusting your search.")
        );
        let no_results_default = NoResultsEmptyState("", "").into_string();
        assert!(no_results_default.contains("No Results Found"));
    }
}
