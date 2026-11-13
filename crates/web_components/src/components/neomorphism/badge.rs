#![allow(non_snake_case)]
#![allow(clippy::needless_pass_by_value)]

use crate::components::ComponentInfo;
use crate::components::shared;
use crate::components::unstyled::badge as base;
use maud::{Markup, html};

pub use base::BadgeProps;

pub const COMPONENTS: &[ComponentInfo] = &[
    ComponentInfo {
        family: "neomorphism",
        module: "badge",
        name: "Badge",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "badge",
        name: "BadgeWithIcon",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "badge",
        name: "BadgeGroup",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "badge",
        name: "DotBadge",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "badge",
        name: "PrimaryBadge",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "badge",
        name: "SuccessBadge",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "badge",
        name: "WarningBadge",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "badge",
        name: "InfoBadge",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "badge",
        name: "DestructiveBadge",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "badge",
        name: "GradientBadge",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "badge",
        name: "GradientBadgeWithIcon",
    },
];

#[must_use]
fn addNeomorphClasses(props: &BadgeProps) -> BadgeProps {
    let mut props = props.clone();
    let neomorph_class = match props.variant {
        shared::VARIANT_PRIMARY | shared::VARIANT_SECONDARY => "neomorph-badge",
        shared::VARIANT_DESTRUCTIVE
        | shared::VARIANT_SUCCESS
        | shared::VARIANT_WARNING
        | shared::VARIANT_INFO
        | shared::VARIANT_OUTLINE => "neomorph-flat-sm",
        _ => "neomorph-flat-sm",
    };
    props.class = shared::build_classes(&[neomorph_class, props.class.as_str()]);
    props
}

#[must_use]
pub fn Badge(props: &BadgeProps, children: Markup) -> Markup {
    let neo_props = addNeomorphClasses(props);
    base::Badge(&neo_props, children)
}

#[must_use]
pub fn BadgeWithIcon(props: &BadgeProps, children: Markup) -> Markup {
    let neo_props = addNeomorphClasses(props);
    base::BadgeWithIcon(&neo_props, children)
}

#[must_use]
pub fn BadgeGroup(class: impl Into<String>, children: Markup) -> Markup {
    base::BadgeGroup(class, children)
}

#[must_use]
pub fn DotBadge(variant: &'static str, class: impl Into<String>) -> Markup {
    let class = class.into();
    base::DotBadge(
        variant,
        shared::build_classes(&["neomorph-flat-sm", class.as_str()]),
    )
}

#[must_use]
fn buildGradientBadgeClasses(from: &str, to: &str, size: &str, class: &str) -> String {
    shared::build_classes(&[
        "inline-flex items-center gap-1.5 font-medium rounded-full",
        shared::badge_size_classes(size),
        format!("bg-gradient-to-r from-{from} to-{to} text-primary-foreground").as_str(),
        class,
    ])
}

#[must_use]
pub fn PrimaryBadge(size: &'static str, pill: bool, class: &str, children: Markup) -> Markup {
    Badge(
        &BadgeProps::default()
            .with_variant(shared::VARIANT_PRIMARY)
            .with_size(size)
            .with_pill(pill)
            .with_class(class),
        children,
    )
}

#[must_use]
pub fn SuccessBadge(size: &'static str, pill: bool, class: &str, children: Markup) -> Markup {
    Badge(
        &BadgeProps::default()
            .with_variant(shared::VARIANT_SUCCESS)
            .with_size(size)
            .with_pill(pill)
            .with_class(class),
        children,
    )
}

#[must_use]
pub fn WarningBadge(size: &'static str, pill: bool, class: &str, children: Markup) -> Markup {
    Badge(
        &BadgeProps::default()
            .with_variant(shared::VARIANT_WARNING)
            .with_size(size)
            .with_pill(pill)
            .with_class(class),
        children,
    )
}

#[must_use]
pub fn InfoBadge(size: &'static str, pill: bool, class: &str, children: Markup) -> Markup {
    Badge(
        &BadgeProps::default()
            .with_variant(shared::VARIANT_INFO)
            .with_size(size)
            .with_pill(pill)
            .with_class(class),
        children,
    )
}

#[must_use]
pub fn DestructiveBadge(size: &'static str, pill: bool, class: &str, children: Markup) -> Markup {
    Badge(
        &BadgeProps::default()
            .with_variant(shared::VARIANT_DESTRUCTIVE)
            .with_size(size)
            .with_pill(pill)
            .with_class(class),
        children,
    )
}

#[must_use]
pub fn GradientBadge(label: &str, from: &str, to: &str) -> Markup {
    html! { span class=(buildGradientBadgeClasses(from, to, shared::SIZE_MD, "")) { (label) } }
}

#[must_use]
pub fn GradientBadgeWithIcon(label: &str, from: &str, to: &str, icon: Markup) -> Markup {
    html! {
        span class=(buildGradientBadgeClasses(from, to, shared::SIZE_MD, "")) {
            span class="w-4 h-4 flex items-center justify-center" { (icon) }
            span { (label) }
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
        assert_eq!(COMPONENTS.len(), 11);
        assert!(
            COMPONENTS
                .iter()
                .all(|component| component.family == "neomorphism" && component.module == "badge")
        );
        assert!(COMPONENTS.iter().any(|component| component.name == "Badge"));
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "BadgeWithIcon")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "BadgeGroup")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "DotBadge")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "PrimaryBadge")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "SuccessBadge")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "WarningBadge")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "InfoBadge")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "DestructiveBadge")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "GradientBadge")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "GradientBadgeWithIcon")
        );
    }

    #[test]
    fn neomorphism_only_components_render_html_without_generated_stub_markers() {
        let props = BadgeProps::default()
            .with_variant(shared::VARIANT_OUTLINE)
            .with_class("custom");
        let neo_props = addNeomorphClasses(&props);
        assert!(neo_props.class.contains("neomorph-flat-sm"));
        assert!(neo_props.class.contains("custom"));
        let default_props = addNeomorphClasses(&BadgeProps::default());
        assert!(default_props.class.contains("neomorph-flat-sm"));

        let html = BadgeWithIcon(
            &BadgeProps::default().with_variant(shared::VARIANT_SECONDARY),
            html! { span class="icon" { "!" } "Label" },
        )
        .into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.contains("neomorph-badge"));
        assert!(html.contains("icon"));
        assert!(html.contains("Label"));

        let html = BadgeGroup("gap-test", html! { span { "One" } span { "Two" } }).into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.contains("<div class=\"flex flex-wrap gap-2 gap-test\">"));
        assert!(html.contains(">One</span>"));
        assert!(html.contains(">Two</span>"));

        let html = DotBadge(shared::VARIANT_WARNING, "dot-extra").into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.contains("rounded-full"));
        assert!(html.contains("bg-warning"));
        assert!(html.contains("neomorph-flat-sm"));
        assert!(html.contains("dot-extra"));

        let html = PrimaryBadge(shared::SIZE_MD, true, "sample", html! { "child" }).into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.contains("<span class=\"inline-flex"));
        assert!(html.contains("neomorph-badge"));
        assert!(html.contains("child"));
        let html = SuccessBadge(shared::SIZE_MD, true, "sample", html! { "child" }).into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.contains("neomorph-flat-sm"));
        assert!(html.contains("bg-success"));
        let html = WarningBadge(shared::SIZE_MD, true, "sample", html! { "child" }).into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.contains("neomorph-flat-sm"));
        assert!(html.contains("bg-warning"));
        let html = InfoBadge(shared::SIZE_MD, true, "sample", html! { "child" }).into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.contains("neomorph-flat-sm"));
        assert!(html.contains("bg-info"));
        let html =
            DestructiveBadge(shared::SIZE_MD, true, "sample", html! { "child" }).into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.contains("neomorph-flat-sm"));
        assert!(html.contains("bg-destructive"));
        let html = GradientBadge("sample", "sample", "sample").into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.contains("bg-gradient-to-r from-sample to-sample"));
        assert!(html.contains(">sample</span>"));
        let html =
            GradientBadgeWithIcon("sample", "sample", "sample", html! { "child" }).into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.contains("w-4 h-4 flex items-center justify-center"));
        assert!(html.contains("child"));
    }
}
