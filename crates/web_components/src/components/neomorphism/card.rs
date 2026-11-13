#![allow(non_snake_case)]
#![allow(clippy::needless_pass_by_value)]

use crate::components::ComponentInfo;
use crate::components::shared;
use crate::components::unstyled::card as base;
use maud::{Markup, html};

pub use base::{
    CardContent, CardContentProps, CardDescription, CardDescriptionProps, CardFooter,
    CardFooterProps, CardHeader, CardHeaderProps, CardProps, CardTitle, CardTitleProps,
    CardWithFooter, FeatureCard, InteractiveCard, LinkCard, ProductCard, SimpleCard, StatsCard,
};

pub const COMPONENTS: &[ComponentInfo] = &[
    ComponentInfo {
        family: "neomorphism",
        module: "card",
        name: "Card",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "card",
        name: "CardHeader",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "card",
        name: "CardTitle",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "card",
        name: "CardDescription",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "card",
        name: "CardContent",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "card",
        name: "CardFooter",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "card",
        name: "SimpleCard",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "card",
        name: "CardWithFooter",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "card",
        name: "InteractiveCard",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "card",
        name: "LinkCard",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "card",
        name: "FlatCard",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "card",
        name: "GlassCard",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "card",
        name: "StatsCard",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "card",
        name: "FeatureCard",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "card",
        name: "ProductCard",
    },
];

#[must_use]
fn addNeomorphClasses(props: &CardProps) -> CardProps {
    let mut props = props.clone();
    props.class = shared::build_classes(&[props.class.as_str(), "neomorph-card"]);
    props
}

#[must_use]
pub fn Card(props: &CardProps, children: Markup) -> Markup {
    let neo_props = addNeomorphClasses(props);
    base::Card(&neo_props, children)
}

#[must_use]
pub fn FlatCard(title: &str, description: &str, children: Markup) -> Markup {
    Card(
        &CardProps::default().with_class("neomorph-card-flat"),
        html! {
            (CardHeader(&CardHeaderProps::default(), html! {
                (CardTitle(&CardTitleProps::default(), html! { (title) }))
                @if !description.is_empty() { (CardDescription(&CardDescriptionProps::default(), html! { (description) })) }
            }))
            (CardContent(&CardContentProps::default(), children))
        },
    )
}

#[must_use]
pub fn GlassCard(title: &str, description: &str, children: Markup) -> Markup {
    Card(
        &CardProps::default().with_class("neomorph-glass"),
        html! {
            (CardHeader(&CardHeaderProps::default(), html! {
                (CardTitle(&CardTitleProps::default(), html! { (title) }))
                @if !description.is_empty() { (CardDescription(&CardDescriptionProps::default(), html! { (description) })) }
            }))
            (CardContent(&CardContentProps::default(), children))
        },
    )
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
        assert_eq!(COMPONENTS.len(), 15);
        assert!(
            COMPONENTS
                .iter()
                .all(|component| component.family == "neomorphism" && component.module == "card")
        );
        assert!(COMPONENTS.iter().any(|component| component.name == "Card"));
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "CardHeader")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "CardTitle")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "CardDescription")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "CardContent")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "CardFooter")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "SimpleCard")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "CardWithFooter")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "InteractiveCard")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "LinkCard")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "FlatCard")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "GlassCard")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "StatsCard")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "FeatureCard")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "ProductCard")
        );
    }

    #[test]
    fn wrapped_components_render_unstyled_html_with_neomorphism_classes() {
        let html = Card(&CardProps::default(), html! { "child" }).into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.starts_with("<div"));
        assert!(
            html.contains("rounded-lg border bg-card text-card-foreground shadow-sm transition-all duration-200 ease-out neomorph-card")
        );
        assert!(html.contains(">child</div>"));
    }

    #[test]
    fn neomorphism_only_components_render_html_without_generated_stub_markers() {
        let html = FlatCard("sample", "sample", html! { "child" }).into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.starts_with("<div"));
        assert!(html.contains("neomorph-card-flat neomorph-card"));
        assert!(html.contains("<h3"));
        assert!(html.contains(">sample</h3>"));
        assert!(html.contains("<p class=\"text-sm text-muted-foreground\">sample</p>"));
        assert!(html.contains("<div class=\"p-6 pt-0\">child</div>"));
        let html = GlassCard("sample", "sample", html! { "child" }).into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.starts_with("<div"));
        assert!(html.contains("neomorph-glass neomorph-card"));
        assert!(html.contains("<h3"));
        assert!(html.contains(">sample</h3>"));
        assert!(html.contains("<p class=\"text-sm text-muted-foreground\">sample</p>"));
        assert!(html.contains("<div class=\"p-6 pt-0\">child</div>"));
    }
}
