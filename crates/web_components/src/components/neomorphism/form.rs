#![allow(non_snake_case)]
#![allow(clippy::needless_pass_by_value)]

use crate::components::ComponentInfo;
use crate::components::shared;
use crate::components::unstyled::form as base;
use maud::{Markup, html};

pub use base::{DatastarForm, FormProps, FormWithCSRF};

pub const COMPONENTS: &[ComponentInfo] = &[
    ComponentInfo {
        family: "neomorphism",
        module: "form",
        name: "Form",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "form",
        name: "FormWithCSRF",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "form",
        name: "DatastarForm",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "form",
        name: "CardForm",
    },
];

#[must_use]
fn addNeomorphClasses(props: &FormProps) -> FormProps {
    let mut props = props.clone();
    props.class =
        shared::build_classes(&[props.class.as_str(), "neomorph-container p-6 rounded-xl"]);
    props
}

#[must_use]
pub fn Form(props: &FormProps, children: Markup) -> Markup {
    let neo_props = addNeomorphClasses(props);
    base::Form(&neo_props, children)
}

#[must_use]
pub fn CardForm(props: &FormProps, title: &str, children: Markup) -> Markup {
    html! {
        div class="neomorph-card p-8 rounded-2xl" {
            @if !title.is_empty() { h2 class="text-2xl font-bold mb-6" { (title) } }
            (Form(props, children))
        }
    }
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
                .all(|component| component.family == "neomorphism" && component.module == "form")
        );
        assert!(COMPONENTS.iter().any(|component| component.name == "Form"));
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "FormWithCSRF")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "DatastarForm")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "CardForm")
        );
    }

    #[test]
    fn wrapped_components_render_unstyled_html_with_neomorphism_classes() {
        let html = Form(&FormProps::default(), html! { "child" }).into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.starts_with("<form"), "expected form root: {html}");
        assert!(html.contains("class=\"form neomorph-container p-6 rounded-xl\""));
        assert!(html.contains("method=\"POST\""));
        assert!(html.contains("autocomplete=\"on\""));
        assert!(html.contains(">child</form>"));
    }

    #[test]
    fn neomorphism_only_components_render_html_without_generated_stub_markers() {
        let html = CardForm(&FormProps::default(), "Title", html! { "child" }).into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.starts_with("<div class=\"neomorph-card p-8 rounded-2xl\""));
        assert!(html.contains("<h2 class=\"text-2xl font-bold mb-6\">Title</h2>"));
        assert!(
            html.contains("<form method=\"POST\" class=\"form neomorph-container p-6 rounded-xl\"")
        );
        assert!(html.contains("autocomplete=\"on\""));
        assert!(html.contains(">child</form>"));
    }
}
