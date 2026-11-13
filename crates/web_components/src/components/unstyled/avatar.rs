#![allow(non_snake_case)]

use crate::components::{ComponentInfo, shared};
use maud::{Markup, html};
use std::collections::BTreeMap;

pub const COMPONENTS: &[ComponentInfo] = &[
    ComponentInfo {
        family: "unstyled",
        module: "avatar",
        name: "Avatar",
    },
    ComponentInfo {
        family: "unstyled",
        module: "avatar",
        name: "AvatarWithIcon",
    },
    ComponentInfo {
        family: "unstyled",
        module: "avatar",
        name: "AvatarGroup",
    },
    ComponentInfo {
        family: "unstyled",
        module: "avatar",
        name: "AvatarFallbackIcon",
    },
];

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AvatarProps {
    pub src: String,
    pub alt: String,
    pub initials: String,
    pub size: &'static str,
    pub shape: &'static str,
    pub status: String,
    pub class: String,
    pub attributes: BTreeMap<String, String>,
    pub aria_label: String,
}

impl Default for AvatarProps {
    fn default() -> Self {
        Self {
            src: String::new(),
            alt: String::new(),
            initials: String::new(),
            size: shared::SIZE_MD,
            shape: shared::AVATAR_SHAPE_CIRCLE,
            status: String::new(),
            class: String::new(),
            attributes: BTreeMap::new(),
            aria_label: String::new(),
        }
    }
}

impl AvatarProps {
    #[must_use]
    pub fn with_src(mut self, src: impl Into<String>) -> Self {
        self.src = src.into();
        self
    }

    #[must_use]
    pub fn with_alt(mut self, alt: impl Into<String>) -> Self {
        self.alt = alt.into();
        self
    }

    #[must_use]
    pub fn with_initials(mut self, initials: impl Into<String>) -> Self {
        self.initials = initials.into();
        self
    }

    #[must_use]
    pub const fn with_size(mut self, size: &'static str) -> Self {
        self.size = size;
        self
    }

    #[must_use]
    pub const fn with_shape(mut self, shape: &'static str) -> Self {
        self.shape = shape;
        self
    }

    #[must_use]
    pub fn with_status(mut self, status: impl Into<String>) -> Self {
        self.status = status.into();
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
    pub fn build_classes(&self) -> String {
        let shape_class = match self.shape {
            shared::AVATAR_SHAPE_SQUARE => "rounded-none",
            shared::AVATAR_SHAPE_ROUNDED => "rounded-md",
            _ => "rounded-full",
        };

        shared::build_classes(&[
            "relative inline-flex items-center justify-center overflow-hidden bg-muted text-muted-foreground font-medium",
            shared::avatar_size_classes(self.size),
            shape_class,
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
    pub fn status_indicator_class(&self) -> String {
        let size_class = match self.size {
            shared::SIZE_XS => "h-1.5 w-1.5",
            shared::SIZE_SM => "h-2 w-2",
            shared::SIZE_LG => "h-3 w-3",
            shared::SIZE_XL => "h-4 w-4",
            _ => "h-2.5 w-2.5",
        };

        let color_class = match self.status.as_str() {
            shared::AVATAR_STATUS_ONLINE => "bg-success",
            shared::AVATAR_STATUS_OFFLINE => "bg-muted-foreground",
            shared::AVATAR_STATUS_BUSY => "bg-destructive",
            _ => "",
        };

        shared::build_classes(&[
            "absolute bottom-0 right-0 block rounded-full ring-2 ring-background",
            size_class,
            color_class,
        ])
    }
}

fn status_indicator(props: &AvatarProps) -> Markup {
    html! {
        @if !props.status.is_empty() {
            span class=(props.status_indicator_class()) {}
        }
    }
}

#[must_use]
pub fn Avatar(props: &AvatarProps, children: Markup) -> Markup {
    let attrs = props.build_attributes();
    html! {
        span
            class=(props.build_classes())
            id=[shared::attr_value(&attrs, "id")]
            title=[shared::attr_value(&attrs, "title")]
            role=[shared::attr_value(&attrs, "role")]
            aria-label=[shared::attr_value(&attrs, "aria-label")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
        {
            @if !props.src.is_empty() {
                img
                    src=(&props.src)
                    alt=(&props.alt)
                    class="h-full w-full object-cover"
                    onerror="this.style.display='none'; this.nextElementSibling.style.display='flex';"
                {}
                span class="hidden h-full w-full items-center justify-center" {
                    @if !props.initials.is_empty() {
                        (&props.initials)
                    } @else {
                        (children)
                    }
                }
            } @else if !props.initials.is_empty() {
                span class="flex h-full w-full items-center justify-center" {
                    (&props.initials)
                }
            } @else {
                (children)
            }
            (status_indicator(props))
        }
    }
}

#[must_use]
pub fn AvatarWithIcon(props: &AvatarProps, icon: Markup) -> Markup {
    let attrs = props.build_attributes();
    html! {
        span
            class=(props.build_classes())
            id=[shared::attr_value(&attrs, "id")]
            title=[shared::attr_value(&attrs, "title")]
            role=[shared::attr_value(&attrs, "role")]
            aria-label=[shared::attr_value(&attrs, "aria-label")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
        {
            @if !props.src.is_empty() {
                img
                    src=(&props.src)
                    alt=(&props.alt)
                    class="h-full w-full object-cover"
                    onerror="this.style.display='none'; this.nextElementSibling.style.display='flex';"
                {}
                span class="hidden h-full w-full items-center justify-center" { (icon) }
            } @else {
                span class="flex h-full w-full items-center justify-center" { (icon) }
            }
            (status_indicator(props))
        }
    }
}

#[must_use]
pub fn AvatarGroup(class: impl Into<String>, _max_visible: i32, children: Markup) -> Markup {
    let class = class.into();
    html! {
        div class=(shared::build_classes(&["flex -space-x-2", class.as_str()])) role="group" {
            (children)
        }
    }
}

#[must_use]
pub fn AvatarFallbackIcon() -> Markup {
    html! {
        svg class="h-3/5 w-3/5" fill="currentColor" viewBox="0 0 24 24" xmlns="http://www.w3.org/2000/svg" {
            path d="M12 12c2.21 0 4-1.79 4-4s-1.79-4-4-4-4 1.79-4 4 1.79 4 4 4zm0 2c-2.67 0-8 1.34-8 4v2h16v-2c0-2.66-5.33-4-8-4z" {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_props_match_go_defaults() {
        let props = AvatarProps::default();
        assert_eq!(props.size, shared::SIZE_MD);
        assert_eq!(props.shape, shared::AVATAR_SHAPE_CIRCLE);
        assert!(props.src.is_empty());
        assert!(props.initials.is_empty());
    }

    #[test]
    fn helper_methods_cover_size_shape_status_attrs_and_classes() {
        for (shape, expected) in [
            (shared::AVATAR_SHAPE_CIRCLE, "rounded-full"),
            (shared::AVATAR_SHAPE_SQUARE, "rounded-none"),
            (shared::AVATAR_SHAPE_ROUNDED, "rounded-md"),
            ("unknown", "rounded-full"),
        ] {
            assert!(
                AvatarProps::default()
                    .with_shape(shape)
                    .build_classes()
                    .contains(expected)
            );
        }

        for (size, expected) in [
            (shared::SIZE_XS, "h-1.5 w-1.5"),
            (shared::SIZE_SM, "h-2 w-2"),
            (shared::SIZE_MD, "h-2.5 w-2.5"),
            (shared::SIZE_LG, "h-3 w-3"),
            (shared::SIZE_XL, "h-4 w-4"),
        ] {
            assert!(
                AvatarProps::default()
                    .with_size(size)
                    .with_status("online")
                    .status_indicator_class()
                    .contains(expected)
            );
        }

        assert!(
            AvatarProps::default()
                .with_status("online")
                .status_indicator_class()
                .contains("bg-success")
        );
        assert!(
            AvatarProps::default()
                .with_status("offline")
                .status_indicator_class()
                .contains("bg-muted-foreground")
        );
        assert!(
            AvatarProps::default()
                .with_status("busy")
                .status_indicator_class()
                .contains("bg-destructive")
        );
        assert!(
            !AvatarProps::default()
                .with_status("away")
                .status_indicator_class()
                .contains("bg-success")
        );

        let attrs = AvatarProps::default()
            .with_aria_label("User")
            .with_attribute("title", "Profile")
            .build_attributes();
        assert_eq!(attrs.get("aria-label").map(String::as_str), Some("User"));
        assert_eq!(attrs.get("title").map(String::as_str), Some("Profile"));
    }

    #[test]
    fn avatar_renders_image_fallback_initials_children_status_and_escaping() {
        let html = Avatar(
            &AvatarProps::default()
                .with_src("/avatar.png")
                .with_alt("<Alice>")
                .with_initials("AG")
                .with_status("online")
                .with_class("custom")
                .with_attribute("data-testid", "avatar"),
            html! { "fallback child" },
        )
        .into_string();

        assert!(html.starts_with("<span "));
        assert!(html.contains("custom"));
        assert!(html.contains("src=\"/avatar.png\""));
        assert!(html.contains("alt=\"&lt;Alice&gt;\""));
        assert!(html.contains(
            "onerror=\"this.style.display='none'; this.nextElementSibling.style.display='flex';\""
        ));
        assert!(html.contains("class=\"hidden h-full w-full items-center justify-center\""));
        assert!(html.contains(">AG</span>"));
        assert!(html.contains("bg-success"));
        assert!(html.contains("data-testid=\"avatar\""));

        let initials = Avatar(
            &AvatarProps::default().with_initials("<BB>"),
            html! { "ignored" },
        )
        .into_string();
        assert!(initials.contains("&lt;BB&gt;"));
        assert!(!initials.contains("ignored"));

        let child = Avatar(&AvatarProps::default(), html! { "<icon>" }).into_string();
        assert!(child.contains("&lt;icon&gt;"));
    }

    #[test]
    fn avatar_with_icon_group_and_fallback_icon_render_expected_html() {
        let icon_avatar = AvatarWithIcon(
            &AvatarProps::default()
                .with_src("/x.png")
                .with_status("busy"),
            html! { svg class="icon" {} },
        )
        .into_string();
        assert!(icon_avatar.contains("src=\"/x.png\""));
        assert!(icon_avatar.contains(
            "class=\"hidden h-full w-full items-center justify-center\"><svg class=\"icon\"></svg>"
        ));
        assert!(icon_avatar.contains("bg-destructive"));

        let group = AvatarGroup("custom", 2, html! { span { "A" } span { "B" } }).into_string();
        assert_eq!(
            group,
            "<div class=\"flex -space-x-2 custom\" role=\"group\"><span>A</span><span>B</span></div>"
        );

        let fallback = AvatarFallbackIcon().into_string();
        assert!(fallback.contains("<svg class=\"h-3/5 w-3/5\""));
        assert!(fallback.contains("M12 12c2.21"));
    }
}
