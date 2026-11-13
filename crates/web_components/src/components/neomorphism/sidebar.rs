#![allow(non_snake_case)]
#![allow(clippy::needless_pass_by_value)]

use crate::components::ComponentInfo;
use crate::components::shared;
use crate::components::unstyled::sidebar as base;
use maud::Markup;

pub use base::{SidebarItemProps, SidebarProps, SimpleSidebar, SimpleSidebarItem};

pub const COMPONENTS: &[ComponentInfo] = &[
    ComponentInfo {
        family: "neomorphism",
        module: "sidebar",
        name: "Sidebar",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "sidebar",
        name: "SidebarItem",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "sidebar",
        name: "SimpleSidebar",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "sidebar",
        name: "SimpleSidebarItem",
    },
];

#[must_use]
fn addSidebarClasses(props: &SidebarProps) -> SidebarProps {
    let mut props = props.clone();
    props.class = shared::build_classes(&[props.class.as_str(), "neomorph-flat"]);
    props
}

#[must_use]
pub fn Sidebar(props: &SidebarProps, children: Markup) -> Markup {
    let neo_props = addSidebarClasses(props);
    base::Sidebar(&neo_props, children)
}

#[must_use]
fn addItemClasses(props: &SidebarItemProps) -> SidebarItemProps {
    let mut props = props.clone();
    props.class = shared::build_classes(&[props.class.as_str(), "neomorph-hover-lift"]);
    props
}

#[must_use]
pub fn SidebarItem(props: &SidebarItemProps) -> Markup {
    let neo_props = addItemClasses(props);
    base::SidebarItem(&neo_props)
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
        assert!(COMPONENTS.iter().all(|component| component.family == "neomorphism" && component.module == "sidebar"));
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "Sidebar")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "SidebarItem")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "SimpleSidebar")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "SimpleSidebarItem")
        );
    }

    #[test]
    fn wrapped_components_render_unstyled_html_with_neomorphism_classes() {
        let html = Sidebar(&SidebarProps::default(), html! { "child" }).into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.starts_with("<"), "expected rendered element: {html}");
        assert!(
            html.contains("neomorph-flat"),
            "missing neomorph-flat: {html}"
        );
        let html = SidebarItem(&SidebarItemProps::default()).into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.starts_with("<"), "expected rendered element: {html}");
        assert!(
            html.contains("neomorph-hover-lift"),
            "missing neomorph-hover-lift: {html}"
        );
    }
}
