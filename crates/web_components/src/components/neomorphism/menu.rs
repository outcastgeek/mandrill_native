#![allow(non_snake_case)]
#![allow(clippy::needless_pass_by_value)]

use crate::components::ComponentInfo;
use crate::components::shared;
use crate::components::unstyled::menu as base;
use maud::Markup;

pub use base::{
    DestructiveMenuItem, Menu, MenuContentProps, MenuDisclosureProps, MenuGroup, MenuGroupProps,
    MenuItemProps, MenuItemWithIcon, MenuLabel, MenuLabelProps, MenuProps, MenuSeparator,
    MenuSeparatorProps, MenuSummaryProps, SimpleMenu, SimpleMenuContent, SimpleMenuGroup,
    SimpleMenuItem, SimpleSeparator,
};

pub const COMPONENTS: &[ComponentInfo] = &[
    ComponentInfo {
        family: "neomorphism",
        module: "menu",
        name: "Menu",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "menu",
        name: "MenuContent",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "menu",
        name: "MenuItem",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "menu",
        name: "MenuSeparator",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "menu",
        name: "MenuGroup",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "menu",
        name: "MenuLabel",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "menu",
        name: "SimpleMenu",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "menu",
        name: "SimpleMenuContent",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "menu",
        name: "SimpleMenuItem",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "menu",
        name: "MenuItemWithIcon",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "menu",
        name: "DestructiveMenuItem",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "menu",
        name: "SimpleSeparator",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "menu",
        name: "SimpleMenuGroup",
    },
];

#[must_use]
fn addContentNeomorphClasses(props: &MenuContentProps) -> MenuContentProps {
    let mut props = props.clone();
    props.class = shared::build_classes(&[props.class.as_str(), "neomorph-flat"]);
    props
}

#[must_use]
fn addItemNeomorphClasses(props: &MenuItemProps) -> MenuItemProps {
    let mut props = props.clone();
    let neomorph_class = if props.disabled {
        ""
    } else if props.destructive {
        "hover:neomorph-flat-sm"
    } else {
        "hover:neomorph-hover-lift"
    };
    props.class = shared::build_classes(&[neomorph_class, props.class.as_str()]);
    props
}

#[must_use]
pub fn MenuContent(props: &MenuContentProps, children: Markup) -> Markup {
    let neo_props = addContentNeomorphClasses(props);
    base::MenuContent(&neo_props, children)
}

#[must_use]
pub fn MenuItem(props: &MenuItemProps, children: Markup) -> Markup {
    let neo_props = addItemNeomorphClasses(props);
    base::MenuItem(&neo_props, children)
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
        assert_eq!(COMPONENTS.len(), 13);
        assert!(
            COMPONENTS
                .iter()
                .all(|component| component.family == "neomorphism" && component.module == "menu")
        );
        assert!(COMPONENTS.iter().any(|component| component.name == "Menu"));
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "MenuContent")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "MenuItem")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "MenuSeparator")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "MenuGroup")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "MenuLabel")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "SimpleMenu")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "SimpleMenuContent")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "SimpleMenuItem")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "MenuItemWithIcon")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "DestructiveMenuItem")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "SimpleSeparator")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "SimpleMenuGroup")
        );
    }

    #[test]
    fn wrapped_components_render_unstyled_html_with_neomorphism_classes() {
        let html = MenuContent(&MenuContentProps::default(), html! { "child" }).into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.starts_with("<"), "expected rendered element: {html}");
        assert!(
            html.contains("neomorph-flat"),
            "missing neomorph-flat: {html}"
        );

        let normal = addItemNeomorphClasses(&MenuItemProps::default().with_class("custom"));
        assert!(normal.class.contains("hover:neomorph-hover-lift"));
        assert!(normal.class.contains("custom"));
        let destructive = addItemNeomorphClasses(&MenuItemProps::default().with_destructive(true));
        assert!(destructive.class.contains("hover:neomorph-flat-sm"));
        let disabled = addItemNeomorphClasses(&MenuItemProps::default().with_disabled(true));
        assert!(!disabled.class.contains("neomorph"));

        let item = MenuItem(&MenuItemProps::default(), html! { "Open" }).into_string();
        assert!(item.contains("hover:neomorph-hover-lift"));
        assert!(item.contains("Open"));
    }
}
