#![allow(non_snake_case)]
#![allow(clippy::needless_pass_by_value)]

use crate::components::ComponentInfo;
use crate::components::shared;
use crate::components::unstyled::tooltip as base;
use maud::Markup;

pub use base::{
    BottomTooltip, LeftTooltip, RightTooltip, SimpleTooltip, Tooltip, TooltipContentProps,
    TooltipProps, TooltipTrigger, TooltipTriggerProps, TooltipWithTrigger, TopTooltip,
};

pub const COMPONENTS: &[ComponentInfo] = &[
    ComponentInfo {
        family: "neomorphism",
        module: "tooltip",
        name: "Tooltip",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "tooltip",
        name: "TooltipTrigger",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "tooltip",
        name: "TooltipContent",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "tooltip",
        name: "SimpleTooltip",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "tooltip",
        name: "TooltipWithTrigger",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "tooltip",
        name: "TopTooltip",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "tooltip",
        name: "RightTooltip",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "tooltip",
        name: "BottomTooltip",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "tooltip",
        name: "LeftTooltip",
    },
];

#[must_use]
fn addNeomorphClasses(props: &TooltipContentProps) -> TooltipContentProps {
    let mut props = props.clone();
    props.class = shared::build_classes(&[
        props.class.as_str(),
        "neomorph-glass bg-background/95 text-foreground border border-border/50",
    ]);
    props
}

#[must_use]
pub fn TooltipContent(props: &TooltipContentProps, children: Markup) -> Markup {
    let neo_props = addNeomorphClasses(props);
    base::TooltipContent(&neo_props, children)
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
        assert_eq!(COMPONENTS.len(), 9);
        assert!(COMPONENTS.iter().all(|component| component.family == "neomorphism" && component.module == "tooltip"));
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "Tooltip")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "TooltipTrigger")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "TooltipContent")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "SimpleTooltip")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "TooltipWithTrigger")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "TopTooltip")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "RightTooltip")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "BottomTooltip")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "LeftTooltip")
        );
    }

    #[test]
    fn wrapped_components_render_unstyled_html_with_neomorphism_classes() {
        let html = TooltipContent(&TooltipContentProps::default(), html! { "child" }).into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.starts_with("<"), "expected rendered element: {html}");
        assert!(
            html.contains("neomorph-glass"),
            "missing neomorph-glass: {html}"
        );
        assert!(
            html.contains("bg-background/95"),
            "missing bg-background/95: {html}"
        );
        assert!(
            html.contains("text-foreground"),
            "missing text-foreground: {html}"
        );
        assert!(html.contains("border"), "missing border: {html}");
        assert!(
            html.contains("border-border/50"),
            "missing border-border/50: {html}"
        );
    }
}
