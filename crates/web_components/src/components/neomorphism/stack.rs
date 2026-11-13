#![allow(non_snake_case)]
#![allow(clippy::needless_pass_by_value)]

use crate::components::ComponentInfo;
use crate::components::shared;
use crate::components::unstyled::stack as base;
use maud::Markup;

pub use base::{HStack, Spacer, StackProps, VStack};

pub const COMPONENTS: &[ComponentInfo] = &[
    ComponentInfo {
        family: "neomorphism",
        module: "stack",
        name: "Stack",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "stack",
        name: "VStack",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "stack",
        name: "HStack",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "stack",
        name: "Spacer",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "stack",
        name: "CardStack",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "stack",
        name: "GlassStack",
    },
];

#[must_use]
fn addNeomorphClasses(props: &StackProps) -> StackProps {
    let mut props = props.clone();
    props.class = shared::build_classes(&[props.class.as_str(), "neomorph-flat-sm"]);
    props
}

#[must_use]
pub fn Stack(props: &StackProps, children: Markup) -> Markup {
    let neo_props = addNeomorphClasses(props);
    base::Stack(&neo_props, children)
}

#[must_use]
pub fn CardStack(
    direction: &'static str,
    gap: &'static str,
    class: &str,
    children: Markup,
) -> Markup {
    Stack(
        &StackProps::default()
            .with_direction(direction)
            .with_gap(gap)
            .with_align("stretch")
            .with_justify("start")
            .with_class(shared::build_classes(&[
                "neomorph-flat p-4 rounded-lg",
                class,
            ])),
        children,
    )
}

#[must_use]
pub fn GlassStack(
    direction: &'static str,
    gap: &'static str,
    class: &str,
    children: Markup,
) -> Markup {
    Stack(
        &StackProps::default()
            .with_direction(direction)
            .with_gap(gap)
            .with_align("stretch")
            .with_justify("start")
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
        assert_eq!(COMPONENTS.len(), 6);
        assert!(
            COMPONENTS
                .iter()
                .all(|component| component.family == "neomorphism" && component.module == "stack")
        );
        assert!(COMPONENTS.iter().any(|component| component.name == "Stack"));
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "VStack")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "HStack")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "Spacer")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "CardStack")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "GlassStack")
        );
    }

    #[test]
    fn wrapped_components_render_unstyled_html_with_neomorphism_classes() {
        let html = Stack(&StackProps::default(), html! { "child" }).into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.starts_with("<div"));
        assert!(html.contains("flex flex-col gap-4 items-stretch justify-start neomorph-flat-sm"));
        assert!(html.contains(">child</div>"));
    }

    #[test]
    fn neomorphism_only_components_render_html_without_generated_stub_markers() {
        let html = CardStack("vertical", "md", "sample", html! { "child" }).into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.starts_with("<div"));
        assert!(html.contains("flex flex-col gap-4 items-stretch justify-start neomorph-flat p-4 rounded-lg sample neomorph-flat-sm"));
        assert!(html.contains(">child</div>"));
        let html = GlassStack("horizontal", "md", "sample", html! { "child" }).into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.starts_with("<div"));
        assert!(html.contains("flex flex-row gap-4 items-stretch justify-start neomorph-glass p-4 rounded-lg sample neomorph-flat-sm"));
        assert!(html.contains(">child</div>"));
    }
}
