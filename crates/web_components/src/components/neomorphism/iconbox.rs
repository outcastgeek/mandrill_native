#![allow(non_snake_case)]
#![allow(clippy::needless_pass_by_value)]

use crate::components::ComponentInfo;
use crate::components::shared;
use crate::components::unstyled::iconbox as base;
use maud::{Markup, html};

pub use base::IconBoxProps;

pub const COMPONENTS: &[ComponentInfo] = &[
    ComponentInfo {
        family: "neomorphism",
        module: "iconbox",
        name: "IconBox",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "iconbox",
        name: "FloatingIconBox",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "iconbox",
        name: "GradientIconBox",
    },
];

#[must_use]
pub fn DefaultIconBoxProps() -> IconBoxProps {
    IconBoxProps {
        background: "gradient",
        ..IconBoxProps::default()
    }
}

#[must_use]
pub fn IconBox(props: &IconBoxProps, children: Markup) -> Markup {
    let attrs = props.build_attributes();
    let style = props.build_styles();
    html! {
        div
            class=(props.build_classes())
            style=[shared::optional_attr(&style)]
            id=[shared::attr_value(&attrs, "id")]
            title=[shared::attr_value(&attrs, "title")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
        {
            @if props.icon_type == "emoji" && !props.image_url.is_empty() {
                span class="text-2xl" { (&props.image_url) }
            } @else if props.icon_type == "image" && !props.image_url.is_empty() {
                img src=(&props.image_url) alt="icon" class="w-8 h-8" {}
            } @else if let Some(icon) = &props.icon {
                span class="text-primary-foreground" {
                    (icon)
                }
            } @else {
                (children)
            }
        }
    }
}

#[must_use]
pub fn FloatingIconBox(icon: Markup, gradientFrom: &str, to: &str, delay: &str) -> Markup {
    IconBox(
        &DefaultIconBoxProps()
            .with_icon(icon)
            .with_size(shared::SIZE_MD)
            .with_shape("rounded")
            .with_gradient(gradientFrom, to)
            .with_animation("float", delay),
        html! {},
    )
}

#[must_use]
pub fn GradientIconBox(icon: Markup, from: &str, to: &str) -> Markup {
    IconBox(
        &DefaultIconBoxProps()
            .with_icon(icon)
            .with_size(shared::SIZE_MD)
            .with_shape("rounded")
            .with_gradient(from, to),
        html! {},
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
        assert_eq!(COMPONENTS.len(), 3);
        assert!(COMPONENTS.iter().all(|component| component.family == "neomorphism" && component.module == "iconbox"));
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "IconBox")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "FloatingIconBox")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "GradientIconBox")
        );
    }

    #[test]
    fn neomorphism_only_components_render_html_without_generated_stub_markers() {
        let default_props = DefaultIconBoxProps();
        assert_eq!(default_props.background, "gradient");
        assert_eq!(default_props.icon_type, "svg");

        let html = IconBox(
            &DefaultIconBoxProps()
                .with_emoji(":)")
                .with_class("emoji-extra")
                .with_attribute("id", "emoji-box"),
            html! {},
        )
        .into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.contains("id=\"emoji-box\""));
        assert!(html.contains("emoji-extra"));
        assert!(html.contains("<span class=\"text-2xl\">:)</span>"));

        let html = IconBox(
            &DefaultIconBoxProps().with_image("/icon.png"),
            html! { "fallback" },
        )
        .into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.contains("<img src=\"/icon.png\" alt=\"icon\" class=\"w-8 h-8\">"));
        assert!(!html.contains("fallback"));

        let html = IconBox(&DefaultIconBoxProps(), html! { "fallback" }).into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.contains(">fallback</div>"));

        let html =
            FloatingIconBox(html! { "child" }, "sky-500", "indigo-600", "250ms").into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.contains("w-16 h-16"));
        assert!(html.contains("rounded-2xl"));
        assert!(html.contains("bg-gradient-to-br from-sky-500 to-indigo-600"));
        assert!(html.contains("animate-float"));
        assert!(html.contains("style=\"animation-delay: 250ms;\""));
        assert!(html.contains("<span class=\"text-primary-foreground\">child</span>"));

        let html = GradientIconBox(html! { "gradient" }, "emerald-500", "cyan-600").into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.contains("bg-gradient-to-br from-emerald-500 to-cyan-600"));
        assert!(html.contains("<span class=\"text-primary-foreground\">gradient</span>"));
        assert!(!html.contains("animation-delay"));
    }
}
