#![allow(non_snake_case)]
#![allow(clippy::needless_pass_by_value)]

use crate::components::ComponentInfo;
use crate::components::shared;
use crate::components::unstyled::input as base;
use maud::Markup;

pub use base::{
    EmailInput, FloatingInput, INPUT_TYPE_DATE, INPUT_TYPE_EMAIL, INPUT_TYPE_FILE,
    INPUT_TYPE_HIDDEN, INPUT_TYPE_NUMBER, INPUT_TYPE_PASSWORD, INPUT_TYPE_SEARCH, INPUT_TYPE_TEL,
    INPUT_TYPE_TEXT, INPUT_TYPE_TIME, INPUT_TYPE_URL, InputProps, InputWithIcon, InputWithLabel,
    NumberInput, PasswordInput, SearchInput, TelInput, TextArea, TextAreaWithLabel, URLInput,
};

pub const COMPONENTS: &[ComponentInfo] = &[
    ComponentInfo {
        family: "neomorphism",
        module: "input",
        name: "Input",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "input",
        name: "InputWithLabel",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "input",
        name: "FloatingInput",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "input",
        name: "TextArea",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "input",
        name: "TextAreaWithLabel",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "input",
        name: "InputWithIcon",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "input",
        name: "SearchInput",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "input",
        name: "EmailInput",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "input",
        name: "PasswordInput",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "input",
        name: "NumberInput",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "input",
        name: "URLInput",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "input",
        name: "TelInput",
    },
];

#[must_use]
fn addNeomorphClasses(props: &InputProps) -> InputProps {
    let mut props = props.clone();
    props.class = shared::build_classes(&[props.class.as_str(), "neomorph-input"]);
    props
}

#[must_use]
pub fn Input(props: &InputProps) -> Markup {
    let neo_props = addNeomorphClasses(props);
    base::Input(&neo_props)
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
        assert_eq!(COMPONENTS.len(), 12);
        assert!(
            COMPONENTS
                .iter()
                .all(|component| component.family == "neomorphism" && component.module == "input")
        );
        assert!(COMPONENTS.iter().any(|component| component.name == "Input"));
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "InputWithLabel")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "FloatingInput")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "TextArea")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "TextAreaWithLabel")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "InputWithIcon")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "SearchInput")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "EmailInput")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "PasswordInput")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "NumberInput")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "URLInput")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "TelInput")
        );
    }

    #[test]
    fn wrapped_components_render_unstyled_html_with_neomorphism_classes() {
        let html = Input(&InputProps::default()).into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.starts_with("<"), "expected rendered element: {html}");
        assert!(
            html.contains("neomorph-input"),
            "missing neomorph-input: {html}"
        );
    }
}
