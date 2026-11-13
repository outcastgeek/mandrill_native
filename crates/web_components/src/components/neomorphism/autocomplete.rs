#![allow(non_snake_case)]
#![allow(clippy::needless_pass_by_value)]

use crate::components::ComponentInfo;
use crate::components::shared;
use crate::components::unstyled::autocomplete as base;
use maud::Markup;

pub use base::{
    AutocompleteProps, AutocompleteWithDatastar, AutocompleteWithDatastarAndLabel,
    AutocompleteWithLabel, Option,
};

pub const COMPONENTS: &[ComponentInfo] = &[
    ComponentInfo {
        family: "neomorphism",
        module: "autocomplete",
        name: "Autocomplete",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "autocomplete",
        name: "AutocompleteWithLabel",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "autocomplete",
        name: "AutocompleteWithDatastar",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "autocomplete",
        name: "AutocompleteWithDatastarAndLabel",
    },
];

#[must_use]
fn addNeomorphClasses(props: &AutocompleteProps) -> AutocompleteProps {
    let mut props = props.clone();
    props.class = shared::build_classes(&[props.class.as_str(), "neomorph-flat"]);
    props
}

#[must_use]
pub fn Autocomplete(props: &AutocompleteProps) -> Markup {
    let neo_props = addNeomorphClasses(props);
    base::Autocomplete(&neo_props)
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
        assert_eq!(COMPONENTS.len(), 4);
        assert!(COMPONENTS.iter().all(
            |component| component.family == "neomorphism" && component.module == "autocomplete"
        ));
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "Autocomplete")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "AutocompleteWithLabel")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "AutocompleteWithDatastar")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "AutocompleteWithDatastarAndLabel")
        );
    }

    #[test]
    fn wrapped_components_render_unstyled_html_with_neomorphism_classes() {
        let html = Autocomplete(&AutocompleteProps::default()).into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.starts_with("<"), "expected rendered element: {html}");
        assert!(
            html.contains("neomorph-flat"),
            "missing neomorph-flat: {html}"
        );
    }
}
