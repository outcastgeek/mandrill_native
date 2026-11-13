#![allow(non_snake_case)]

use crate::components::{ComponentInfo, shared};
use maud::{Markup, html};
use std::collections::BTreeMap;

pub const DRAWER_POSITION_LEFT: &str = "left";
pub const DRAWER_POSITION_RIGHT: &str = "right";

pub const COMPONENTS: &[ComponentInfo] = &[
    ComponentInfo {
        family: "unstyled",
        module: "drawer",
        name: "Drawer",
    },
    ComponentInfo {
        family: "unstyled",
        module: "drawer",
        name: "DrawerCloseButton",
    },
    ComponentInfo {
        family: "unstyled",
        module: "drawer",
        name: "CloseIcon",
    },
    ComponentInfo {
        family: "unstyled",
        module: "drawer",
        name: "DrawerTrigger",
    },
    ComponentInfo {
        family: "unstyled",
        module: "drawer",
        name: "HamburgerIcon",
    },
    ComponentInfo {
        family: "unstyled",
        module: "drawer",
        name: "SimpleTrigger",
    },
];

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DrawerProps {
    pub open: bool,
    pub signal_name: String,
    pub title: String,
    pub position: &'static str,
    pub panel_width: String,
    pub show_close_button: bool,
    pub show_header: bool,
    pub class: String,
    pub panel_class: String,
    pub backdrop_class: String,
    pub content_class: String,
    pub attributes: BTreeMap<String, String>,
    pub after_close_action: String,
}

impl Default for DrawerProps {
    fn default() -> Self {
        Self {
            open: false,
            signal_name: String::new(),
            title: String::new(),
            position: DRAWER_POSITION_LEFT,
            panel_width: "w-64".to_string(),
            show_close_button: true,
            show_header: true,
            class: String::new(),
            panel_class: "bg-card shadow-xl".to_string(),
            backdrop_class: "bg-background/80 backdrop-blur-sm".to_string(),
            content_class: "flex-1 min-h-0 p-2 overflow-y-auto".to_string(),
            attributes: BTreeMap::new(),
            after_close_action: String::new(),
        }
    }
}

impl DrawerProps {
    #[must_use]
    pub const fn with_open(mut self, value: bool) -> Self {
        self.open = value;
        self
    }
    #[must_use]
    pub fn with_signal_name(mut self, value: impl Into<String>) -> Self {
        self.signal_name = value.into();
        self
    }
    #[must_use]
    pub fn with_title(mut self, value: impl Into<String>) -> Self {
        self.title = value.into();
        self
    }
    #[must_use]
    pub const fn with_position(mut self, value: &'static str) -> Self {
        self.position = value;
        self
    }
    #[must_use]
    pub fn with_panel_width(mut self, value: impl Into<String>) -> Self {
        self.panel_width = value.into();
        self
    }
    #[must_use]
    pub const fn with_show_close_button(mut self, value: bool) -> Self {
        self.show_close_button = value;
        self
    }
    #[must_use]
    pub const fn with_show_header(mut self, value: bool) -> Self {
        self.show_header = value;
        self
    }
    #[must_use]
    pub fn with_class(mut self, value: impl Into<String>) -> Self {
        self.class = value.into();
        self
    }
    #[must_use]
    pub fn with_panel_class(mut self, value: impl Into<String>) -> Self {
        self.panel_class = value.into();
        self
    }
    #[must_use]
    pub fn with_backdrop_class(mut self, value: impl Into<String>) -> Self {
        self.backdrop_class = value.into();
        self
    }
    #[must_use]
    pub fn with_content_class(mut self, value: impl Into<String>) -> Self {
        self.content_class = value.into();
        self
    }
    #[must_use]
    pub fn with_after_close_action(mut self, value: impl Into<String>) -> Self {
        self.after_close_action = value.into();
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
            "fixed inset-0 z-50",
            if self.signal_name.is_empty() && !self.open {
                "hidden"
            } else {
                ""
            },
            self.class.as_str(),
        ])
    }

    #[must_use]
    pub fn is_signal_based(&self) -> bool {
        !self.signal_name.is_empty()
    }

    #[must_use]
    pub fn signal_expression(&self) -> String {
        if self.signal_name.is_empty() {
            String::new()
        } else {
            format!("${}", self.signal_name)
        }
    }

    #[must_use]
    pub fn close_signal_action(&self) -> String {
        if self.signal_name.is_empty() {
            return String::new();
        }
        let base = format!("${} = false", self.signal_name);
        let extra = self.after_close_action.trim();
        if extra.is_empty() {
            base
        } else {
            format!("{base}; {extra}")
        }
    }

    #[must_use]
    pub fn panel_position_class(&self) -> &'static str {
        if self.position == DRAWER_POSITION_RIGHT {
            "right-0"
        } else {
            "left-0"
        }
    }

    #[must_use]
    pub fn panel_animation_class(&self) -> &'static str {
        if self.position == DRAWER_POSITION_RIGHT {
            "animate-drawer-in-right"
        } else {
            "animate-drawer-in-left"
        }
    }

    #[must_use]
    pub fn build_panel_classes(&self) -> String {
        shared::build_classes(&[
            "absolute top-0 bottom-0 flex flex-col",
            self.panel_width.as_str(),
            self.panel_class.as_str(),
            self.panel_position_class(),
            self.panel_animation_class(),
        ])
    }

    #[must_use]
    pub fn build_backdrop_classes(&self) -> String {
        shared::build_classes(&[
            "absolute inset-0 animate-overlay-in",
            self.backdrop_class.as_str(),
        ])
    }

    #[must_use]
    pub fn build_content_classes(&self) -> String {
        self.content_class.clone()
    }

    #[must_use]
    pub fn build_attributes(&self) -> BTreeMap<String, String> {
        let mut attrs = self.attributes.clone();
        attrs.insert("role".to_string(), "dialog".to_string());
        attrs.insert("aria-modal".to_string(), "true".to_string());
        attrs.insert("aria-label".to_string(), self.title.clone());
        if self.is_signal_based() {
            if self.open {
                attrs.insert("aria-hidden".to_string(), "false".to_string());
            } else {
                let existing = attrs.get("style").cloned();
                attrs.insert("style".to_string(), appendDisplayNone(existing.as_deref()));
                attrs.insert("aria-hidden".to_string(), "true".to_string());
            }
            attrs.insert(
                "data-on:keydown__window".to_string(),
                format!(
                    "if (evt.key === 'Escape') {{ {} }}",
                    self.close_signal_action()
                ),
            );
        } else if self.open {
            attrs.insert("aria-hidden".to_string(), "false".to_string());
        } else {
            attrs.insert("aria-hidden".to_string(), "true".to_string());
        }
        attrs
    }
}

#[must_use]
pub fn appendDisplayNone(existing: Option<&str>) -> String {
    match existing {
        None | Some("") => "display:none".to_string(),
        Some(value) if value.ends_with(';') => format!("{value}display:none"),
        Some(value) => format!("{value}; display:none"),
    }
}

#[must_use]
pub fn drawerTitle(props: &DrawerProps) -> String {
    if props.title.is_empty() {
        "Menu".to_string()
    } else {
        props.title.clone()
    }
}

#[must_use]
pub fn closeButtonAttributes(props: &DrawerProps) -> BTreeMap<String, String> {
    let mut attrs = BTreeMap::from([
        ("aria-label".to_string(), "Close menu".to_string()),
        ("data-drawer-close".to_string(), "true".to_string()),
    ]);
    if props.is_signal_based() {
        attrs.insert("data-on:click".to_string(), props.close_signal_action());
    }
    attrs
}

#[must_use]
pub fn triggerButtonAttributes(aria_label: &str, signal_name: &str) -> BTreeMap<String, String> {
    let mut attrs = BTreeMap::from([
        (
            "aria-label".to_string(),
            resolveDrawerTriggerAriaLabel(aria_label),
        ),
        ("data-drawer-trigger".to_string(), "true".to_string()),
    ]);
    if !signal_name.is_empty() {
        attrs.insert(
            "data-on:click".to_string(),
            format!("${signal_name} = !${signal_name}"),
        );
    }
    attrs
}

#[must_use]
pub fn buildTriggerClasses(custom_class: &str) -> String {
    shared::build_classes(&["p-2", custom_class])
}

#[must_use]
pub fn resolveDrawerTriggerAriaLabel(aria_label: &str) -> String {
    if aria_label.is_empty() {
        "Toggle menu".to_string()
    } else {
        aria_label.to_string()
    }
}

fn attr<'a>(attrs: &'a BTreeMap<String, String>, name: &str) -> Option<&'a str> {
    shared::attr_value(attrs, name)
}

#[must_use]
pub fn Drawer(props: &DrawerProps, children: Markup) -> Markup {
    let attrs = props.build_attributes();
    html! {
        div
            class=(props.build_classes())
            data-show=[if props.is_signal_based() { Some(props.signal_expression()) } else { None }]
            role=[attr(&attrs, "role")]
            aria-modal=[attr(&attrs, "aria-modal")]
            aria-label=[attr(&attrs, "aria-label")]
            aria-hidden=[attr(&attrs, "aria-hidden")]
            style=[attr(&attrs, "style")]
            data-on:keydown__window=[attr(&attrs, "data-on:keydown__window")]
            data-testid=[attr(&attrs, "data-testid")]
            data-signals=[shared::attr_value(&attrs, "data-signals")]
            data-on:load=[shared::attr_value(&attrs, "data-on:load")]
        {
            div
                class=(props.build_backdrop_classes())
                data-on:click=[if props.is_signal_based() { Some(props.close_signal_action()) } else { None }]
                data-drawer-backdrop="true"
                aria-hidden="true"
            {}
            div class=(props.build_panel_classes()) {
                @if props.show_header {
                    div class="p-4 border-b border-border flex items-center justify-between" {
                        h2 class="text-lg font-bold text-foreground" { (drawerTitle(props)) }
                        @if props.show_close_button {
                            (DrawerCloseButton(props))
                        }
                    }
                }
                div class=(props.build_content_classes()) {
                    (children)
                }
            }
        }
    }
}

#[must_use]
pub fn DrawerCloseButton(props: &DrawerProps) -> Markup {
    let attrs = closeButtonAttributes(props);
    html! {
        button
            type="button"
            class=(shared::build_classes(&[
                "inline-flex items-center justify-center gap-2 whitespace-nowrap font-medium transition-all duration-150 ease-out cursor-pointer select-none focus-visible:outline-2 focus-visible:outline-ring focus-visible:outline-offset-2 disabled:pointer-events-none disabled:opacity-50 active:scale-[0.98]",
                shared::button_variant_classes(shared::VARIANT_GHOST),
                shared::button_size_classes(shared::SIZE_SM),
                "p-1.5",
            ]))
            aria-label=[attr(&attrs, "aria-label")]
            data-drawer-close=[attr(&attrs, "data-drawer-close")]
            data-on:click=[attr(&attrs, "data-on:click")]
        {
            (CloseIcon())
        }
    }
}

#[must_use]
pub fn CloseIcon() -> Markup {
    html! {
        svg class="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24" {
            path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M6 18L18 6M6 6l12 12" {}
        }
    }
}

#[must_use]
pub fn DrawerTrigger(aria_label: &str, signal_name: &str, class: &str) -> Markup {
    let attrs = triggerButtonAttributes(aria_label, signal_name);
    html! {
        button
            type="button"
            class=(shared::build_classes(&[
                "inline-flex items-center justify-center gap-2 whitespace-nowrap font-medium transition-all duration-150 ease-out cursor-pointer select-none focus-visible:outline-2 focus-visible:outline-ring focus-visible:outline-offset-2 disabled:pointer-events-none disabled:opacity-50 active:scale-[0.98]",
                shared::button_variant_classes(shared::VARIANT_GHOST),
                shared::button_size_classes(shared::SIZE_SM),
                buildTriggerClasses(class).as_str(),
            ]))
            aria-label=[attr(&attrs, "aria-label")]
            data-drawer-trigger=[attr(&attrs, "data-drawer-trigger")]
            data-on:click=[attr(&attrs, "data-on:click")]
        {
            (HamburgerIcon())
        }
    }
}

#[must_use]
pub fn HamburgerIcon() -> Markup {
    html! {
        svg class="w-6 h-6" fill="none" stroke="currentColor" viewBox="0 0 24 24" {
            path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M4 6h16M4 12h16M4 18h16" {}
        }
    }
}

#[must_use]
pub fn SimpleTrigger(signal_name: &str) -> Markup {
    DrawerTrigger("", signal_name, "")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drawer_props_helpers_cover_defaults_positions_signals_attrs_and_styles() {
        let props = DrawerProps::default();
        assert_eq!(props.position, DRAWER_POSITION_LEFT);
        assert_eq!(props.panel_width, "w-64");
        assert!(props.show_close_button);
        assert!(props.show_header);
        assert_eq!(props.build_classes(), "fixed inset-0 z-50 hidden");
        assert!(!props.is_signal_based());
        assert_eq!(props.signal_expression(), "");
        assert_eq!(props.close_signal_action(), "");
        assert_eq!(props.panel_position_class(), "left-0");
        assert_eq!(props.panel_animation_class(), "animate-drawer-in-left");
        assert!(props.build_panel_classes().contains("absolute top-0 bottom-0 flex flex-col w-64 bg-card shadow-xl left-0 animate-drawer-in-left"));
        assert!(
            props
                .build_backdrop_classes()
                .contains("absolute inset-0 animate-overlay-in bg-background/80 backdrop-blur-sm")
        );
        assert_eq!(
            props.build_content_classes(),
            "flex-1 min-h-0 p-2 overflow-y-auto"
        );
        assert_eq!(appendDisplayNone(None), "display:none");
        assert_eq!(appendDisplayNone(Some("")), "display:none");
        assert_eq!(
            appendDisplayNone(Some("color:red")),
            "color:red; display:none"
        );
        assert_eq!(
            appendDisplayNone(Some("color:red;")),
            "color:red;display:none"
        );

        let open = DrawerProps::default()
            .with_open(true)
            .with_signal_name("menuOpen")
            .with_title("Menu")
            .with_position(DRAWER_POSITION_RIGHT)
            .with_panel_width("w-80")
            .with_show_close_button(false)
            .with_show_header(false)
            .with_class("root")
            .with_panel_class("panel")
            .with_backdrop_class("backdrop")
            .with_content_class("content")
            .with_after_close_action("@get('/closed')")
            .with_attribute("data-testid", "drawer");
        assert_eq!(open.build_classes(), "fixed inset-0 z-50 root");
        assert!(open.is_signal_based());
        assert_eq!(open.signal_expression(), "$menuOpen");
        assert_eq!(
            open.close_signal_action(),
            "$menuOpen = false; @get('/closed')"
        );
        assert_eq!(open.panel_position_class(), "right-0");
        assert_eq!(open.panel_animation_class(), "animate-drawer-in-right");
        assert!(
            open.build_panel_classes()
                .contains("w-80 panel right-0 animate-drawer-in-right")
        );
        assert_eq!(
            open.build_backdrop_classes(),
            "absolute inset-0 animate-overlay-in backdrop"
        );
        assert_eq!(open.build_content_classes(), "content");
        let attrs = open.build_attributes();
        assert_eq!(attr(&attrs, "role"), Some("dialog"));
        assert_eq!(attr(&attrs, "aria-modal"), Some("true"));
        assert_eq!(attr(&attrs, "aria-label"), Some("Menu"));
        assert_eq!(attr(&attrs, "aria-hidden"), Some("false"));
        assert_eq!(attr(&attrs, "data-testid"), Some("drawer"));
        assert!(
            attr(&attrs, "data-on:keydown__window").is_some_and(|value| value.contains("Escape"))
        );

        let hidden_signal = DrawerProps::default()
            .with_signal_name("open")
            .with_attribute("style", "color:red");
        let hidden_attrs = hidden_signal.build_attributes();
        assert_eq!(
            attr(&hidden_attrs, "style"),
            Some("color:red; display:none")
        );
        assert_eq!(attr(&hidden_attrs, "aria-hidden"), Some("true"));
    }

    #[test]
    fn trigger_and_close_helpers_render_expected_attributes_and_icons() {
        assert_eq!(drawerTitle(&DrawerProps::default()), "Menu");
        assert_eq!(
            drawerTitle(&DrawerProps::default().with_title("Nav")),
            "Nav"
        );
        assert_eq!(resolveDrawerTriggerAriaLabel(""), "Toggle menu");
        assert_eq!(resolveDrawerTriggerAriaLabel("Open"), "Open");
        assert_eq!(buildTriggerClasses("extra"), "p-2 extra");
        let trigger_attrs = triggerButtonAttributes("Open menu", "menuOpen");
        assert_eq!(attr(&trigger_attrs, "aria-label"), Some("Open menu"));
        assert_eq!(attr(&trigger_attrs, "data-drawer-trigger"), Some("true"));
        assert_eq!(
            attr(&trigger_attrs, "data-on:click"),
            Some("$menuOpen = !$menuOpen")
        );
        let close_attrs =
            closeButtonAttributes(&DrawerProps::default().with_signal_name("menuOpen"));
        assert_eq!(attr(&close_attrs, "aria-label"), Some("Close menu"));
        assert_eq!(attr(&close_attrs, "data-drawer-close"), Some("true"));
        assert_eq!(
            attr(&close_attrs, "data-on:click"),
            Some("$menuOpen = false")
        );

        let close = CloseIcon().into_string();
        assert!(close.contains("class=\"w-5 h-5\""));
        assert!(close.contains("M6 18L18 6M6 6l12 12"));
        let hamburger = HamburgerIcon().into_string();
        assert!(hamburger.contains("class=\"w-6 h-6\""));
        assert!(hamburger.contains("M4 6h16M4 12h16M4 18h16"));

        let trigger = DrawerTrigger("Open", "menuOpen", "extra").into_string();
        assert!(trigger.contains("aria-label=\"Open\""));
        assert!(trigger.contains("data-drawer-trigger=\"true\""));
        assert!(trigger.contains("data-on:click=\"$menuOpen = !$menuOpen\""));
        assert!(trigger.contains("p-2 extra"));
        assert!(
            SimpleTrigger("menuOpen")
                .into_string()
                .contains("aria-label=\"Toggle menu\"")
        );
    }

    #[test]
    fn drawer_renders_static_and_signal_variations_with_real_html() {
        let hidden = Drawer(
            &DrawerProps::default().with_title("Static"),
            html! { nav { "Links" } },
        )
        .into_string();
        assert!(hidden.starts_with("<div class=\"fixed inset-0 z-50 hidden\""));
        assert!(hidden.contains("role=\"dialog\""));
        assert!(hidden.contains("aria-modal=\"true\""));
        assert!(hidden.contains("aria-label=\"Static\""));
        assert!(hidden.contains("aria-hidden=\"true\""));
        assert!(hidden.contains("data-drawer-backdrop=\"true\""));
        assert!(hidden.contains("<h2 class=\"text-lg font-bold text-foreground\">Static</h2>"));
        assert!(hidden.contains("data-drawer-close=\"true\""));
        assert!(hidden.contains("<nav>Links</nav>"));

        let signal = Drawer(
            &DrawerProps::default()
                .with_open(false)
                .with_signal_name("drawerOpen")
                .with_title("<Menu>")
                .with_position(DRAWER_POSITION_RIGHT)
                .with_panel_width("w-80")
                .with_after_close_action("@post('/close')")
                .with_attribute("data-testid", "drawer"),
            html! { p { "Body" } },
        )
        .into_string();
        assert!(signal.contains("data-show=\"$drawerOpen\""));
        assert!(signal.contains("style=\"display:none\""));
        assert!(signal.contains("data-testid=\"drawer\""));
        assert!(signal.contains("data-on:keydown__window="));
        assert!(signal.contains("Escape"));
        assert!(signal.contains("data-on:click="));
        assert!(signal.contains("$drawerOpen = false; @post"));
        assert!(signal.contains("aria-label=\"&lt;Menu&gt;\""));
        assert!(signal.contains("w-80 bg-card shadow-xl right-0 animate-drawer-in-right"));

        let no_header =
            Drawer(&DrawerProps::default().with_show_header(false), html! {}).into_string();
        assert!(!no_header.contains("<h2"));
        let no_close = Drawer(
            &DrawerProps::default()
                .with_open(true)
                .with_show_close_button(false),
            html! {},
        )
        .into_string();
        assert!(!no_close.contains("data-drawer-close"));
        assert!(no_close.contains("aria-hidden=\"false\""));
    }
}
