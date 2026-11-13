#![allow(non_snake_case)]
#![allow(clippy::needless_pass_by_value)]

use crate::components::ComponentInfo;
use crate::components::shared;
use crate::components::unstyled::avatar as base;
use maud::Markup;

pub use base::{AvatarFallbackIcon, AvatarGroup, AvatarProps};

pub const COMPONENTS: &[ComponentInfo] = &[
    ComponentInfo {
        family: "neomorphism",
        module: "avatar",
        name: "Avatar",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "avatar",
        name: "AvatarWithIcon",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "avatar",
        name: "AvatarGroup",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "avatar",
        name: "AvatarFallbackIcon",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "avatar",
        name: "CircleAvatar",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "avatar",
        name: "SquareAvatar",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "avatar",
        name: "RoundedAvatar",
    },
];

#[must_use]
fn addNeomorphClasses(props: &AvatarProps) -> AvatarProps {
    let mut props = props.clone();
    props.class = shared::build_classes(&[props.class.as_str(), "neomorph-flat-sm"]);
    props
}

#[must_use]
pub fn Avatar(props: &AvatarProps, children: Markup) -> Markup {
    let neo_props = addNeomorphClasses(props);
    base::Avatar(&neo_props, children)
}

#[must_use]
pub fn AvatarWithIcon(props: &AvatarProps, icon: Markup) -> Markup {
    let neo_props = addNeomorphClasses(props);
    base::AvatarWithIcon(&neo_props, icon)
}

#[must_use]
pub fn CircleAvatar(
    src: &str,
    alt: &str,
    initials: &str,
    size: &'static str,
    status: &str,
) -> Markup {
    Avatar(
        &AvatarProps::default()
            .with_src(src)
            .with_alt(alt)
            .with_initials(initials)
            .with_size(size)
            .with_shape(shared::AVATAR_SHAPE_CIRCLE)
            .with_status(status),
        AvatarFallbackIcon(),
    )
}

#[must_use]
pub fn SquareAvatar(src: &str, alt: &str, initials: &str, size: &'static str) -> Markup {
    Avatar(
        &AvatarProps::default()
            .with_src(src)
            .with_alt(alt)
            .with_initials(initials)
            .with_size(size)
            .with_shape(shared::AVATAR_SHAPE_SQUARE),
        AvatarFallbackIcon(),
    )
}

#[must_use]
pub fn RoundedAvatar(src: &str, alt: &str, initials: &str, size: &'static str) -> Markup {
    Avatar(
        &AvatarProps::default()
            .with_src(src)
            .with_alt(alt)
            .with_initials(initials)
            .with_size(size)
            .with_shape(shared::AVATAR_SHAPE_ROUNDED),
        AvatarFallbackIcon(),
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
        assert_eq!(COMPONENTS.len(), 7);
        assert!(
            COMPONENTS
                .iter()
                .all(|component| component.family == "neomorphism" && component.module == "avatar")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "Avatar")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "AvatarWithIcon")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "AvatarGroup")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "AvatarFallbackIcon")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "CircleAvatar")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "SquareAvatar")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "RoundedAvatar")
        );
    }

    #[test]
    fn wrapped_components_render_unstyled_html_with_neomorphism_classes() {
        let html = Avatar(&AvatarProps::default(), html! { "child" }).into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.starts_with("<"), "expected rendered element: {html}");
        assert!(
            html.contains("neomorph-flat-sm"),
            "missing neomorph-flat-sm: {html}"
        );
        let html = AvatarWithIcon(&AvatarProps::default(), html! { "child" }).into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.starts_with("<"), "expected rendered element: {html}");
        assert!(
            html.contains("neomorph-flat-sm"),
            "missing neomorph-flat-sm: {html}"
        );
    }

    #[test]
    fn neomorphism_only_components_render_html_without_generated_stub_markers() {
        let html = CircleAvatar("sample", "sample", "sample", "sample", "sample").into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.contains("<span class=\"relative inline-flex"));
        assert!(html.contains("neomorph-flat-sm"));
        assert!(html.contains("src=\"sample\""));
        let html = SquareAvatar("sample", "sample", "sample", "sample").into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.contains("rounded-none"));
        assert!(html.contains("neomorph-flat-sm"));
        let html = RoundedAvatar("sample", "sample", "sample", "sample").into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.contains("rounded-md"));
        assert!(html.contains("neomorph-flat-sm"));
    }
}
