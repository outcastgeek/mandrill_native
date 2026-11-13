#![allow(non_snake_case)]
#![allow(clippy::needless_pass_by_value)]

use crate::components::ComponentInfo;
use crate::components::shared;
use crate::components::unstyled::emptystate as base;
use maud::Markup;

pub use base::{
    EmptyStateProps, EmptyStateWithAction, NoDataEmptyState, NoResultsEmptyState, SimpleEmptyState,
    VARIANT_CARD, VARIANT_FULL_PAGE, VARIANT_INLINE,
};

pub const COMPONENTS: &[ComponentInfo] = &[
    ComponentInfo {
        family: "neomorphism",
        module: "emptystate",
        name: "EmptyState",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "emptystate",
        name: "SimpleEmptyState",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "emptystate",
        name: "EmptyStateWithAction",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "emptystate",
        name: "NoDataEmptyState",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "emptystate",
        name: "NoResultsEmptyState",
    },
];

#[must_use]
fn addNeomorphClasses(props: &EmptyStateProps) -> EmptyStateProps {
    let mut props = props.clone();
    props.class = shared::build_classes(&[props.class.as_str(), "neomorph-flat"]);
    props
}

#[must_use]
pub fn EmptyState(props: &EmptyStateProps, children: Markup) -> Markup {
    let neo_props = addNeomorphClasses(props);
    base::EmptyState(&neo_props, children)
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
        assert_eq!(COMPONENTS.len(), 5);
        assert!(
            COMPONENTS
                .iter()
                .all(|component| component.family == "neomorphism"
                    && component.module == "emptystate")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "EmptyState")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "SimpleEmptyState")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "EmptyStateWithAction")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "NoDataEmptyState")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "NoResultsEmptyState")
        );
    }

    #[test]
    fn wrapped_components_render_unstyled_html_with_neomorphism_classes() {
        let html = EmptyState(&EmptyStateProps::default(), html! { "child" }).into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.starts_with("<"), "expected rendered element: {html}");
        assert!(
            html.contains("neomorph-flat"),
            "missing neomorph-flat: {html}"
        );
    }
}
