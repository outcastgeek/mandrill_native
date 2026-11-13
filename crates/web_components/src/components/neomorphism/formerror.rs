#![allow(non_snake_case)]
#![allow(clippy::needless_pass_by_value)]

use crate::components::ComponentInfo;
use crate::components::shared;
use crate::components::unstyled::formerror as base;
use maud::{Markup, html};

pub use base::FormErrorProps;

pub const COMPONENTS: &[ComponentInfo] = &[
    ComponentInfo {
        family: "neomorphism",
        module: "formerror",
        name: "FormError",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "formerror",
        name: "FormErrorWithIcon",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "formerror",
        name: "FormErrorList",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "formerror",
        name: "InlineError",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "formerror",
        name: "NeomorphicErrorBox",
    },
];

#[must_use]
fn addNeomorphClasses(props: &FormErrorProps) -> FormErrorProps {
    let mut props = props.clone();
    props.class = shared::build_classes(&[props.class.as_str(), "mt-1"]);
    props
}

#[must_use]
pub fn FormError(props: &FormErrorProps) -> Markup {
    let neo_props = addNeomorphClasses(props);
    base::FormError(&neo_props)
}

#[must_use]
pub fn FormErrorWithIcon(props: &FormErrorProps, icon: Option<Markup>) -> Markup {
    let neo_props = addNeomorphClasses(props);
    base::FormErrorWithIcon(&neo_props, icon)
}

#[must_use]
pub fn FormErrorList(errors: &[&str], props: &FormErrorProps) -> Markup {
    let neo_props = addNeomorphClasses(props);
    base::FormErrorList(errors, &neo_props)
}

#[must_use]
pub fn InlineError(message: impl Into<String>) -> Markup {
    base::InlineError(message)
}

#[must_use]
pub fn NeomorphicErrorBox(message: &str, title: &str) -> Markup {
    if message.is_empty() {
        return html! {};
    }

    html! {
        div class="neomorph-concave rounded-lg p-4 border-l-4 border-destructive" {
            @if !title.is_empty() {
                h4 class="text-sm font-semibold text-destructive mb-2" { (title) }
            }
            (FormError(&FormErrorProps::default()
                .with_message(message)
                .with_size(shared::SIZE_SM)
                .with_show_icon(true)))
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
        assert_eq!(COMPONENTS.len(), 5);
        assert!(
            COMPONENTS
                .iter()
                .all(|component| component.family == "neomorphism"
                    && component.module == "formerror")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "FormError")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "FormErrorWithIcon")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "FormErrorList")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "InlineError")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "NeomorphicErrorBox")
        );
    }

    #[test]
    fn wrapped_components_render_unstyled_html_with_neomorphism_classes() {
        let html = FormError(&FormErrorProps::default().with_message("Required")).into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.starts_with("<"), "expected rendered element: {html}");
        assert!(html.contains("mt-1"), "missing mt-1: {html}");
    }

    #[test]
    fn neomorphism_only_components_render_html_without_generated_stub_markers() {
        let props = FormErrorProps::default().with_message("Broken");
        let html = FormErrorWithIcon(&props, Some(html! { span class="custom-icon" { "!" } }))
            .into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.contains("mt-1"));
        assert!(html.contains("custom-icon"));
        assert!(html.contains("Broken"));

        let html = FormErrorList(&["First", "Second"], &props).into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.contains("mt-1"));
        assert!(html.contains("<li>First</li>"));
        assert!(html.contains("<li>Second</li>"));

        let html = InlineError("Inline broken").into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.contains("Inline broken"));
        assert!(html.contains("role=\"alert\""));

        let html = NeomorphicErrorBox("sample", "sample title").into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.contains("neomorph-concave rounded-lg p-4 border-l-4 border-destructive"));
        assert!(html.contains(
            "<h4 class=\"text-sm font-semibold text-destructive mb-2\">sample title</h4>"
        ));
        assert!(html.contains("sample"));
        assert!(html.contains("mt-1"));

        let html = NeomorphicErrorBox("message only", "").into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.contains("message only"));
        assert!(!html.contains("<h4"));

        let html = NeomorphicErrorBox("", "ignored").into_string();
        assert_eq!(html, "");
    }
}
