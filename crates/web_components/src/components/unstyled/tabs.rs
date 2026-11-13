#![allow(non_snake_case)]

use crate::components::{ComponentInfo, shared};
use maud::{Markup, html};
use std::collections::BTreeMap;

const TABS_SIGNAL_NAME: &str = "activeTab";
pub const TAB_LIST_VARIANT_PILL: &str = "pill";
pub const TAB_LIST_VARIANT_UNDERLINE: &str = "underline";

pub const COMPONENTS: &[ComponentInfo] = &[
    ComponentInfo {
        family: "unstyled",
        module: "tabs",
        name: "Tabs",
    },
    ComponentInfo {
        family: "unstyled",
        module: "tabs",
        name: "TabList",
    },
    ComponentInfo {
        family: "unstyled",
        module: "tabs",
        name: "TabTrigger",
    },
    ComponentInfo {
        family: "unstyled",
        module: "tabs",
        name: "TabContent",
    },
    ComponentInfo {
        family: "unstyled",
        module: "tabs",
        name: "SimpleTabs",
    },
    ComponentInfo {
        family: "unstyled",
        module: "tabs",
        name: "VerticalTabs",
    },
    ComponentInfo {
        family: "unstyled",
        module: "tabs",
        name: "SimpleTabList",
    },
    ComponentInfo {
        family: "unstyled",
        module: "tabs",
        name: "SimpleTabTrigger",
    },
    ComponentInfo {
        family: "unstyled",
        module: "tabs",
        name: "SimpleTabContent",
    },
];

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TabsProps {
    pub default_value: String,
    pub orientation: String,
    pub class: String,
    pub attributes: BTreeMap<String, String>,
    pub data_attributes: BTreeMap<String, String>,
}

impl Default for TabsProps {
    fn default() -> Self {
        Self {
            default_value: String::new(),
            orientation: "horizontal".to_string(),
            class: String::new(),
            attributes: BTreeMap::new(),
            data_attributes: BTreeMap::new(),
        }
    }
}

impl TabsProps {
    #[must_use]
    pub fn with_default_value(mut self, value: impl Into<String>) -> Self {
        self.default_value = value.into();
        self
    }

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
        let orientation_classes = if self.orientation == "vertical" {
            "flex flex-row gap-4"
        } else {
            "flex flex-col gap-2"
        };
        shared::build_classes(&["tabs", orientation_classes, self.class.as_str()])
    }

    #[must_use]
    pub fn build_attributes(&self) -> BTreeMap<String, String> {
        let mut attrs = self.attributes.clone();
        if !self.default_value.is_empty() {
            attrs.insert("data-default-value".to_string(), self.default_value.clone());
        }
        attrs
            .entry("data-signals".to_string())
            .or_insert_with(|| buildTabsSignals(&self.default_value));
        if !self.orientation.is_empty() {
            attrs.insert("data-orientation".to_string(), self.orientation.clone());
        }
        for (key, value) in &self.data_attributes {
            attrs.insert(format!("data-{key}"), value.clone());
        }
        attrs
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct TabListProps {
    pub variant: &'static str,
    pub class: String,
    pub attributes: BTreeMap<String, String>,
    pub aria_label: String,
}

impl TabListProps {
    #[must_use]
    pub const fn with_variant(mut self, value: &'static str) -> Self {
        self.variant = value;
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
        let base = if self.variant == TAB_LIST_VARIANT_UNDERLINE {
            "tabs-underlined-list"
        } else {
            "tabs-pill-list"
        };
        shared::build_classes(&[base, self.class.as_str()])
    }

    #[must_use]
    pub fn build_attributes(&self) -> BTreeMap<String, String> {
        self.attributes.clone()
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct TabTriggerProps {
    pub value: String,
    pub disabled: bool,
    pub list_variant: &'static str,
    pub class: String,
    pub attributes: BTreeMap<String, String>,
    pub aria_label: String,
}

impl TabTriggerProps {
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
    pub const fn with_list_variant(mut self, value: &'static str) -> Self {
        self.list_variant = value;
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
        let disabled_classes = if self.disabled {
            "pointer-events-none opacity-50"
        } else {
            ""
        };
        let base = if self.list_variant == TAB_LIST_VARIANT_UNDERLINE {
            "tab-trigger-underline focus-visible:outline-2 focus-visible:outline-ring focus-visible:-outline-offset-2 focus-visible:rounded-sm"
        } else {
            "tab-trigger-pill focus-visible:outline-2 focus-visible:outline-ring focus-visible:outline-offset-2"
        };
        shared::build_classes(&[base, disabled_classes, self.class.as_str()])
    }

    #[must_use]
    pub fn build_attributes(&self) -> BTreeMap<String, String> {
        let mut attrs = self.attributes.clone();
        if !self.value.is_empty() {
            attrs.insert("data-value".to_string(), self.value.clone());
            if !self.disabled {
                attrs.insert("data-on:click".to_string(), buildTabClickExpr(&self.value));
            }
            attrs.insert(
                "data-attr:data-state".to_string(),
                buildTabStateExpr(&self.value),
            );
            attrs.insert(
                "data-attr:aria-selected".to_string(),
                buildTabSelectedExpr(&self.value),
            );
            attrs.insert(
                "data-attr:tabindex".to_string(),
                buildTabTabIndexExpr(&self.value),
            );
        }
        if !self.aria_label.is_empty() {
            attrs.insert("aria-label".to_string(), self.aria_label.clone());
        }
        if self.disabled {
            attrs.insert("aria-disabled".to_string(), "true".to_string());
        }
        attrs
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct TabContentProps {
    pub value: String,
    pub class: String,
    pub attributes: BTreeMap<String, String>,
}

impl TabContentProps {
    #[must_use]
    pub fn with_value(mut self, value: impl Into<String>) -> Self {
        self.value = value.into();
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
            "mt-2 ring-offset-background focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 data-[state=active]:animate-tab-content-in",
            self.class.as_str(),
        ])
    }

    #[must_use]
    pub fn build_attributes(&self) -> BTreeMap<String, String> {
        let mut attrs = self.attributes.clone();
        if !self.value.is_empty() {
            attrs.insert("data-value".to_string(), self.value.clone());
            attrs.insert("data-show".to_string(), buildTabSelectedExpr(&self.value));
            attrs.insert(
                "data-attr:data-state".to_string(),
                buildTabStateExpr(&self.value),
            );
        }
        attrs
            .entry("style".to_string())
            .or_insert_with(|| "display: none;".to_string());
        attrs
    }
}

fn quote_value(value: &str) -> String {
    format!("{value:?}")
}

#[must_use]
pub fn buildTabsSignals(default_value: &str) -> String {
    format!("{{{TABS_SIGNAL_NAME}: {}}}", quote_value(default_value))
}

#[must_use]
pub fn buildTabClickExpr(value: &str) -> String {
    format!("${TABS_SIGNAL_NAME} = {}", quote_value(value))
}

#[must_use]
pub fn buildTabSelectedExpr(value: &str) -> String {
    format!("${TABS_SIGNAL_NAME} === {}", quote_value(value))
}

#[must_use]
pub fn buildTabStateExpr(value: &str) -> String {
    format!(
        "${TABS_SIGNAL_NAME} === {} ? 'active' : 'inactive'",
        quote_value(value)
    )
}

#[must_use]
pub fn buildTabTabIndexExpr(value: &str) -> String {
    format!(
        "${TABS_SIGNAL_NAME} === {} ? '0' : '-1'",
        quote_value(value)
    )
}

#[must_use]
pub fn resolveTabListAriaLabel(props: &TabListProps) -> String {
    if props.aria_label.is_empty() {
        "Tabs".to_string()
    } else {
        props.aria_label.clone()
    }
}

#[must_use]
pub fn Tabs(props: &TabsProps, children: Markup) -> Markup {
    let attrs = props.build_attributes();
    html! {
        div
            class=(props.build_classes())
            data-component="tabs"
            data-default-value=[shared::attr_value(&attrs, "data-default-value")]
            data-signals=[shared::attr_value(&attrs, "data-signals")]
            data-orientation=[shared::attr_value(&attrs, "data-orientation")]
            id=[shared::attr_value(&attrs, "id")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
            data-track=[shared::attr_value(&attrs, "data-track")]
        {
            (children)
        }
    }
}

#[must_use]
pub fn TabList(props: &TabListProps, children: Markup) -> Markup {
    let attrs = props.build_attributes();
    html! {
        div
            role="tablist"
            class=(props.build_classes())
            aria-label=(resolveTabListAriaLabel(props))
            id=[shared::attr_value(&attrs, "id")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
        {
            (children)
        }
    }
}

#[must_use]
pub fn TabTrigger(props: &TabTriggerProps, children: Markup) -> Markup {
    let attrs = props.build_attributes();
    html! {
        button
            type="button"
            role="tab"
            class=(props.build_classes())
            disabled[props.disabled]
            data-value=[shared::attr_value(&attrs, "data-value")]
            data-on:click=[shared::attr_value(&attrs, "data-on:click")]
            data-attr:data-state=[shared::attr_value(&attrs, "data-attr:data-state")]
            data-attr:aria-selected=[shared::attr_value(&attrs, "data-attr:aria-selected")]
            data-attr:tabindex=[shared::attr_value(&attrs, "data-attr:tabindex")]
            aria-label=[shared::attr_value(&attrs, "aria-label")]
            aria-disabled=[shared::attr_value(&attrs, "aria-disabled")]
            id=[shared::attr_value(&attrs, "id")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
        {
            (children)
        }
    }
}

#[must_use]
pub fn TabContent(props: &TabContentProps, children: Markup) -> Markup {
    let attrs = props.build_attributes();
    html! {
        div
            role="tabpanel"
            class=(props.build_classes())
            tabindex="0"
            data-value=[shared::attr_value(&attrs, "data-value")]
            data-show=[shared::attr_value(&attrs, "data-show")]
            data-attr:data-state=[shared::attr_value(&attrs, "data-attr:data-state")]
            style=[shared::attr_value(&attrs, "style")]
            id=[shared::attr_value(&attrs, "id")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
        {
            (children)
        }
    }
}

#[must_use]
pub fn SimpleTabs(default_value: &str, class: &str, children: Markup) -> Markup {
    Tabs(
        &TabsProps::default()
            .with_default_value(default_value)
            .with_orientation("horizontal")
            .with_class(class),
        children,
    )
}

#[must_use]
pub fn VerticalTabs(default_value: &str, class: &str, children: Markup) -> Markup {
    Tabs(
        &TabsProps::default()
            .with_default_value(default_value)
            .with_orientation("vertical")
            .with_class(class),
        children,
    )
}

#[must_use]
pub fn SimpleTabList(aria_label: &str, class: &str, children: Markup) -> Markup {
    TabList(
        &TabListProps::default()
            .with_aria_label(aria_label)
            .with_class(class),
        children,
    )
}

#[must_use]
pub fn SimpleTabTrigger(value: &str, class: &str, children: Markup) -> Markup {
    TabTrigger(
        &TabTriggerProps::default()
            .with_value(value)
            .with_class(class),
        children,
    )
}

#[must_use]
pub fn SimpleTabContent(value: &str, class: &str, children: Markup) -> Markup {
    TabContent(
        &TabContentProps::default()
            .with_value(value)
            .with_class(class),
        children,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tabs_helpers_cover_classes_attributes_data_and_expressions() {
        let props = TabsProps::default();
        assert_eq!(props.orientation, "horizontal");
        assert_eq!(props.build_classes(), "tabs flex flex-col gap-2");
        assert_eq!(
            TabsProps::default()
                .with_orientation("vertical")
                .with_class("custom")
                .build_classes(),
            "tabs flex flex-row gap-4 custom"
        );
        assert_eq!(buildTabsSignals("overview"), "{activeTab: \"overview\"}");
        assert_eq!(buildTabClickExpr("overview"), "$activeTab = \"overview\"");
        assert_eq!(
            buildTabSelectedExpr("overview"),
            "$activeTab === \"overview\""
        );
        assert_eq!(
            buildTabStateExpr("overview"),
            "$activeTab === \"overview\" ? 'active' : 'inactive'"
        );
        assert_eq!(
            buildTabTabIndexExpr("overview"),
            "$activeTab === \"overview\" ? '0' : '-1'"
        );
        let attrs = TabsProps::default()
            .with_default_value("overview")
            .with_attribute("id", "tabs")
            .with_attribute("data-signals", "{custom: true}")
            .with_data_attribute("track", "settings")
            .build_attributes();
        assert_eq!(
            attrs.get("data-default-value").map(String::as_str),
            Some("overview")
        );
        assert_eq!(
            attrs.get("data-signals").map(String::as_str),
            Some("{custom: true}")
        );
        assert_eq!(
            attrs.get("data-orientation").map(String::as_str),
            Some("horizontal")
        );
        assert_eq!(
            attrs.get("data-track").map(String::as_str),
            Some("settings")
        );
    }

    #[test]
    fn tab_list_trigger_and_content_helpers_cover_variants_and_overrides() {
        assert_eq!(TabListProps::default().build_classes(), "tabs-pill-list");
        assert_eq!(
            TabListProps::default()
                .with_variant(TAB_LIST_VARIANT_UNDERLINE)
                .with_class("mb-4")
                .build_classes(),
            "tabs-underlined-list mb-4"
        );
        assert_eq!(resolveTabListAriaLabel(&TabListProps::default()), "Tabs");
        assert_eq!(
            resolveTabListAriaLabel(&TabListProps::default().with_aria_label("Sections")),
            "Sections"
        );
        let list_attrs = TabListProps::default()
            .with_attribute("data-testid", "tab-list")
            .build_attributes();
        assert_eq!(
            list_attrs.get("data-testid").map(String::as_str),
            Some("tab-list")
        );
        assert_eq!(
            TabTriggerProps::default()
                .with_disabled(true)
                .with_list_variant(TAB_LIST_VARIANT_UNDERLINE)
                .with_class("px-4")
                .build_classes(),
            "tab-trigger-underline focus-visible:outline-2 focus-visible:outline-ring focus-visible:-outline-offset-2 focus-visible:rounded-sm pointer-events-none opacity-50 px-4"
        );
        let disabled_attrs = TabTriggerProps::default()
            .with_value("disabled")
            .with_disabled(true)
            .with_aria_label("Disabled tab")
            .with_attribute("data-testid", "disabled-tab")
            .build_attributes();
        assert_eq!(disabled_attrs.get("data-on:click"), None);
        assert_eq!(
            disabled_attrs.get("aria-disabled").map(String::as_str),
            Some("true")
        );
        assert_eq!(
            disabled_attrs.get("data-testid").map(String::as_str),
            Some("disabled-tab")
        );
        assert_eq!(
            disabled_attrs.get("data-attr:tabindex").map(String::as_str),
            Some("$activeTab === \"disabled\" ? '0' : '-1'")
        );
        let content_attrs = TabContentProps::default()
            .with_value("overview")
            .with_attribute("style", "display: block;")
            .build_attributes();
        assert_eq!(
            content_attrs.get("style").map(String::as_str),
            Some("display: block;")
        );
        assert_eq!(
            content_attrs.get("data-show").map(String::as_str),
            Some("$activeTab === \"overview\"")
        );
        assert!(
            !TabTriggerProps::default()
                .build_attributes()
                .contains_key("data-attr:tabindex")
        );
    }

    #[test]
    fn tabs_parts_render_datastar_attributes_and_escaped_children() {
        let rendered = Tabs(
            &TabsProps::default()
                .with_default_value("overview")
                .with_attribute("id", "settings-tabs")
                .with_data_attribute("track", "settings"),
            html! {
                (TabList(
                    &TabListProps::default().with_aria_label("Settings sections"),
                    html! {
                        (TabTrigger(&TabTriggerProps::default().with_value("overview"), html! { "<Overview>" }))
                        (TabTrigger(&TabTriggerProps::default().with_value("billing").with_disabled(true), html! { "Billing" }))
                    },
                ))
                (TabContent(&TabContentProps::default().with_value("overview"), html! { p { "<Panel>" } }))
            },
        )
        .into_string();
        assert!(
            rendered.starts_with("<div class=\"tabs flex flex-col gap-2\" data-component=\"tabs\"")
        );
        assert!(rendered.contains("data-default-value=\"overview\""));
        assert!(rendered.contains("data-signals=\"{activeTab: &quot;overview&quot;}\""));
        assert!(rendered.contains("data-orientation=\"horizontal\""));
        assert!(rendered.contains("id=\"settings-tabs\""));
        assert!(rendered.contains("data-track=\"settings\""));
        assert!(rendered.contains(
            "<div role=\"tablist\" class=\"tabs-pill-list\" aria-label=\"Settings sections\">"
        ));
        assert!(rendered.contains("data-on:click=\"$activeTab = &quot;overview&quot;\""));
        assert!(rendered.contains(
            "data-attr:data-state=\"$activeTab === &quot;overview&quot; ? 'active' : 'inactive'\""
        ));
        assert!(
            rendered.contains("data-attr:aria-selected=\"$activeTab === &quot;overview&quot;\"")
        );
        assert!(rendered.contains("&lt;Overview&gt;"));
        assert!(rendered.contains("disabled"));
        assert!(rendered.contains("aria-disabled=\"true\""));
        assert!(rendered.contains("<div role=\"tabpanel\""));
        assert!(rendered.contains("tabindex=\"0\""));
        assert!(rendered.contains("data-show=\"$activeTab === &quot;overview&quot;\""));
        assert!(rendered.contains("style=\"display: none;\""));
        assert!(rendered.contains("<p>&lt;Panel&gt;</p>"));
    }

    #[test]
    fn convenience_tab_components_render_expected_variations() {
        let simple = SimpleTabs("home", "custom", html! { "Body" }).into_string();
        assert!(simple.contains("class=\"tabs flex flex-col gap-2 custom\""));
        assert!(simple.contains("data-default-value=\"home\""));
        let vertical = VerticalTabs("home", "", html! {}).into_string();
        assert!(vertical.contains("class=\"tabs flex flex-row gap-4\""));
        assert!(vertical.contains("data-orientation=\"vertical\""));
        let list = SimpleTabList("Primary", "list", html! { "Tabs" }).into_string();
        assert_eq!(
            list,
            "<div role=\"tablist\" class=\"tabs-pill-list list\" aria-label=\"Primary\">Tabs</div>"
        );
        let trigger = SimpleTabTrigger("home", "trigger", html! { "Home" }).into_string();
        assert!(trigger.contains("class=\"tab-trigger-pill focus-visible:outline-2 focus-visible:outline-ring focus-visible:outline-offset-2 trigger\""));
        assert!(trigger.contains("data-value=\"home\""));
        let content = SimpleTabContent("home", "panel", html! { "Home panel" }).into_string();
        assert!(content.contains("class=\"mt-2 ring-offset-background"));
        assert!(content.contains("panel\""));
        assert!(content.contains(">Home panel</div>"));
    }
}
