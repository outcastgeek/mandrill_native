#![allow(non_snake_case)]
#![allow(clippy::needless_pass_by_value)]

use crate::components::ComponentInfo;
use crate::components::shared;
use crate::components::unstyled::languageselector as base;
use maud::Markup;

pub use base::{CompactLanguageSelector, Language, LanguageSelectorProps, SimpleLanguageSelector};

pub const COMPONENTS: &[ComponentInfo] = &[
    ComponentInfo {
        family: "neomorphism",
        module: "languageselector",
        name: "LanguageSelector",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "languageselector",
        name: "SimpleLanguageSelector",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "languageselector",
        name: "CompactLanguageSelector",
    },
];

#[must_use]
fn addNeomorphClasses(props: &LanguageSelectorProps) -> LanguageSelectorProps {
    let mut props = props.clone();
    props.class = shared::build_classes(&[props.class.as_str(), "neomorph-flat"]);
    props
}

#[must_use]
pub fn LanguageSelector(props: &LanguageSelectorProps) -> Markup {
    let neo_props = addNeomorphClasses(props);
    base::LanguageSelector(&neo_props)
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
        assert!(
            COMPONENTS
                .iter()
                .all(|component| component.family == "neomorphism"
                    && component.module == "languageselector")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "LanguageSelector")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "SimpleLanguageSelector")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "CompactLanguageSelector")
        );
    }

    #[test]
    fn wrapped_components_render_unstyled_html_with_neomorphism_classes() {
        let html = LanguageSelector(&LanguageSelectorProps::default()).into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.starts_with("<"), "expected rendered element: {html}");
        assert!(
            html.contains("neomorph-flat"),
            "missing neomorph-flat: {html}"
        );
    }
}
