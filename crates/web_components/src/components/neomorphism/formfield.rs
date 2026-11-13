#![allow(non_snake_case)]
#![allow(clippy::needless_pass_by_value)]

use crate::components::ComponentInfo;
use crate::components::shared;
use crate::components::unstyled::formfield as base;
use maud::{Markup, html};

pub use base::{FormFieldProps, FormFieldWithIcon, HorizontalFormField};

pub const COMPONENTS: &[ComponentInfo] = &[
    ComponentInfo {
        family: "neomorphism",
        module: "formfield",
        name: "FormField",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "formfield",
        name: "FormFieldWithIcon",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "formfield",
        name: "HorizontalFormField",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "formfield",
        name: "NeomorphicFormField",
    },
];

#[must_use]
fn addNeomorphClasses(props: &FormFieldProps) -> FormFieldProps {
    let mut props = props.clone();
    props.class = shared::build_classes(&[props.class.as_str(), "mb-4"]);
    props
}

#[must_use]
pub fn FormField(props: &FormFieldProps, children: Markup) -> Markup {
    let neo_props = addNeomorphClasses(props);
    base::FormField(&neo_props, children)
}

#[must_use]
pub fn NeomorphicFormField(props: &FormFieldProps, children: Markup) -> Markup {
    let neo_props = addNeomorphClasses(props);
    base::FormField(
        &neo_props,
        html! { div class="neomorph-concave rounded-lg p-1" { (children) } },
    )
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
                    && component.module == "formfield")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "FormField")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "FormFieldWithIcon")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "HorizontalFormField")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "NeomorphicFormField")
        );
    }

    #[test]
    fn wrapped_components_render_unstyled_html_with_neomorphism_classes() {
        let html = FormField(&FormFieldProps::default(), html! { "child" }).into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.starts_with("<div"));
        assert!(html.contains("class=\"form-field flex flex-col gap-1.5 mb-4\""));
        assert!(html.contains("child"));
    }

    #[test]
    fn neomorphism_only_components_render_html_without_generated_stub_markers() {
        let html = NeomorphicFormField(
            &FormFieldProps::default().with_label("Label"),
            html! { "child" },
        )
        .into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.starts_with("<div"));
        assert!(html.contains("class=\"form-field flex flex-col gap-1.5 mb-4\""));
        assert!(html.contains("<label"));
        assert!(html.contains(">Label</label>"));
        assert!(html.contains("<div class=\"neomorph-concave rounded-lg p-1\">child</div>"));
    }
}
