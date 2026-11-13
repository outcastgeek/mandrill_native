#![allow(non_snake_case)]
#![allow(clippy::needless_pass_by_value)]

use crate::components::ComponentInfo;
use crate::components::shared;
use crate::components::unstyled::collapsible as base;
use maud::Markup;

pub use base::{CollapsibleContentProps, CollapsibleProps, CollapsibleTriggerProps};

pub const COMPONENTS: &[ComponentInfo] = &[
    ComponentInfo {
        family: "neomorphism",
        module: "collapsible",
        name: "Collapsible",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "collapsible",
        name: "CollapsibleTrigger",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "collapsible",
        name: "CollapsibleContent",
    },
];

#[must_use]
fn addCollapsibleClasses(props: &CollapsibleProps) -> CollapsibleProps {
    let mut props = props.clone();
    props.class = shared::build_classes(&[props.class.as_str(), "neomorph-flat"]);
    props
}

#[must_use]
pub fn Collapsible(props: &CollapsibleProps, children: Markup) -> Markup {
    let neo_props = addCollapsibleClasses(props);
    base::Collapsible(&neo_props, children)
}

#[must_use]
fn addTriggerClasses(props: &CollapsibleTriggerProps) -> CollapsibleTriggerProps {
    let mut props = props.clone();
    props.class = shared::build_classes(&[props.class.as_str(), "neomorph-hover-lift"]);
    props
}

#[must_use]
pub fn CollapsibleTrigger(props: &CollapsibleTriggerProps, children: Markup) -> Markup {
    let neo_props = addTriggerClasses(props);
    base::CollapsibleTrigger(&neo_props, children)
}

#[must_use]
fn addContentClasses(props: &CollapsibleContentProps) -> CollapsibleContentProps {
    props.clone()
}

#[must_use]
pub fn CollapsibleContent(props: &CollapsibleContentProps, children: Markup) -> Markup {
    let neo_props = addContentClasses(props);
    base::CollapsibleContent(&neo_props, children)
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
        assert_eq!(COMPONENTS.len(), 3);
        assert!(COMPONENTS.iter().all(
            |component| component.family == "neomorphism" && component.module == "collapsible"
        ));
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "Collapsible")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "CollapsibleTrigger")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "CollapsibleContent")
        );
    }

    #[test]
    fn wrapped_components_render_unstyled_html_with_neomorphism_classes() {
        let html = Collapsible(&CollapsibleProps::default(), html! { "child" }).into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.starts_with("<"), "expected rendered element: {html}");
        assert!(
            html.contains("neomorph-flat"),
            "missing neomorph-flat: {html}"
        );
        let html = CollapsibleTrigger(&CollapsibleTriggerProps::default(), html! { "child" })
            .into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.starts_with("<"), "expected rendered element: {html}");
        assert!(
            html.contains("neomorph-hover-lift"),
            "missing neomorph-hover-lift: {html}"
        );
        let html = CollapsibleContent(&CollapsibleContentProps::default(), html! { "child" })
            .into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.starts_with("<"), "expected rendered element: {html}");
    }
}
