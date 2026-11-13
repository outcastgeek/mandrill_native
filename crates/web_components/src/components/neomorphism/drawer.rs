#![allow(non_snake_case)]
#![allow(clippy::needless_pass_by_value)]

use crate::components::ComponentInfo;
use crate::components::shared;
use crate::components::unstyled::drawer as base;
use maud::Markup;

pub use base::{
    DRAWER_POSITION_LEFT, DRAWER_POSITION_RIGHT, DrawerCloseButton, DrawerProps, DrawerTrigger,
    SimpleTrigger,
};

pub const COMPONENTS: &[ComponentInfo] = &[
    ComponentInfo {
        family: "neomorphism",
        module: "drawer",
        name: "Drawer",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "drawer",
        name: "DrawerCloseButton",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "drawer",
        name: "DrawerTrigger",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "drawer",
        name: "SimpleTrigger",
    },
];

#[must_use]
fn addDrawerClasses(props: &DrawerProps) -> DrawerProps {
    let mut props = props.clone();
    props.class = shared::build_classes(&[props.class.as_str(), "neomorph-flat"]);
    props
}

#[must_use]
pub fn Drawer(props: &DrawerProps, children: Markup) -> Markup {
    let neo_props = addDrawerClasses(props);
    base::Drawer(&neo_props, children)
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
                .all(|component| component.family == "neomorphism" && component.module == "drawer")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "Drawer")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "DrawerCloseButton")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "DrawerTrigger")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "SimpleTrigger")
        );
    }

    #[test]
    fn wrapped_components_render_unstyled_html_with_neomorphism_classes() {
        let html = Drawer(&DrawerProps::default(), html! { "child" }).into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.starts_with("<"), "expected rendered element: {html}");
        assert!(
            html.contains("neomorph-flat"),
            "missing neomorph-flat: {html}"
        );
    }
}
