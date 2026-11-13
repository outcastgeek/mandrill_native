#![allow(non_snake_case)]
#![allow(clippy::needless_pass_by_value)]

use crate::components::ComponentInfo;
use crate::components::shared;
use crate::components::unstyled::select as base;
use maud::Markup;

pub use base::{SelectOption, SelectProps, SelectWithLabel};

pub const COMPONENTS: &[ComponentInfo] = &[
    ComponentInfo {
        family: "neomorphism",
        module: "select",
        name: "Select",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "select",
        name: "SelectWithLabel",
    },
];

#[must_use]
fn addNeomorphClasses(props: &SelectProps) -> SelectProps {
    let mut props = props.clone();
    props.class = shared::build_classes(&[props.class.as_str(), "neomorph-input appearance-none"]);
    props
}

#[must_use]
pub fn Select(props: &SelectProps) -> Markup {
    let neo_props = addNeomorphClasses(props);
    base::Select(&neo_props)
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
        assert_eq!(COMPONENTS.len(), 2);
        assert!(
            COMPONENTS
                .iter()
                .all(|component| component.family == "neomorphism" && component.module == "select")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "Select")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "SelectWithLabel")
        );
    }

    #[test]
    fn wrapped_components_render_unstyled_html_with_neomorphism_classes() {
        let html = Select(&SelectProps::default()).into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.starts_with("<"), "expected rendered element: {html}");
        assert!(
            html.contains("neomorph-input"),
            "missing neomorph-input: {html}"
        );
        assert!(
            html.contains("appearance-none"),
            "missing appearance-none: {html}"
        );
    }
}
