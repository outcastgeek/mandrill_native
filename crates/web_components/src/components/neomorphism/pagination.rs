#![allow(non_snake_case)]
#![allow(clippy::needless_pass_by_value)]

use crate::components::ComponentInfo;
use crate::components::shared;
use crate::components::unstyled::pagination as base;
use maud::Markup;

pub use base::{PaginationItemProps, PaginationProps, SimplePaginationProps};

pub const COMPONENTS: &[ComponentInfo] = &[
    ComponentInfo {
        family: "neomorphism",
        module: "pagination",
        name: "PaginationItem",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "pagination",
        name: "Pagination",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "pagination",
        name: "SimplePagination",
    },
];

#[must_use]
fn addItemClasses(props: &PaginationItemProps) -> PaginationItemProps {
    let mut props = props.clone();
    props.class = shared::build_classes(&[props.class.as_str(), "neomorph-flat"]);
    props
}

#[must_use]
pub fn PaginationItem(props: &PaginationItemProps) -> Markup {
    let neo_props = addItemClasses(props);
    base::PaginationItem(&neo_props)
}

#[must_use]
fn addPaginationClasses(props: &PaginationProps) -> PaginationProps {
    let mut props = props.clone();
    props.class = shared::build_classes(&[props.class.as_str(), "neomorph-flat"]);
    props
}

#[must_use]
pub fn Pagination(props: &PaginationProps) -> Markup {
    let neo_props = addPaginationClasses(props);
    base::Pagination(&neo_props)
}

#[must_use]
fn addSimplePaginationClasses(props: &SimplePaginationProps) -> SimplePaginationProps {
    let mut props = props.clone();
    props.class = shared::build_classes(&[props.class.as_str(), "neomorph-flat"]);
    props
}

#[must_use]
pub fn SimplePagination(props: &SimplePaginationProps) -> Markup {
    let neo_props = addSimplePaginationClasses(props);
    base::SimplePagination(&neo_props)
}

#[cfg(test)]
mod tests {
    use super::*;
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
        assert_eq!(COMPONENTS.len(), 3);
        assert!(
            COMPONENTS
                .iter()
                .all(|component| component.family == "neomorphism"
                    && component.module == "pagination")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "PaginationItem")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "Pagination")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "SimplePagination")
        );
    }

    #[test]
    fn wrapped_components_render_unstyled_html_with_neomorphism_classes() {
        let html = PaginationItem(&PaginationItemProps::default()).into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.starts_with("<"), "expected rendered element: {html}");
        assert!(
            html.contains("neomorph-flat"),
            "missing neomorph-flat: {html}"
        );
        let html = Pagination(
            &PaginationProps::default()
                .with_total_pages(3)
                .with_base_href("/page"),
        )
        .into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.starts_with("<"), "expected rendered element: {html}");
        assert!(
            html.contains("neomorph-flat"),
            "missing neomorph-flat: {html}"
        );
        let html = SimplePagination(
            &SimplePaginationProps::default()
                .with_current_page(2)
                .with_total_pages(3)
                .with_prev_href("/page/1")
                .with_next_href("/page/3"),
        )
        .into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.starts_with("<"), "expected rendered element: {html}");
        assert!(
            html.contains("neomorph-flat"),
            "missing neomorph-flat: {html}"
        );
    }
}
