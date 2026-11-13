#![allow(non_snake_case)]
#![allow(clippy::needless_pass_by_value)]

use crate::components::ComponentInfo;
use crate::components::shared;
use crate::components::unstyled::command as base;
use maud::Markup;

pub use base::{
    CommandGroupProps, CommandInputProps, CommandItemProps, CommandItemWithIcon, CommandListProps,
    CommandProps,
};

pub const COMPONENTS: &[ComponentInfo] = &[
    ComponentInfo {
        family: "neomorphism",
        module: "command",
        name: "Command",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "command",
        name: "CommandInput",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "command",
        name: "CommandList",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "command",
        name: "CommandGroup",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "command",
        name: "CommandItem",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "command",
        name: "CommandItemWithIcon",
    },
];

#[must_use]
fn addCommandClasses(props: &CommandProps) -> CommandProps {
    let mut props = props.clone();
    props.class = shared::build_classes(&[props.class.as_str(), "neomorph-flat"]);
    props
}

#[must_use]
pub fn Command(props: &CommandProps, children: Markup) -> Markup {
    let neo_props = addCommandClasses(props);
    base::Command(&neo_props, children)
}

#[must_use]
fn addInputClasses(props: &CommandInputProps) -> CommandInputProps {
    let mut props = props.clone();
    props.class = shared::build_classes(&[props.class.as_str(), "neomorph-inset"]);
    props
}

#[must_use]
pub fn CommandInput(props: &CommandInputProps) -> Markup {
    let neo_props = addInputClasses(props);
    base::CommandInput(&neo_props)
}

#[must_use]
fn addListClasses(props: &CommandListProps) -> CommandListProps {
    let mut props = props.clone();
    props.class = shared::build_classes(&[props.class.as_str(), "neomorph-flat"]);
    props
}

#[must_use]
pub fn CommandList(props: &CommandListProps, children: Markup) -> Markup {
    let neo_props = addListClasses(props);
    base::CommandList(&neo_props, children)
}

#[must_use]
fn addGroupClasses(props: &CommandGroupProps) -> CommandGroupProps {
    let mut props = props.clone();
    props.class = shared::build_classes(&[props.class.as_str(), "neomorph-flat"]);
    props
}

#[must_use]
pub fn CommandGroup(props: &CommandGroupProps, children: Markup) -> Markup {
    let neo_props = addGroupClasses(props);
    base::CommandGroup(&neo_props, children)
}

#[must_use]
fn addItemClasses(props: &CommandItemProps) -> CommandItemProps {
    let mut props = props.clone();
    props.class = shared::build_classes(&[props.class.as_str(), "neomorph-hover-lift"]);
    props
}

#[must_use]
pub fn CommandItem(props: &CommandItemProps, children: Markup) -> Markup {
    let neo_props = addItemClasses(props);
    base::CommandItem(&neo_props, children)
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
        assert_eq!(COMPONENTS.len(), 6);
        assert!(COMPONENTS.iter().all(|component| component.family == "neomorphism" && component.module == "command"));
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "Command")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "CommandInput")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "CommandList")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "CommandGroup")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "CommandItem")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "CommandItemWithIcon")
        );
    }

    #[test]
    fn wrapped_components_render_unstyled_html_with_neomorphism_classes() {
        let html =
            Command(&CommandProps::default().with_open(true), html! { "child" }).into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.starts_with("<"), "expected rendered element: {html}");
        assert!(
            html.contains("neomorph-flat"),
            "missing neomorph-flat: {html}"
        );
        let html = CommandInput(&CommandInputProps::default()).into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.starts_with("<"), "expected rendered element: {html}");
        assert!(
            html.contains("neomorph-inset"),
            "missing neomorph-inset: {html}"
        );
        let html = CommandList(&CommandListProps::default(), html! { "child" }).into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.starts_with("<"), "expected rendered element: {html}");
        assert!(
            html.contains("neomorph-flat"),
            "missing neomorph-flat: {html}"
        );
        let html = CommandGroup(&CommandGroupProps::default(), html! { "child" }).into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.starts_with("<"), "expected rendered element: {html}");
        assert!(
            html.contains("neomorph-flat"),
            "missing neomorph-flat: {html}"
        );
        let html = CommandItem(&CommandItemProps::default(), html! { "child" }).into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.starts_with("<"), "expected rendered element: {html}");
        assert!(
            html.contains("neomorph-hover-lift"),
            "missing neomorph-hover-lift: {html}"
        );
    }
}
