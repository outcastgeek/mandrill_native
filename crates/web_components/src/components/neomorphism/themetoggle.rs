#![allow(non_snake_case)]
#![allow(clippy::needless_pass_by_value)]

use crate::components::ComponentInfo;
use crate::components::shared;
use crate::components::unstyled::themetoggle as base;
use maud::Markup;

pub use base::{SimpleThemeToggle, ThemeToggleButtonProps, ThemeToggleWithLabel};

pub const COMPONENTS: &[ComponentInfo] = &[
    ComponentInfo {
        family: "neomorphism",
        module: "themetoggle",
        name: "ThemeToggleButton",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "themetoggle",
        name: "SimpleThemeToggle",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "themetoggle",
        name: "ThemeToggleWithLabel",
    },
];

#[must_use]
fn addNeomorphClasses(props: &ThemeToggleButtonProps) -> ThemeToggleButtonProps {
    let mut props = props.clone();
    props.class = shared::build_classes(&[props.class.as_str(), "neomorph-flat"]);
    props
}

#[must_use]
pub fn ThemeToggleButton(props: &ThemeToggleButtonProps) -> Markup {
    let neo_props = addNeomorphClasses(props);
    base::ThemeToggleButton(&neo_props)
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
        assert_eq!(COMPONENTS.len(), 3);
        assert!(COMPONENTS.iter().all(
            |component| component.family == "neomorphism" && component.module == "themetoggle"
        ));
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "ThemeToggleButton")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "SimpleThemeToggle")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "ThemeToggleWithLabel")
        );
    }

    #[test]
    fn wrapped_components_render_unstyled_html_with_neomorphism_classes() {
        let html = ThemeToggleButton(&ThemeToggleButtonProps::default()).into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.starts_with("<"), "expected rendered element: {html}");
        assert!(
            html.contains("neomorph-flat"),
            "missing neomorph-flat: {html}"
        );
    }
}
