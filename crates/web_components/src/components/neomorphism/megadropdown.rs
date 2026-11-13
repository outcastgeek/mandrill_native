#![allow(non_snake_case)]
#![allow(clippy::needless_pass_by_value)]

use crate::components::ComponentInfo;
use crate::components::shared;
use crate::components::unstyled::megadropdown as base;
use maud::{Markup, html};

pub use base::{
    MegaDropdownFeatureLinkProps, MegaDropdownFeaturedCardProps, MegaDropdownFooter,
    MegaDropdownFooterLink, MegaDropdownFooterProps, MegaDropdownNavProps, MegaDropdownPanel,
    MegaDropdownPanelProps, MegaDropdownSection, MegaDropdownSectionProps, MegaDropdownTrigger,
    MegaDropdownTriggerProps, SimpleMegaDropdown,
};

pub const COMPONENTS: &[ComponentInfo] = &[
    ComponentInfo {
        family: "neomorphism",
        module: "megadropdown",
        name: "MegaDropdownNav",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "megadropdown",
        name: "MegaDropdownTrigger",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "megadropdown",
        name: "MegaDropdownPanel",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "megadropdown",
        name: "MegaDropdownPanelNoCard",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "megadropdown",
        name: "MegaDropdownFeatureLink",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "megadropdown",
        name: "MegaDropdownFeaturedCard",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "megadropdown",
        name: "MegaDropdownFooter",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "megadropdown",
        name: "MegaDropdownSection",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "megadropdown",
        name: "SimpleMegaDropdown",
    },
];

#[must_use]
fn addNeomorphNavClasses(props: &MegaDropdownNavProps) -> MegaDropdownNavProps {
    props.clone()
}

#[must_use]
fn addNeomorphFeatureLinkClasses(
    props: &MegaDropdownFeatureLinkProps,
) -> MegaDropdownFeatureLinkProps {
    let mut props = props.clone();
    if props.class.is_empty() {
        props.class = "neomorph-hover-subtle".to_string();
    }
    props
}

#[must_use]
fn addNeomorphFeaturedCardClasses(
    props: &MegaDropdownFeaturedCardProps,
) -> MegaDropdownFeaturedCardProps {
    let mut props = props.clone();
    if props.class.is_empty() {
        props.class = "neomorph-card".to_string();
    }
    props
}

#[must_use]
pub fn MegaDropdownNav(props: &MegaDropdownNavProps) -> Markup {
    let neo_props = addNeomorphNavClasses(props);
    base::MegaDropdownNav(&neo_props)
}

#[must_use]
pub fn MegaDropdownFeatureLink(props: &MegaDropdownFeatureLinkProps) -> Markup {
    let neo_props = addNeomorphFeatureLinkClasses(props);
    base::MegaDropdownFeatureLink(&neo_props)
}

#[must_use]
pub fn MegaDropdownFeaturedCard(props: &MegaDropdownFeaturedCardProps) -> Markup {
    let neo_props = addNeomorphFeaturedCardClasses(props);
    base::MegaDropdownFeaturedCard(&neo_props)
}

#[must_use]
pub fn MegaDropdownPanelNoCard(props: &MegaDropdownPanelProps, children: Markup) -> Markup {
    let attrs = props.build_attributes();
    html! {
        div class=(props.build_panel_classes()) style=(format!("display: none; {}", props.build_panel_style())) data-show=(base::panelShowExpr(props)) data-testid=[shared::attr_value(&attrs, "data-testid")] {
            div class=(props.build_content_classes()) { (children) }
        }
    }
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
        assert!(COMPONENTS.iter().all(
            |component| component.family == "neomorphism" && component.module == "megadropdown"
        ));
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "MegaDropdownNav")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "MegaDropdownTrigger")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "MegaDropdownPanel")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "MegaDropdownPanelNoCard")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "MegaDropdownFeatureLink")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "MegaDropdownFeaturedCard")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "MegaDropdownFooter")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "MegaDropdownSection")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "SimpleMegaDropdown")
        );
    }

    #[test]
    fn wrapped_components_render_unstyled_html_with_neomorphism_classes() {
        let html = MegaDropdownNav(&MegaDropdownNavProps::default()).into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.starts_with("<"), "expected rendered element: {html}");

        let feature = addNeomorphFeatureLinkClasses(&MegaDropdownFeatureLinkProps::default());
        assert_eq!(feature.class, "neomorph-hover-subtle");
        let custom_feature = addNeomorphFeatureLinkClasses(
            &MegaDropdownFeatureLinkProps::default().with_class("custom"),
        );
        assert_eq!(custom_feature.class, "custom");
        let feature_html = MegaDropdownFeatureLink(&MegaDropdownFeatureLinkProps {
            title: "Feature".to_string(),
            href: "/feature".to_string(),
            ..Default::default()
        })
        .into_string();
        assert!(feature_html.contains("neomorph-hover-subtle"));
        assert!(feature_html.contains(">Feature</"));

        let card = addNeomorphFeaturedCardClasses(&MegaDropdownFeaturedCardProps::default());
        assert_eq!(card.class, "neomorph-card");
        let custom_card = addNeomorphFeaturedCardClasses(
            &MegaDropdownFeaturedCardProps::default().with_class("custom"),
        );
        assert_eq!(custom_card.class, "custom");
        let card_html = MegaDropdownFeaturedCard(&MegaDropdownFeaturedCardProps {
            title: "Card".to_string(),
            href: "/card".to_string(),
            ..Default::default()
        })
        .into_string();
        assert!(card_html.contains("neomorph-card"));
        assert!(card_html.contains(">Card</"));
    }

    #[test]
    fn neomorphism_only_components_render_html_without_generated_stub_markers() {
        let html = MegaDropdownPanelNoCard(
            &MegaDropdownPanelProps::default().with_id("panel"),
            html! { "child" },
        )
        .into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.contains("data-show=\"$activeDropdown === 'panel'\""));
        assert!(html.contains("overflow-hidden rounded-2xl"));
        assert!(html.contains("child"));
    }
}
