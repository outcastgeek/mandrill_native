#![allow(non_snake_case)]
#![allow(clippy::needless_pass_by_value)]

use crate::components::ComponentInfo;
use crate::components::shared;
use crate::components::unstyled::skeleton as base;
use maud::{Markup, html};

pub use base::{
    SkeletonCard, SkeletonCircle, SkeletonList, SkeletonProps, SkeletonRectangle, SkeletonTable,
    SkeletonText,
};

pub const COMPONENTS: &[ComponentInfo] = &[
    ComponentInfo {
        family: "neomorphism",
        module: "skeleton",
        name: "Skeleton",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "skeleton",
        name: "SkeletonText",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "skeleton",
        name: "SkeletonCircle",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "skeleton",
        name: "SkeletonRectangle",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "skeleton",
        name: "SkeletonCard",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "skeleton",
        name: "SkeletonTable",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "skeleton",
        name: "SkeletonList",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "skeleton",
        name: "NeomorphCard",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "skeleton",
        name: "NeomorphList",
    },
];

#[must_use]
fn addNeomorphClasses(props: &SkeletonProps) -> SkeletonProps {
    let mut props = props.clone();
    props.class = shared::build_classes(&[props.class.as_str(), "skeleton-loader"]);
    props
}

#[must_use]
pub fn Skeleton(props: &SkeletonProps) -> Markup {
    let neo_props = addNeomorphClasses(props);
    base::Skeleton(&neo_props)
}

#[must_use]
pub fn NeomorphCard(animated: bool) -> Markup {
    html! {
        div class="neomorph-card p-6 space-y-4" role="status" aria-label="Loading" {
            div class="flex items-center space-x-4" {
                (SkeletonCircle("3rem", animated, ""))
                div class="flex-1 space-y-2" {
                    (SkeletonRectangle("60%", "1rem", animated, ""))
                    (SkeletonRectangle("40%", "0.875rem", animated, ""))
                }
            }
            div class="space-y-2" { (SkeletonText(3, animated, "")) }
            div class="flex space-x-2" {
                (SkeletonRectangle("5rem", "2rem", animated, "neomorph-button"))
                (SkeletonRectangle("5rem", "2rem", animated, "neomorph-button"))
            }
        }
    }
}

#[must_use]
pub fn NeomorphList(items: i32, animated: bool) -> Markup {
    html! {
        div class="space-y-4" role="status" aria-label="Loading list" {
            @for _ in 0..items.max(0) {
                div class="neomorph-card p-4 flex items-center space-x-3" {
                    (SkeletonCircle("2.5rem", animated, ""))
                    div class="flex-1 space-y-2" {
                        (SkeletonRectangle("70%", "1rem", animated, ""))
                        (SkeletonRectangle("50%", "0.875rem", animated, ""))
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
        assert!(
            COMPONENTS.iter().all(
                |component| component.family == "neomorphism" && component.module == "skeleton"
            )
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "Skeleton")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "SkeletonText")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "SkeletonCircle")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "SkeletonRectangle")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "SkeletonCard")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "SkeletonTable")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "SkeletonList")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "NeomorphCard")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "NeomorphList")
        );
    }

    #[test]
    fn wrapped_components_render_unstyled_html_with_neomorphism_classes() {
        let html = Skeleton(&SkeletonProps::default()).into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.starts_with("<div"));
        assert!(html.contains("animate-pulse"));
        assert!(html.contains("skeleton-loader"));
        assert!(html.contains("role=\"status\""));
        assert!(html.contains("aria-label=\"Loading\""));
        assert!(html.contains("aria-live=\"polite\""));
        assert!(html.contains("style=\"width: 100%; height: 1rem;\""));
    }

    #[test]
    fn neomorphism_only_components_render_html_without_generated_stub_markers() {
        let html = NeomorphCard(true).into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.starts_with("<div class=\"neomorph-card p-6 space-y-4\""));
        assert!(html.contains("role=\"status\""));
        assert!(html.contains("aria-label=\"Loading\""));
        assert!(html.matches("neomorph-button").count() >= 2);
        assert!(html.contains("style=\"width: 3rem; height: 3rem;\""));
        let html = NeomorphList(2, true).into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.starts_with("<div class=\"space-y-4\""));
        assert!(html.contains("aria-label=\"Loading list\""));
        assert_eq!(
            html.matches("neomorph-card p-4 flex items-center space-x-3")
                .count(),
            2
        );
        assert!(html.contains("style=\"width: 2.5rem; height: 2.5rem;\""));
    }
}
