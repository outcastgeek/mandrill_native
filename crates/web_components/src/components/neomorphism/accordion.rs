#![allow(non_snake_case)]
#![allow(clippy::needless_pass_by_value)]

use crate::components::ComponentInfo;
use crate::components::shared;
use crate::components::unstyled::accordion as base;
use maud::Markup;

pub use base::{
    ACCORDION_TYPE_MULTIPLE, ACCORDION_TYPE_SINGLE, AccordionContentProps, AccordionItemProps,
    AccordionProps, AccordionTriggerProps,
};

pub const COMPONENTS: &[ComponentInfo] = &[
    ComponentInfo {
        family: "neomorphism",
        module: "accordion",
        name: "Accordion",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "accordion",
        name: "AccordionItem",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "accordion",
        name: "AccordionTrigger",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "accordion",
        name: "AccordionContent",
    },
];

#[must_use]
fn addAccordionClasses(props: &AccordionProps) -> AccordionProps {
    let mut props = props.clone();
    props.class = shared::build_classes(&[props.class.as_str(), "neomorph-flat"]);
    props
}

#[must_use]
pub fn Accordion(props: &AccordionProps, children: Markup) -> Markup {
    let neo_props = addAccordionClasses(props);
    base::Accordion(&neo_props, children)
}

#[must_use]
fn addItemClasses(props: &AccordionItemProps) -> AccordionItemProps {
    let mut props = props.clone();
    props.class = shared::build_classes(&[props.class.as_str(), "neomorph-flat"]);
    props
}

#[must_use]
pub fn AccordionItem(props: &AccordionItemProps, children: Markup) -> Markup {
    let neo_props = addItemClasses(props);
    base::AccordionItem(&neo_props, children)
}

#[must_use]
fn addTriggerClasses(props: &AccordionTriggerProps) -> AccordionTriggerProps {
    let mut props = props.clone();
    props.class = shared::build_classes(&[props.class.as_str(), "neomorph-hover-lift"]);
    props
}

#[must_use]
pub fn AccordionTrigger(props: &AccordionTriggerProps, children: Markup) -> Markup {
    let neo_props = addTriggerClasses(props);
    base::AccordionTrigger(&neo_props, children)
}

#[must_use]
fn addContentClasses(props: &AccordionContentProps) -> AccordionContentProps {
    props.clone()
}

#[must_use]
pub fn AccordionContent(props: &AccordionContentProps, children: Markup) -> Markup {
    let neo_props = addContentClasses(props);
    base::AccordionContent(&neo_props, children)
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
                    && component.module == "accordion")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "Accordion")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "AccordionItem")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "AccordionTrigger")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "AccordionContent")
        );
    }

    #[test]
    fn wrapped_components_render_unstyled_html_with_neomorphism_classes() {
        let html = Accordion(&AccordionProps::default(), html! { "child" }).into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.starts_with("<"), "expected rendered element: {html}");
        assert!(
            html.contains("neomorph-flat"),
            "missing neomorph-flat: {html}"
        );
        let html = AccordionItem(&AccordionItemProps::default(), html! { "child" }).into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.starts_with("<"), "expected rendered element: {html}");
        assert!(
            html.contains("neomorph-flat"),
            "missing neomorph-flat: {html}"
        );
        let html =
            AccordionTrigger(&AccordionTriggerProps::default(), html! { "child" }).into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.starts_with("<"), "expected rendered element: {html}");
        assert!(
            html.contains("neomorph-hover-lift"),
            "missing neomorph-hover-lift: {html}"
        );
        let html =
            AccordionContent(&AccordionContentProps::default(), html! { "child" }).into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.starts_with("<"), "expected rendered element: {html}");
    }
}
