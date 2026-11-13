#![allow(non_snake_case)]

use crate::components::{ComponentInfo, shared};
use maud::{Markup, html};
use std::collections::BTreeMap;

pub const COMPONENTS: &[ComponentInfo] = &[
    ComponentInfo {
        family: "unstyled",
        module: "topnav",
        name: "TopNavLink",
    },
    ComponentInfo {
        family: "unstyled",
        module: "topnav",
        name: "TopNav",
    },
    ComponentInfo {
        family: "unstyled",
        module: "topnav",
        name: "SimpleTopNav",
    },
    ComponentInfo {
        family: "unstyled",
        module: "topnav",
        name: "SimpleTopNavLink",
    },
];

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct TopNavLinkProps {
    pub href: String,
    pub label: String,
    pub is_active: bool,
    pub is_external: bool,
    pub class: String,
    pub attributes: BTreeMap<String, String>,
}

impl TopNavLinkProps {
    #[must_use]
    pub fn with_href(mut self, value: impl Into<String>) -> Self {
        self.href = value.into();
        self
    }

    #[must_use]
    pub fn with_label(mut self, value: impl Into<String>) -> Self {
        self.label = value.into();
        self
    }

    #[must_use]
    pub const fn with_active(mut self, value: bool) -> Self {
        self.is_active = value;
        self
    }

    #[must_use]
    pub const fn with_external(mut self, value: bool) -> Self {
        self.is_external = value;
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
            "inline-flex items-center gap-1 px-3 py-2 text-sm font-medium transition-colors duration-150 rounded-md",
            if self.is_active {
                "bg-accent text-accent-foreground"
            } else {
                "text-muted-foreground hover:text-foreground hover:bg-accent/50"
            },
            self.class.as_str(),
        ])
    }

    #[must_use]
    pub fn build_attributes(&self) -> BTreeMap<String, String> {
        let mut attrs = self.attributes.clone();
        if self.is_external {
            attrs.insert("target".to_string(), "_blank".to_string());
            attrs.insert("rel".to_string(), "noopener noreferrer".to_string());
        }
        attrs
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TopNavProps {
    pub links: Vec<TopNavLinkProps>,
    pub logo_href: String,
    pub logo_text: String,
    pub logo_src: String,
    pub show_search: bool,
    pub class: String,
    pub attributes: BTreeMap<String, String>,
}

impl Default for TopNavProps {
    fn default() -> Self {
        Self {
            links: Vec::new(),
            logo_href: "/".to_string(),
            logo_text: String::new(),
            logo_src: String::new(),
            show_search: false,
            class: String::new(),
            attributes: BTreeMap::new(),
        }
    }
}

impl TopNavProps {
    #[must_use]
    pub fn with_links(mut self, value: Vec<TopNavLinkProps>) -> Self {
        self.links = value;
        self
    }

    #[must_use]
    pub fn with_logo(
        mut self,
        href: impl Into<String>,
        text: impl Into<String>,
        src: impl Into<String>,
    ) -> Self {
        self.logo_href = href.into();
        self.logo_text = text.into();
        self.logo_src = src.into();
        self
    }

    #[must_use]
    pub const fn with_search(mut self, value: bool) -> Self {
        self.show_search = value;
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
            "sticky top-0 z-40 w-full border-b border-border bg-background/95 backdrop-blur supports-[backdrop-filter]:bg-background/60",
            self.class.as_str(),
        ])
    }

    #[must_use]
    pub fn build_attributes(&self) -> BTreeMap<String, String> {
        self.attributes.clone()
    }
}

fn external_icon() -> Markup {
    html! {
        svg xmlns="http://www.w3.org/2000/svg" width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" class="opacity-50" {
            path d="M18 13v6a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h6" {}
            polyline points="15 3 21 3 21 9" {}
            line x1="10" y1="14" x2="21" y2="3" {}
        }
    }
}

fn search_button() -> Markup {
    html! {
        div class="w-full flex-1 md:w-auto md:flex-none" {
            button type="button" class="inline-flex items-center whitespace-nowrap rounded-md font-medium text-sm border border-input px-4 py-2 hover:bg-accent hover:text-accent-foreground w-full justify-start text-muted-foreground sm:pr-12 md:w-40 lg:w-64" {
                svg xmlns="http://www.w3.org/2000/svg" width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" class="mr-2" {
                    circle cx="11" cy="11" r="8" {}
                    path d="m21 21-4.3-4.3" {}
                }
                span class="hidden lg:inline-flex" { "Search..." }
                span class="inline-flex lg:hidden" { "Search..." }
            }
        }
    }
}

#[must_use]
pub fn TopNavLink(props: &TopNavLinkProps, children: Markup) -> Markup {
    let attrs = props.build_attributes();
    html! {
        a
            href=(&props.href)
            class=(props.build_classes())
            aria-current=[if props.is_active { Some("page") } else { None }]
            target=[shared::attr_value(&attrs, "target")]
            rel=[shared::attr_value(&attrs, "rel")]
            id=[shared::attr_value(&attrs, "id")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
        {
            @if props.label.is_empty() {
                (children)
            } @else {
                (&props.label)
            }
            @if props.is_external {
                (external_icon())
            }
        }
    }
}

#[must_use]
pub fn TopNav(props: &TopNavProps, children: Markup) -> Markup {
    let attrs = props.build_attributes();
    html! {
        header
            class=(props.build_classes())
            id=[shared::attr_value(&attrs, "id")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
        {
            div class="container flex h-14 items-center" {
                div class="mr-4 flex" {
                    a href=(&props.logo_href) class="mr-6 flex items-center space-x-2" {
                        @if !props.logo_src.is_empty() {
                            img src=(&props.logo_src) alt=(&props.logo_text) class="h-6 w-auto";
                        }
                        @if !props.logo_text.is_empty() {
                            span class="font-bold" { (&props.logo_text) }
                        }
                    }
                }
                nav class="flex items-center gap-1 text-sm" {
                    @for link in &props.links {
                        (TopNavLink(link, html! {}))
                    }
                }
                div class="flex flex-1 items-center justify-end space-x-4" {
                    @if props.show_search {
                        (search_button())
                    }
                    (children)
                }
            }
        }
    }
}

#[must_use]
pub fn SimpleTopNav(logo_text: &str, links: Vec<TopNavLinkProps>, class: &str) -> Markup {
    TopNav(
        &TopNavProps::default()
            .with_logo("/", logo_text, "")
            .with_links(links)
            .with_class(class),
        html! {},
    )
}

#[must_use]
pub fn SimpleTopNavLink(label: &str, href: &str, is_active: bool) -> Markup {
    TopNavLink(
        &TopNavLinkProps::default()
            .with_label(label)
            .with_href(href)
            .with_active(is_active),
        html! {},
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn helpers_cover_link_nav_classes_external_attrs_and_defaults() {
        assert_eq!(
            TopNavLinkProps::default().build_classes(),
            "inline-flex items-center gap-1 px-3 py-2 text-sm font-medium transition-colors duration-150 rounded-md text-muted-foreground hover:text-foreground hover:bg-accent/50"
        );
        assert_eq!(
            TopNavLinkProps::default()
                .with_active(true)
                .with_class("active")
                .build_classes(),
            "inline-flex items-center gap-1 px-3 py-2 text-sm font-medium transition-colors duration-150 rounded-md bg-accent text-accent-foreground active"
        );
        let attrs = TopNavLinkProps::default()
            .with_external(true)
            .with_attribute("id", "docs")
            .build_attributes();
        assert_eq!(attrs.get("target").map(String::as_str), Some("_blank"));
        assert_eq!(
            attrs.get("rel").map(String::as_str),
            Some("noopener noreferrer")
        );
        assert_eq!(attrs.get("id").map(String::as_str), Some("docs"));
        assert_eq!(TopNavProps::default().logo_href, "/");
        assert_eq!(
            TopNavProps::default().with_class("shadow").build_classes(),
            "sticky top-0 z-40 w-full border-b border-border bg-background/95 backdrop-blur supports-[backdrop-filter]:bg-background/60 shadow"
        );
    }

    #[test]
    fn top_nav_link_renders_label_child_external_icon_and_escaping() {
        let external = TopNavLink(
            &TopNavLinkProps::default()
                .with_href("https://example.com")
                .with_label("<Docs>")
                .with_external(true)
                .with_active(true)
                .with_attribute("data-testid", "docs-link"),
            html! { "Ignored" },
        )
        .into_string();
        assert!(external.starts_with("<a href=\"https://example.com\""));
        assert!(external.contains("aria-current=\"page\""));
        assert!(external.contains("target=\"_blank\""));
        assert!(external.contains("rel=\"noopener noreferrer\""));
        assert!(external.contains("data-testid=\"docs-link\""));
        assert!(external.contains("&lt;Docs&gt;"));
        assert!(!external.contains("Ignored"));
        assert!(external.contains("<svg "));
        assert!(external.contains("<line x1=\"10\" y1=\"14\" x2=\"21\" y2=\"3\"></line>"));

        let child = TopNavLink(
            &TopNavLinkProps::default().with_href("/custom"),
            html! { strong { "Child" } },
        )
        .into_string();
        assert!(child.contains("<strong>Child</strong>"));
        assert!(!child.contains("target=\"_blank\""));
    }

    #[test]
    fn top_nav_renders_logo_links_search_children_and_attrs() {
        let links = vec![
            TopNavLinkProps::default()
                .with_href("/features")
                .with_label("Features"),
            TopNavLinkProps::default()
                .with_href("/pricing")
                .with_label("Pricing")
                .with_active(true),
        ];
        let html = TopNav(
            &TopNavProps::default()
                .with_logo("/", "<OG>", "/logo.svg")
                .with_links(links)
                .with_search(true)
                .with_attribute("id", "site-header"),
            html! { button type="button" { "Account" } },
        )
        .into_string();
        assert!(html.starts_with("<header class=\"sticky top-0 z-40 w-full border-b border-border bg-background/95 backdrop-blur supports-[backdrop-filter]:bg-background/60\""));
        assert!(html.contains("id=\"site-header\""));
        assert!(html.contains("<div class=\"container flex h-14 items-center\">"));
        assert!(html.contains("<a href=\"/\" class=\"mr-6 flex items-center space-x-2\"><img src=\"/logo.svg\" alt=\"&lt;OG&gt;\" class=\"h-6 w-auto\"><span class=\"font-bold\">&lt;OG&gt;</span></a>"));
        assert!(html.contains("<nav class=\"flex items-center gap-1 text-sm\">"));
        assert!(html.contains(">Features</a>"));
        assert!(html.contains("aria-current=\"page\">Pricing</a>"));
        assert!(html.contains("<span class=\"hidden lg:inline-flex\">Search...</span>"));
        assert!(html.contains("<button type=\"button\">Account</button>"));
    }

    #[test]
    fn simple_top_nav_variants_render_expected_html() {
        let simple = SimpleTopNav(
            "Brand",
            vec![TopNavLinkProps::default().with_href("/").with_label("Home")],
            "custom",
        )
        .into_string();
        assert!(simple.contains("custom"));
        assert!(simple.contains("<span class=\"font-bold\">Brand</span>"));
        assert!(simple.contains(">Home</a>"));

        let link = SimpleTopNavLink("Home", "/", true).into_string();
        assert!(link.contains("href=\"/\""));
        assert!(link.contains("aria-current=\"page\""));
        assert!(link.contains(">Home</a>"));
    }
}
