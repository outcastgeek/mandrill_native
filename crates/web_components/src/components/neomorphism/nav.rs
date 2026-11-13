#![allow(non_snake_case)]
#![allow(clippy::needless_pass_by_value)]

use crate::components::unstyled::nav as base;
use crate::components::{ComponentInfo, shared};
use maud::Markup;

pub use base::{
    HorizontalNav, Nav, NavItemProps, NavItemWithIcon, NavProps, SimpleNavItem, VerticalNav,
};

pub const COMPONENTS: &[ComponentInfo] = &[
    ComponentInfo {
        family: "neomorphism",
        module: "nav",
        name: "Nav",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "nav",
        name: "NavItem",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "nav",
        name: "HorizontalNav",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "nav",
        name: "VerticalNav",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "nav",
        name: "SimpleNavItem",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "nav",
        name: "NavItemWithIcon",
    },
];

#[must_use]
fn addNeomorphClasses(props: &NavItemProps) -> NavItemProps {
    let mut props = props.clone();
    let neomorph_class = if props.active {
        "neomorph-sidebar-item bg-accent text-accent-foreground"
    } else if !props.disabled {
        "neomorph-hover-lift hover:neomorph-flat-sm"
    } else {
        ""
    };
    props.class = shared::build_classes(&[neomorph_class, props.class.as_str()]);
    props
}

#[must_use]
pub fn NavItem(props: &NavItemProps, children: Markup) -> Markup {
    let neo_props = addNeomorphClasses(props);
    base::NavItem(&neo_props, children)
}

#[cfg(test)]
mod tests {
    use super::*;
    use maud::html;
    #[test]
    fn registry_matches_neomorphism_source_components() {
        assert_eq!(COMPONENTS.len(), 6);
        assert!(
            COMPONENTS
                .iter()
                .all(|component| component.family == "neomorphism" && component.module == "nav")
        );
        assert!(COMPONENTS.iter().any(|component| component.name == "Nav"));
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "NavItem")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "HorizontalNav")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "VerticalNav")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "SimpleNavItem")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "NavItemWithIcon")
        );
    }

    #[test]
    fn nav_item_helper_adds_state_specific_neomorph_classes() {
        let active = addNeomorphClasses(&NavItemProps::default().with_active(true));
        assert!(active.class.contains("neomorph-sidebar-item"));
        assert!(active.class.contains("bg-accent"));

        let inactive = addNeomorphClasses(&NavItemProps::default().with_class("custom"));
        assert!(inactive.class.contains("neomorph-hover-lift"));
        assert!(inactive.class.contains("hover:neomorph-flat-sm"));
        assert!(inactive.class.contains("custom"));

        let disabled = addNeomorphClasses(&NavItemProps::default().with_disabled(true));
        assert!(!disabled.class.contains("neomorph"));

        let rendered = NavItem(&NavItemProps::default(), html! { "Home" }).into_string();
        assert!(rendered.contains("neomorph-hover-lift"));
        assert!(rendered.contains(">Home</"));
    }
}
