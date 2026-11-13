#![allow(non_snake_case)]
#![allow(clippy::needless_pass_by_value)]

use crate::components::ComponentInfo;
use crate::components::shared;
use crate::components::unstyled::slider as base;
use maud::Markup;

pub use base::{SliderProps, SliderWithLabel};

pub const COMPONENTS: &[ComponentInfo] = &[
    ComponentInfo {
        family: "neomorphism",
        module: "slider",
        name: "Slider",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "slider",
        name: "SliderWithLabel",
    },
];

#[must_use]
fn addNeomorphClasses(props: &SliderProps) -> SliderProps {
    let mut props = props.clone();
    props.class = shared::build_classes(&[props.class.as_str(), "neomorph-flat"]);
    props
}

#[must_use]
pub fn Slider(props: &SliderProps) -> Markup {
    let neo_props = addNeomorphClasses(props);
    base::Slider(&neo_props)
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
                .all(|component| component.family == "neomorphism" && component.module == "slider")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "Slider")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "SliderWithLabel")
        );
    }

    #[test]
    fn wrapped_components_render_unstyled_html_with_neomorphism_classes() {
        let html = Slider(&SliderProps::default()).into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.starts_with("<"), "expected rendered element: {html}");
        assert!(
            html.contains("neomorph-flat"),
            "missing neomorph-flat: {html}"
        );
    }
}
