#![allow(non_snake_case)]

use crate::components::{ComponentInfo, shared};
use maud::{Markup, html};
use std::collections::BTreeMap;

pub const COMPONENTS: &[ComponentInfo] = &[
    ComponentInfo { family: "unstyled", module: "button", name: "ButtonWithLeadingIcon" },
    ComponentInfo { family: "unstyled", module: "button", name: "ButtonWithTrailingIcon" },
    ComponentInfo {
        family: "unstyled",
        module: "button",
        name: "loadingSpinner",
    },
    ComponentInfo {
        family: "unstyled",
        module: "button",
        name: "Button",
    },
    ComponentInfo {
        family: "unstyled",
        module: "button",
        name: "ButtonWithIcon",
    },
    ComponentInfo {
        family: "unstyled",
        module: "button",
        name: "iconButtonContent",
    },
    ComponentInfo {
        family: "unstyled",
        module: "button",
        name: "IconButton",
    },
    ComponentInfo {
        family: "unstyled",
        module: "button",
        name: "PrimaryButton",
    },
    ComponentInfo {
        family: "unstyled",
        module: "button",
        name: "SecondaryButton",
    },
    ComponentInfo {
        family: "unstyled",
        module: "button",
        name: "DestructiveButton",
    },
    ComponentInfo {
        family: "unstyled",
        module: "button",
        name: "GhostButton",
    },
    ComponentInfo {
        family: "unstyled",
        module: "button",
        name: "OutlineButton",
    },
    ComponentInfo {
        family: "unstyled",
        module: "button",
        name: "LinkButton",
    },
    ComponentInfo {
        family: "unstyled",
        module: "button",
        name: "SimpleButton",
    },
    ComponentInfo {
        family: "unstyled",
        module: "button",
        name: "LoadingButton",
    },
    ComponentInfo {
        family: "unstyled",
        module: "button",
        name: "SubmitButton",
    },
    ComponentInfo {
        family: "unstyled",
        module: "button",
        name: "FullWidthButton",
    },
    ComponentInfo {
        family: "unstyled",
        module: "button",
        name: "AnchorButton",
    },
    ComponentInfo {
        family: "unstyled",
        module: "button",
        name: "PrimaryAnchorButton",
    },
    ComponentInfo {
        family: "unstyled",
        module: "button",
        name: "SecondaryAnchorButton",
    },
];

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ButtonProps {
    pub as_element: &'static str,
    pub variant: &'static str,
    pub size: &'static str,
    pub button_type: &'static str,
    pub href: String,
    pub disabled: bool,
    pub full_width: bool,
    pub loading: bool,
    pub class: String,
    pub attributes: BTreeMap<String, String>,
    pub aria_label: String,
    pub aria_described_by: String,
    pub aria_pressed: String,
}

impl Default for ButtonProps {
    fn default() -> Self {
        Self {
            as_element: shared::ELEMENT_BUTTON,
            variant: shared::VARIANT_DEFAULT,
            size: shared::SIZE_MD,
            button_type: "button",
            href: String::new(),
            disabled: false,
            full_width: false,
            loading: false,
            class: String::new(),
            attributes: BTreeMap::new(),
            aria_label: String::new(),
            aria_described_by: String::new(),
            aria_pressed: String::new(),
        }
    }
}

impl ButtonProps {
    #[must_use]
    pub fn with_as(mut self, as_element: &'static str) -> Self {
        self.as_element = as_element;
        self
    }

    #[must_use]
    pub fn with_href(mut self, href: impl Into<String>) -> Self {
        self.href = href.into();
        self
    }

    #[must_use]
    pub fn with_variant(mut self, variant: &'static str) -> Self {
        self.variant = variant;
        self
    }

    #[must_use]
    pub fn with_size(mut self, size: &'static str) -> Self {
        self.size = size;
        self
    }

    #[must_use]
    pub fn with_type(mut self, button_type: &'static str) -> Self {
        self.button_type = button_type;
        self
    }

    #[must_use]
    pub const fn with_disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    #[must_use]
    pub const fn with_full_width(mut self, full_width: bool) -> Self {
        self.full_width = full_width;
        self
    }

    #[must_use]
    pub const fn with_loading(mut self, loading: bool) -> Self {
        self.loading = loading;
        self
    }

    #[must_use]
    pub fn with_class(mut self, class: impl Into<String>) -> Self {
        self.class = class.into();
        self
    }

    #[must_use]
    pub fn with_attribute(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.attributes.insert(name.into(), value.into());
        self
    }

    #[must_use]
    pub fn with_aria_label(mut self, label: impl Into<String>) -> Self {
        self.aria_label = label.into();
        self
    }

    #[must_use]
    pub fn with_aria_described_by(mut self, described_by: impl Into<String>) -> Self {
        self.aria_described_by = described_by.into();
        self
    }

    #[must_use]
    pub fn with_aria_pressed(mut self, pressed: impl Into<String>) -> Self {
        self.aria_pressed = pressed.into();
        self
    }

    #[must_use]
    pub fn build_classes(&self) -> String {
        let width_class = if self.full_width { "w-full" } else { "" };
        shared::build_classes(&[
            "inline-flex items-center justify-center gap-2 whitespace-nowrap font-medium transition-all duration-150 ease-out cursor-pointer select-none focus-visible:outline-2 focus-visible:outline-ring focus-visible:outline-offset-2 disabled:pointer-events-none disabled:opacity-50 active:scale-[0.98]",
            shared::button_variant_classes(self.variant),
            shared::button_size_classes(self.size),
            width_class,
            self.class.as_str(),
        ])
    }

    #[must_use]
    pub fn build_attributes(&self) -> BTreeMap<String, String> {
        let mut attrs = self.attributes.clone();
        if !self.aria_label.is_empty() {
            attrs.insert("aria-label".to_string(), self.aria_label.clone());
        }
        if !self.aria_described_by.is_empty() {
            attrs.insert(
                "aria-describedby".to_string(),
                self.aria_described_by.clone(),
            );
        }
        if !self.aria_pressed.is_empty() {
            attrs.insert("aria-pressed".to_string(), self.aria_pressed.clone());
        }
        if self.loading {
            attrs.insert("aria-busy".to_string(), "true".to_string());
        }
        attrs
    }
}

#[must_use]
pub fn loadingSpinner() -> Markup {
    html! {
        span class="animate-spin inline-block w-4 h-4 border-2 border-current border-t-transparent rounded-full" role="status" aria-label="Loading" {
            span class="sr-only" { "Loading..." }
        }
    }
}

fn render_children(props: &ButtonProps, children: Markup) -> Markup {
    html! {
        @if props.loading {
            (loadingSpinner())
        }
        (children)
    }
}

#[must_use]
pub fn Button(props: &ButtonProps, children: Markup) -> Markup {
    let attrs = props.build_attributes();
    let classes = props.build_classes();
    let disabled_or_loading = props.disabled || props.loading;
    let aria_disabled = disabled_or_loading.then_some("true");

    match props.as_element {
        shared::ELEMENT_A => crate::html! {
            a
                href=(props.href)
                class=(classes)
                aria-disabled=[aria_disabled]
                ..(&attrs)
            {
                (render_children(props, children))
            }
        },
        shared::ELEMENT_SPAN => crate::html! {
            span
                class=(classes)
                aria-disabled=[aria_disabled]
                ..(&attrs)
            {
                (render_children(props, children))
            }
        },
        shared::ELEMENT_DIV => crate::html! {
            div
                class=(classes)
                role="button"
                tabindex="0"
                aria-disabled=[aria_disabled]
                ..(&attrs)
            {
                (render_children(props, children))
            }
        },
        _ => crate::html! {
            button
                type=(props.button_type)
                class=(classes)
                disabled[disabled_or_loading]
                ..(&attrs)
            {
                (render_children(props, children))
            }
        },
    }
}

#[must_use]
pub fn ButtonWithIcon(props: &ButtonProps, icon_position: &str, children: Markup) -> Markup {
    let classes = props.build_classes();
    let attrs = props.build_attributes();
    let disabled_or_loading = props.disabled || props.loading;
    html! {
        button
            type=(props.button_type)
            class=(classes)
            disabled[disabled_or_loading]
            aria-label=[shared::attr_value(&attrs, "aria-label")]
            aria-describedby=[shared::attr_value(&attrs, "aria-describedby")]
            aria-pressed=[shared::attr_value(&attrs, "aria-pressed")]
            aria-busy=[shared::attr_value(&attrs, "aria-busy")]
            title=[shared::attr_value(&attrs, "title")]
            id=[shared::attr_value(&attrs, "id")]
            data-on:click=[shared::attr_value(&attrs, "data-on:click")]
            data-on:change=[shared::attr_value(&attrs, "data-on:change")]
            data-on:submit=[shared::attr_value(&attrs, "data-on:submit")]
            data-on:input=[shared::attr_value(&attrs, "data-on:input")]
            data-on:load=[shared::attr_value(&attrs, "data-on:load")]
            data-on:click__outside=[shared::attr_value(&attrs, "data-on:click__outside")]
            data-bind=[shared::attr_value(&attrs, "data-bind")]
            data-show=[shared::attr_value(&attrs, "data-show")]
            data-text=[shared::attr_value(&attrs, "data-text")]
            data-signals=[shared::attr_value(&attrs, "data-signals")]
            data-init=[shared::attr_value(&attrs, "data-init")]
        {
            @if props.loading {
                (loadingSpinner())
            } @else if icon_position == "left" {
                span class="button-icon-left" { (children.clone()) }
            }
            span class="button-content" { (children.clone()) }
            @if !props.loading && icon_position == "right" {
                span class="button-icon-right" { (children) }
            }
        }
    }
}

#[must_use]
pub fn iconButtonContent(loading: bool, children: Markup) -> Markup {
    html! {
        @if loading {
            (loadingSpinner())
        } @else {
            (children)
        }
    }
}

#[must_use]
pub fn IconButton(props: &ButtonProps, children: Markup) -> Markup {
    let mut icon_props = props.clone();
    icon_props.class = shared::build_classes(&[props.class.as_str(), "aspect-square p-0"]);
    Button(&icon_props, iconButtonContent(props.loading, children))
}

#[must_use]
pub fn PrimaryButton(class: impl Into<String>, children: Markup) -> Markup {
    Button(
        &ButtonProps::default()
            .with_variant(shared::VARIANT_PRIMARY)
            .with_class(class),
        children,
    )
}

#[must_use]
pub fn SecondaryButton(class: impl Into<String>, children: Markup) -> Markup {
    Button(
        &ButtonProps::default()
            .with_variant(shared::VARIANT_SECONDARY)
            .with_class(class),
        children,
    )
}

#[must_use]
pub fn DestructiveButton(class: impl Into<String>, children: Markup) -> Markup {
    Button(
        &ButtonProps::default()
            .with_variant(shared::VARIANT_DESTRUCTIVE)
            .with_class(class),
        children,
    )
}

#[must_use]
pub fn GhostButton(class: impl Into<String>, children: Markup) -> Markup {
    Button(
        &ButtonProps::default()
            .with_variant(shared::VARIANT_GHOST)
            .with_class(class),
        children,
    )
}

#[must_use]
pub fn OutlineButton(class: impl Into<String>, children: Markup) -> Markup {
    Button(
        &ButtonProps::default()
            .with_variant(shared::VARIANT_OUTLINE)
            .with_class(class),
        children,
    )
}

#[must_use]
pub fn LinkButton(class: impl Into<String>, children: Markup) -> Markup {
    Button(
        &ButtonProps::default()
            .with_variant(shared::VARIANT_LINK)
            .with_class(class),
        children,
    )
}

#[must_use]
pub fn SimpleButton(label: &str, variant: &'static str, class: impl Into<String>) -> Markup {
    Button(
        &ButtonProps::default()
            .with_variant(variant)
            .with_class(class),
        html! { (label) },
    )
}

#[must_use]
pub fn LoadingButton(label: &str, is_loading: bool, class: impl Into<String>) -> Markup {
    Button(
        &ButtonProps::default()
            .with_loading(is_loading)
            .with_class(class),
        html! { (label) },
    )
}

#[must_use]
pub fn SubmitButton(label: &str, class: impl Into<String>) -> Markup {
    Button(
        &ButtonProps::default()
            .with_type("submit")
            .with_variant(shared::VARIANT_PRIMARY)
            .with_class(class),
        html! { (label) },
    )
}

#[must_use]
pub fn FullWidthButton(label: &str, variant: &'static str, class: impl Into<String>) -> Markup {
    Button(
        &ButtonProps::default()
            .with_variant(variant)
            .with_full_width(true)
            .with_class(class),
        html! { (label) },
    )
}

#[must_use]
pub fn AnchorButton(
    href: impl Into<String>,
    variant: &'static str,
    class: impl Into<String>,
    children: Markup,
) -> Markup {
    Button(
        &ButtonProps::default()
            .with_as(shared::ELEMENT_A)
            .with_href(href)
            .with_variant(variant)
            .with_class(class),
        children,
    )
}

#[must_use]
pub fn PrimaryAnchorButton(
    href: impl Into<String>,
    class: impl Into<String>,
    children: Markup,
) -> Markup {
    Button(
        &ButtonProps::default()
            .with_as(shared::ELEMENT_A)
            .with_href(href)
            .with_variant(shared::VARIANT_PRIMARY)
            .with_class(class),
        children,
    )
}

#[must_use]
pub fn SecondaryAnchorButton(
    href: impl Into<String>,
    class: impl Into<String>,
    children: Markup,
) -> Markup {
    Button(
        &ButtonProps::default()
            .with_as(shared::ELEMENT_A)
            .with_href(href)
            .with_variant(shared::VARIANT_SECONDARY)
            .with_class(class),
        children,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const VARIANTS: &[&str] = &[
        shared::VARIANT_DEFAULT,
        shared::VARIANT_PRIMARY,
        shared::VARIANT_SECONDARY,
        shared::VARIANT_DESTRUCTIVE,
        shared::VARIANT_SUCCESS,
        shared::VARIANT_WARNING,
        shared::VARIANT_INFO,
        shared::VARIANT_GHOST,
        shared::VARIANT_OUTLINE,
        shared::VARIANT_LINK,
    ];
    const SIZES: &[&str] = &[
        shared::SIZE_XS,
        shared::SIZE_SM,
        shared::SIZE_MD,
        shared::SIZE_LG,
        shared::SIZE_XL,
    ];

    #[test]
    fn default_props_match_templ_defaults() {
        let props = ButtonProps::default();
        assert_eq!(props.as_element, shared::ELEMENT_BUTTON);
        assert_eq!(props.variant, shared::VARIANT_DEFAULT);
        assert_eq!(props.size, shared::SIZE_MD);
        assert_eq!(props.button_type, "button");
        assert!(!props.disabled);
        assert!(!props.loading);
    }

    #[test]
    fn build_classes_covers_all_variant_size_width_combinations() {
        for &variant in VARIANTS {
            for &size in SIZES {
                for full_width in [false, true] {
                    let classes = ButtonProps::default()
                        .with_variant(variant)
                        .with_size(size)
                        .with_full_width(full_width)
                        .with_class("custom")
                        .build_classes();

                    assert!(classes.contains(shared::button_variant_classes(variant)));
                    assert!(classes.contains(shared::button_size_classes(size)));
                    assert!(classes.contains("custom"));
                    assert_eq!(classes.contains("w-full"), full_width);
                }
            }
        }
    }

    #[test]
    fn build_attributes_covers_aria_and_loading_variations() {
        let attrs = ButtonProps::default()
            .with_attribute("data-on:click", "$open = true")
            .with_aria_label("Open")
            .with_aria_described_by("hint")
            .with_aria_pressed("mixed")
            .with_loading(true)
            .build_attributes();

        assert_eq!(
            attrs.get("data-on:click"),
            Some(&"$open = true".to_string())
        );
        assert_eq!(attrs.get("aria-label"), Some(&"Open".to_string()));
        assert_eq!(attrs.get("aria-describedby"), Some(&"hint".to_string()));
        assert_eq!(attrs.get("aria-pressed"), Some(&"mixed".to_string()));
        assert_eq!(attrs.get("aria-busy"), Some(&"true".to_string()));
    }

    #[test]
    fn renders_all_polymorphic_elements_and_disabled_loading_states() {
        for as_element in [
            shared::ELEMENT_BUTTON,
            shared::ELEMENT_A,
            shared::ELEMENT_SPAN,
            shared::ELEMENT_DIV,
        ] {
            for disabled in [false, true] {
                for loading in [false, true] {
                    let html = Button(
                        &ButtonProps::default()
                            .with_as(as_element)
                            .with_href("/go")
                            .with_disabled(disabled)
                            .with_loading(loading),
                        html! { "Action" },
                    )
                    .into_string();

                    match as_element {
                        shared::ELEMENT_A => assert!(html.starts_with("<a ")),
                        shared::ELEMENT_SPAN => assert!(html.starts_with("<span ")),
                        shared::ELEMENT_DIV => assert!(html.starts_with("<div ")),
                        _ => assert!(html.starts_with("<button ")),
                    }
                    if disabled || loading {
                        let has_disabled_attr = html.contains(" disabled");
                        let has_aria_disabled_attr = html.contains("aria-disabled");
                        assert_ne!(
                            (has_disabled_attr as u8) + (has_aria_disabled_attr as u8),
                            0
                        );
                    }
                    if loading {
                        assert!(html.contains("Loading..."));
                    }
                }
            }
        }
    }

    #[test]
    fn convenience_buttons_render_expected_variants() {
        assert!(
            PrimaryButton("", html! { "Save" })
                .into_string()
                .contains("btn-primary-gradient")
        );
        assert!(
            SecondaryButton("", html! { "More" })
                .into_string()
                .contains("bg-secondary")
        );
        assert!(
            DestructiveButton("", html! { "Delete" })
                .into_string()
                .contains("bg-destructive")
        );
        assert!(
            GhostButton("", html! { "Ghost" })
                .into_string()
                .contains("hover:bg-muted")
        );
        assert!(
            OutlineButton("", html! { "Outline" })
                .into_string()
                .contains("border-border")
        );
        assert!(
            LinkButton("", html! { "Link" })
                .into_string()
                .contains("btn-link-underline")
        );
        assert!(
            SubmitButton("Submit", "")
                .into_string()
                .contains(r#"type="submit""#)
        );
        assert!(
            FullWidthButton("Full", shared::VARIANT_PRIMARY, "")
                .into_string()
                .contains("w-full")
        );
    }

    #[test]
    fn anchor_buttons_render_links() {
        let primary = PrimaryAnchorButton("/primary", "", html! { "Primary" }).into_string();
        let secondary =
            SecondaryAnchorButton("/secondary", "", html! { "Secondary" }).into_string();
        let custom =
            AnchorButton("/custom", shared::VARIANT_GHOST, "", html! { "Custom" }).into_string();

        assert!(primary.starts_with(r#"<a href="/primary""#));
        assert!(secondary.starts_with(r#"<a href="/secondary""#));
        assert!(custom.starts_with(r#"<a href="/custom""#));
        assert!(custom.contains("hover:bg-muted"));
    }

    #[test]
    fn renders_maud_escaped_attributes_and_datastar_colon_names() {
        let html = Button(
            &ButtonProps::default()
                .with_attribute("title", r#""Quoted" & <safe>"#)
                .with_attribute("data-on:click", "$open = true")
                .with_attribute("data-on:load__interval.1000ms", "@get('/tick')")
                .with_attribute("data-bind", "query")
                .with_aria_label("Open"),
            html! { "<Save>" },
        )
        .into_string();

        assert!(html.contains(r#"title="&quot;Quoted&quot; &amp; &lt;safe&gt;""#));
        assert!(html.contains(r#"data-on:click="$open = true""#));
        assert!(html.contains(r#"data-on:load__interval.1000ms="@get('/tick')""#));
        assert!(html.contains(r#"data-bind="query""#));
        assert!(html.contains(r#"aria-label="Open""#));
        assert!(html.contains("&lt;Save&gt;"));
        assert!(!html.contains("data-on-click"));
    }

    #[test]
    fn icon_button_helpers_and_simple_variants_render_actual_html() {
        let left = ButtonWithIcon(
            &ButtonProps::default()
                .with_attribute("id", "left")
                .with_attribute("data-on:click", "$left = true"),
            "left",
            html! { "Icon" },
        )
        .into_string();
        assert!(left.starts_with(r#"<button type="button" class=""#));
        assert!(left.contains(r#"id="left""#));
        assert!(left.contains(r#"data-on:click="$left = true""#));
        assert!(left.contains(r#"<span class="button-icon-left">Icon</span>"#));
        assert!(left.contains(r#"<span class="button-content">Icon</span>"#));

        let right =
            ButtonWithIcon(&ButtonProps::default(), "right", html! { "<Icon>" }).into_string();
        assert!(right.contains(r#"<span class="button-content">&lt;Icon&gt;</span>"#));
        assert!(right.contains(r#"<span class="button-icon-right">&lt;Icon&gt;</span>"#));

        let loading = ButtonWithIcon(&ButtonProps::default().with_loading(true), "left", html! {})
            .into_string();
        assert!(loading.contains(r#"aria-busy="true""#));
        assert!(loading.contains("Loading..."));
        assert!(!loading.contains("button-icon-left"));

        assert_eq!(
            iconButtonContent(false, html! { "<Glyph>" }).into_string(),
            "&lt;Glyph&gt;"
        );
        assert!(
            iconButtonContent(true, html! {})
                .into_string()
                .contains("Loading...")
        );

        let icon_button = IconButton(
            &ButtonProps::default()
                .with_aria_label("Settings")
                .with_attribute("data-testid", "settings"),
            html! { "S" },
        )
        .into_string();
        assert!(icon_button.contains("aspect-square p-0"));
        assert!(icon_button.contains(r#"aria-label="Settings""#));
        assert!(icon_button.contains(">S</button>"));

        let simple = SimpleButton("<Save>", shared::VARIANT_INFO, "info").into_string();
        assert!(simple.contains("bg-info"));
        assert!(simple.contains("info"));
        assert!(simple.contains("&lt;Save&gt;"));

        let idle = LoadingButton("Load", false, "idle").into_string();
        assert!(idle.contains(">Load</button>"));
        assert!(!idle.contains("Loading..."));
        let busy = LoadingButton("Load", true, "busy").into_string();
        assert!(busy.contains("busy"));
        assert!(busy.contains("Loading..."));
    }
}

/// Compose a decorative leading icon using the canonical button renderer.
pub fn ButtonWithLeadingIcon(props:&ButtonProps,icon:Markup,children:Markup)->Markup{
    Button(props,html!{span class="inline-flex shrink-0" aria-hidden="true" {(icon)}span {(children)}})
}
/// Compose a decorative trailing icon using the canonical button renderer.
pub fn ButtonWithTrailingIcon(props:&ButtonProps,icon:Markup,children:Markup)->Markup{
    Button(props,html!{span {(children)}span class="inline-flex shrink-0" aria-hidden="true" {(icon)}})
}
