#![allow(non_snake_case)]
#![allow(clippy::needless_pass_by_value)]

use crate::components::ComponentInfo;
use crate::components::shared;
use crate::components::unstyled::heading as base;
use maud::Markup;

pub use base::{H1, H2, H3, H4, H5, H6, HeadingProps, PageTitle, SectionTitle, SubsectionTitle};

pub const COMPONENTS: &[ComponentInfo] = &[
    ComponentInfo {
        family: "neomorphism",
        module: "heading",
        name: "Heading",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "heading",
        name: "H1",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "heading",
        name: "H2",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "heading",
        name: "H3",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "heading",
        name: "H4",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "heading",
        name: "H5",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "heading",
        name: "H6",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "heading",
        name: "PageTitle",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "heading",
        name: "SectionTitle",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "heading",
        name: "SubsectionTitle",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "heading",
        name: "NeomorphicHeroTitle",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "heading",
        name: "NeomorphicCardTitle",
    },
];

#[must_use]
fn addNeomorphClasses(props: &HeadingProps) -> HeadingProps {
    let mut props = props.clone();
    let neomorph_class = if props.gradient {
        "neomorph-text-shadow"
    } else {
        ""
    };
    props.class = shared::build_classes(&[neomorph_class, props.class.as_str()]);
    props
}

#[must_use]
pub fn Heading(props: &HeadingProps, children: Markup) -> Markup {
    let neo_props = addNeomorphClasses(props);
    base::Heading(&neo_props, children)
}

#[must_use]
pub fn NeomorphicHeroTitle(gradient: bool, children: Markup) -> Markup {
    base::Heading(
        &HeadingProps::default()
            .with_level(1)
            .with_size("6xl")
            .with_weight("extrabold")
            .with_gradient(gradient)
            .with_align(shared::ALIGN_CENTER)
            .with_class("neomorph-text-emboss mb-12"),
        children,
    )
}

#[must_use]
pub fn NeomorphicCardTitle(level: i32, size: &'static str, children: Markup) -> Markup {
    base::Heading(
        &HeadingProps::default()
            .with_level(level)
            .with_size(size)
            .with_weight(shared::TEXT_WEIGHT_BOLD)
            .with_class("mb-4"),
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
        assert_eq!(COMPONENTS.len(), 12);
        assert!(COMPONENTS.iter().all(|component| component.family == "neomorphism" && component.module == "heading"));
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "Heading")
        );
        assert!(COMPONENTS.iter().any(|component| component.name == "H1"));
        assert!(COMPONENTS.iter().any(|component| component.name == "H2"));
        assert!(COMPONENTS.iter().any(|component| component.name == "H3"));
        assert!(COMPONENTS.iter().any(|component| component.name == "H4"));
        assert!(COMPONENTS.iter().any(|component| component.name == "H5"));
        assert!(COMPONENTS.iter().any(|component| component.name == "H6"));
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "PageTitle")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "SectionTitle")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "SubsectionTitle")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "NeomorphicHeroTitle")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "NeomorphicCardTitle")
        );
    }

    #[test]
    fn neomorphism_only_components_render_html_without_generated_stub_markers() {
        let html = NeomorphicHeroTitle(true, html! { "Hero" }).into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.contains("<h1"));
        assert!(html.contains("font-extrabold"));
        assert!(html.contains("neomorph-text-emboss mb-12"));
        let html = NeomorphicCardTitle(2, "xl", html! { "Card" }).into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.contains("<h2"));
        assert!(html.contains("text-xl"));
        assert!(html.contains(">Card</h2>"));
    }

    #[test]
    fn heading_helper_adds_shadow_only_for_gradients() {
        let plain = addNeomorphClasses(&HeadingProps::default().with_class("custom"));
        assert_eq!(plain.class, "custom");

        let gradient = addNeomorphClasses(
            &HeadingProps::default()
                .with_gradient(true)
                .with_class("custom"),
        );
        assert_eq!(gradient.class, "neomorph-text-shadow custom");

        let rendered = Heading(
            &HeadingProps::default()
                .with_gradient(true)
                .with_class("custom"),
            html! { "Gradient" },
        )
        .into_string();
        assert!(rendered.contains("neomorph-text-shadow"));
        assert!(rendered.contains("custom"));
        assert!(rendered.contains("Gradient"));
    }
}
