#![allow(non_snake_case)]
#![allow(clippy::needless_pass_by_value)]

use crate::components::ComponentInfo;
use crate::components::shared;
use crate::components::unstyled::divider as base;
use maud::Markup;

pub use base::{DIVIDER_HORIZONTAL, DIVIDER_VERTICAL, DividerProps};

pub const COMPONENTS: &[ComponentInfo] = &[ComponentInfo {
    family: "neomorphism",
    module: "divider",
    name: "Divider",
}];

#[must_use]
fn addNeomorphClasses(props: &DividerProps) -> DividerProps {
    let mut props = props.clone();
    props.class = shared::build_classes(&[props.class.as_str(), "neomorph-divider"]);
    props
}

#[must_use]
pub fn Divider(props: &DividerProps) -> Markup {
    let neo_props = addNeomorphClasses(props);
    base::Divider(&neo_props)
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
        assert_eq!(COMPONENTS.len(), 1);
        assert!(COMPONENTS.iter().all(|component| component.family == "neomorphism" && component.module == "divider"));
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "Divider")
        );
    }

    #[test]
    fn wrapped_components_render_unstyled_html_with_neomorphism_classes() {
        let html = Divider(&DividerProps::default()).into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.starts_with("<"), "expected rendered element: {html}");
        assert!(
            html.contains("neomorph-divider"),
            "missing neomorph-divider: {html}"
        );
    }
}
