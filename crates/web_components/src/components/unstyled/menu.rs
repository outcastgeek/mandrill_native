#![allow(non_snake_case)]

use crate::components::{ComponentInfo, shared};
use maud::{Markup, html};
use std::collections::BTreeMap;

pub const COMPONENTS: &[ComponentInfo] = &[
    ComponentInfo {
        family: "unstyled",
        module: "menu",
        name: "Menu",
    },
    ComponentInfo {
        family: "unstyled",
        module: "menu",
        name: "MenuDisclosure",
    },
    ComponentInfo {
        family: "unstyled",
        module: "menu",
        name: "MenuSummary",
    },
    ComponentInfo {
        family: "unstyled",
        module: "menu",
        name: "MenuContent",
    },
    ComponentInfo {
        family: "unstyled",
        module: "menu",
        name: "MenuItem",
    },
    ComponentInfo {
        family: "unstyled",
        module: "menu",
        name: "MenuSeparator",
    },
    ComponentInfo {
        family: "unstyled",
        module: "menu",
        name: "MenuGroup",
    },
    ComponentInfo {
        family: "unstyled",
        module: "menu",
        name: "MenuLabel",
    },
    ComponentInfo {
        family: "unstyled",
        module: "menu",
        name: "MenuTrigger",
    },
    ComponentInfo {
        family: "unstyled",
        module: "menu",
        name: "SimpleMenu",
    },
    ComponentInfo {
        family: "unstyled",
        module: "menu",
        name: "SimpleMenuContent",
    },
    ComponentInfo {
        family: "unstyled",
        module: "menu",
        name: "SimpleMenuItem",
    },
    ComponentInfo {
        family: "unstyled",
        module: "menu",
        name: "MenuItemWithIcon",
    },
    ComponentInfo {
        family: "unstyled",
        module: "menu",
        name: "DestructiveMenuItem",
    },
    ComponentInfo {
        family: "unstyled",
        module: "menu",
        name: "SimpleSeparator",
    },
    ComponentInfo {
        family: "unstyled",
        module: "menu",
        name: "SimpleMenuGroup",
    },
];

#[derive(Clone, Debug, Default)]
pub struct MenuProps {
    pub trigger: Option<Markup>,
    pub class: String,
    pub attributes: BTreeMap<String, String>,
    pub data_attributes: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct MenuDisclosureProps {
    pub class: String,
    pub attributes: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MenuSummaryProps {
    pub variant: &'static str,
    pub size: &'static str,
    pub icon_only: bool,
    pub class: String,
    pub attributes: BTreeMap<String, String>,
    pub aria_label: String,
}

impl Default for MenuSummaryProps {
    fn default() -> Self {
        Self {
            variant: shared::VARIANT_GHOST,
            size: shared::SIZE_SM,
            icon_only: false,
            class: String::new(),
            attributes: BTreeMap::new(),
            aria_label: String::new(),
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct MenuContentProps {
    pub class: String,
    pub attributes: BTreeMap<String, String>,
    pub aria_label: String,
}

#[derive(Clone, Debug, Default)]
pub struct MenuItemProps {
    pub href: String,
    pub disabled: bool,
    pub destructive: bool,
    pub icon: Option<Markup>,
    pub shortcut: String,
    pub class: String,
    pub attributes: BTreeMap<String, String>,
    pub aria_label: String,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct MenuSeparatorProps {
    pub class: String,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct MenuGroupProps {
    pub label: String,
    pub class: String,
    pub attributes: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct MenuLabelProps {
    pub class: String,
    pub attributes: BTreeMap<String, String>,
}

impl MenuProps {
    #[must_use]
    pub fn with_trigger(mut self, trigger: Markup) -> Self {
        self.trigger = Some(trigger);
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
    pub fn with_data_attribute(
        mut self,
        name: impl Into<String>,
        value: impl Into<String>,
    ) -> Self {
        self.data_attributes.insert(name.into(), value.into());
        self
    }
    #[must_use]
    pub fn build_classes(&self) -> String {
        shared::build_classes(&["relative inline-block", self.class.as_str()])
    }
    #[must_use]
    pub fn build_attributes(&self) -> BTreeMap<String, String> {
        let mut attrs = self.attributes.clone();
        for (key, value) in &self.data_attributes {
            attrs.insert(format!("data-{key}"), value.clone());
        }
        attrs
    }
}

impl MenuDisclosureProps {
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
        shared::build_classes(&["group relative inline-block", self.class.as_str()])
    }
    #[must_use]
    pub fn build_attributes(&self) -> BTreeMap<String, String> {
        let mut attrs = self.attributes.clone();
        attrs
            .entry("data-on:click__outside".to_string())
            .or_insert_with(|| "el.open = false".to_string());
        attrs
            .entry("data-on:keydown".to_string())
            .or_insert_with(|| {
                "if (evt.key === 'Escape') { evt.preventDefault(); el.open = false }".to_string()
            });
        attrs
    }
}

impl MenuSummaryProps {
    #[must_use]
    pub const fn with_variant(mut self, value: &'static str) -> Self {
        self.variant = value;
        self
    }
    #[must_use]
    pub const fn with_size(mut self, value: &'static str) -> Self {
        self.size = value;
        self
    }
    #[must_use]
    pub const fn with_icon_only(mut self, value: bool) -> Self {
        self.icon_only = value;
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
        let variant = if self.variant.is_empty() {
            shared::VARIANT_GHOST
        } else {
            self.variant
        };
        let size = if self.size.is_empty() {
            shared::SIZE_SM
        } else {
            self.size
        };
        shared::build_classes(&[
            "inline-flex items-center justify-center gap-2 whitespace-nowrap font-medium transition-all duration-150 ease-out cursor-pointer select-none focus-visible:outline-2 focus-visible:outline-ring focus-visible:outline-offset-2 list-none [&::-webkit-details-marker]:hidden",
            shared::button_variant_classes(variant),
            shared::button_size_classes(size),
            if self.icon_only {
                "aspect-square p-0"
            } else {
                ""
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
        attrs.insert("role".to_string(), "button".to_string());
        attrs.insert("aria-haspopup".to_string(), "menu".to_string());
        attrs
    }
}

impl MenuContentProps {
    #[must_use]
    pub fn with_class(mut self, value: impl Into<String>) -> Self {
        self.class = value.into();
        self
    }
    #[must_use]
    pub fn with_aria_label(mut self, value: impl Into<String>) -> Self {
        self.aria_label = value.into();
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
            "absolute z-50 min-w-[12rem] overflow-hidden rounded-md border bg-popover p-1 text-popover-foreground shadow-md hidden data-[state=open]:block group-open:block",
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
}

impl MenuItemProps {
    #[must_use]
    pub fn with_href(mut self, value: impl Into<String>) -> Self {
        self.href = value.into();
        self
    }
    #[must_use]
    pub const fn with_disabled(mut self, value: bool) -> Self {
        self.disabled = value;
        self
    }
    #[must_use]
    pub const fn with_destructive(mut self, value: bool) -> Self {
        self.destructive = value;
        self
    }
    #[must_use]
    pub fn with_icon(mut self, icon: Markup) -> Self {
        self.icon = Some(icon);
        self
    }
    #[must_use]
    pub fn with_shortcut(mut self, value: impl Into<String>) -> Self {
        self.shortcut = value.into();
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
        let state = if self.disabled {
            "pointer-events-none opacity-50"
        } else if self.destructive {
            "text-destructive hover:bg-destructive hover:text-destructive-foreground"
        } else {
            "hover:bg-accent hover:text-accent-foreground"
        };
        shared::build_classes(&[
            "relative flex w-full cursor-pointer select-none items-center gap-2 rounded-sm px-2 py-1.5 text-sm outline-none transition-colors duration-150 focus-visible:bg-accent focus-visible:text-accent-foreground",
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
        if self.disabled {
            attrs.insert("aria-disabled".to_string(), "true".to_string());
        }
        attrs
    }
}

impl MenuSeparatorProps {
    #[must_use]
    pub fn with_class(mut self, value: impl Into<String>) -> Self {
        self.class = value.into();
        self
    }
    #[must_use]
    pub fn build_classes(&self) -> String {
        shared::build_classes(&["-mx-1 my-1 h-px bg-muted", self.class.as_str()])
    }
}

impl MenuGroupProps {
    #[must_use]
    pub fn with_label(mut self, value: impl Into<String>) -> Self {
        self.label = value.into();
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
        shared::build_classes(&["py-1", self.class.as_str()])
    }
}

impl MenuLabelProps {
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
            "px-2 py-1.5 text-sm font-semibold text-muted-foreground",
            self.class.as_str(),
        ])
    }
}

#[must_use]
pub fn Menu(props: &MenuProps, children: Markup) -> Markup {
    let attrs = props.build_attributes();
    html! {
        div
            class=(props.build_classes())
            data-component="menu"
            id=[shared::attr_value(&attrs, "id")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
            data-track=[shared::attr_value(&attrs, "data-track")]
        {
            @if let Some(trigger) = &props.trigger {
                div data-menu-trigger="" {
                    (trigger.clone())
                }
            }
            (children)
        }
    }
}

#[must_use]
pub fn MenuDisclosure(props: &MenuDisclosureProps, children: Markup) -> Markup {
    let attrs = props.build_attributes();
    html! {
        details
            class=(props.build_classes())
            data-component="menu-disclosure"
            data-on:click__outside=[shared::attr_value(&attrs, "data-on:click__outside")]
            data-on:keydown=[shared::attr_value(&attrs, "data-on:keydown")]
            id=[shared::attr_value(&attrs, "id")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
        {
            (children)
        }
    }
}

#[must_use]
pub fn MenuSummary(props: &MenuSummaryProps, children: Markup) -> Markup {
    let attrs = props.build_attributes();
    html! {
        summary
            class=(props.build_classes())
            role=[shared::attr_value(&attrs, "role")]
            aria-haspopup=[shared::attr_value(&attrs, "aria-haspopup")]
            aria-label=[shared::attr_value(&attrs, "aria-label")]
            id=[shared::attr_value(&attrs, "id")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
        {
            (children)
        }
    }
}

#[must_use]
pub fn MenuContent(props: &MenuContentProps, children: Markup) -> Markup {
    let attrs = props.build_attributes();
    html! {
        div
            role="menu"
            class=(props.build_classes())
            data-state="closed"
            aria-label=[shared::attr_value(&attrs, "aria-label")]
            id=[shared::attr_value(&attrs, "id")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
        {
            (children)
        }
    }
}

fn menu_item_inner(props: &MenuItemProps, children: Markup) -> Markup {
    html! {
        @if let Some(icon) = &props.icon {
            span class="menu-item-icon" { (icon.clone()) }
        }
        span class="menu-item-content min-w-0 flex-1" { (children) }
        @if !props.shortcut.is_empty() {
            span class="menu-item-shortcut ml-auto text-xs tracking-widest text-muted-foreground" {
                (&props.shortcut)
            }
        }
    }
}

#[must_use]
pub fn MenuItem(props: &MenuItemProps, children: Markup) -> Markup {
    let attrs = props.build_attributes();
    if !props.href.is_empty() && !props.disabled {
        html! {
            a
                href=(&props.href)
                role="menuitem"
                class=(props.build_classes())
                aria-label=[shared::attr_value(&attrs, "aria-label")]
                id=[shared::attr_value(&attrs, "id")]
                data-testid=[shared::attr_value(&attrs, "data-testid")]
            {
                (menu_item_inner(props, children))
            }
        }
    } else {
        html! {
            div
                role="menuitem"
                class=(props.build_classes())
                aria-label=[shared::attr_value(&attrs, "aria-label")]
                aria-disabled=[shared::attr_value(&attrs, "aria-disabled")]
                id=[shared::attr_value(&attrs, "id")]
                data-testid=[shared::attr_value(&attrs, "data-testid")]
            {
                (menu_item_inner(props, children))
            }
        }
    }
}

#[must_use]
pub fn MenuSeparator(props: &MenuSeparatorProps) -> Markup {
    html! {
        div role="separator" class=(props.build_classes()) aria-orientation="horizontal" {}
    }
}

#[must_use]
pub fn MenuGroup(props: &MenuGroupProps, children: Markup) -> Markup {
    let attrs = props.attributes.clone();
    html! {
        div
            role="group"
            class=(props.build_classes())
            id=[shared::attr_value(&attrs, "id")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
        {
            @if !props.label.is_empty() {
                (MenuLabel(&MenuLabelProps::default(), html! { (&props.label) }))
            }
            (children)
        }
    }
}

#[must_use]
pub fn MenuLabel(props: &MenuLabelProps, children: Markup) -> Markup {
    let attrs = props.attributes.clone();
    html! {
        div
            class=(props.build_classes())
            id=[shared::attr_value(&attrs, "id")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
        {
            (children)
        }
    }
}

#[must_use]
pub fn MenuTrigger(class: &str, children: Markup) -> Markup {
    html! {
        button
            type="button"
            class=(format!("inline-flex items-center justify-center {class}"))
            data-menu-trigger=""
            aria-haspopup="true"
            aria-expanded="false"
        {
            (children)
        }
    }
}

#[must_use]
pub fn SimpleMenu(trigger: Markup, class: &str, children: Markup) -> Markup {
    Menu(
        &MenuProps::default().with_trigger(trigger).with_class(class),
        children,
    )
}
#[must_use]
pub fn SimpleMenuContent(aria_label: &str, class: &str, children: Markup) -> Markup {
    MenuContent(
        &MenuContentProps::default()
            .with_aria_label(aria_label)
            .with_class(class),
        children,
    )
}
#[must_use]
pub fn SimpleMenuItem(href: &str, class: &str, children: Markup) -> Markup {
    MenuItem(
        &MenuItemProps::default().with_href(href).with_class(class),
        children,
    )
}
#[must_use]
pub fn MenuItemWithIcon(
    href: &str,
    icon: Markup,
    shortcut: &str,
    class: &str,
    children: Markup,
) -> Markup {
    MenuItem(
        &MenuItemProps::default()
            .with_href(href)
            .with_icon(icon)
            .with_shortcut(shortcut)
            .with_class(class),
        children,
    )
}
#[must_use]
pub fn DestructiveMenuItem(icon: Markup, class: &str, children: Markup) -> Markup {
    MenuItem(
        &MenuItemProps::default()
            .with_destructive(true)
            .with_icon(icon)
            .with_class(class),
        children,
    )
}
#[must_use]
pub fn SimpleSeparator() -> Markup {
    MenuSeparator(&MenuSeparatorProps::default())
}
#[must_use]
pub fn SimpleMenuGroup(label: &str, class: &str, children: Markup) -> Markup {
    MenuGroup(
        &MenuGroupProps::default()
            .with_label(label)
            .with_class(class),
        children,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn helpers_cover_classes_attributes_and_defaults() {
        assert_eq!(
            MenuProps::default().build_classes(),
            "relative inline-block"
        );
        assert_eq!(
            MenuProps::default().with_class("menu").build_classes(),
            "relative inline-block menu"
        );
        let menu_attrs = MenuProps::default()
            .with_attribute("id", "menu")
            .with_data_attribute("track", "account")
            .build_attributes();
        assert_eq!(menu_attrs.get("id").map(String::as_str), Some("menu"));
        assert_eq!(
            menu_attrs.get("data-track").map(String::as_str),
            Some("account")
        );
        let disclosure_attrs = MenuDisclosureProps::default().build_attributes();
        assert_eq!(
            disclosure_attrs
                .get("data-on:click__outside")
                .map(String::as_str),
            Some("el.open = false")
        );
        assert_eq!(
            disclosure_attrs.get("data-on:keydown").map(String::as_str),
            Some("if (evt.key === 'Escape') { evt.preventDefault(); el.open = false }")
        );
        assert!(
            MenuSummaryProps::default()
                .build_classes()
                .contains("list-none [&::-webkit-details-marker]:hidden")
        );
        assert!(
            MenuSummaryProps::default()
                .with_icon_only(true)
                .build_classes()
                .contains("aspect-square p-0")
        );
        let summary_attrs = MenuSummaryProps::default()
            .with_aria_label("Open")
            .build_attributes();
        assert_eq!(
            summary_attrs.get("role").map(String::as_str),
            Some("button")
        );
        assert_eq!(
            summary_attrs.get("aria-haspopup").map(String::as_str),
            Some("menu")
        );
        assert_eq!(
            summary_attrs.get("aria-label").map(String::as_str),
            Some("Open")
        );
        assert!(
            MenuContentProps::default()
                .with_class("right-0")
                .build_classes()
                .contains("right-0")
        );
        assert_eq!(
            MenuSeparatorProps::default()
                .with_class("sep")
                .build_classes(),
            "-mx-1 my-1 h-px bg-muted sep"
        );
        assert_eq!(
            MenuGroupProps::default()
                .with_class("group")
                .build_classes(),
            "py-1 group"
        );
        assert_eq!(
            MenuLabelProps::default()
                .with_class("label")
                .build_classes(),
            "px-2 py-1.5 text-sm font-semibold text-muted-foreground label"
        );
    }

    #[test]
    fn menu_item_helpers_cover_link_disabled_destructive_attrs() {
        assert!(
            MenuItemProps::default()
                .build_classes()
                .contains("hover:bg-accent hover:text-accent-foreground")
        );
        assert!(
            MenuItemProps::default()
                .with_disabled(true)
                .build_classes()
                .contains("pointer-events-none opacity-50")
        );
        assert!(
            MenuItemProps::default()
                .with_destructive(true)
                .build_classes()
                .contains("text-destructive hover:bg-destructive")
        );
        let attrs = MenuItemProps::default()
            .with_disabled(true)
            .with_aria_label("Delete")
            .with_attribute("id", "delete")
            .build_attributes();
        assert_eq!(attrs.get("aria-disabled").map(String::as_str), Some("true"));
        assert_eq!(attrs.get("aria-label").map(String::as_str), Some("Delete"));
        assert_eq!(attrs.get("id").map(String::as_str), Some("delete"));
    }

    #[test]
    fn menu_renders_disclosure_summary_content_items_group_and_labels() {
        let trigger = MenuTrigger("trigger", html! { "Open" });
        let rendered = Menu(
            &MenuProps::default().with_trigger(trigger).with_data_attribute("track", "account"),
            html! {
                (MenuDisclosure(&MenuDisclosureProps::default(), html! {
                    (MenuSummary(&MenuSummaryProps::default().with_aria_label("Menu"), html! { "Summary" }))
                    (MenuContent(&MenuContentProps::default().with_aria_label("Actions"), html! {
                        (MenuGroup(&MenuGroupProps::default().with_label("<Group>"), html! {
                            (MenuItem(
                                &MenuItemProps::default()
                                    .with_href("/settings")
                                    .with_icon(html! { svg data-testid="settings-icon" {} })
                                    .with_shortcut("⌘S"),
                                html! { "<Settings>" },
                            ))
                            (MenuItem(&MenuItemProps::default().with_disabled(true), html! { "Disabled" }))
                            (MenuSeparator(&MenuSeparatorProps::default()))
                            (MenuItem(&MenuItemProps::default().with_destructive(true), html! { "Delete" }))
                        }))
                    }))
                }))
            },
        )
        .into_string();
        assert!(
            rendered.starts_with("<div class=\"relative inline-block\" data-component=\"menu\"")
        );
        assert!(rendered.contains("data-track=\"account\""));
        assert!(rendered.contains("<div data-menu-trigger=\"\"><button type=\"button\" class=\"inline-flex items-center justify-center trigger\" data-menu-trigger=\"\" aria-haspopup=\"true\" aria-expanded=\"false\">Open</button></div>"));
        assert!(rendered.contains("<details class=\"group relative inline-block\" data-component=\"menu-disclosure\" data-on:click__outside=\"el.open = false\""));
        assert!(rendered.contains("<summary class=\"inline-flex items-center justify-center"));
        assert!(rendered.contains("role=\"button\" aria-haspopup=\"menu\" aria-label=\"Menu\""));
        assert!(rendered.contains("<div role=\"menu\" class=\"absolute z-50"));
        assert!(rendered.contains("data-state=\"closed\" aria-label=\"Actions\""));
        assert!(rendered.contains("<div role=\"group\" class=\"py-1\"><div class=\"px-2 py-1.5 text-sm font-semibold text-muted-foreground\">&lt;Group&gt;</div>"));
        assert!(rendered.contains("<a href=\"/settings\" role=\"menuitem\""));
        assert!(rendered.contains(
            "<span class=\"menu-item-icon\"><svg data-testid=\"settings-icon\"></svg></span>"
        ));
        assert!(
            rendered.contains(
                "<span class=\"menu-item-content min-w-0 flex-1\">&lt;Settings&gt;</span>"
            )
        );
        assert!(rendered.contains("<span class=\"menu-item-shortcut ml-auto text-xs tracking-widest text-muted-foreground\">⌘S</span>"));
        assert!(rendered.contains("role=\"menuitem\" class=\"relative flex w-full cursor-pointer select-none items-center gap-2 rounded-sm px-2 py-1.5 text-sm outline-none transition-colors duration-150 focus-visible:bg-accent focus-visible:text-accent-foreground pointer-events-none opacity-50\" aria-disabled=\"true\""));
        assert!(rendered.contains("<div role=\"separator\" class=\"-mx-1 my-1 h-px bg-muted\" aria-orientation=\"horizontal\"></div>"));
        assert!(
            rendered.contains(
                "text-destructive hover:bg-destructive hover:text-destructive-foreground"
            )
        );
    }

    #[test]
    fn convenience_components_render_expected_html() {
        let simple = SimpleMenu(html! { button { "T" } }, "menu", html! { "Body" }).into_string();
        assert!(simple.contains("relative inline-block menu"));
        assert!(simple.contains("<button>T</button>"));
        assert!(simple.contains("Body"));
        let content = SimpleMenuContent("Menu", "right", html! { "Items" }).into_string();
        assert!(content.contains("aria-label=\"Menu\""));
        assert!(content.contains("right"));
        let item = SimpleMenuItem("/profile", "item", html! { "Profile" }).into_string();
        assert!(item.contains("href=\"/profile\""));
        assert!(item.contains("item\""));
        let with_icon =
            MenuItemWithIcon("/x", html! { svg {} }, "X", "", html! { "Xray" }).into_string();
        assert!(with_icon.contains("menu-item-icon"));
        assert!(with_icon.contains("menu-item-shortcut"));
        let destructive =
            DestructiveMenuItem(html! { svg {} }, "danger", html! { "Delete" }).into_string();
        assert!(destructive.contains("text-destructive"));
        assert!(destructive.contains("danger"));
        assert_eq!(
            SimpleSeparator().into_string(),
            "<div role=\"separator\" class=\"-mx-1 my-1 h-px bg-muted\" aria-orientation=\"horizontal\"></div>"
        );
        let group = SimpleMenuGroup("Group", "g", html! { "One" }).into_string();
        assert!(group.contains("role=\"group\""));
        assert!(group.contains(">Group</div>"));
        assert!(group.contains("One"));
    }

    #[test]
    fn remaining_builder_methods_render_preserved_attributes() {
        let disclosure = MenuDisclosure(
            &MenuDisclosureProps::default()
                .with_class("disclosure-extra")
                .with_attribute("id", "disclosure")
                .with_attribute("data-on:keydown", "customEscape()"),
            html! { "Panel" },
        )
        .into_string();
        assert!(disclosure.contains("group relative inline-block disclosure-extra"));
        assert!(disclosure.contains(r#"id="disclosure""#));
        assert!(disclosure.contains(r#"data-on:keydown="customEscape()""#));
        assert!(disclosure.contains(">Panel</details>"));

        let summary = MenuSummary(
            &MenuSummaryProps::default()
                .with_variant("")
                .with_size("")
                .with_icon_only(true)
                .with_class("summary-extra")
                .with_aria_label("Open menu")
                .with_attribute("id", "summary"),
            html! { "Summary" },
        )
        .into_string();
        assert!(summary.contains("hover:bg-muted"));
        assert!(summary.contains("h-9 px-3 text-sm"));
        assert!(summary.contains("aspect-square p-0"));
        assert!(summary.contains("summary-extra"));
        assert!(summary.contains(r#"aria-label="Open menu" id="summary""#));

        let content = MenuContent(
            &MenuContentProps::default()
                .with_aria_label("Actions")
                .with_class("content-extra")
                .with_attribute("id", "content"),
            html! { "Actions" },
        )
        .into_string();
        assert!(content.contains("content-extra"));
        assert!(content.contains(r#"aria-label="Actions" id="content""#));

        let group = MenuGroup(
            &MenuGroupProps::default()
                .with_label("Group")
                .with_class("group-extra")
                .with_attribute("id", "group"),
            html! { "Item" },
        )
        .into_string();
        assert!(group.contains(r#"role="group" class="py-1 group-extra" id="group""#));
        assert!(group.contains(">Group</div>"));
        assert!(group.contains("Item"));

        let label = MenuLabel(
            &MenuLabelProps::default()
                .with_class("label-extra")
                .with_attribute("data-testid", "label"),
            html! { "<Label>" },
        )
        .into_string();
        assert!(label.contains("label-extra"));
        assert!(label.contains(r#"data-testid="label""#));
        assert!(label.contains("&lt;Label&gt;"));
    }
}
