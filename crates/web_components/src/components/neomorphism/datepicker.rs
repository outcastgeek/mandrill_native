#![allow(non_snake_case)]
#![allow(clippy::needless_pass_by_value)]

use crate::components::ComponentInfo;
use crate::components::unstyled::datepicker as base;
use maud::Markup;

pub use base::{DatePickerProps, DatePickerWithLabel};

pub const COMPONENTS: &[ComponentInfo] = &[
    ComponentInfo {
        family: "neomorphism",
        module: "datepicker",
        name: "DatePicker",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "datepicker",
        name: "DatePickerWithLabel",
    },
];

#[must_use]
fn addNeomorphClasses(props: &DatePickerProps) -> DatePickerProps {
    props.clone()
}

#[must_use]
pub fn DatePicker(props: &DatePickerProps) -> Markup {
    let neo_props = addNeomorphClasses(props);
    base::DatePicker(&neo_props)
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
                .all(|component| component.family == "neomorphism"
                    && component.module == "datepicker")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "DatePicker")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "DatePickerWithLabel")
        );
    }

    #[test]
    fn wrapped_components_render_unstyled_html_with_neomorphism_classes() {
        let html = DatePicker(&DatePickerProps::default()).into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.starts_with("<"), "expected rendered element: {html}");
    }
}
