#![allow(non_snake_case)]

use crate::components::{ComponentInfo, shared};
use maud::{Markup, html};
use std::collections::BTreeMap;

pub const COMPONENTS: &[ComponentInfo] = &[
    ComponentInfo {
        family: "unstyled",
        module: "sidebar",
        name: "SidebarItem",
    },
    ComponentInfo {
        family: "unstyled",
        module: "sidebar",
        name: "Sidebar",
    },
    ComponentInfo {
        family: "unstyled",
        module: "sidebar",
        name: "SimpleSidebar",
    },
    ComponentInfo {
        family: "unstyled",
        module: "sidebar",
        name: "SimpleSidebarItem",
    },
    ComponentInfo {
        family: "unstyled",
        module: "sidebar",
        name: "SidebarGroupHeader",
    },
    ComponentInfo {
        family: "unstyled",
        module: "sidebar",
        name: "SidebarCollapseToggle",
    },
];

#[derive(Clone, Debug, Default)]
pub struct SidebarItemProps {
    pub href: String,
    pub label: String,
    pub icon: String,
    pub icon_content: Option<Markup>,
    pub is_active: bool,
    pub is_expanded: bool,
    pub badge: String,
    pub badge_content: Option<Markup>,
    pub children: Vec<SidebarItemProps>,
    pub class: String,
    pub attributes: BTreeMap<String, String>,
}

impl SidebarItemProps {
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
    pub fn with_icon(mut self, value: impl Into<String>) -> Self {
        self.icon = value.into();
        self.icon_content = None;
        self
    }
    #[must_use]
    pub fn with_icon_content(mut self, value: Markup) -> Self {
        self.icon_content = Some(value);
        self.icon.clear();
        self
    }
    #[must_use]
    pub const fn with_active(mut self, value: bool) -> Self {
        self.is_active = value;
        self
    }
    #[must_use]
    pub const fn with_expanded(mut self, value: bool) -> Self {
        self.is_expanded = value;
        self
    }
    #[must_use]
    pub fn with_badge(mut self, value: impl Into<String>) -> Self {
        self.badge = value.into();
        self.badge_content = None;
        self
    }
    #[must_use]
    pub fn with_badge_content(mut self, value: Markup) -> Self {
        self.badge_content = Some(value);
        self.badge.clear();
        self
    }
    #[must_use]
    pub fn with_children(mut self, value: Vec<SidebarItemProps>) -> Self {
        self.children = value;
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
        let mut base = "relative flex items-center gap-3 rounded-lg px-3 py-2 text-sm font-medium transition-[background-color,color] duration-150 whitespace-nowrap".to_string();
        if self.is_active {
            base.push_str(" bg-sidebar-accent text-sidebar-accent-foreground border-l-[3px] border-l-sidebar-primary pl-[calc(0.75rem-3px)]");
        } else {
            base.push_str(" text-sidebar-muted-foreground hover:text-sidebar-foreground hover:bg-sidebar-accent/50");
        }
        shared::build_classes(&[base.as_str(), self.class.as_str()])
    }
    #[must_use]
    pub fn build_attributes(&self) -> BTreeMap<String, String> {
        self.attributes.clone()
    }
    #[must_use]
    pub fn has_children(&self) -> bool {
        !self.children.is_empty()
    }
}

#[derive(Clone, Debug)]
pub struct SidebarProps {
    pub items: Vec<SidebarItemProps>,
    pub header: String,
    pub footer: String,
    pub is_collapsed: bool,
    pub width: String,
    pub collapsed_width: String,
    pub signal: String,
    pub class: String,
    pub attributes: BTreeMap<String, String>,
}

impl Default for SidebarProps {
    fn default() -> Self {
        Self {
            items: Vec::new(),
            header: String::new(),
            footer: String::new(),
            is_collapsed: false,
            width: "w-64".to_string(),
            collapsed_width: "w-16".to_string(),
            signal: String::new(),
            class: String::new(),
            attributes: BTreeMap::new(),
        }
    }
}

impl SidebarProps {
    #[must_use]
    pub fn with_items(mut self, value: Vec<SidebarItemProps>) -> Self {
        self.items = value;
        self
    }
    #[must_use]
    pub fn with_header(mut self, value: impl Into<String>) -> Self {
        self.header = value.into();
        self
    }
    #[must_use]
    pub fn with_footer(mut self, value: impl Into<String>) -> Self {
        self.footer = value.into();
        self
    }
    #[must_use]
    pub const fn with_collapsed(mut self, value: bool) -> Self {
        self.is_collapsed = value;
        self
    }
    #[must_use]
    pub fn with_width(mut self, value: impl Into<String>) -> Self {
        self.width = value.into();
        self
    }
    #[must_use]
    pub fn with_collapsed_width(mut self, value: impl Into<String>) -> Self {
        self.collapsed_width = value.into();
        self
    }
    #[must_use]
    pub fn with_signal(mut self, value: impl Into<String>) -> Self {
        self.signal = value.into();
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
        let mut base = "flex flex-col border-r border-sidebar-border bg-sidebar text-sidebar-foreground h-screen overflow-hidden shrink-0 transition-[width] duration-300 ease-[cubic-bezier(0.16,1,0.3,1)] motion-reduce:transition-none".to_string();
        let collapsed_width = if self.collapsed_width.is_empty() {
            "w-16"
        } else {
            self.collapsed_width.as_str()
        };
        if self.is_collapsed {
            base.push(' ');
            base.push_str(collapsed_width);
            base.push_str(" sidebar-collapsed");
        } else {
            base.push(' ');
            base.push_str(&self.width);
        }
        shared::build_classes(&[base.as_str(), self.class.as_str()])
    }
    #[must_use]
    pub fn build_attributes(&self) -> BTreeMap<String, String> {
        let mut attrs = self.attributes.clone();
        if !self.signal.is_empty() {
            attrs.insert("data-show".to_string(), format!("${}", self.signal));
        }
        attrs
    }
}

#[must_use]
pub fn boolStr(value: bool) -> &'static str {
    if value { "true" } else { "false" }
}

#[must_use]
pub fn expandedRotate(expanded: bool) -> &'static str {
    if expanded { "rotate-90" } else { "" }
}

fn item_icon(props: &SidebarItemProps) -> Markup {
    html! {
        @if let Some(icon) = &props.icon_content {
            span class="shrink-0 text-sidebar-muted-foreground" { (icon.clone()) }
        } @else if !props.icon.is_empty() {
            span class="text-lg shrink-0" { (&props.icon) }
        }
    }
}

#[must_use]
pub fn SidebarItem(props: &SidebarItemProps) -> Markup {
    let attrs = props.build_attributes();
    if props.has_children() {
        html! {
            div class="space-y-1" {
                button
                    type="button"
                    class=(format!("{} w-full justify-between", props.build_classes()))
                    aria-expanded=(boolStr(props.is_expanded))
                    id=[shared::attr_value(&attrs, "id")]
                    data-testid=[shared::attr_value(&attrs, "data-testid")]
                {
                    span class="flex items-center gap-3" {
                        (item_icon(props))
                        span class="sidebar-label" { (&props.label) }
                    }
                    svg
                        xmlns="http://www.w3.org/2000/svg"
                        width="16"
                        height="16"
                        viewBox="0 0 24 24"
                        fill="none"
                        stroke="currentColor"
                        stroke-width="2"
                        class=(format!("transition-transform {}", expandedRotate(props.is_expanded)))
                    {
                        path d="m9 18 6-6-6-6" {}
                    }
                }
                @if props.is_expanded {
                    div class="ml-4 pl-4 border-l border-sidebar-border space-y-1" {
                        @for child in &props.children {
                            (SidebarItem(child))
                        }
                    }
                }
            }
        }
    } else {
        html! {
            a
                href=(&props.href)
                class=(format!("{} sidebar-item-tooltip", props.build_classes()))
                aria-current=[if props.is_active { Some("page") } else { None }]
                data-tooltip=(&props.label)
                id=[shared::attr_value(&attrs, "id")]
                data-testid=[shared::attr_value(&attrs, "data-testid")]
            {
                (item_icon(props))
                span class="flex-1 sidebar-label" { (&props.label) }
                @if let Some(badge) = &props.badge_content {
                    (badge.clone())
                } @else if !props.badge.is_empty() {
                    span class="min-w-[20px] h-5 px-1.5 text-xs font-medium bg-sidebar-primary text-sidebar-primary-foreground rounded-full flex items-center justify-center sidebar-badge" {
                        (&props.badge)
                    }
                }
            }
        }
    }
}

#[must_use]
pub fn Sidebar(props: &SidebarProps, children: Markup) -> Markup {
    let attrs = props.build_attributes();
    html! {
        aside
            class=(props.build_classes())
            aria-label=[shared::optional_attr(&props.header)]
            data-show=[shared::attr_value(&attrs, "data-show")]
            id=[shared::attr_value(&attrs, "id")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
        {
            @if !props.header.is_empty() {
                div class="flex h-14 items-center border-b border-sidebar-border px-4" {
                    span class="font-semibold" { (&props.header) }
                }
            }
            nav class="flex-1 overflow-y-auto p-4 space-y-1" aria-label="Navigation" {
                @for item in &props.items {
                    (SidebarItem(item))
                }
                (children)
            }
            @if !props.footer.is_empty() {
                div class="border-t border-sidebar-border p-4 text-sm text-sidebar-muted-foreground" {
                    (&props.footer)
                }
            }
        }
    }
}

#[must_use]
pub fn SimpleSidebar(header: &str, items: Vec<SidebarItemProps>, class: &str) -> Markup {
    Sidebar(
        &SidebarProps::default()
            .with_header(header)
            .with_items(items)
            .with_class(class),
        html! {},
    )
}

#[must_use]
pub fn SimpleSidebarItem(icon: &str, label: &str, href: &str, is_active: bool) -> Markup {
    SidebarItem(
        &SidebarItemProps::default()
            .with_icon(icon)
            .with_label(label)
            .with_href(href)
            .with_active(is_active),
    )
}

#[must_use]
pub fn SidebarGroupHeader(label: &str) -> Markup {
    html! {
        div class="sidebar-section-title px-2 py-1.5 text-[10px] font-semibold uppercase tracking-wider text-sidebar-foreground/50" {
            (label)
        }
    }
}

#[must_use]
pub fn SidebarCollapseToggle(aria_label: &str) -> Markup {
    html! {
        button
            type="button"
            class="sidebar-toggle cursor-pointer p-1.5 rounded-lg hover:bg-sidebar-accent text-sidebar-foreground/70 hover:text-sidebar-foreground transition-colors duration-150 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-sidebar-ring"
            data-on:click="$sidebarCollapsed = !$sidebarCollapsed"
            aria-label=(aria_label)
        {
            svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" class="transition-transform duration-200" {
                path d="m15 18-6-6 6-6" {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn helpers_cover_item_sidebar_classes_attrs_and_small_helpers() {
        assert_eq!(boolStr(true), "true");
        assert_eq!(boolStr(false), "false");
        assert_eq!(expandedRotate(true), "rotate-90");
        assert_eq!(expandedRotate(false), "");

        let item = SidebarItemProps::default()
            .with_href("/home")
            .with_label("Home")
            .with_icon("H")
            .with_icon_content(html! { svg data-testid="icon" {} })
            .with_active(true)
            .with_badge("3")
            .with_badge_content(html! { span data-testid="badge" {} })
            .with_children(vec![SidebarItemProps::default().with_label("Child")])
            .with_class("item")
            .with_attribute("data-testid", "item");
        assert_eq!(item.href, "/home");
        assert_eq!(item.label, "Home");
        assert!(item.icon.is_empty());
        assert!(item.icon_content.is_some());
        assert!(item.is_active);
        assert!(item.badge.is_empty());
        assert!(item.badge_content.is_some());
        assert!(item.has_children());
        assert!(
            item.build_classes()
                .contains("bg-sidebar-accent text-sidebar-accent-foreground")
        );
        assert!(
            SidebarItemProps::default()
                .build_classes()
                .contains("hover:bg-sidebar-accent/50")
        );
        assert_eq!(
            item.build_attributes()
                .get("data-testid")
                .map(String::as_str),
            Some("item")
        );

        let sidebar = SidebarProps::default()
            .with_items(vec![SidebarItemProps::default()])
            .with_header("Main")
            .with_footer("Footer")
            .with_collapsed(true)
            .with_width("w-72")
            .with_collapsed_width("")
            .with_signal("sidebarOpen")
            .with_class("custom")
            .with_attribute("id", "side");
        assert!(
            sidebar
                .build_classes()
                .contains("w-16 sidebar-collapsed custom")
        );
        assert_eq!(
            sidebar
                .build_attributes()
                .get("data-show")
                .map(String::as_str),
            Some("$sidebarOpen")
        );
        assert_eq!(
            sidebar.build_attributes().get("id").map(String::as_str),
            Some("side")
        );
    }

    #[test]
    fn sidebar_item_renders_leaf_active_badge_icon_and_escaping() {
        let rendered = SidebarItem(
            &SidebarItemProps::default()
                .with_href("/settings?x=<y>")
                .with_label("<Settings>")
                .with_icon("*")
                .with_active(true)
                .with_badge("<2>")
                .with_attribute("data-testid", "settings"),
        )
        .into_string();
        assert!(rendered.starts_with(
            "<a href=\"/settings?x=&lt;y&gt;\" class=\"relative flex items-center gap-3"
        ));
        assert!(rendered.contains(
            "aria-current=\"page\" data-tooltip=\"&lt;Settings&gt;\" data-testid=\"settings\""
        ));
        assert!(rendered.contains("<span class=\"text-lg shrink-0\">*</span>"));
        assert!(rendered.contains("<span class=\"flex-1 sidebar-label\">&lt;Settings&gt;</span>"));
        assert!(rendered.contains("<span class=\"min-w-[20px] h-5 px-1.5 text-xs font-medium bg-sidebar-primary text-sidebar-primary-foreground rounded-full flex items-center justify-center sidebar-badge\">&lt;2&gt;</span>"));

        let custom = SidebarItem(
            &SidebarItemProps::default()
                .with_href("/x")
                .with_label("X")
                .with_icon_content(html! { svg data-testid="custom-icon" {} })
                .with_badge_content(html! { strong { "New" } }),
        )
        .into_string();
        assert!(custom.contains("<span class=\"shrink-0 text-sidebar-muted-foreground\"><svg data-testid=\"custom-icon\"></svg></span>"));
        assert!(custom.contains("<strong>New</strong>"));
    }

    #[test]
    fn sidebar_item_renders_nested_items_only_when_expanded() {
        let collapsed = SidebarItem(
            &SidebarItemProps::default()
                .with_label("Parent")
                .with_children(vec![
                    SidebarItemProps::default()
                        .with_label("Child")
                        .with_href("/child"),
                ]),
        )
        .into_string();
        assert!(collapsed.contains("<button type=\"button\""));
        assert!(collapsed.contains("aria-expanded=\"false\""));
        assert!(!collapsed.contains("ml-4 pl-4"));

        let expanded = SidebarItem(
            &SidebarItemProps::default()
                .with_label("Parent")
                .with_expanded(true)
                .with_children(vec![
                    SidebarItemProps::default()
                        .with_label("Child")
                        .with_href("/child"),
                ]),
        )
        .into_string();
        assert!(expanded.contains("aria-expanded=\"true\""));
        assert!(expanded.contains("class=\"transition-transform rotate-90\""));
        assert!(
            expanded.contains("<div class=\"ml-4 pl-4 border-l border-sidebar-border space-y-1\">")
        );
        assert!(expanded.contains("data-tooltip=\"Child\""));
    }

    #[test]
    fn sidebar_and_convenience_components_render_expected_html() {
        let rendered = Sidebar(
            &SidebarProps::default()
                .with_header("Workspace")
                .with_footer("<Footer>")
                .with_items(vec![
                    SidebarItemProps::default()
                        .with_label("Home")
                        .with_href("/"),
                ])
                .with_signal("sidebarOpen")
                .with_attribute("data-testid", "sidebar"),
            html! { (SidebarGroupHeader("<Admin>")) },
        )
        .into_string();
        assert!(
            rendered.starts_with("<aside class=\"flex flex-col border-r border-sidebar-border")
        );
        assert!(rendered.contains(
            "aria-label=\"Workspace\" data-show=\"$sidebarOpen\" data-testid=\"sidebar\""
        ));
        assert!(rendered.contains("<span class=\"font-semibold\">Workspace</span>"));
        assert!(rendered.contains(
            "<nav class=\"flex-1 overflow-y-auto p-4 space-y-1\" aria-label=\"Navigation\">"
        ));
        assert!(rendered.contains("data-tooltip=\"Home\""));
        assert!(rendered.contains("sidebar-section-title"));
        assert!(rendered.contains("&lt;Admin&gt;"));
        assert!(rendered.contains("<div class=\"border-t border-sidebar-border p-4 text-sm text-sidebar-muted-foreground\">&lt;Footer&gt;</div>"));

        let simple_sidebar = SimpleSidebar(
            "Main",
            vec![
                SidebarItemProps::default()
                    .with_label("One")
                    .with_href("/one"),
            ],
            "plain",
        )
        .into_string();
        assert!(simple_sidebar.contains("plain\" aria-label=\"Main\""));
        assert!(simple_sidebar.contains("data-tooltip=\"One\""));

        let simple_item = SimpleSidebarItem("*", "Home", "/", true).into_string();
        assert!(simple_item.contains("aria-current=\"page\""));
        assert!(simple_item.contains("data-tooltip=\"Home\""));

        let toggle = SidebarCollapseToggle("Toggle sidebar").into_string();
        assert!(toggle.starts_with("<button type=\"button\" class=\"sidebar-toggle"));
        assert!(toggle.contains(
            "data-on:click=\"$sidebarCollapsed = !$sidebarCollapsed\" aria-label=\"Toggle sidebar\""
        ));
        assert!(toggle.contains("<path d=\"m15 18-6-6 6-6\"></path>"));
    }
}
