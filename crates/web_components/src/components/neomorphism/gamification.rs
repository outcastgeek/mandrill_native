#![allow(non_snake_case)]
#![allow(clippy::needless_pass_by_value)]

use crate::components::ComponentInfo;
use crate::components::shared;
use crate::components::unstyled::gamification as base;
use maud::Markup;

pub use base::{
    ActionItemCardProps, DifficultyBadgeProps, EarnedItemCard, FileItemCardProps,
    GradientStatCardProps, ItemCardProps, LevelUpOverlayProps, LockedItemCardProps,
    NotificationPillContainer, NotificationPillProps, PrimaryGradientStatCard, ScoreBadgeProps,
    SimpleDifficultyBadge, SimpleFileItemCard, SimpleGradientStatCard, SimpleItemCard,
    SimpleLockedItemCard, SimpleScoreBadge, SimpleStarRating, SimpleStatusBadge, StarRatingProps,
    StarRatingWithLabel, StatusBadgeProps, SuccessGradientStatCard, WarningGradientStatCard,
};

pub const COMPONENTS: &[ComponentInfo] = &[
    ComponentInfo {
        family: "neomorphism",
        module: "gamification",
        name: "ScoreBadge",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "gamification",
        name: "StatusBadge",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "gamification",
        name: "DifficultyBadge",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "gamification",
        name: "SimpleScoreBadge",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "gamification",
        name: "SimpleStatusBadge",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "gamification",
        name: "SimpleDifficultyBadge",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "gamification",
        name: "GradientStatCard",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "gamification",
        name: "SimpleGradientStatCard",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "gamification",
        name: "SuccessGradientStatCard",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "gamification",
        name: "PrimaryGradientStatCard",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "gamification",
        name: "WarningGradientStatCard",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "gamification",
        name: "ItemCard",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "gamification",
        name: "SimpleItemCard",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "gamification",
        name: "EarnedItemCard",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "gamification",
        name: "LockedItemCard",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "gamification",
        name: "SimpleLockedItemCard",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "gamification",
        name: "ActionItemCard",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "gamification",
        name: "StarRating",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "gamification",
        name: "SimpleStarRating",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "gamification",
        name: "StarRatingWithLabel",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "gamification",
        name: "FileItemCard",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "gamification",
        name: "SimpleFileItemCard",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "gamification",
        name: "FileItemCardWithAction",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "gamification",
        name: "NotificationPill",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "gamification",
        name: "NotificationPillContainer",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "gamification",
        name: "LevelUpOverlay",
    },
];

#[must_use]
fn addScoreBadgeClasses(props: &ScoreBadgeProps) -> ScoreBadgeProps {
    let mut props = props.clone();
    props.class = shared::build_classes(&[props.class.as_str(), "neomorph-flat"]);
    props
}

#[must_use]
pub fn ScoreBadge(props: &ScoreBadgeProps) -> Markup {
    let neo_props = addScoreBadgeClasses(props);
    base::ScoreBadge(&neo_props)
}

#[must_use]
fn addStatusBadgeClasses(props: &StatusBadgeProps) -> StatusBadgeProps {
    let mut props = props.clone();
    props.class = shared::build_classes(&[props.class.as_str(), "neomorph-flat"]);
    props
}

#[must_use]
pub fn StatusBadge(props: &StatusBadgeProps) -> Markup {
    let neo_props = addStatusBadgeClasses(props);
    base::StatusBadge(&neo_props)
}

#[must_use]
fn addDifficultyBadgeClasses(props: &DifficultyBadgeProps) -> DifficultyBadgeProps {
    let mut props = props.clone();
    props.class = shared::build_classes(&[props.class.as_str(), "neomorph-flat"]);
    props
}

#[must_use]
pub fn DifficultyBadge(props: &DifficultyBadgeProps) -> Markup {
    let neo_props = addDifficultyBadgeClasses(props);
    base::DifficultyBadge(&neo_props)
}

#[must_use]
fn addGradientStatCardClasses(props: &GradientStatCardProps) -> GradientStatCardProps {
    let mut props = props.clone();
    props.class =
        shared::build_classes(&[props.class.as_str(), "neomorph-flat neomorph-hover-lift"]);
    props
}

#[must_use]
pub fn GradientStatCard(props: &GradientStatCardProps) -> Markup {
    let neo_props = addGradientStatCardClasses(props);
    base::GradientStatCard(&neo_props)
}

#[must_use]
fn addItemCardClasses(props: &ItemCardProps) -> ItemCardProps {
    let mut props = props.clone();
    props.class =
        shared::build_classes(&[props.class.as_str(), "neomorph-flat neomorph-hover-lift"]);
    props
}

#[must_use]
pub fn ItemCard(props: &ItemCardProps) -> Markup {
    let neo_props = addItemCardClasses(props);
    base::ItemCard(&neo_props)
}

#[must_use]
fn addLockedItemCardClasses(props: &LockedItemCardProps) -> LockedItemCardProps {
    let mut props = props.clone();
    props.class = shared::build_classes(&[props.class.as_str(), "neomorph-flat"]);
    props
}

#[must_use]
fn addActionItemCardClasses(props: &ActionItemCardProps) -> ActionItemCardProps {
    let mut props = props.clone();
    props.item.class = shared::build_classes(&[
        props.item.class.as_str(),
        "neomorph-flat neomorph-hover-lift",
    ]);
    props
}

#[must_use]
pub fn LockedItemCard(props: &LockedItemCardProps) -> Markup {
    let neo_props = addLockedItemCardClasses(props);
    base::LockedItemCard(&neo_props)
}

#[must_use]
pub fn ActionItemCard(props: &ActionItemCardProps, children: Markup) -> Markup {
    let neo_props = addActionItemCardClasses(props);
    base::ActionItemCard(&neo_props, children)
}

#[must_use]
fn addStarRatingClasses(props: &StarRatingProps) -> StarRatingProps {
    let mut props = props.clone();
    props.class = shared::build_classes(&[props.class.as_str(), "neomorph-flat"]);
    props
}

#[must_use]
pub fn StarRating(props: &StarRatingProps) -> Markup {
    let neo_props = addStarRatingClasses(props);
    base::StarRating(&neo_props)
}

#[must_use]
fn addFileItemCardClasses(props: &FileItemCardProps) -> FileItemCardProps {
    let mut props = props.clone();
    props.class =
        shared::build_classes(&[props.class.as_str(), "neomorph-flat neomorph-hover-lift"]);
    props
}

#[must_use]
pub fn FileItemCard(props: &FileItemCardProps) -> Markup {
    let neo_props = addFileItemCardClasses(props);
    base::FileItemCard(&neo_props)
}

#[must_use]
pub fn FileItemCardWithAction(props: &FileItemCardProps, children: Markup) -> Markup {
    let neo_props = addFileItemCardClasses(props);
    base::FileItemCardWithAction(&neo_props, children)
}

#[must_use]
fn addNotificationPillClasses(props: &NotificationPillProps) -> NotificationPillProps {
    let mut props = props.clone();
    props.class = shared::build_classes(&[props.class.as_str(), "neomorph-flat"]);
    props
}

#[must_use]
pub fn NotificationPill(props: &NotificationPillProps) -> Markup {
    let neo_props = addNotificationPillClasses(props);
    base::NotificationPill(&neo_props)
}

#[must_use]
fn addLevelUpOverlayClasses(props: &LevelUpOverlayProps) -> LevelUpOverlayProps {
    let mut props = props.clone();
    props.class = shared::build_classes(&[props.class.as_str(), "neomorph-flat"]);
    props
}

#[must_use]
pub fn LevelUpOverlay(props: &LevelUpOverlayProps, children: Markup) -> Markup {
    let neo_props = addLevelUpOverlayClasses(props);
    base::LevelUpOverlay(&neo_props, children)
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
        assert_eq!(COMPONENTS.len(), 26);
        assert!(COMPONENTS.iter().all(
            |component| component.family == "neomorphism" && component.module == "gamification"
        ));
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "ScoreBadge")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "StatusBadge")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "DifficultyBadge")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "SimpleScoreBadge")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "SimpleStatusBadge")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "SimpleDifficultyBadge")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "GradientStatCard")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "SimpleGradientStatCard")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "SuccessGradientStatCard")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "PrimaryGradientStatCard")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "WarningGradientStatCard")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "ItemCard")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "SimpleItemCard")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "EarnedItemCard")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "LockedItemCard")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "SimpleLockedItemCard")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "ActionItemCard")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "StarRating")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "SimpleStarRating")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "StarRatingWithLabel")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "FileItemCard")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "SimpleFileItemCard")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "FileItemCardWithAction")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "NotificationPill")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "NotificationPillContainer")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "LevelUpOverlay")
        );
    }

    #[test]
    fn wrapped_components_render_unstyled_html_with_neomorphism_classes() {
        let html = ScoreBadge(&ScoreBadgeProps::default()).into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.starts_with("<"), "expected rendered element: {html}");
        assert!(
            html.contains("neomorph-flat"),
            "missing neomorph-flat: {html}"
        );
        let html = StatusBadge(&StatusBadgeProps::default()).into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.starts_with("<"), "expected rendered element: {html}");
        assert!(
            html.contains("neomorph-flat"),
            "missing neomorph-flat: {html}"
        );
        let html = DifficultyBadge(&DifficultyBadgeProps::default()).into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.starts_with("<"), "expected rendered element: {html}");
        assert!(
            html.contains("neomorph-flat"),
            "missing neomorph-flat: {html}"
        );
        let html = GradientStatCard(&GradientStatCardProps::default()).into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.starts_with("<"), "expected rendered element: {html}");
        assert!(
            html.contains("neomorph-flat"),
            "missing neomorph-flat: {html}"
        );
        assert!(
            html.contains("neomorph-hover-lift"),
            "missing neomorph-hover-lift: {html}"
        );
        let html = ItemCard(&ItemCardProps::default()).into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.starts_with("<"), "expected rendered element: {html}");
        assert!(
            html.contains("neomorph-flat"),
            "missing neomorph-flat: {html}"
        );
        assert!(
            html.contains("neomorph-hover-lift"),
            "missing neomorph-hover-lift: {html}"
        );
        let html = LockedItemCard(&LockedItemCardProps::default()).into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.starts_with("<"), "expected rendered element: {html}");
        assert!(
            html.contains("neomorph-flat"),
            "missing neomorph-flat: {html}"
        );
        let html = StarRating(&StarRatingProps::default()).into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.starts_with("<"), "expected rendered element: {html}");
        assert!(
            html.contains("neomorph-flat"),
            "missing neomorph-flat: {html}"
        );
        let html = FileItemCard(&FileItemCardProps::default()).into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.starts_with("<"), "expected rendered element: {html}");
        assert!(
            html.contains("neomorph-flat"),
            "missing neomorph-flat: {html}"
        );
        assert!(
            html.contains("neomorph-hover-lift"),
            "missing neomorph-hover-lift: {html}"
        );
        let html =
            FileItemCardWithAction(&FileItemCardProps::default(), html! { "child" }).into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.starts_with("<"), "expected rendered element: {html}");
        assert!(
            html.contains("neomorph-flat"),
            "missing neomorph-flat: {html}"
        );
        assert!(
            html.contains("neomorph-hover-lift"),
            "missing neomorph-hover-lift: {html}"
        );
        let action_props = addActionItemCardClasses(
            &ActionItemCardProps::default()
                .with_item(ItemCardProps::default().with_class("custom")),
        );
        assert!(action_props.item.class.contains("custom"));
        assert!(action_props.item.class.contains("neomorph-flat"));
        assert!(action_props.item.class.contains("neomorph-hover-lift"));
        let html =
            ActionItemCard(&ActionItemCardProps::default(), html! { "Action" }).into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.starts_with("<"), "expected rendered element: {html}");
        assert!(html.contains("neomorph-flat"));
        assert!(html.contains("neomorph-hover-lift"));
        assert!(html.contains("Action"));
        let html = NotificationPill(&NotificationPillProps::default()).into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.starts_with("<"), "expected rendered element: {html}");
        assert!(
            html.contains("neomorph-flat"),
            "missing neomorph-flat: {html}"
        );
        let html = LevelUpOverlay(&LevelUpOverlayProps::default(), html! { "child" }).into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.starts_with("<"), "expected rendered element: {html}");
        assert!(
            html.contains("neomorph-flat"),
            "missing neomorph-flat: {html}"
        );
    }
}
