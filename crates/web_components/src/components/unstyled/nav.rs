#![allow(non_snake_case)]

use crate::components::{ComponentInfo, shared};
use maud::{Markup, html};
use std::collections::BTreeMap;

pub const COMPONENTS: &[ComponentInfo] = &[
    ComponentInfo {
        family: "unstyled",
        module: "nav",
        name: "Nav",
    },
    ComponentInfo {
        family: "unstyled",
        module: "nav",
        name: "NavItem",
    },
    ComponentInfo {
        family: "unstyled",
        module: "nav",
        name: "HorizontalNav",
    },
    ComponentInfo {
        family: "unstyled",
        module: "nav",
        name: "VerticalNav",
    },
    ComponentInfo {
        family: "unstyled",
        module: "nav",
        name: "SimpleNavItem",
    },
    ComponentInfo {
        family: "unstyled",
        module: "nav",
        name: "NavItemWithIcon",
    },
];

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NavProps {
    pub orientation: String,
    pub class: String,
    pub attributes: BTreeMap<String, String>,
    pub aria_label: String,
    pub aria_described_by: String,
}

impl Default for NavProps {
    fn default() -> Self {
        Self {
            orientation: "horizontal".to_string(),
            class: String::new(),
            attributes: BTreeMap::new(),
            aria_label: String::new(),
            aria_described_by: String::new(),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct NavItemProps {
    pub href: String,
    pub active: bool,
    pub disabled: bool,
    pub icon: Option<Markup>,
    pub class: String,
    pub attributes: BTreeMap<String, String>,
    pub aria_label: String,
    pub aria_current: String,
}

impl NavProps {
    #[must_use]
    pub fn with_orientation(mut self, value: impl Into<String>) -> Self {
        self.orientation = value.into();
        self
    }

    #[must_use]
    pub fn with_class(mut self, value: impl Into<String>) -> Self {
        self.class = value.into();
        self
    }

    #[must_use]
    pub fn with_attribute(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.attributes.insert(name.into(), value.into());
        self
    }

    #[must_use]
    pub fn with_aria_label(mut self, value: impl Into<String>) -> Self {
        self.aria_label = value.into();
        self
    }

    #[must_use]
    pub fn with_aria_described_by(mut self, value: impl Into<String>) -> Self {
        self.aria_described_by = value.into();
        self
    }

    #[must_use]
    pub fn build_classes(&self) -> String {
        let orientation = if self.orientation == "vertical" {
            "flex-col space-y-1"
        } else {
            "flex-row space-x-1 items-center"
        };
        shared::build_classes(&["flex", orientation, self.class.as_str()])
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
        attrs
    }
}

impl NavItemProps {
    #[must_use]
    pub fn with_href(mut self, value: impl Into<String>) -> Self {
        self.href = value.into();
        self
    }

    #[must_use]
    pub const fn with_active(mut self, value: bool) -> Self {
        self.active = value;
        self
    }

    #[must_use]
    pub const fn with_disabled(mut self, value: bool) -> Self {
        self.disabled = value;
        self
    }

    #[must_use]
    pub fn with_icon(mut self, icon: Markup) -> Self {
        self.icon = Some(icon);
        self
    }

    #[must_use]
    pub fn with_class(mut self, value: impl Into<String>) -> Self {
        self.class = value.into();
        self
    }

    #[must_use]
    pub fn with_attribute(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.attributes.insert(name.into(), value.into());
        self
    }

    #[must_use]
    pub fn with_aria_label(mut self, value: impl Into<String>) -> Self {
        self.aria_label = value.into();
        self
    }

    #[must_use]
    pub fn with_aria_current(mut self, value: impl Into<String>) -> Self {
        self.aria_current = value.into();
        self
    }

    #[must_use]
    pub fn build_classes(&self) -> String {
        let state = if self.active {
            "bg-accent text-accent-foreground"
        } else if self.disabled {
            "pointer-events-none opacity-50 cursor-not-allowed"
        } else {
            "hover:bg-accent hover:text-accent-foreground"
        };
        shared::build_classes(&[
            "inline-flex items-center gap-2 px-3 py-2 text-sm font-medium rounded-md transition-all duration-150 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2",
            state,
            self.class.as_str(),
        ])
    }

    #[must_use]
    pub fn build_attributes(&self) -> BTreeMap<String, String> {
        let mut attrs = self.attributes.clone();
        if !self.aria_label.is_empty() {
            attrs.insert("aria-label".to_string(), self.aria_label.clone());
        }
        if self.active {
            attrs.insert(
                "aria-current".to_string(),
                if self.aria_current.is_empty() {
                    "page".to_string()
                } else {
                    self.aria_current.clone()
                },
            );
        }
        if self.disabled {
            attrs.insert("aria-disabled".to_string(), "true".to_string());
        }
        attrs
    }
}

#[must_use]
pub fn Nav(props: &NavProps, children: Markup) -> Markup {
    let attrs = props.build_attributes();
    html! {
        nav
            class=(props.build_classes())
            role="navigation"
            aria-label=[shared::attr_value(&attrs, "aria-label")]
            aria-describedby=[shared::attr_value(&attrs, "aria-describedby")]
            id=[shared::attr_value(&attrs, "id")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
        {
            (children)
        }
    }
}

fn nav_item_content(props: &NavItemProps, children: Markup) -> Markup {
    html! {
        @if let Some(icon) = &props.icon {
            span class="nav-item-icon" {
                (icon.clone())
            }
        }
        span class="nav-item-content" {
            (children)
        }
    }
}

#[must_use]
pub fn NavItem(props: &NavItemProps, children: Markup) -> Markup {
    let attrs = props.build_attributes();
    if props.disabled {
        html! {
            span
                class=(props.build_classes())
                aria-label=[shared::attr_value(&attrs, "aria-label")]
                aria-current=[shared::attr_value(&attrs, "aria-current")]
                aria-disabled=[shared::attr_value(&attrs, "aria-disabled")]
                id=[shared::attr_value(&attrs, "id")]
                data-testid=[shared::attr_value(&attrs, "data-testid")]
            {
                (nav_item_content(props, children))
            }
        }
    } else {
        html! {
            a
                href=(&props.href)
                class=(props.build_classes())
                aria-label=[shared::attr_value(&attrs, "aria-label")]
                aria-current=[shared::attr_value(&attrs, "aria-current")]
                id=[shared::attr_value(&attrs, "id")]
                data-testid=[shared::attr_value(&attrs, "data-testid")]
            {
                (nav_item_content(props, children))
            }
        }
    }
}

#[must_use]
pub fn HorizontalNav(aria_label: &str, class: &str, children: Markup) -> Markup {
    Nav(
        &NavProps::default()
            .with_orientation("horizontal")
            .with_aria_label(aria_label)
            .with_class(class),
        children,
    )
}

#[must_use]
pub fn VerticalNav(aria_label: &str, class: &str, children: Markup) -> Markup {
    Nav(
        &NavProps::default()
            .with_orientation("vertical")
            .with_aria_label(aria_label)
            .with_class(class),
        children,
    )
}

#[must_use]
pub fn SimpleNavItem(href: &str, active: bool, class: &str, children: Markup) -> Markup {
    NavItem(
        &NavItemProps::default()
            .with_href(href)
            .with_active(active)
            .with_class(class),
        children,
    )
}

#[must_use]
pub fn NavItemWithIcon(
    href: &str,
    active: bool,
    icon: Markup,
    class: &str,
    children: Markup,
) -> Markup {
    NavItem(
        &NavItemProps::default()
            .with_href(href)
            .with_active(active)
            .with_icon(icon)
            .with_class(class),
        children,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nav_helpers_cover_orientations_classes_and_aria_attrs() {
        let props = NavProps::default();
        assert_eq!(props.orientation, "horizontal");
        assert_eq!(
            props.build_classes(),
            "flex flex-row space-x-1 items-center"
        );
        assert_eq!(
            NavProps::default()
                .with_orientation("vertical")
                .with_class("gap-2")
                .build_classes(),
            "flex flex-col space-y-1 gap-2"
        );
        let attrs = NavProps::default()
            .with_aria_label("Primary")
            .with_aria_described_by("nav-help")
            .with_attribute("id", "primary-nav")
            .build_attributes();
        assert_eq!(attrs.get("aria-label").map(String::as_str), Some("Primary"));
        assert_eq!(
            attrs.get("aria-describedby").map(String::as_str),
            Some("nav-help")
        );
        assert_eq!(attrs.get("id").map(String::as_str), Some("primary-nav"));
    }

    #[test]
    fn nav_item_helpers_cover_active_disabled_icon_and_aria_attrs() {
        assert_eq!(
            NavItemProps::default().build_classes(),
            "inline-flex items-center gap-2 px-3 py-2 text-sm font-medium rounded-md transition-all duration-150 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 hover:bg-accent hover:text-accent-foreground"
        );
        assert!(
            NavItemProps::default()
                .with_active(true)
                .build_classes()
                .contains("bg-accent text-accent-foreground")
        );
        assert!(
            NavItemProps::default()
                .with_disabled(true)
                .build_classes()
                .contains("pointer-events-none opacity-50 cursor-not-allowed")
        );
        let attrs = NavItemProps::default()
            .with_active(true)
            .with_aria_current("step")
            .with_aria_label("Step one")
            .with_attribute("id", "step-one")
            .build_attributes();
        assert_eq!(attrs.get("aria-current").map(String::as_str), Some("step"));
        assert_eq!(
            attrs.get("aria-label").map(String::as_str),
            Some("Step one")
        );
        assert_eq!(attrs.get("id").map(String::as_str), Some("step-one"));
        assert_eq!(
            NavItemProps::default()
                .with_active(true)
                .build_attributes()
                .get("aria-current")
                .map(String::as_str),
            Some("page")
        );
        assert_eq!(
            NavItemProps::default()
                .with_disabled(true)
                .build_attributes()
                .get("aria-disabled")
                .map(String::as_str),
            Some("true")
        );
    }

    #[test]
    fn nav_and_items_render_links_disabled_spans_icons_and_escaping() {
        let rendered = Nav(
            &NavProps::default()
                .with_aria_label("Main")
                .with_attribute("data-testid", "main-nav"),
            html! {
                (NavItem(
                    &NavItemProps::default()
                        .with_href("/home")
                        .with_active(true)
                        .with_icon(html! { svg data-testid="home-icon" {} }),
                    html! { "<Home>" },
                ))
                (NavItem(
                    &NavItemProps::default()
                        .with_disabled(true)
                        .with_aria_label("Disabled"),
                    html! { "Disabled" },
                ))
            },
        )
        .into_string();
        assert!(rendered.starts_with(
            "<nav class=\"flex flex-row space-x-1 items-center\" role=\"navigation\""
        ));
        assert!(rendered.contains("aria-label=\"Main\""));
        assert!(rendered.contains("data-testid=\"main-nav\""));
        assert!(rendered.contains("<a href=\"/home\""));
        assert!(rendered.contains("aria-current=\"page\""));
        assert!(rendered.contains(
            "<span class=\"nav-item-icon\"><svg data-testid=\"home-icon\"></svg></span>"
        ));
        assert!(rendered.contains("<span class=\"nav-item-content\">&lt;Home&gt;</span>"));
        assert!(rendered.contains("<span class=\"inline-flex items-center gap-2"));
        assert!(rendered.contains("aria-label=\"Disabled\""));
        assert!(rendered.contains("aria-disabled=\"true\""));
        assert!(rendered.contains("<span class=\"nav-item-content\">Disabled</span>"));
    }

    #[test]
    fn convenience_nav_components_render_expected_html() {
        let horizontal = HorizontalNav("Primary", "top", html! { "Body" }).into_string();
        assert_eq!(
            horizontal,
            "<nav class=\"flex flex-row space-x-1 items-center top\" role=\"navigation\" aria-label=\"Primary\">Body</nav>"
        );
        let vertical = VerticalNav("Side", "", html! {}).into_string();
        assert!(vertical.contains("class=\"flex flex-col space-y-1\""));
        assert!(vertical.contains("aria-label=\"Side\""));
        let simple = SimpleNavItem("/docs", false, "link", html! { "Docs" }).into_string();
        assert!(simple.contains("<a href=\"/docs\""));
        assert!(simple.contains("link\""));
        assert!(simple.contains(">Docs</span></a>"));
        let with_icon = NavItemWithIcon(
            "/settings",
            true,
            html! { svg data-testid="settings-icon" {} },
            "settings",
            html! { "Settings" },
        )
        .into_string();
        assert!(with_icon.contains("aria-current=\"page\""));
        assert!(with_icon.contains(
            "<span class=\"nav-item-icon\"><svg data-testid=\"settings-icon\"></svg></span>"
        ));
    }
}
