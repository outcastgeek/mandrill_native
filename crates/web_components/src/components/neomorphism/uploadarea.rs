#![allow(non_snake_case)]
#![allow(clippy::needless_pass_by_value)]

use crate::components::ComponentInfo;
use crate::components::shared;
use crate::components::unstyled::uploadarea as base;
use maud::Markup;

pub use base::{DocumentUploadArea, ImageUploadArea, SimpleUploadArea, UploadAreaProps};

pub const COMPONENTS: &[ComponentInfo] = &[
    ComponentInfo {
        family: "neomorphism",
        module: "uploadarea",
        name: "UploadArea",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "uploadarea",
        name: "SimpleUploadArea",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "uploadarea",
        name: "ImageUploadArea",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "uploadarea",
        name: "DocumentUploadArea",
    },
];

#[must_use]
fn addNeomorphClasses(props: &UploadAreaProps) -> UploadAreaProps {
    let mut props = props.clone();
    props.class = shared::build_classes(&[props.class.as_str(), "neomorph-flat"]);
    props
}

#[must_use]
pub fn UploadArea(props: &UploadAreaProps, children: Markup) -> Markup {
    let neo_props = addNeomorphClasses(props);
    base::UploadArea(&neo_props, children)
}

#[cfg(test)]
mod tests {
    use super::*;
    use maud::html;
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
        assert!(
            COMPONENTS
                .iter()
                .all(|component| component.family == "neomorphism"
                    && component.module == "uploadarea")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "UploadArea")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "SimpleUploadArea")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "ImageUploadArea")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "DocumentUploadArea")
        );
    }

    #[test]
    fn wrapped_components_render_unstyled_html_with_neomorphism_classes() {
        let html = UploadArea(&UploadAreaProps::default(), html! { "child" }).into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.starts_with("<"), "expected rendered element: {html}");
        assert!(
            html.contains("neomorph-flat"),
            "missing neomorph-flat: {html}"
        );
    }
}
