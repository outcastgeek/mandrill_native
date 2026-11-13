#![allow(non_snake_case)]
#![allow(clippy::needless_pass_by_value)]

use crate::components::ComponentInfo;
use crate::components::shared;
use crate::components::unstyled::topnav as base;
use maud::Markup;

pub use base::{SimpleTopNav, SimpleTopNavLink, TopNavLinkProps, TopNavProps};

pub const COMPONENTS: &[ComponentInfo] = &[
    ComponentInfo {
        family: "neomorphism",
        module: "topnav",
        name: "TopNavLink",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "topnav",
        name: "TopNav",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "topnav",
        name: "SimpleTopNav",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "topnav",
        name: "SimpleTopNavLink",
    },
];

#[must_use]
fn addLinkNeomorphClasses(props: &TopNavLinkProps) -> TopNavLinkProps {
    let mut props = props.clone();
    props.class = shared::build_classes(&[props.class.as_str(), "neomorph-flat"]);
    props
}

#[must_use]
pub fn TopNavLink(props: &TopNavLinkProps, children: Markup) -> Markup {
    let neo_props = addLinkNeomorphClasses(props);
    base::TopNavLink(&neo_props, children)
}

#[must_use]
fn addNavNeomorphClasses(props: &TopNavProps) -> TopNavProps {
    let mut props = props.clone();
    props.class = shared::build_classes(&[props.class.as_str(), "neomorph-flat"]);
    props
}

#[must_use]
pub fn TopNav(props: &TopNavProps, children: Markup) -> Markup {
    let neo_props = addNavNeomorphClasses(props);
    base::TopNav(&neo_props, children)
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
                .all(|component| component.family == "neomorphism" && component.module == "topnav")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "TopNavLink")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "TopNav")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "SimpleTopNav")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "SimpleTopNavLink")
        );
    }

    #[test]
    fn wrapped_components_render_unstyled_html_with_neomorphism_classes() {
        let html = TopNavLink(&TopNavLinkProps::default(), html! { "child" }).into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.starts_with("<"), "expected rendered element: {html}");
        assert!(
            html.contains("neomorph-flat"),
            "missing neomorph-flat: {html}"
        );
        let html = TopNav(&TopNavProps::default(), html! { "child" }).into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.starts_with("<"), "expected rendered element: {html}");
        assert!(
            html.contains("neomorph-flat"),
            "missing neomorph-flat: {html}"
        );
    }
}
