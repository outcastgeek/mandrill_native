#![allow(non_snake_case)]

use crate::components::{ComponentInfo, shared};
use maud::{Markup, html};
use std::collections::BTreeMap;

pub const COMPONENTS: &[ComponentInfo] = &[
    ComponentInfo {
        family: "unstyled",
        module: "command",
        name: "Command",
    },
    ComponentInfo {
        family: "unstyled",
        module: "command",
        name: "CommandInput",
    },
    ComponentInfo {
        family: "unstyled",
        module: "command",
        name: "CommandList",
    },
    ComponentInfo {
        family: "unstyled",
        module: "command",
        name: "CommandGroup",
    },
    ComponentInfo {
        family: "unstyled",
        module: "command",
        name: "CommandItem",
    },
    ComponentInfo {
        family: "unstyled",
        module: "command",
        name: "CommandItemWithIcon",
    },
];

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CommandProps {
    pub open: bool,
    pub class: String,
    pub attributes: BTreeMap<String, String>,
}

impl CommandProps {
    #[must_use]
    pub const fn with_open(mut self, value: bool) -> Self {
        self.open = value;
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
            "flex h-full w-full flex-col overflow-hidden rounded-md bg-popover text-popover-foreground",
            self.class.as_str(),
        ])
    }
    #[must_use]
    pub fn build_attributes(&self) -> BTreeMap<String, String> {
        let mut attrs = self.attributes.clone();
        attrs.insert("role".to_string(), "dialog".to_string());
        attrs
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CommandInputProps {
    pub placeholder: String,
    pub class: String,
    pub attributes: BTreeMap<String, String>,
}

impl CommandInputProps {
    #[must_use]
    pub fn with_placeholder(mut self, value: impl Into<String>) -> Self {
        self.placeholder = value.into();
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
            "flex h-11 w-full rounded-md bg-transparent py-3 text-sm outline-none placeholder:text-muted-foreground disabled:cursor-not-allowed disabled:opacity-50",
            self.class.as_str(),
        ])
    }
    #[must_use]
    pub fn build_attributes(&self) -> BTreeMap<String, String> {
        self.attributes.clone()
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CommandListProps {
    pub class: String,
    pub attributes: BTreeMap<String, String>,
}

impl CommandListProps {
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
            "max-h-[300px] overflow-y-auto overflow-x-hidden",
            self.class.as_str(),
        ])
    }
    #[must_use]
    pub fn build_attributes(&self) -> BTreeMap<String, String> {
        let mut attrs = self.attributes.clone();
        attrs.insert("role".to_string(), "listbox".to_string());
        attrs
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CommandGroupProps {
    pub heading: String,
    pub class: String,
    pub attributes: BTreeMap<String, String>,
}

impl CommandGroupProps {
    #[must_use]
    pub fn with_heading(mut self, value: impl Into<String>) -> Self {
        self.heading = value.into();
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
        shared::build_classes(&["overflow-hidden p-1 text-foreground", self.class.as_str()])
    }
    #[must_use]
    pub fn build_attributes(&self) -> BTreeMap<String, String> {
        let mut attrs = self.attributes.clone();
        attrs.insert("role".to_string(), "group".to_string());
        attrs
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CommandItemProps {
    pub value: String,
    pub disabled: bool,
    pub shortcut: String,
    pub class: String,
    pub attributes: BTreeMap<String, String>,
}

impl CommandItemProps {
    #[must_use]
    pub fn with_value(mut self, value: impl Into<String>) -> Self {
        self.value = value.into();
        self
    }
    #[must_use]
    pub const fn with_disabled(mut self, value: bool) -> Self {
        self.disabled = value;
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
    pub fn build_classes(&self) -> String {
        shared::build_classes(&[
            "relative flex cursor-default select-none items-center rounded-sm px-2 py-1.5 text-sm outline-none aria-selected:bg-accent aria-selected:text-accent-foreground",
            if self.disabled {
                "pointer-events-none opacity-50"
            } else {
                ""
            },
            self.class.as_str(),
        ])
    }
    #[must_use]
    pub fn build_attributes(&self) -> BTreeMap<String, String> {
        let mut attrs = self.attributes.clone();
        attrs.insert("role".to_string(), "option".to_string());
        if !self.value.is_empty() {
            attrs.insert("data-value".to_string(), self.value.clone());
        }
        if self.disabled {
            attrs.insert("aria-disabled".to_string(), "true".to_string());
        }
        attrs
    }
}

fn attr<'a>(attrs: &'a BTreeMap<String, String>, name: &str) -> Option<&'a str> {
    shared::attr_value(attrs, name)
}

#[must_use]
pub const fn resolveCommandAriaLabel() -> &'static str {
    "Command menu"
}

#[must_use]
pub fn resolveCommandPlaceholder(props: &CommandInputProps) -> String {
    if props.placeholder.is_empty() {
        "Type a command or search...".to_string()
    } else {
        props.placeholder.clone()
    }
}

#[must_use]
pub fn Command(props: &CommandProps, children: Markup) -> Markup {
    let attrs = props.build_attributes();
    html! {
        @if props.open {
            div class="fixed inset-0 z-50 bg-foreground/80" {
                div class="fixed left-[50%] top-[50%] z-50 grid w-full max-w-lg translate-x-[-50%] translate-y-[-50%] gap-4 border bg-background shadow-lg sm:rounded-lg" {
                    div
                        class=(props.build_classes())
                        aria-label=(resolveCommandAriaLabel())
                        role=[attr(&attrs, "role")]
                        data-testid=[attr(&attrs, "data-testid")]
                        data-on:keydown=[attr(&attrs, "data-on:keydown")]
                    {
                        (children)
                    }
                }
            }
        }
    }
}

#[must_use]
pub fn CommandInput(props: &CommandInputProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        div class="flex items-center border-b px-3" {
            svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" class="mr-2 h-4 w-4 shrink-0 opacity-50" {
                circle cx="11" cy="11" r="8" {}
                path d="m21 21-4.3-4.3" {}
            }
            input
                type="text"
                class=(props.build_classes())
                placeholder=(resolveCommandPlaceholder(props))
                data-testid=[attr(&attrs, "data-testid")]
                data-bind=[attr(&attrs, "data-bind")]
                data-on:input=[attr(&attrs, "data-on:input")]
                aria-label=[attr(&attrs, "aria-label")]
            {}
        }
    }
}

#[must_use]
pub fn CommandList(props: &CommandListProps, children: Markup) -> Markup {
    let attrs = props.build_attributes();
    html! {
        div
            class=(props.build_classes())
            role=[attr(&attrs, "role")]
            data-testid=[attr(&attrs, "data-testid")]
        {
            (children)
        }
    }
}

#[must_use]
pub fn CommandGroup(props: &CommandGroupProps, children: Markup) -> Markup {
    let attrs = props.build_attributes();
    html! {
        div
            class=(props.build_classes())
            role=[attr(&attrs, "role")]
            data-testid=[attr(&attrs, "data-testid")]
        {
            @if !props.heading.is_empty() {
                div class="px-2 py-1.5 text-xs font-medium text-muted-foreground" {
                    (&props.heading)
                }
            }
            (children)
        }
    }
}

#[must_use]
pub fn CommandItem(props: &CommandItemProps, children: Markup) -> Markup {
    render_command_item(props, None, children)
}

#[must_use]
pub fn CommandItemWithIcon(
    props: &CommandItemProps,
    icon: Option<Markup>,
    children: Markup,
) -> Markup {
    render_command_item(props, icon, children)
}

fn render_command_item(props: &CommandItemProps, icon: Option<Markup>, children: Markup) -> Markup {
    let attrs = props.build_attributes();
    html! {
        div
            class=(props.build_classes())
            role=[attr(&attrs, "role")]
            data-value=[attr(&attrs, "data-value")]
            aria-disabled=[attr(&attrs, "aria-disabled")]
            data-testid=[attr(&attrs, "data-testid")]
            data-on:click=[attr(&attrs, "data-on:click")]
        {
            @if let Some(icon_markup) = icon {
                span class="mr-2 h-4 w-4" { (icon_markup) }
            }
            span { (children) }
            @if !props.shortcut.is_empty() {
                span class="ml-auto text-xs tracking-widest text-muted-foreground" {
                    (&props.shortcut)
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_helpers_cover_defaults_attrs_roles_classes_and_branches() {
        let command = CommandProps::default()
            .with_open(true)
            .with_class("extra")
            .with_attribute("data-testid", "cmd");
        assert!(
            command
                .build_classes()
                .contains("bg-popover text-popover-foreground extra")
        );
        let attrs = command.build_attributes();
        assert_eq!(attr(&attrs, "role"), Some("dialog"));
        assert_eq!(attr(&attrs, "data-testid"), Some("cmd"));
        assert_eq!(resolveCommandAriaLabel(), "Command menu");

        let input = CommandInputProps::default()
            .with_placeholder("Search")
            .with_class("input")
            .with_attribute("data-bind", "query");
        assert!(
            input
                .build_classes()
                .contains("placeholder:text-muted-foreground")
        );
        assert!(input.build_classes().contains(" input"));
        assert_eq!(resolveCommandPlaceholder(&input), "Search");
        assert_eq!(
            resolveCommandPlaceholder(&CommandInputProps::default()),
            "Type a command or search..."
        );
        assert_eq!(
            input
                .build_attributes()
                .get("data-bind")
                .map(String::as_str),
            Some("query")
        );

        let list = CommandListProps::default()
            .with_class("list")
            .with_attribute("data-testid", "list");
        assert!(list.build_classes().contains("max-h-[300px]"));
        assert_eq!(attr(&list.build_attributes(), "role"), Some("listbox"));
        assert_eq!(attr(&list.build_attributes(), "data-testid"), Some("list"));

        let group = CommandGroupProps::default()
            .with_heading("Actions")
            .with_class("group")
            .with_attribute("data-testid", "group");
        assert!(group.build_classes().contains("group"));
        assert_eq!(attr(&group.build_attributes(), "role"), Some("group"));
        assert_eq!(
            attr(&group.build_attributes(), "data-testid"),
            Some("group")
        );

        let item = CommandItemProps::default()
            .with_value("open")
            .with_disabled(true)
            .with_shortcut("⌘O")
            .with_class("item")
            .with_attribute("data-on:click", "$open()");
        assert!(
            item.build_classes()
                .contains("pointer-events-none opacity-50 item")
        );
        let item_attrs = item.build_attributes();
        assert_eq!(attr(&item_attrs, "role"), Some("option"));
        assert_eq!(attr(&item_attrs, "data-value"), Some("open"));
        assert_eq!(attr(&item_attrs, "aria-disabled"), Some("true"));
        assert_eq!(attr(&item_attrs, "data-on:click"), Some("$open()"));
    }

    #[test]
    fn command_renders_only_when_open_and_escapes_children() {
        let closed = Command(&CommandProps::default(), html! { p { "Hidden" } }).into_string();
        assert_eq!(closed, "");

        let open = Command(
            &CommandProps::default()
                .with_open(true)
                .with_attribute("data-testid", "cmd"),
            html! { p { "<Body>" } },
        )
        .into_string();
        assert!(open.starts_with("<div class=\"fixed inset-0 z-50 bg-foreground/80\">"));
        assert!(open.contains("aria-label=\"Command menu\""));
        assert!(open.contains("role=\"dialog\""));
        assert!(open.contains("data-testid=\"cmd\""));
        assert!(open.contains("<p>&lt;Body&gt;</p>"));
    }

    #[test]
    fn command_parts_render_input_list_group_items_icons_shortcuts_and_attrs() {
        let input = CommandInput(
            &CommandInputProps::default()
                .with_placeholder("<Search>")
                .with_attribute("data-bind", "query")
                .with_attribute("data-on:input", "$query = evt.target.value"),
        )
        .into_string();
        assert!(input.starts_with("<div class=\"flex items-center border-b px-3\">"));
        assert!(input.contains("<circle cx=\"11\" cy=\"11\" r=\"8\"></circle>"));
        assert!(input.contains("placeholder=\"&lt;Search&gt;\""));
        assert!(input.contains("data-bind=\"query\""));
        assert!(input.contains("data-on:input=\"$query = evt.target.value\""));

        let list = CommandList(&CommandListProps::default(), html! { p { "Item" } }).into_string();
        assert!(list.contains("role=\"listbox\""));
        assert!(list.contains("<p>Item</p>"));

        let group = CommandGroup(
            &CommandGroupProps::default().with_heading("<Actions>"),
            html! { p { "Child" } },
        )
        .into_string();
        assert!(group.contains("role=\"group\""));
        assert!(group.contains("&lt;Actions&gt;"));
        assert!(group.contains("<p>Child</p>"));

        let item = CommandItem(
            &CommandItemProps::default()
                .with_value("<open>")
                .with_shortcut("⌘O")
                .with_attribute("data-testid", "item"),
            html! { "Open" },
        )
        .into_string();
        assert!(item.contains("role=\"option\""));
        assert!(item.contains("data-value=\"&lt;open&gt;\""));
        assert!(item.contains("data-testid=\"item\""));
        assert!(item.contains("<span>Open</span>"));
        assert!(item.contains(
            "<span class=\"ml-auto text-xs tracking-widest text-muted-foreground\">⌘O</span>"
        ));

        let item_with_icon = CommandItemWithIcon(
            &CommandItemProps::default().with_disabled(true),
            Some(html! { svg data-icon="plus" {} }),
            html! { "Create" },
        )
        .into_string();
        assert!(item_with_icon.contains("aria-disabled=\"true\""));
        assert!(
            item_with_icon
                .contains("<span class=\"mr-2 h-4 w-4\"><svg data-icon=\"plus\"></svg></span>")
        );
        assert!(item_with_icon.contains("<span>Create</span>"));
    }
}
