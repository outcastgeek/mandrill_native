#![allow(non_snake_case)]
#![allow(clippy::needless_pass_by_value)]

use crate::components::ComponentInfo;
use crate::components::shared;
use crate::components::unstyled::breadcrumb as base;
use maud::Markup;

pub use base::{
    BreadcrumbCurrent, BreadcrumbEllipsis, BreadcrumbItemProps, BreadcrumbLink, BreadcrumbProps,
    BreadcrumbSeparator, BreadcrumbWithSeparator, SimpleBreadcrumb,
};

pub const COMPONENTS: &[ComponentInfo] = &[
    ComponentInfo {
        family: "neomorphism",
        module: "breadcrumb",
        name: "Breadcrumb",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "breadcrumb",
        name: "BreadcrumbItem",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "breadcrumb",
        name: "BreadcrumbSeparator",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "breadcrumb",
        name: "BreadcrumbEllipsis",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "breadcrumb",
        name: "SimpleBreadcrumb",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "breadcrumb",
        name: "BreadcrumbLink",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "breadcrumb",
        name: "BreadcrumbCurrent",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "breadcrumb",
        name: "BreadcrumbWithSeparator",
    },
];

#[must_use]
fn addNeomorphClasses(props: &BreadcrumbProps) -> BreadcrumbProps {
    let mut props = props.clone();
    props.class = shared::build_classes(&[props.class.as_str(), "neomorph-flat-sm p-2 rounded-lg"]);
    props
}

#[must_use]
fn addItemNeomorphClasses(props: &BreadcrumbItemProps) -> BreadcrumbItemProps {
    let mut props = props.clone();
    let neomorph_class = if props.current {
        ""
    } else {
        "hover:neomorph-hover-lift px-2 py-1 rounded"
    };
    props.class = shared::build_classes(&[neomorph_class, props.class.as_str()]);
    props
}

#[must_use]
pub fn Breadcrumb(props: &BreadcrumbProps, children: Markup) -> Markup {
    let neo_props = addNeomorphClasses(props);
    base::Breadcrumb(&neo_props, children)
}

#[must_use]
pub fn BreadcrumbItem(props: &BreadcrumbItemProps, children: Markup) -> Markup {
    let neo_props = addItemNeomorphClasses(props);
    base::BreadcrumbItem(&neo_props, children)
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
                .all(|component| component.family == "neomorphism"
                    && component.module == "breadcrumb")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "Breadcrumb")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "BreadcrumbItem")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "BreadcrumbSeparator")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "BreadcrumbEllipsis")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "SimpleBreadcrumb")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "BreadcrumbLink")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "BreadcrumbCurrent")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "BreadcrumbWithSeparator")
        );
    }

    #[test]
    fn wrapped_components_render_unstyled_html_with_neomorphism_classes() {
        let html = Breadcrumb(&BreadcrumbProps::default(), html! { "child" }).into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.starts_with("<"), "expected rendered element: {html}");
        assert!(
            html.contains("neomorph-flat-sm"),
            "missing neomorph-flat-sm: {html}"
        );
        assert!(html.contains("p-2"), "missing p-2: {html}");
        assert!(html.contains("rounded-lg"), "missing rounded-lg: {html}");

        let item_props =
            addItemNeomorphClasses(&BreadcrumbItemProps::default().with_class("custom"));
        assert!(item_props.class.contains("hover:neomorph-hover-lift"));
        assert!(item_props.class.contains("custom"));

        let current_props =
            addItemNeomorphClasses(&BreadcrumbItemProps::default().with_current(true));
        assert!(!current_props.class.contains("hover:neomorph-hover-lift"));

        let item = BreadcrumbItem(&BreadcrumbItemProps::default(), html! { "Home" }).into_string();
        assert!(item.contains("hover:neomorph-hover-lift"));
        assert!(item.contains("Home"));
    }
}
