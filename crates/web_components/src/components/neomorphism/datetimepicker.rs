#![allow(non_snake_case)]
#![allow(clippy::needless_pass_by_value)]

use crate::components::ComponentInfo;
use crate::components::shared;
use crate::components::unstyled::datetimepicker as base;
use maud::Markup;

pub use base::{DateTimePickerProps, DateTimePickerWithLabel};

pub const COMPONENTS: &[ComponentInfo] = &[
    ComponentInfo {
        family: "neomorphism",
        module: "datetimepicker",
        name: "DateTimePicker",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "datetimepicker",
        name: "DateTimePickerWithLabel",
    },
];

#[must_use]
fn buildNeoClasses(props: &DateTimePickerProps) -> DateTimePickerProps {
    let mut props = props.clone();
    props.class = shared::build_classes(&[props.class.as_str(), "neomorph-flat rounded-xl"]);
    props
}

#[must_use]
pub fn DateTimePicker(props: &DateTimePickerProps) -> Markup {
    let neo_props = buildNeoClasses(props);
    base::DateTimePicker(&neo_props)
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
                    && component.module == "datetimepicker")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "DateTimePicker")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "DateTimePickerWithLabel")
        );
    }

    #[test]
    fn wrapped_components_render_unstyled_html_with_neomorphism_classes() {
        let html = DateTimePicker(&DateTimePickerProps::default()).into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.starts_with("<"), "expected rendered element: {html}");
        assert!(
            html.contains("neomorph-flat"),
            "missing neomorph-flat: {html}"
        );
        assert!(html.contains("rounded-xl"), "missing rounded-xl: {html}");
    }
}
