#![allow(non_snake_case)]

use crate::components::{ComponentInfo, shared};
use maud::{Markup, html};
use std::collections::BTreeMap;

pub const COMPONENTS: &[ComponentInfo] = &[
    ComponentInfo {
        family: "unstyled",
        module: "badge",
        name: "Badge",
    },
    ComponentInfo {
        family: "unstyled",
        module: "badge",
        name: "BadgeWithIcon",
    },
    ComponentInfo {
        family: "unstyled",
        module: "badge",
        name: "BadgeGroup",
    },
    ComponentInfo {
        family: "unstyled",
        module: "badge",
        name: "DotBadge",
    },
];

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BadgeProps {
    pub variant: &'static str,
    pub size: &'static str,
    pub pill: bool,
    pub removable: bool,
    pub on_remove: String,
    pub class: String,
    pub attributes: BTreeMap<String, String>,
    pub aria_label: String,
}

impl Default for BadgeProps {
    fn default() -> Self {
        Self {
            variant: shared::VARIANT_DEFAULT,
            size: shared::SIZE_MD,
            pill: false,
            removable: false,
            on_remove: String::new(),
            class: String::new(),
            attributes: BTreeMap::new(),
            aria_label: String::new(),
        }
    }
}

impl BadgeProps {
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
    pub const fn with_pill(mut self, pill: bool) -> Self {
        self.pill = pill;
        self
    }

    #[must_use]
    pub const fn with_removable(mut self, removable: bool) -> Self {
        self.removable = removable;
        self
    }

    #[must_use]
    pub fn with_on_remove(mut self, on_remove: impl Into<String>) -> Self {
        self.on_remove = on_remove.into();
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
    pub fn with_aria_label(mut self, aria_label: impl Into<String>) -> Self {
        self.aria_label = aria_label.into();
        self
    }

    #[must_use]
    pub fn build_classes(&self) -> String {
        shared::build_classes(&[
            "inline-flex items-center justify-center gap-1 font-medium transition-all duration-150 border hover:brightness-95",
            shared::badge_variant_classes(self.variant),
            shared::badge_size_classes(self.size),
            if self.pill {
                "rounded-full"
            } else {
                "rounded-md"
            },
            self.class.as_str(),
        ])
    }

    #[must_use]
    pub fn build_attributes(&self) -> BTreeMap<String, String> {
        let mut attrs = self.attributes.clone();
        if !self.aria_label.is_empty() {
            attrs.insert("aria-label".to_string(), self.aria_label.clone());
        }
        attrs
    }

    #[must_use]
    pub fn get_remove_attributes(&self) -> BTreeMap<String, String> {
        let mut attrs = BTreeMap::new();
        if !self.on_remove.is_empty() {
            attrs.insert("onclick".to_string(), self.on_remove.clone());
        }
        attrs
    }
}

#[must_use]
pub fn Badge(props: &BadgeProps, children: Markup) -> Markup {
    let classes = props.build_classes();
    let attrs = props.build_attributes();
    let remove_attrs = props.get_remove_attributes();
    html! {
        span
            class=(classes)
            aria-label=[shared::attr_value(&attrs, "aria-label")]
            title=[shared::attr_value(&attrs, "title")]
            id=[shared::attr_value(&attrs, "id")]
            data-on:click=[shared::attr_value(&attrs, "data-on:click")]
            data-on:change=[shared::attr_value(&attrs, "data-on:change")]
            data-show=[shared::attr_value(&attrs, "data-show")]
            data-text=[shared::attr_value(&attrs, "data-text")]
            data-bind=[shared::attr_value(&attrs, "data-bind")]
            data-class:active=[shared::attr_value(&attrs, "data-class:active")]
        {
            (children)
            @if props.removable {
                button
                    type="button"
                    class="ml-1 inline-flex items-center justify-center hover:opacity-70 hover:scale-[1.2] transition-all duration-150 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-offset-1 rounded-full"
                    aria-label="Remove"
                    onclick=[shared::attr_value(&remove_attrs, "onclick")]
                    data-on:click=[shared::attr_value(&remove_attrs, "data-on:click")]
                {
                    svg class="h-3 w-3" fill="none" stroke="currentColor" viewBox="0 0 24 24" xmlns="http://www.w3.org/2000/svg" {
                        path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M6 18L18 6M6 6l12 12" {}
                    }
                }
            }
        }
    }
}

#[must_use]
pub fn BadgeWithIcon(props: &BadgeProps, children: Markup) -> Markup {
    let classes = props.build_classes();
    let attrs = props.build_attributes();
    html! {
        span
            class=(classes)
            aria-label=[shared::attr_value(&attrs, "aria-label")]
            title=[shared::attr_value(&attrs, "title")]
            id=[shared::attr_value(&attrs, "id")]
            data-on:click=[shared::attr_value(&attrs, "data-on:click")]
            data-show=[shared::attr_value(&attrs, "data-show")]
            data-text=[shared::attr_value(&attrs, "data-text")]
            data-bind=[shared::attr_value(&attrs, "data-bind")]
        {
            span class="badge-icon" { (children) }
        }
    }
}

#[must_use]
pub fn BadgeGroup(class: impl Into<String>, children: Markup) -> Markup {
    let class = class.into();
    html! {
        div class=(shared::build_classes(&["flex flex-wrap gap-2", class.as_str()])) {
            (children)
        }
    }
}

#[must_use]
pub fn DotBadge(variant: &'static str, class: impl Into<String>) -> Markup {
    let class = class.into();
    html! {
        span
            class=(shared::build_classes(&[
                "inline-block h-2 w-2 rounded-full",
                shared::badge_variant_classes(variant),
                class.as_str(),
            ]))
            role="status" {}
    }
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
        shared::VARIANT_OUTLINE,
    ];
    const SIZES: &[&str] = &[shared::SIZE_SM, shared::SIZE_MD, shared::SIZE_LG];

    #[test]
    fn default_props_match_templ_defaults() {
        let props = BadgeProps::default();
        assert_eq!(props.variant, shared::VARIANT_DEFAULT);
        assert_eq!(props.size, shared::SIZE_MD);
        assert!(!props.pill);
        assert!(!props.removable);
    }

    #[test]
    fn build_classes_covers_variants_sizes_and_pill_state() {
        for &variant in VARIANTS {
            for &size in SIZES {
                for pill in [false, true] {
                    let classes = BadgeProps::default()
                        .with_variant(variant)
                        .with_size(size)
                        .with_pill(pill)
                        .with_class("custom")
                        .build_classes();

                    assert!(classes.contains(shared::badge_variant_classes(variant)));
                    assert!(classes.contains(shared::badge_size_classes(size)));
                    assert!(classes.contains("custom"));
                    assert!(classes.contains(if pill { "rounded-full" } else { "rounded-md" }));
                }
            }
        }
    }

    #[test]
    fn build_attributes_and_remove_attributes_cover_custom_aria_and_onclick() {
        let props = BadgeProps::default()
            .with_attribute("data-on:click", "$active = true")
            .with_aria_label("Status")
            .with_on_remove("$remove()");

        let attrs = props.build_attributes();
        let remove_attrs = props.get_remove_attributes();

        assert_eq!(
            attrs.get("data-on:click"),
            Some(&"$active = true".to_string())
        );
        assert_eq!(attrs.get("aria-label"), Some(&"Status".to_string()));
        assert_eq!(remove_attrs.get("onclick"), Some(&"$remove()".to_string()));
    }

    #[test]
    fn renders_badge_html_for_removable_and_non_removable_variations() {
        for removable in [false, true] {
            let html = Badge(
                &BadgeProps::default()
                    .with_removable(removable)
                    .with_on_remove("$remove()")
                    .with_attribute("data-on:click", "$active = true"),
                html! { "New" },
            )
            .into_string();

            assert!(html.starts_with("<span "));
            assert!(html.contains(r#"data-on:click="$active = true""#));
            assert!(html.contains(">New"));
            assert_eq!(html.contains("<button "), removable);
            if removable {
                assert!(html.contains(r#"aria-label="Remove""#));
                assert!(html.contains(r#"onclick="$remove()""#));
                assert!(html.contains("M6 18L18 6M6 6l12 12"));
            }
        }
    }

    #[test]
    fn renders_badge_with_icon_group_and_dot_badge_html() {
        let icon = BadgeWithIcon(&BadgeProps::default(), html! { "Icon" }).into_string();
        let group = BadgeGroup("custom", html! { "Items" }).into_string();

        assert!(icon.contains(r#"<span class="badge-icon">Icon</span>"#));
        assert_eq!(
            group,
            r#"<div class="flex flex-wrap gap-2 custom">Items</div>"#
        );

        for &variant in VARIANTS {
            let dot = DotBadge(variant, "extra").into_string();
            assert!(dot.contains(r#"role="status""#));
            assert!(dot.contains("inline-block h-2 w-2 rounded-full"));
            assert!(dot.contains(shared::badge_variant_classes(variant)));
            assert!(dot.contains("extra"));
        }
    }

    #[test]
    fn maud_escapes_rendered_attribute_values_and_children() {
        let html = Badge(
            &BadgeProps::default()
                .with_attribute("data-on:click", "$x = \"<>&\"")
                .with_attribute("title", "\"Badge\" & more"),
            html! { "<Badge>" },
        )
        .into_string();

        assert!(html.contains(r#"data-on:click="$x = &quot;&lt;&gt;&amp;&quot;""#));
        assert!(html.contains(r#"title="&quot;Badge&quot; &amp; more""#));
        assert!(html.contains("&lt;Badge&gt;"));
    }
}
