#![allow(non_snake_case)]
#![allow(clippy::needless_pass_by_value)]

use crate::components::ComponentInfo;
use crate::components::shared;
use crate::components::unstyled::popover as base;
use maud::Markup;

pub use base::{
    BottomPopover, LeftPopover, Popover, PopoverClose, PopoverCloseProps, PopoverContentProps,
    PopoverDescription, PopoverDescriptionProps, PopoverHeader, PopoverHeaderProps, PopoverProps,
    PopoverTitle, PopoverTitleProps, PopoverTrigger, PopoverTriggerProps, PopoverWithDescription,
    RightPopover, SimplePopover, TopPopover,
};

pub const COMPONENTS: &[ComponentInfo] = &[
    ComponentInfo {
        family: "neomorphism",
        module: "popover",
        name: "Popover",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "popover",
        name: "PopoverTrigger",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "popover",
        name: "PopoverContent",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "popover",
        name: "PopoverHeader",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "popover",
        name: "PopoverTitle",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "popover",
        name: "PopoverDescription",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "popover",
        name: "PopoverClose",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "popover",
        name: "SimplePopover",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "popover",
        name: "PopoverWithDescription",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "popover",
        name: "TopPopover",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "popover",
        name: "RightPopover",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "popover",
        name: "BottomPopover",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "popover",
        name: "LeftPopover",
    },
];

#[must_use]
fn addNeomorphClasses(props: &PopoverContentProps) -> PopoverContentProps {
    let mut props = props.clone();
    props.class = shared::build_classes(&[props.class.as_str(), "neomorph-card"]);
    props
}

#[must_use]
pub fn PopoverContent(props: &PopoverContentProps, children: Markup) -> Markup {
    let neo_props = addNeomorphClasses(props);
    base::PopoverContent(&neo_props, children)
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
        assert!(COMPONENTS.iter().all(|component| component.family == "neomorphism" && component.module == "popover"));
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "Popover")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "PopoverTrigger")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "PopoverContent")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "PopoverHeader")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "PopoverTitle")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "PopoverDescription")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "PopoverClose")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "SimplePopover")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "PopoverWithDescription")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "TopPopover")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "RightPopover")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "BottomPopover")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "LeftPopover")
        );
    }

    #[test]
    fn wrapped_components_render_unstyled_html_with_neomorphism_classes() {
        let html = PopoverContent(&PopoverContentProps::default(), html! { "child" }).into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.starts_with("<"), "expected rendered element: {html}");
        assert!(
            html.contains("neomorph-card"),
            "missing neomorph-card: {html}"
        );
    }
}
