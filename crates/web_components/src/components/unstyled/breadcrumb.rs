#![allow(non_snake_case)]

use crate::components::{ComponentInfo, shared};
use maud::{Markup, html};
use std::collections::BTreeMap;

pub const COMPONENTS: &[ComponentInfo] = &[
    ComponentInfo {
        family: "unstyled",
        module: "breadcrumb",
        name: "Breadcrumb",
    },
    ComponentInfo {
        family: "unstyled",
        module: "breadcrumb",
        name: "BreadcrumbItem",
    },
    ComponentInfo {
        family: "unstyled",
        module: "breadcrumb",
        name: "BreadcrumbSeparator",
    },
    ComponentInfo {
        family: "unstyled",
        module: "breadcrumb",
        name: "BreadcrumbEllipsis",
    },
    ComponentInfo {
        family: "unstyled",
        module: "breadcrumb",
        name: "SimpleBreadcrumb",
    },
    ComponentInfo {
        family: "unstyled",
        module: "breadcrumb",
        name: "BreadcrumbLink",
    },
    ComponentInfo {
        family: "unstyled",
        module: "breadcrumb",
        name: "BreadcrumbCurrent",
    },
    ComponentInfo {
        family: "unstyled",
        module: "breadcrumb",
        name: "BreadcrumbWithSeparator",
    },
];

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BreadcrumbProps {
    pub separator: String,
    pub max_items: i32,
    pub class: String,
    pub attributes: BTreeMap<String, String>,
    pub aria_label: String,
}

impl Default for BreadcrumbProps {
    fn default() -> Self {
        Self {
            separator: "/".to_string(),
            max_items: 0,
            class: String::new(),
            attributes: BTreeMap::new(),
            aria_label: String::new(),
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct BreadcrumbItemProps {
    pub href: String,
    pub current: bool,
    pub class: String,
    pub attributes: BTreeMap<String, String>,
}

impl BreadcrumbProps {
    #[must_use]
    pub fn with_separator(mut self, value: impl Into<String>) -> Self {
        self.separator = value.into();
        self
    }

    #[must_use]
    pub const fn with_max_items(mut self, value: i32) -> Self {
        self.max_items = value;
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
    pub fn build_classes(&self) -> String {
        shared::build_classes(&[
            "flex items-center space-x-2 text-sm text-muted-foreground",
            self.class.as_str(),
        ])
    }

    #[must_use]
    pub fn build_attributes(&self) -> BTreeMap<String, String> {
        self.attributes.clone()
    }
}

impl BreadcrumbItemProps {
    #[must_use]
    pub fn with_href(mut self, value: impl Into<String>) -> Self {
        self.href = value.into();
        self
    }

    #[must_use]
    pub const fn with_current(mut self, value: bool) -> Self {
        self.current = value;
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
    pub fn build_classes(&self) -> String {
        shared::build_classes(&[
            "hover:text-foreground transition-colors duration-150",
            if self.current {
                "text-foreground font-medium pointer-events-none"
            } else {
                ""
            },
            self.class.as_str(),
        ])
    }

    #[must_use]
    pub fn build_attributes(&self) -> BTreeMap<String, String> {
        let mut attrs = self.attributes.clone();
        if self.current {
            attrs.insert("aria-current".to_string(), "page".to_string());
        }
        attrs
    }
}

#[must_use]
pub fn resolveBreadcrumbAriaLabel(props: &BreadcrumbProps) -> String {
    if props.aria_label.is_empty() {
        "Breadcrumb".to_string()
    } else {
        props.aria_label.clone()
    }
}

#[must_use]
pub fn Breadcrumb(props: &BreadcrumbProps, children: Markup) -> Markup {
    let attrs = props.build_attributes();
    html! {
        nav
            class=(props.build_classes())
            aria-label=(resolveBreadcrumbAriaLabel(props))
            id=[shared::attr_value(&attrs, "id")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
        {
            ol class="flex items-center space-x-2 list-none" {
                (children)
            }
        }
    }
}

#[must_use]
pub fn BreadcrumbItem(props: &BreadcrumbItemProps, children: Markup) -> Markup {
    let attrs = props.build_attributes();
    html! {
        li class="flex items-center space-x-2" {
            @if props.current {
                span
                    class=(props.build_classes())
                    aria-current=[shared::attr_value(&attrs, "aria-current")]
                    id=[shared::attr_value(&attrs, "id")]
                    data-testid=[shared::attr_value(&attrs, "data-testid")]
                {
                    (children)
                }
            } @else {
                a
                    href=(&props.href)
                    class=(props.build_classes())
                    id=[shared::attr_value(&attrs, "id")]
                    data-testid=[shared::attr_value(&attrs, "data-testid")]
                {
                    (children)
                }
            }
        }
    }
}

#[must_use]
pub fn BreadcrumbSeparator(separator: &str) -> Markup {
    html! {
        li class="flex items-center" role="presentation" aria-hidden="true" {
            span class="text-muted-foreground" { (separator) }
        }
    }
}

#[must_use]
pub fn BreadcrumbEllipsis() -> Markup {
    html! {
        li class="flex items-center" role="presentation" aria-hidden="true" {
            span class="text-muted-foreground" { "..." }
        }
    }
}

#[must_use]
pub fn SimpleBreadcrumb(
    separator: &str,
    aria_label: &str,
    class: &str,
    children: Markup,
) -> Markup {
    Breadcrumb(
        &BreadcrumbProps::default()
            .with_separator(separator)
            .with_aria_label(aria_label)
            .with_class(class),
        children,
    )
}

#[must_use]
pub fn BreadcrumbLink(href: &str, class: &str, children: Markup) -> Markup {
    BreadcrumbItem(
        &BreadcrumbItemProps::default()
            .with_href(href)
            .with_current(false)
            .with_class(class),
        children,
    )
}

#[must_use]
pub fn BreadcrumbCurrent(class: &str, children: Markup) -> Markup {
    BreadcrumbItem(
        &BreadcrumbItemProps::default()
            .with_current(true)
            .with_class(class),
        children,
    )
}

#[must_use]
pub fn BreadcrumbWithSeparator(
    props: &BreadcrumbItemProps,
    separator: &str,
    is_last: bool,
    children: Markup,
) -> Markup {
    html! {
        (BreadcrumbItem(props, children))
        @if !is_last {
            (BreadcrumbSeparator(separator))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn helpers_cover_defaults_classes_attrs_aria_and_item_state() {
        let props = BreadcrumbProps::default();
        assert_eq!(props.separator, "/");
        assert_eq!(props.max_items, 0);
        assert_eq!(
            props.build_classes(),
            "flex items-center space-x-2 text-sm text-muted-foreground"
        );
        assert_eq!(resolveBreadcrumbAriaLabel(&props), "Breadcrumb");
        assert_eq!(
            resolveBreadcrumbAriaLabel(&props.clone().with_aria_label("Path")),
            "Path"
        );
        assert_eq!(
            props.with_class("mb-4").build_classes(),
            "flex items-center space-x-2 text-sm text-muted-foreground mb-4"
        );
        let attrs = BreadcrumbProps::default()
            .with_max_items(3)
            .with_attribute("id", "crumbs")
            .build_attributes();
        assert_eq!(attrs.get("id").map(String::as_str), Some("crumbs"));

        let item = BreadcrumbItemProps::default()
            .with_current(true)
            .with_class("current");
        assert_eq!(
            item.build_classes(),
            "hover:text-foreground transition-colors duration-150 text-foreground font-medium pointer-events-none current"
        );
        assert_eq!(
            item.build_attributes()
                .get("aria-current")
                .map(String::as_str),
            Some("page")
        );
    }

    #[test]
    fn breadcrumb_renders_nav_items_separators_and_escaping() {
        let html = Breadcrumb(
            &BreadcrumbProps::default()
                .with_aria_label("Site path")
                .with_class("custom")
                .with_attribute("data-testid", "breadcrumb"),
            html! {
                (BreadcrumbItem(&BreadcrumbItemProps::default().with_href("/"), html! { "Home" }))
                (BreadcrumbSeparator(">"))
                (BreadcrumbItem(
                    &BreadcrumbItemProps::default()
                        .with_current(true)
                        .with_attribute("id", "current"),
                    html! { "<Settings>" },
                ))
            },
        )
        .into_string();
        assert!(html.starts_with(
            "<nav class=\"flex items-center space-x-2 text-sm text-muted-foreground custom\""
        ));
        assert!(html.contains("aria-label=\"Site path\""));
        assert!(html.contains("data-testid=\"breadcrumb\""));
        assert!(html.contains("<ol class=\"flex items-center space-x-2 list-none\">"));
        assert!(html.contains(
            "<a href=\"/\" class=\"hover:text-foreground transition-colors duration-150\">Home</a>"
        ));
        assert!(html.contains("<span class=\"text-muted-foreground\">&gt;</span>"));
        assert!(html.contains("<span class=\"hover:text-foreground transition-colors duration-150 text-foreground font-medium pointer-events-none\" aria-current=\"page\" id=\"current\">&lt;Settings&gt;</span>"));
    }

    #[test]
    fn convenience_components_render_expected_html() {
        let simple = SimpleBreadcrumb("/", "Trail", "small", html! { "Body" }).into_string();
        assert_eq!(
            simple,
            "<nav class=\"flex items-center space-x-2 text-sm text-muted-foreground small\" aria-label=\"Trail\"><ol class=\"flex items-center space-x-2 list-none\">Body</ol></nav>"
        );
        let link = BreadcrumbLink("/docs", "link", html! { "Docs" }).into_string();
        assert!(link.contains("<a href=\"/docs\" class=\"hover:text-foreground transition-colors duration-150 link\">Docs</a>"));
        let current = BreadcrumbCurrent("current", html! { "Now" }).into_string();
        assert!(current.contains("aria-current=\"page\""));
        assert!(current.contains(">Now</span>"));
        assert_eq!(
            BreadcrumbEllipsis().into_string(),
            "<li class=\"flex items-center\" role=\"presentation\" aria-hidden=\"true\"><span class=\"text-muted-foreground\">...</span></li>"
        );
        let with_separator = BreadcrumbWithSeparator(
            &BreadcrumbItemProps::default().with_href("/a"),
            "/",
            false,
            html! { "A" },
        )
        .into_string();
        assert!(with_separator.contains("<a href=\"/a\""));
        assert!(with_separator.contains("<span class=\"text-muted-foreground\">/</span>"));
        let last = BreadcrumbWithSeparator(
            &BreadcrumbItemProps::default().with_href("/b"),
            "/",
            true,
            html! { "B" },
        )
        .into_string();
        assert!(!last.contains("role=\"presentation\""));
    }
}
