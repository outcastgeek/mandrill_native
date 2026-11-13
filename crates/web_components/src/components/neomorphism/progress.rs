#![allow(non_snake_case)]
#![allow(clippy::needless_pass_by_value)]

use crate::components::ComponentInfo;
use crate::components::shared;
use crate::components::unstyled::progress as base;
use maud::Markup;

pub use base::{
    CircularProgress, IndeterminateProgress, InfoProgress, PrimaryProgress, ProgressProps,
    SuccessProgress, WarningProgress,
};

pub const COMPONENTS: &[ComponentInfo] = &[
    ComponentInfo {
        family: "neomorphism",
        module: "progress",
        name: "Progress",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "progress",
        name: "ProgressWithLabel",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "progress",
        name: "CircularProgress",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "progress",
        name: "PrimaryProgress",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "progress",
        name: "SuccessProgress",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "progress",
        name: "WarningProgress",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "progress",
        name: "InfoProgress",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "progress",
        name: "IndeterminateProgress",
    },
];

#[must_use]
fn addNeomorphClasses(props: &ProgressProps) -> ProgressProps {
    let mut props = props.clone();
    props.class = shared::build_classes(&[props.class.as_str(), "neomorph-concave"]);
    props
}

#[must_use]
pub fn Progress(props: &ProgressProps) -> Markup {
    let neo_props = addNeomorphClasses(props);
    base::Progress(&neo_props)
}

#[must_use]
pub fn ProgressWithLabel(props: &ProgressProps) -> Markup {
    let neo_props = addNeomorphClasses(props);
    base::ProgressWithLabel(&neo_props)
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
        assert_eq!(COMPONENTS.len(), 8);
        assert!(
            COMPONENTS.iter().all(
                |component| component.family == "neomorphism" && component.module == "progress"
            )
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "Progress")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "ProgressWithLabel")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "CircularProgress")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "PrimaryProgress")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "SuccessProgress")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "WarningProgress")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "InfoProgress")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "IndeterminateProgress")
        );
    }

    #[test]
    fn wrapped_components_render_unstyled_html_with_neomorphism_classes() {
        let html = Progress(&ProgressProps::default()).into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.starts_with("<"), "expected rendered element: {html}");
        assert!(
            html.contains("neomorph-concave"),
            "missing neomorph-concave: {html}"
        );
        let html = ProgressWithLabel(&ProgressProps::default()).into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.starts_with("<"), "expected rendered element: {html}");
        assert!(
            html.contains("neomorph-concave"),
            "missing neomorph-concave: {html}"
        );
    }
}
