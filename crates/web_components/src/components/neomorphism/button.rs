#![allow(non_snake_case)]

use crate::components::unstyled::button as base;
use crate::components::{ComponentInfo, shared};
use maud::Markup;

pub use base::ButtonProps;

pub const COMPONENTS: &[ComponentInfo] = &[
    ComponentInfo {
        family: "neomorphism",
        module: "button",
        name: "Button",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "button",
        name: "ButtonWithIcon",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "button",
        name: "IconButton",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "button",
        name: "PrimaryButton",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "button",
        name: "SecondaryButton",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "button",
        name: "DestructiveButton",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "button",
        name: "GhostButton",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "button",
        name: "OutlineButton",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "button",
        name: "LinkButton",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "button",
        name: "AnchorButton",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "button",
        name: "PrimaryAnchorButton",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "button",
        name: "SecondaryAnchorButton",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "button",
        name: "SubmitButton",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "button",
        name: "LoadingButton",
    },
];

#[must_use]
fn neomorph_class_for_variant(variant: &str) -> &'static str {
    match variant {
        shared::VARIANT_PRIMARY => "neomorph-button neomorph-hover-lift neomorph-active-press",
        shared::VARIANT_SECONDARY => "neomorph-flat neomorph-hover-lift",
        shared::VARIANT_DESTRUCTIVE => "neomorph-button neomorph-hover-lift",
        shared::VARIANT_GHOST | shared::VARIANT_LINK => "",
        shared::VARIANT_OUTLINE => "neomorph-flat-sm",
        _ => "neomorph-button",
    }
}

#[must_use]
fn addNeomorphClasses(props: &ButtonProps) -> ButtonProps {
    let mut props = props.clone();
    props.class = shared::build_classes(&[
        neomorph_class_for_variant(props.variant),
        props.class.as_str(),
    ]);
    props
}

#[must_use]
pub fn Button(props: &ButtonProps, children: Markup) -> Markup {
    base::Button(&addNeomorphClasses(props), children)
}

#[must_use]
pub fn ButtonWithIcon(props: &ButtonProps, icon_position: &str, children: Markup) -> Markup {
    base::ButtonWithIcon(&addNeomorphClasses(props), icon_position, children)
}

#[must_use]
pub fn IconButton(props: &ButtonProps, children: Markup) -> Markup {
    base::IconButton(&addNeomorphClasses(props), children)
}

#[must_use]
pub fn PrimaryButton(
    size: &'static str,
    disabled: bool,
    class: impl Into<String>,
    children: Markup,
) -> Markup {
    Button(
        &ButtonProps::default()
            .with_variant(shared::VARIANT_PRIMARY)
            .with_size(size)
            .with_disabled(disabled)
            .with_type("button")
            .with_class(class),
        children,
    )
}

#[must_use]
pub fn SecondaryButton(
    size: &'static str,
    disabled: bool,
    class: impl Into<String>,
    children: Markup,
) -> Markup {
    Button(
        &ButtonProps::default()
            .with_variant(shared::VARIANT_SECONDARY)
            .with_size(size)
            .with_disabled(disabled)
            .with_type("button")
            .with_class(class),
        children,
    )
}

#[must_use]
pub fn DestructiveButton(
    size: &'static str,
    disabled: bool,
    class: impl Into<String>,
    children: Markup,
) -> Markup {
    Button(
        &ButtonProps::default()
            .with_variant(shared::VARIANT_DESTRUCTIVE)
            .with_size(size)
            .with_disabled(disabled)
            .with_type("button")
            .with_class(class),
        children,
    )
}

#[must_use]
pub fn GhostButton(
    size: &'static str,
    disabled: bool,
    class: impl Into<String>,
    children: Markup,
) -> Markup {
    Button(
        &ButtonProps::default()
            .with_variant(shared::VARIANT_GHOST)
            .with_size(size)
            .with_disabled(disabled)
            .with_type("button")
            .with_class(class),
        children,
    )
}

#[must_use]
pub fn OutlineButton(
    size: &'static str,
    disabled: bool,
    class: impl Into<String>,
    children: Markup,
) -> Markup {
    Button(
        &ButtonProps::default()
            .with_variant(shared::VARIANT_OUTLINE)
            .with_size(size)
            .with_disabled(disabled)
            .with_type("button")
            .with_class(class),
        children,
    )
}

#[must_use]
pub fn LinkButton(
    size: &'static str,
    disabled: bool,
    class: impl Into<String>,
    children: Markup,
) -> Markup {
    Button(
        &ButtonProps::default()
            .with_variant(shared::VARIANT_LINK)
            .with_size(size)
            .with_disabled(disabled)
            .with_type("button")
            .with_class(class),
        children,
    )
}

#[must_use]
pub fn AnchorButton(
    href: impl Into<String>,
    variant: &'static str,
    size: &'static str,
    class: impl Into<String>,
    children: Markup,
) -> Markup {
    Button(
        &ButtonProps::default()
            .with_as(shared::ELEMENT_A)
            .with_href(href)
            .with_variant(variant)
            .with_size(size)
            .with_class(class),
        children,
    )
}

#[must_use]
pub fn PrimaryAnchorButton(
    href: impl Into<String>,
    size: &'static str,
    class: impl Into<String>,
    children: Markup,
) -> Markup {
    AnchorButton(href, shared::VARIANT_PRIMARY, size, class, children)
}

#[must_use]
pub fn SecondaryAnchorButton(
    href: impl Into<String>,
    size: &'static str,
    class: impl Into<String>,
    children: Markup,
) -> Markup {
    AnchorButton(href, shared::VARIANT_SECONDARY, size, class, children)
}

#[must_use]
pub fn SubmitButton(
    size: &'static str,
    disabled: bool,
    class: impl Into<String>,
    children: Markup,
) -> Markup {
    Button(
        &ButtonProps::default()
            .with_variant(shared::VARIANT_PRIMARY)
            .with_size(size)
            .with_type("submit")
            .with_disabled(disabled)
            .with_class(class),
        children,
    )
}

#[must_use]
pub fn LoadingButton(
    size: &'static str,
    is_loading: bool,
    class: impl Into<String>,
    children: Markup,
) -> Markup {
    Button(
        &ButtonProps::default()
            .with_variant(shared::VARIANT_PRIMARY)
            .with_size(size)
            .with_loading(is_loading)
            .with_class(class),
        children,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use maud::html;

    fn assert_button_html(html: &str, label: &str) {
        assert!(html.contains("<button"), "expected button element: {html}");
        assert!(html.contains(label), "missing label {label}: {html}");
        assert!(
            !html.contains("data-family=") && !html.contains("data-component="),
            "placeholder marker attributes leaked: {html}"
        );
    }

    #[test]
    fn add_neomorph_classes_matches_variants_and_preserves_user_class() {
        let cases = [
            (
                shared::VARIANT_PRIMARY,
                "neomorph-button neomorph-hover-lift neomorph-active-press custom",
            ),
            (
                shared::VARIANT_SECONDARY,
                "neomorph-flat neomorph-hover-lift custom",
            ),
            (
                shared::VARIANT_DESTRUCTIVE,
                "neomorph-button neomorph-hover-lift custom",
            ),
            (shared::VARIANT_GHOST, "custom"),
            (shared::VARIANT_OUTLINE, "neomorph-flat-sm custom"),
            (shared::VARIANT_LINK, "custom"),
            (shared::VARIANT_DEFAULT, "neomorph-button custom"),
        ];

        for (variant, expected) in cases {
            let props = addNeomorphClasses(
                &ButtonProps::default()
                    .with_variant(variant)
                    .with_class("custom"),
            );
            assert_eq!(props.class, expected);
        }
    }

    #[test]
    fn button_variations_render_real_unstyled_button_html() {
        let html = Button(
            &ButtonProps::default()
                .with_variant(shared::VARIANT_PRIMARY)
                .with_attribute("data-on:click", "$saved = true"),
            html! { "Save" },
        )
        .into_string();
        assert_button_html(&html, "Save");
        assert!(html.contains("neomorph-button"));
        assert!(html.contains("data-on:click=\"$saved = true\""));

        let html = ButtonWithIcon(
            &ButtonProps::default().with_variant(shared::VARIANT_SECONDARY),
            "left",
            html! { "Icon" },
        )
        .into_string();
        assert_button_html(&html, "Icon");
        assert!(html.contains("button-icon-left"));
        assert!(html.contains("neomorph-flat"));

        let html = IconButton(
            &ButtonProps::default().with_variant(shared::VARIANT_OUTLINE),
            html! { "I" },
        )
        .into_string();
        assert_button_html(&html, "I");
        assert!(html.contains("aspect-square"));
        assert!(html.contains("neomorph-flat-sm"));
    }

    #[test]
    fn convenience_buttons_cover_variants_sizes_disabled_submit_anchor_and_loading() {
        let primary =
            PrimaryButton(shared::SIZE_SM, true, "extra", html! { "Primary" }).into_string();
        assert_button_html(&primary, "Primary");
        assert!(primary.contains("disabled"));
        assert!(primary.contains("h-9"));
        assert!(primary.contains("extra"));

        let secondary =
            SecondaryButton(shared::SIZE_MD, false, "", html! { "Secondary" }).into_string();
        assert_button_html(&secondary, "Secondary");
        assert!(secondary.contains("neomorph-flat"));

        let destructive =
            DestructiveButton(shared::SIZE_LG, false, "", html! { "Delete" }).into_string();
        assert_button_html(&destructive, "Delete");
        assert!(destructive.contains("neomorph-button"));

        let ghost =
            GhostButton(shared::SIZE_MD, false, "ghost-extra", html! { "Ghost" }).into_string();
        assert_button_html(&ghost, "Ghost");
        assert!(!ghost.contains("neomorph-button"));
        assert!(ghost.contains("ghost-extra"));

        let outline = OutlineButton(shared::SIZE_MD, false, "", html! { "Outline" }).into_string();
        assert_button_html(&outline, "Outline");
        assert!(outline.contains("neomorph-flat-sm"));

        let link = LinkButton(shared::SIZE_MD, false, "link-extra", html! { "Link" }).into_string();
        assert_button_html(&link, "Link");
        assert!(!link.contains("neomorph-button"));
        assert!(link.contains("link-extra"));

        let submit = SubmitButton(shared::SIZE_MD, false, "", html! { "Send" }).into_string();
        assert_button_html(&submit, "Send");
        assert!(submit.contains("type=\"submit\""));

        let loading = LoadingButton(shared::SIZE_MD, true, "", html! { "Working" }).into_string();
        assert!(loading.contains("aria-busy=\"true\""));
        assert!(loading.contains("Loading..."));
        assert!(loading.contains("Working"));

        let anchor = AnchorButton(
            "/docs",
            shared::VARIANT_PRIMARY,
            shared::SIZE_MD,
            "",
            html! { "Docs" },
        )
        .into_string();
        assert!(anchor.contains("<a href=\"/docs\""));
        assert!(anchor.contains("Docs"));

        let primary_anchor =
            PrimaryAnchorButton("/primary", shared::SIZE_MD, "", html! { "Primary Link" })
                .into_string();
        assert!(primary_anchor.contains("<a href=\"/primary\""));
        assert!(primary_anchor.contains("neomorph-button"));

        let secondary_anchor = SecondaryAnchorButton(
            "/secondary",
            shared::SIZE_MD,
            "",
            html! { "Secondary Link" },
        )
        .into_string();
        assert!(secondary_anchor.contains("<a href=\"/secondary\""));
        assert!(secondary_anchor.contains("neomorph-flat"));
    }
}
