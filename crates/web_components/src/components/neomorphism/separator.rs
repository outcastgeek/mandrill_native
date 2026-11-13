#![allow(non_snake_case)]
#![allow(clippy::needless_pass_by_value)]

use crate::components::ComponentInfo;
use crate::components::shared;
use crate::components::unstyled::separator as base;
use maud::Markup;

pub use base::{
    DashedSeparator, DottedSeparator, HorizontalSeparator, SeparatorProps, SeparatorWithText,
    VerticalSeparator,
};

pub const COMPONENTS: &[ComponentInfo] = &[
    ComponentInfo {
        family: "neomorphism",
        module: "separator",
        name: "Separator",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "separator",
        name: "HorizontalSeparator",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "separator",
        name: "VerticalSeparator",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "separator",
        name: "DashedSeparator",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "separator",
        name: "DottedSeparator",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "separator",
        name: "SeparatorWithText",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "separator",
        name: "InsetSeparator",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "separator",
        name: "RaisedSeparator",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "separator",
        name: "GlassSeparator",
    },
];

#[must_use]
fn addNeomorphClasses(props: &SeparatorProps) -> SeparatorProps {
    let mut props = props.clone();
    props.class = shared::build_classes(&[props.class.as_str(), "neomorph-divider"]);
    props
}

#[must_use]
pub fn Separator(props: &SeparatorProps) -> Markup {
    let neo_props = addNeomorphClasses(props);
    base::Separator(&neo_props)
}

#[must_use]
pub fn InsetSeparator(orientation: &'static str, spacing: &'static str, class: &str) -> Markup {
    Separator(
        &SeparatorProps::default()
            .with_orientation(orientation)
            .with_style("solid")
            .with_spacing(spacing)
            .with_class(shared::build_classes(&["neomorph-concave-sm", class])),
    )
}

#[must_use]
pub fn RaisedSeparator(orientation: &'static str, spacing: &'static str, class: &str) -> Markup {
    Separator(
        &SeparatorProps::default()
            .with_orientation(orientation)
            .with_style("solid")
            .with_spacing(spacing)
            .with_class(shared::build_classes(&["neomorph-convex", class])),
    )
}

#[must_use]
pub fn GlassSeparator(orientation: &'static str, spacing: &'static str, class: &str) -> Markup {
    Separator(
        &SeparatorProps::default()
            .with_orientation(orientation)
            .with_style("solid")
            .with_spacing(spacing)
            .with_class(shared::build_classes(&["neomorph-glass opacity-50", class])),
    )
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
        assert_eq!(COMPONENTS.len(), 9);
        assert!(
            COMPONENTS
                .iter()
                .all(|component| component.family == "neomorphism"
                    && component.module == "separator")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "Separator")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "HorizontalSeparator")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "VerticalSeparator")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "DashedSeparator")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "DottedSeparator")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "SeparatorWithText")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "InsetSeparator")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "RaisedSeparator")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "GlassSeparator")
        );
    }

    #[test]
    fn wrapped_components_render_unstyled_html_with_neomorphism_classes() {
        let html = Separator(&SeparatorProps::default()).into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.starts_with("<div"));
        assert!(html.contains("class=\"w-full my-4 neomorph-divider\""));
        assert!(
            html.contains("<div class=\"border-border border-solid border-t w-full h-0\"></div>")
        );
        assert!(html.contains("role=\"separator\""));
    }

    #[test]
    fn neomorphism_only_components_render_html_without_generated_stub_markers() {
        let html = InsetSeparator("horizontal", "md", "sample").into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.starts_with("<div"));
        assert!(html.contains("class=\"w-full my-4 neomorph-concave-sm sample neomorph-divider\""));
        assert!(
            html.contains("<div class=\"border-border border-solid border-t w-full h-0\"></div>")
        );
        assert!(html.contains("role=\"separator\""));
        let html = RaisedSeparator("vertical", "sm", "sample").into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.starts_with("<div"));
        assert!(html.contains(
            "class=\"inline-block h-full mx-2 neomorph-convex sample neomorph-divider\""
        ));
        assert!(
            html.contains("<div class=\"border-border border-solid border-l h-full w-0\"></div>")
        );
        assert!(html.contains("aria-orientation=\"vertical\""));
        assert!(html.contains("role=\"separator\""));
        let html = GlassSeparator("horizontal", "lg", "sample").into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.starts_with("<div"));
        assert!(
            html.contains(
                "class=\"w-full my-6 neomorph-glass opacity-50 sample neomorph-divider\""
            )
        );
        assert!(
            html.contains("<div class=\"border-border border-solid border-t w-full h-0\"></div>")
        );
        assert!(html.contains("role=\"separator\""));
    }
}
