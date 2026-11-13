#![allow(non_snake_case)]
#![allow(clippy::needless_pass_by_value)]

use crate::components::ComponentInfo;
use crate::components::shared;
use crate::components::unstyled::grid as base;
use maud::Markup;

pub use base::{
    FourColumnGrid, GRID_AUTO_FILL, GRID_AUTO_FIT, GRID_AUTO_NONE, GridItem, GridProps,
    ResponsiveGrid, ThreeColumnGrid, TwoColumnGrid,
};

pub const COMPONENTS: &[ComponentInfo] = &[
    ComponentInfo {
        family: "neomorphism",
        module: "grid",
        name: "Grid",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "grid",
        name: "ResponsiveGrid",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "grid",
        name: "TwoColumnGrid",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "grid",
        name: "ThreeColumnGrid",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "grid",
        name: "FourColumnGrid",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "grid",
        name: "GridItem",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "grid",
        name: "CardGridItem",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "grid",
        name: "CardGrid",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "grid",
        name: "GlassGrid",
    },
];

#[must_use]
fn addNeomorphClasses(props: &GridProps) -> GridProps {
    let mut props = props.clone();
    props.class = shared::build_classes(&[props.class.as_str(), "neomorph-flat-sm"]);
    props
}

#[must_use]
pub fn Grid(props: &GridProps, children: Markup) -> Markup {
    let neo_props = addNeomorphClasses(props);
    base::Grid(&neo_props, children)
}

#[must_use]
pub fn CardGridItem(colSpan: i32, class: &str, children: Markup) -> Markup {
    base::GridItem(
        colSpan,
        shared::build_classes(&["neomorph-flat p-4 rounded-lg", class]),
        children,
    )
}

#[must_use]
pub fn CardGrid(cols: i32, gap: &'static str, class: &str, children: Markup) -> Markup {
    Grid(
        &GridProps::default()
            .with_cols(cols)
            .with_gap(gap)
            .with_class(shared::build_classes(&[
                "neomorph-flat p-4 rounded-lg",
                class,
            ])),
        children,
    )
}

#[must_use]
pub fn GlassGrid(cols: i32, gap: &'static str, class: &str, children: Markup) -> Markup {
    Grid(
        &GridProps::default()
            .with_cols(cols)
            .with_gap(gap)
            .with_class(shared::build_classes(&[
                "neomorph-glass p-4 rounded-lg",
                class,
            ])),
        children,
    )
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
        assert_eq!(COMPONENTS.len(), 9);
        assert!(
            COMPONENTS
                .iter()
                .all(|component| component.family == "neomorphism" && component.module == "grid")
        );
        assert!(COMPONENTS.iter().any(|component| component.name == "Grid"));
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "ResponsiveGrid")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "TwoColumnGrid")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "ThreeColumnGrid")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "FourColumnGrid")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "GridItem")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "CardGridItem")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "CardGrid")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "GlassGrid")
        );
    }

    #[test]
    fn wrapped_components_render_unstyled_html_with_neomorphism_classes() {
        let html = Grid(&GridProps::default(), html! { "child" }).into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.starts_with("<div"));
        assert!(html.contains("grid grid-cols-1 gap-4 neomorph-flat-sm"));
        assert!(html.contains(">child</div>"));
    }

    #[test]
    fn neomorphism_only_components_render_html_without_generated_stub_markers() {
        let html = CardGridItem(2, "sample", html! { "child" }).into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.starts_with("<div"));
        assert!(html.contains("col-span-2 neomorph-flat p-4 rounded-lg sample"));
        assert!(html.contains(">child</div>"));
        let html = CardGrid(2, "gap-4", "sample", html! { "child" }).into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.starts_with("<div"));
        assert!(html.contains(
            "grid grid-cols-2 gap-4 neomorph-flat p-4 rounded-lg sample neomorph-flat-sm"
        ));
        assert!(html.contains(">child</div>"));
        let html = GlassGrid(2, "gap-4", "sample", html! { "child" }).into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.starts_with("<div"));
        assert!(html.contains(
            "grid grid-cols-2 gap-4 neomorph-glass p-4 rounded-lg sample neomorph-flat-sm"
        ));
        assert!(html.contains(">child</div>"));
    }
}
