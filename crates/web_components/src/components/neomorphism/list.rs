#![allow(non_snake_case)]
#![allow(clippy::needless_pass_by_value)]

use crate::components::ComponentInfo;
use crate::components::shared;
use crate::components::unstyled::list as base;
use maud::Markup;

pub use base::{
    DescriptionDetails, DescriptionDetailsProps, DescriptionListProps, DescriptionTerm,
    DescriptionTermProps, IconList, ListItemProps, ListProps, OrderedList, UnorderedList,
};

pub const COMPONENTS: &[ComponentInfo] = &[
    ComponentInfo {
        family: "neomorphism",
        module: "list",
        name: "List",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "list",
        name: "ListItem",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "list",
        name: "DescriptionList",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "list",
        name: "DescriptionTerm",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "list",
        name: "DescriptionDetails",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "list",
        name: "UnorderedList",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "list",
        name: "OrderedList",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "list",
        name: "IconList",
    },
];

#[must_use]
fn addListNeomorphClasses(props: &ListProps) -> ListProps {
    let mut props = props.clone();
    props.class = shared::build_classes(&[props.class.as_str(), "neomorph-flat p-4 rounded-xl"]);
    props
}

#[must_use]
pub fn setOrdered(props: &ListProps, ordered: bool) -> ListProps {
    let mut props = props.clone();
    props.ordered = ordered;
    props
}

#[must_use]
pub fn List(props: &ListProps, children: Markup) -> Markup {
    let neo_props = addListNeomorphClasses(props);
    base::List(&neo_props, children)
}

#[must_use]
fn addItemNeomorphClasses(props: &ListItemProps) -> ListItemProps {
    let mut props = props.clone();
    props.class = shared::build_classes(&[
        props.class.as_str(),
        "neomorph-concave-sm p-2 my-2 rounded-lg",
    ]);
    props
}

#[must_use]
pub fn ListItem(props: &ListItemProps, children: Markup) -> Markup {
    let neo_props = addItemNeomorphClasses(props);
    base::ListItem(&neo_props, children)
}

#[must_use]
fn addDescriptionListNeomorphClasses(props: &DescriptionListProps) -> DescriptionListProps {
    let mut props = props.clone();
    props.class = shared::build_classes(&[props.class.as_str(), "neomorph-card p-6"]);
    props
}

#[must_use]
pub fn DescriptionList(props: &DescriptionListProps, children: Markup) -> Markup {
    let neo_props = addDescriptionListNeomorphClasses(props);
    base::DescriptionList(&neo_props, children)
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
        assert_eq!(COMPONENTS.len(), 8);
        assert!(
            COMPONENTS
                .iter()
                .all(|component| component.family == "neomorphism" && component.module == "list")
        );
        assert!(COMPONENTS.iter().any(|component| component.name == "List"));
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "ListItem")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "DescriptionList")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "DescriptionTerm")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "DescriptionDetails")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "UnorderedList")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "OrderedList")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "IconList")
        );
    }

    #[test]
    fn wrapped_components_render_unstyled_html_with_neomorphism_classes() {
        assert!(setOrdered(&ListProps::default(), true).ordered);
        assert!(!setOrdered(&ListProps::default().with_ordered(true), false).ordered);

        let html = List(&ListProps::default(), html! { "child" }).into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.starts_with("<"), "expected rendered element: {html}");
        assert!(
            html.contains("neomorph-flat"),
            "missing neomorph-flat: {html}"
        );
        assert!(html.contains("p-4"), "missing p-4: {html}");
        assert!(html.contains("rounded-xl"), "missing rounded-xl: {html}");
        let html = ListItem(&ListItemProps::default(), html! { "child" }).into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.starts_with("<"), "expected rendered element: {html}");
        assert!(
            html.contains("neomorph-concave-sm"),
            "missing neomorph-concave-sm: {html}"
        );
        assert!(html.contains("p-2"), "missing p-2: {html}");
        assert!(html.contains("my-2"), "missing my-2: {html}");
        assert!(html.contains("rounded-lg"), "missing rounded-lg: {html}");
        let html =
            DescriptionList(&DescriptionListProps::default(), html! { "child" }).into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.starts_with("<"), "expected rendered element: {html}");
        assert!(
            html.contains("neomorph-card"),
            "missing neomorph-card: {html}"
        );
        assert!(html.contains("p-6"), "missing p-6: {html}");
    }
}
