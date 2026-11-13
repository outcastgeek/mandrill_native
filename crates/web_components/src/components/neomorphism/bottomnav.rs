#![allow(non_snake_case)]
#![allow(clippy::needless_pass_by_value)]

use crate::components::ComponentInfo;
use crate::components::shared;
use crate::components::unstyled::bottomnav as base;
use maud::Markup;

pub use base::{BottomNavItemProps, BottomNavProps, SimpleBottomNav, SimpleBottomNavItem};

pub const COMPONENTS: &[ComponentInfo] = &[
    ComponentInfo {
        family: "neomorphism",
        module: "bottomnav",
        name: "BottomNavItem",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "bottomnav",
        name: "BottomNav",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "bottomnav",
        name: "SimpleBottomNav",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "bottomnav",
        name: "SimpleBottomNavItem",
    },
];

#[must_use]
fn addItemNeomorphClasses(props: &BottomNavItemProps) -> BottomNavItemProps {
    let mut props = props.clone();
    props.class = shared::build_classes(&[props.class.as_str(), "neomorph-flat"]);
    props
}

#[must_use]
pub fn BottomNavItem(props: &BottomNavItemProps, children: Markup) -> Markup {
    let neo_props = addItemNeomorphClasses(props);
    base::BottomNavItem(&neo_props, children)
}

#[must_use]
fn addNavNeomorphClasses(props: &BottomNavProps) -> BottomNavProps {
    let mut props = props.clone();
    props.class = shared::build_classes(&[props.class.as_str(), "neomorph-flat"]);
    props
}

#[must_use]
pub fn BottomNav(props: &BottomNavProps, children: Markup) -> Markup {
    let neo_props = addNavNeomorphClasses(props);
    base::BottomNav(&neo_props, children)
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
                    && component.module == "bottomnav")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "BottomNavItem")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "BottomNav")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "SimpleBottomNav")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "SimpleBottomNavItem")
        );
    }

    #[test]
    fn wrapped_components_render_unstyled_html_with_neomorphism_classes() {
        let html = BottomNavItem(&BottomNavItemProps::default(), html! { "child" }).into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.starts_with("<"), "expected rendered element: {html}");
        assert!(
            html.contains("neomorph-flat"),
            "missing neomorph-flat: {html}"
        );
        let html = BottomNav(&BottomNavProps::default(), html! { "child" }).into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.starts_with("<"), "expected rendered element: {html}");
        assert!(
            html.contains("neomorph-flat"),
            "missing neomorph-flat: {html}"
        );
    }
}
