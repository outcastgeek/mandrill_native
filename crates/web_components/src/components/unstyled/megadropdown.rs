#![allow(non_snake_case)]

use crate::components::{ComponentInfo, shared};
use maud::{Markup, html};
use std::collections::BTreeMap;

type Attrs = BTreeMap<String, String>;

pub const COMPONENTS: &[ComponentInfo] = &[
    ComponentInfo {
        family: "unstyled",
        module: "megadropdown",
        name: "MegaDropdownNav",
    },
    ComponentInfo {
        family: "unstyled",
        module: "megadropdown",
        name: "MegaDropdownTrigger",
    },
    ComponentInfo {
        family: "unstyled",
        module: "megadropdown",
        name: "MegaDropdownPanel",
    },
    ComponentInfo {
        family: "unstyled",
        module: "megadropdown",
        name: "MegaDropdownFeatureLink",
    },
    ComponentInfo {
        family: "unstyled",
        module: "megadropdown",
        name: "MegaDropdownFeaturedCard",
    },
    ComponentInfo {
        family: "unstyled",
        module: "megadropdown",
        name: "MegaDropdownFooter",
    },
    ComponentInfo {
        family: "unstyled",
        module: "megadropdown",
        name: "MegaDropdownSection",
    },
    ComponentInfo {
        family: "unstyled",
        module: "megadropdown",
        name: "MegaDropdownTwoColumn",
    },
    ComponentInfo {
        family: "unstyled",
        module: "megadropdown",
        name: "SimpleMegaDropdown",
    },
];

fn attr<'a>(attrs: &'a Attrs, name: &str) -> Option<&'a str> {
    shared::attr_value(attrs, name)
}

fn optional(value: &str) -> Option<&str> {
    if value.is_empty() { None } else { Some(value) }
}

macro_rules! attrs_impl {
    ($name:ident) => {
        impl $name {
            #[must_use]
            pub fn with_class(mut self, value: impl Into<String>) -> Self {
                self.class = value.into();
                self
            }

            #[must_use]
            pub fn with_attribute(
                mut self,
                name: impl Into<String>,
                value: impl Into<String>,
            ) -> Self {
                self.attributes.insert(name.into(), value.into());
                self
            }

            #[must_use]
            pub fn build_attributes(&self) -> Attrs {
                self.attributes.clone()
            }
        }
    };
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MegaDropdownNavProps {
    pub signal_prefix: String,
    pub class: String,
    pub backdrop_class: String,
    pub header_class: String,
    pub backdrop_z_index: String,
    pub header_z_index: String,
    pub panel_z_index: String,
    pub attributes: Attrs,
}

impl Default for MegaDropdownNavProps {
    fn default() -> Self {
        Self {
            signal_prefix: String::new(),
            class: String::new(),
            backdrop_class: String::new(),
            header_class: String::new(),
            backdrop_z_index: "z-[9998]".to_string(),
            header_z_index: "z-[9999]".to_string(),
            panel_z_index: "z-[10000]".to_string(),
            attributes: Attrs::new(),
        }
    }
}

attrs_impl!(MegaDropdownNavProps);

impl MegaDropdownNavProps {
    #[must_use]
    pub fn with_signal_prefix(mut self, value: impl Into<String>) -> Self {
        self.signal_prefix = value.into();
        self
    }
    #[must_use]
    pub fn active_dropdown_signal(&self) -> String {
        prefixed_signal(&self.signal_prefix, "ActiveDropdown", "activeDropdown")
    }
    #[must_use]
    pub fn mobile_menu_signal(&self) -> String {
        prefixed_signal(&self.signal_prefix, "MobileMenuOpen", "mobileMenuOpen")
    }
    #[must_use]
    pub fn expanded_section_signal(&self) -> String {
        prefixed_signal(&self.signal_prefix, "ExpandedSection", "expandedSection")
    }
    #[must_use]
    pub fn build_nav_classes(&self) -> String {
        shared::build_classes(&["relative", self.class.as_str()])
    }
    #[must_use]
    pub fn build_backdrop_classes(&self) -> String {
        shared::build_classes(&[
            "fixed inset-0 bg-transparent",
            self.backdrop_z_index.as_str(),
            self.backdrop_class.as_str(),
        ])
    }
    #[must_use]
    pub fn build_header_classes(&self) -> String {
        shared::build_classes(&[
            "relative",
            self.header_z_index.as_str(),
            self.header_class.as_str(),
        ])
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MegaDropdownTriggerProps {
    pub id: String,
    pub signal_prefix: String,
    pub label: String,
    pub class: String,
    pub button_class: String,
    pub show_chevron: bool,
    pub attributes: Attrs,
}

impl Default for MegaDropdownTriggerProps {
    fn default() -> Self {
        Self {
            id: String::new(),
            signal_prefix: String::new(),
            label: String::new(),
            class: String::new(),
            button_class: String::new(),
            show_chevron: true,
            attributes: Attrs::new(),
        }
    }
}

attrs_impl!(MegaDropdownTriggerProps);

impl MegaDropdownTriggerProps {
    #[must_use]
    pub fn with_id(mut self, value: impl Into<String>) -> Self {
        self.id = value.into();
        self
    }
    #[must_use]
    pub fn with_label(mut self, value: impl Into<String>) -> Self {
        self.label = value.into();
        self
    }
    #[must_use]
    pub fn with_signal_prefix(mut self, value: impl Into<String>) -> Self {
        self.signal_prefix = value.into();
        self
    }
    #[must_use]
    pub const fn with_show_chevron(mut self, value: bool) -> Self {
        self.show_chevron = value;
        self
    }
    #[must_use]
    pub fn active_dropdown_signal(&self) -> String {
        prefixed_signal(&self.signal_prefix, "ActiveDropdown", "activeDropdown")
    }
    #[must_use]
    pub fn build_trigger_classes(&self) -> String {
        shared::build_classes(&["relative", self.class.as_str()])
    }
    #[must_use]
    pub fn build_button_classes(&self) -> String {
        shared::build_classes(&[
            "inline-flex items-center gap-1.5 rounded-md px-3 py-2 text-sm font-medium text-foreground hover:bg-muted",
            self.button_class.as_str(),
        ])
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MegaDropdownPanelProps {
    pub id: String,
    pub signal_prefix: String,
    pub class: String,
    pub content_class: String,
    pub width: String,
    pub max_height: String,
    pub top_offset: String,
    pub horizontal_style: String,
    pub z_index: String,
    pub attributes: Attrs,
}

impl Default for MegaDropdownPanelProps {
    fn default() -> Self {
        Self {
            id: String::new(),
            signal_prefix: String::new(),
            class: String::new(),
            content_class: String::new(),
            width: "w-[calc(100vw-2rem)] xl:w-[1000px]".to_string(),
            max_height: "max-h-[450px]".to_string(),
            top_offset: "top-[4.5rem]".to_string(),
            horizontal_style: "right: 1rem; left: 1rem;".to_string(),
            z_index: "z-[10000]".to_string(),
            attributes: Attrs::new(),
        }
    }
}

attrs_impl!(MegaDropdownPanelProps);

impl MegaDropdownPanelProps {
    #[must_use]
    pub fn with_id(mut self, value: impl Into<String>) -> Self {
        self.id = value.into();
        self
    }
    #[must_use]
    pub fn with_signal_prefix(mut self, value: impl Into<String>) -> Self {
        self.signal_prefix = value.into();
        self
    }
    #[must_use]
    pub fn active_dropdown_signal(&self) -> String {
        prefixed_signal(&self.signal_prefix, "ActiveDropdown", "activeDropdown")
    }
    #[must_use]
    pub fn build_panel_classes(&self) -> String {
        shared::build_classes(&[
            "fixed overflow-y-auto rounded-2xl border border-border bg-popover p-0 text-popover-foreground shadow-xl",
            self.width.as_str(),
            self.max_height.as_str(),
            self.top_offset.as_str(),
            self.z_index.as_str(),
            self.class.as_str(),
        ])
    }
    #[must_use]
    pub fn build_panel_style(&self) -> String {
        self.horizontal_style.clone()
    }
    #[must_use]
    pub fn build_content_classes(&self) -> String {
        shared::build_classes(&["overflow-hidden rounded-2xl", self.content_class.as_str()])
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Default)]
pub struct MegaDropdownFeatureLinkProps {
    pub title: String,
    pub description: String,
    pub href: String,
    pub icon: String,
    pub class: String,
    pub icon_class: String,
    pub attributes: Attrs,
}

attrs_impl!(MegaDropdownFeatureLinkProps);

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MegaDropdownFeaturedCardProps {
    pub title: String,
    pub subtitle: String,
    pub description: String,
    pub icon: String,
    pub href: String,
    pub gradient_class: String,
    pub class: String,
    pub width: String,
    pub attributes: Attrs,
}

impl Default for MegaDropdownFeaturedCardProps {
    fn default() -> Self {
        Self {
            title: String::new(),
            subtitle: String::new(),
            description: String::new(),
            icon: String::new(),
            href: String::new(),
            gradient_class: "from-primary to-accent".to_string(),
            class: String::new(),
            width: "w-[330px]".to_string(),
            attributes: Attrs::new(),
        }
    }
}

attrs_impl!(MegaDropdownFeaturedCardProps);

#[derive(Clone, Debug, Eq, PartialEq, Default)]
pub struct MegaDropdownFooterLink {
    pub text: String,
    pub href: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Default)]
pub struct MegaDropdownFooterProps {
    pub links: Vec<MegaDropdownFooterLink>,
    pub class: String,
    pub attributes: Attrs,
}

attrs_impl!(MegaDropdownFooterProps);

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MegaDropdownSectionProps {
    pub title: String,
    pub class: String,
    pub grid_cols: String,
    pub attributes: Attrs,
}

impl Default for MegaDropdownSectionProps {
    fn default() -> Self {
        Self {
            title: String::new(),
            class: String::new(),
            grid_cols: "grid-cols-2".to_string(),
            attributes: Attrs::new(),
        }
    }
}

attrs_impl!(MegaDropdownSectionProps);

#[must_use]
pub fn prefixed_signal(prefix: &str, suffix: &str, fallback: &str) -> String {
    if prefix.is_empty() {
        fallback.to_string()
    } else {
        format!("{prefix}{suffix}")
    }
}

#[must_use]
pub fn buildSignals(props: &MegaDropdownNavProps) -> String {
    if props.signal_prefix.is_empty() {
        "{mobileMenuOpen: false, activeDropdown: '', expandedSection: ''}".to_string()
    } else {
        format!(
            "{{{}MobileMenuOpen: false, {}ActiveDropdown: '', {}ExpandedSection: ''}}",
            props.signal_prefix, props.signal_prefix, props.signal_prefix
        )
    }
}

#[must_use]
pub fn triggerMouseEnterExpr(props: &MegaDropdownTriggerProps) -> String {
    format!("${} = '{}'", props.active_dropdown_signal(), props.id)
}

#[must_use]
pub fn triggerClickExpr(props: &MegaDropdownTriggerProps) -> String {
    format!(
        "${0} = ${0} === '{1}' ? '' : '{1}'",
        props.active_dropdown_signal(),
        props.id
    )
}

#[must_use]
pub fn triggerChevronClassExpr(props: &MegaDropdownTriggerProps) -> String {
    format!(
        "{{'rotate-180': ${} === '{}'}}",
        props.active_dropdown_signal(),
        props.id
    )
}

#[must_use]
pub fn panelShowExpr(props: &MegaDropdownPanelProps) -> String {
    format!("${} === '{}'", props.active_dropdown_signal(), props.id)
}

#[must_use]
pub fn MegaDropdownNav(props: &MegaDropdownNavProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        div class=(props.build_nav_classes()) data-signals=(buildSignals(props)) data-testid=[attr(&attrs, "data-testid")] {
            div data-show=(format!("${} !== ''", props.active_dropdown_signal())) data-on:click=(format!("${} = ''", props.active_dropdown_signal())) class=(props.build_backdrop_classes()) style="display: none;" {}
            header role="banner" class=(props.build_header_classes()) {}
        }
    }
}

#[must_use]
pub fn MegaDropdownTrigger(props: &MegaDropdownTriggerProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        div class=(props.build_trigger_classes()) data-on:mouseenter=(triggerMouseEnterExpr(props)) data-testid=[attr(&attrs, "data-testid")] {
            button class=(props.build_button_classes()) data-on:click=(triggerClickExpr(props)) {
                (&props.label)
                @if props.show_chevron {
                    svg class="w-3 h-3 transition-transform" data-class=(triggerChevronClassExpr(props)) fill="none" stroke="currentColor" viewBox="0 0 24 24" {
                        path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M19 9l-7 7-7-7" {}
                    }
                }
            }
        }
    }
}

#[must_use]
pub fn MegaDropdownPanel(props: &MegaDropdownPanelProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        div class=(props.build_panel_classes()) style=(format!("display: none; {}", props.build_panel_style())) data-show=(panelShowExpr(props)) data-testid=[attr(&attrs, "data-testid")] {
            div class=(props.build_content_classes()) {}
        }
    }
}

#[must_use]
pub fn MegaDropdownFeatureLink(props: &MegaDropdownFeatureLinkProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        a href=(&props.href) class=(shared::build_classes(&["group flex items-start gap-3 rounded-xl p-3 transition-colors hover:bg-muted/60", props.class.as_str()])) data-testid=[attr(&attrs, "data-testid")] {
            div class=(shared::build_classes(&["flex h-10 w-10 shrink-0 items-center justify-center rounded-xl bg-muted text-foreground", props.icon_class.as_str()])) {
                span class="text-lg leading-none" { (&props.icon) }
            }
            div class="min-w-0" {
                div class="text-sm font-medium text-foreground leading-tight group-hover:text-primary transition-colors duration-150" { (&props.title) }
                div class="text-xs text-muted-foreground mt-0.5 leading-relaxed line-clamp-1" { (&props.description) }
            }
        }
    }
}

#[must_use]
pub fn MegaDropdownFeaturedCard(props: &MegaDropdownFeaturedCardProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        div class=(shared::build_classes(&[props.width.as_str(), props.class.as_str()])) data-testid=[attr(&attrs, "data-testid")] {
            a href=(&props.href) class="block" {
                div class="bg-muted rounded-xl overflow-hidden flex flex-col hover:shadow-lg transition-shadow" {
                    div class="p-4" {
                        div class="flex items-center justify-between mb-2" {
                            div class="flex items-center gap-2" {
                                span class=(shared::build_classes(&["w-8 h-8 rounded-full bg-gradient-to-br flex items-center justify-center text-primary-foreground text-sm font-bold", props.gradient_class.as_str()])) { (&props.icon) }
                                span class="font-medium text-foreground" { (&props.title) }
                            }
                            div class="w-7 h-7 rounded-full bg-muted hover:bg-muted/80 flex items-center justify-center transition-colors duration-150" { "→" }
                        }
                        h4 class="text-base font-medium text-foreground" { (&props.subtitle) }
                        p class="text-sm text-muted-foreground mt-1" { (&props.description) }
                    }
                    div class=(shared::build_classes(&["h-[140px] relative bg-gradient-to-br", props.gradient_class.as_str()])) {
                        div class="absolute inset-0 flex items-center justify-center" { span class="text-primary-foreground/30 text-5xl" { "⚡" } }
                    }
                }
            }
        }
    }
}

#[must_use]
pub fn MegaDropdownFooter(props: &MegaDropdownFooterProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        div class=(shared::build_classes(&["flex items-center gap-2 border-t border-border bg-primary px-4 py-3", props.class.as_str()])) data-testid=[attr(&attrs, "data-testid")] {
            @for (idx, link) in props.links.iter().enumerate() {
                @if idx > 0 { span class="text-muted-foreground/30" { "|" } }
                a href=(&link.href) class="text-xs text-primary-foreground hover:text-muted-foreground transition-colors duration-150" { (&link.text) }
            }
        }
    }
}

#[must_use]
pub fn MegaDropdownSection(props: &MegaDropdownSectionProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        div class=(shared::build_classes(&["grid gap-2", props.grid_cols.as_str(), props.class.as_str()])) data-testid=[attr(&attrs, "data-testid")] {
            @if !props.title.is_empty() {
                div class="col-span-full mb-2 text-xs font-semibold uppercase tracking-wider text-muted-foreground" { (&props.title) }
            }
        }
    }
}

#[must_use]
pub fn MegaDropdownTwoColumn(leftWidth: &str, rightWidth: &str) -> Markup {
    html! {
        div class="flex gap-6" {
            div class=[optional(leftWidth)] {}
            div class=[optional(rightWidth)] {}
        }
    }
}

#[must_use]
pub fn SimpleMegaDropdown(id: &str, label: &str, panelProps: &MegaDropdownPanelProps) -> Markup {
    html! {
        div {
            (MegaDropdownTrigger(&MegaDropdownTriggerProps::default().with_id(id).with_label(label).with_show_chevron(true)))
            (MegaDropdownPanel(&MegaDropdownPanelProps {
                id: id.to_string(),
                width: panelProps.width.clone(),
                max_height: panelProps.max_height.clone(),
                top_offset: panelProps.top_offset.clone(),
                horizontal_style: panelProps.horizontal_style.clone(),
                z_index: panelProps.z_index.clone(),
                class: panelProps.class.clone(),
                content_class: panelProps.content_class.clone(),
                signal_prefix: panelProps.signal_prefix.clone(),
                attributes: Attrs::new(),
            }))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_contains(html: &str, expected: &str) {
        assert!(html.contains(expected), "missing `{expected}` in `{html}`");
    }

    #[test]
    fn helper_functions_build_signals_and_expressions() {
        assert_eq!(
            prefixed_signal("", "ActiveDropdown", "activeDropdown"),
            "activeDropdown"
        );
        assert_eq!(
            prefixed_signal("demo", "ActiveDropdown", "activeDropdown"),
            "demoActiveDropdown"
        );
        let nav = MegaDropdownNavProps::default().with_signal_prefix("demo");
        assert_eq!(
            buildSignals(&MegaDropdownNavProps::default()),
            "{mobileMenuOpen: false, activeDropdown: '', expandedSection: ''}"
        );
        assert_eq!(
            buildSignals(&nav),
            "{demoMobileMenuOpen: false, demoActiveDropdown: '', demoExpandedSection: ''}"
        );
        assert_eq!(nav.mobile_menu_signal(), "demoMobileMenuOpen");
        assert_eq!(nav.expanded_section_signal(), "demoExpandedSection");
        let trigger = MegaDropdownTriggerProps::default()
            .with_id("products")
            .with_signal_prefix("demo")
            .with_class("trigger-extra")
            .with_attribute("data-testid", "trigger");
        assert_eq!(trigger.class, "trigger-extra");
        assert_eq!(
            trigger.attributes.get("data-testid").map(String::as_str),
            Some("trigger")
        );
        assert_eq!(
            triggerMouseEnterExpr(&trigger),
            "$demoActiveDropdown = 'products'"
        );
        assert_eq!(
            triggerClickExpr(&trigger),
            "$demoActiveDropdown = $demoActiveDropdown === 'products' ? '' : 'products'"
        );
        assert_eq!(
            triggerChevronClassExpr(&trigger),
            "{'rotate-180': $demoActiveDropdown === 'products'}"
        );
        let panel = MegaDropdownPanelProps::default()
            .with_id("products")
            .with_signal_prefix("demo");
        assert_eq!(panelShowExpr(&panel), "$demoActiveDropdown === 'products'");
    }

    #[test]
    fn nav_trigger_and_panel_render_datastar_attributes() {
        let nav = MegaDropdownNav(
            &MegaDropdownNavProps::default()
                .with_signal_prefix("demo")
                .with_attribute("data-testid", "nav"),
        )
        .into_string();
        assert_contains(
            &nav,
            r#"data-signals="{demoMobileMenuOpen: false, demoActiveDropdown: '', demoExpandedSection: ''}""#,
        );
        assert_contains(&nav, r#"data-show="$demoActiveDropdown !== ''""#);
        assert_contains(&nav, r#"data-on:click="$demoActiveDropdown = ''""#);
        assert_contains(&nav, r#"role="banner""#);
        assert_contains(&nav, r#"data-testid="nav""#);

        let trigger = MegaDropdownTrigger(
            &MegaDropdownTriggerProps::default()
                .with_id("products")
                .with_label("Products")
                .with_signal_prefix("demo"),
        )
        .into_string();
        assert_contains(
            &trigger,
            r#"data-on:mouseenter="$demoActiveDropdown = 'products'""#,
        );
        assert_contains(
            &trigger,
            r#"data-on:click="$demoActiveDropdown = $demoActiveDropdown === 'products' ? '' : 'products'""#,
        );
        assert_contains(
            &trigger,
            r#"data-class="{'rotate-180': $demoActiveDropdown === 'products'}""#,
        );
        assert_contains(&trigger, "Products");

        let panel = MegaDropdownPanel(
            &MegaDropdownPanelProps::default()
                .with_id("products")
                .with_signal_prefix("demo"),
        )
        .into_string();
        assert_contains(&panel, r#"data-show="$demoActiveDropdown === 'products'""#);
        assert_contains(&panel, "display: none;");
        assert_contains(&panel, "w-[calc(100vw-2rem)]");
    }

    #[test]
    fn feature_card_footer_section_and_layout_render() {
        let link = MegaDropdownFeatureLink(&MegaDropdownFeatureLinkProps {
            title: "Analytics".to_string(),
            description: "Reports".to_string(),
            href: "/analytics".to_string(),
            icon: "A".to_string(),
            ..Default::default()
        })
        .into_string();
        assert_contains(&link, r#"href="/analytics""#);
        assert_contains(&link, "Analytics");
        assert_contains(&link, "Reports");

        let card = MegaDropdownFeaturedCard(&MegaDropdownFeaturedCardProps {
            title: "Featured".to_string(),
            subtitle: "Launch".to_string(),
            description: "Go now".to_string(),
            icon: "F".to_string(),
            href: "/featured".to_string(),
            ..Default::default()
        })
        .into_string();
        assert_contains(&card, r#"href="/featured""#);
        assert_contains(&card, "from-primary to-accent");
        assert_contains(&card, "Launch");

        let footer = MegaDropdownFooter(&MegaDropdownFooterProps {
            links: vec![
                MegaDropdownFooterLink {
                    text: "Terms".to_string(),
                    href: "/terms".to_string(),
                },
                MegaDropdownFooterLink {
                    text: "Privacy".to_string(),
                    href: "/privacy".to_string(),
                },
            ],
            ..Default::default()
        })
        .into_string();
        assert_contains(&footer, "Terms");
        assert_contains(&footer, "|");
        assert_contains(&footer, r#"href="/privacy""#);

        let section = MegaDropdownSection(&MegaDropdownSectionProps {
            title: "Products".to_string(),
            grid_cols: "grid-cols-3".to_string(),
            ..Default::default()
        })
        .into_string();
        assert_contains(&section, "grid-cols-3");
        assert_contains(&section, "Products");
        let columns = MegaDropdownTwoColumn("w-1/3", "w-2/3").into_string();
        assert_contains(&columns, "w-1/3");
        assert_contains(&columns, "w-2/3");
    }

    #[test]
    fn simple_mega_dropdown_combines_trigger_and_panel() {
        let output =
            SimpleMegaDropdown("lxp", "Learning", &MegaDropdownPanelProps::default()).into_string();
        assert_contains(&output, "Learning");
        assert_contains(&output, r#"data-show="$activeDropdown === 'lxp'""#);
        assert_contains(
            &output,
            r#"data-on:click="$activeDropdown = $activeDropdown === 'lxp' ? '' : 'lxp'""#,
        );
    }
}
