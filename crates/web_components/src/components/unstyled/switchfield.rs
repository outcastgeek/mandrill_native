#![allow(non_snake_case)]

use crate::components::{ComponentInfo, shared};
use maud::{Markup, Render, html};
use std::collections::BTreeMap;

pub const COMPONENTS: &[ComponentInfo] = &[
    ComponentInfo {
        family: "unstyled",
        module: "switchfield",
        name: "Switch",
    },
    ComponentInfo {
        family: "unstyled",
        module: "switchfield",
        name: "SwitchWithLabel",
    },
    ComponentInfo {
        family: "unstyled",
        module: "switchfield",
        name: "SwitchWithInlineLabel",
    },
    ComponentInfo {
        family: "unstyled",
        module: "switchfield",
        name: "SwitchCard",
    },
    ComponentInfo {
        family: "unstyled",
        module: "switchfield",
        name: "SwitchGroup",
    },
];

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SwitchProps {
    pub name: String,
    pub id: String,
    pub checked: bool,
    pub disabled: bool,
    pub loading: bool,
    pub size: &'static str,
    pub class: String,
    pub attributes: BTreeMap<String, String>,
    pub aria_label: String,
    pub aria_described_by: String,
}

impl Default for SwitchProps {
    fn default() -> Self {
        Self {
            name: String::new(),
            id: String::new(),
            checked: false,
            disabled: false,
            loading: false,
            size: shared::SIZE_MD,
            class: String::new(),
            attributes: BTreeMap::new(),
            aria_label: String::new(),
            aria_described_by: String::new(),
        }
    }
}

impl SwitchProps {
    #[must_use]
    pub fn with_name(mut self, value: impl Into<String>) -> Self {
        self.name = value.into();
        self
    }
    #[must_use]
    pub fn with_id(mut self, value: impl Into<String>) -> Self {
        self.id = value.into();
        self
    }
    #[must_use]
    pub const fn with_checked(mut self, checked: bool) -> Self {
        self.checked = checked;
        self
    }
    #[must_use]
    pub const fn with_disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }
    #[must_use]
    pub const fn with_loading(mut self, loading: bool) -> Self {
        self.loading = loading;
        self
    }
    #[must_use]
    pub const fn with_size(mut self, size: &'static str) -> Self {
        self.size = size;
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
    pub fn build_container_classes(&self) -> String {
        let disabled_class = if self.disabled {
            "opacity-50 cursor-not-allowed"
        } else {
            ""
        };
        shared::build_classes(&[
            "relative inline-flex items-center cursor-pointer transition-all duration-150",
            self.get_container_size_classes(),
            disabled_class,
            self.class.as_str(),
        ])
    }

    #[must_use]
    pub fn build_track_classes(&self) -> String {
        shared::build_classes(&[
            "flex items-center rounded-full transition-all duration-200 ease-in-out ring-1 ring-inset ring-border",
            self.get_track_size_classes(),
            if self.checked {
                "bg-success"
            } else {
                "bg-[var(--switch-track)]"
            },
        ])
    }

    #[must_use]
    pub fn build_thumb_classes(&self) -> String {
        shared::build_classes(&[
            "bg-[var(--switch-thumb)] rounded-full shadow-lg ring-1 ring-border/20 transition-all duration-200 ease-in-out",
            self.get_thumb_size_classes(),
            if self.checked {
                self.get_thumb_checked_position()
            } else {
                "translate-x-0.5"
            },
        ])
    }

    #[must_use]
    pub fn get_container_size_classes(&self) -> &'static str {
        match self.size {
            shared::SIZE_XS => "h-4",
            shared::SIZE_SM => "h-5",
            shared::SIZE_LG => "h-7",
            shared::SIZE_XL => "h-8",
            _ => "h-6",
        }
    }
    #[must_use]
    pub fn get_track_size_classes(&self) -> &'static str {
        match self.size {
            shared::SIZE_XS => "w-7 h-4",
            shared::SIZE_SM => "w-9 h-5",
            shared::SIZE_LG => "w-[52px] h-7",
            shared::SIZE_XL => "w-[60px] h-8",
            _ => "w-11 h-6",
        }
    }
    #[must_use]
    pub fn get_thumb_size_classes(&self) -> &'static str {
        match self.size {
            shared::SIZE_XS => "h-3 w-3",
            shared::SIZE_SM => "h-4 w-4",
            shared::SIZE_LG | shared::SIZE_XL => "h-7 w-7",
            _ => "h-5 w-5",
        }
    }
    #[must_use]
    pub fn get_thumb_checked_position(&self) -> &'static str {
        match self.size {
            shared::SIZE_XS => "translate-x-3",
            shared::SIZE_SM => "translate-x-4",
            shared::SIZE_LG => "translate-x-6",
            shared::SIZE_XL => "translate-x-7",
            _ => "translate-x-5",
        }
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
        attrs.insert("role".to_string(), "switch".to_string());
        attrs.insert(
            "data-attr:aria-checked".to_string(),
            format!("${}", self.get_signal_name()),
        );
        attrs.insert(
            "data-on:click".to_string(),
            format!("${0} = !${0}", self.get_signal_name()),
        );
        attrs
    }

    #[must_use]
    pub fn get_signal_name(&self) -> String {
        if self.name.is_empty() {
            "switchChecked".to_string()
        } else {
            self.name.replace(['-', '.'], "_")
        }
    }

    #[must_use]
    pub fn get_data_signals(&self) -> String {
        format!("{{ {}: {} }}", self.get_signal_name(), self.checked)
    }

    #[must_use]
    pub fn get_track_data_class(&self) -> String {
        let signal = self.get_signal_name();
        format!("{{ 'bg-success': ${signal}, 'bg-[var(--switch-track)]': !${signal} }}")
    }

    #[must_use]
    pub fn get_thumb_data_class(&self) -> String {
        let signal = self.get_signal_name();
        format!(
            "{{ '{}': ${signal}, 'translate-x-0.5': !${signal} }}",
            self.get_thumb_checked_position()
        )
    }

    #[must_use]
    pub fn build_track_base_classes(&self) -> String {
        shared::build_classes(&[
            "flex items-center rounded-full transition-all duration-200 ease-in-out ring-1 ring-inset ring-border",
            self.get_track_size_classes(),
        ])
    }

    #[must_use]
    pub fn build_thumb_base_classes(&self) -> String {
        shared::build_classes(&[
            "bg-[var(--switch-thumb)] rounded-full shadow-lg ring-1 ring-border/20 transition-all duration-200 ease-in-out",
            self.get_thumb_size_classes(),
        ])
    }

    #[must_use]
    pub const fn get_checkbox_bind_attribute_name(&self) -> &'static str {
        "data-bind"
    }

    #[must_use]
    pub fn get_checkbox_bind_value(&self) -> String {
        self.get_signal_name()
    }
}

#[must_use]
pub fn camel_to_kebab(s: &str) -> String {
    let mut result = String::new();
    for (i, ch) in s.chars().enumerate() {
        if ch.is_ascii_uppercase() {
            if i > 0 {
                result.push('-');
            }
            result.push(ch.to_ascii_lowercase());
        } else {
            result.push(ch);
        }
    }
    result
}

fn escape_attr(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

struct BoundCheckbox<'a>(&'a SwitchProps);

impl Render for BoundCheckbox<'_> {
    fn render_to(&self, output: &mut String) {
        let props = self.0;
        output.push_str("<input type=\"checkbox\"");
        if !props.name.is_empty() {
            output.push_str(" name=\"");
            output.push_str(&escape_attr(&props.name));
            output.push('"');
        }
        output.push_str(" value=\"on\" class=\"sr-only\" tabindex=\"-1\" aria-hidden=\"true\" ");
        output.push_str(&props.get_checkbox_bind_attribute_name());
        output.push_str("=\"");
        output.push_str(&escape_attr(&props.get_checkbox_bind_value()));
        output.push_str("\">");
    }
}

#[must_use]
pub fn Switch(props: &SwitchProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        div data-signals=(props.get_data_signals()) {
            button
                type="button"
                id=[shared::optional_attr(&props.id)]
                class=(props.build_container_classes())
                disabled[props.disabled || props.loading]
                role=[shared::attr_value(&attrs, "role")]
                data-attr:aria-checked=[shared::attr_value(&attrs, "data-attr:aria-checked")]
                data-on:click=[shared::attr_value(&attrs, "data-on:click")]
                aria-label=[shared::attr_value(&attrs, "aria-label")]
                aria-describedby=[shared::attr_value(&attrs, "aria-describedby")]
                data-testid=[shared::attr_value(&attrs, "data-testid")]
            {
                (BoundCheckbox(props))
                span class=(props.build_track_base_classes()) data-class=(props.get_track_data_class()) {
                    span class=(props.build_thumb_base_classes()) data-class=(props.get_thumb_data_class()) {
                        @if props.loading {
                            svg class="animate-spin h-3 w-3 text-primary mx-auto" xmlns="http://www.w3.org/2000/svg" fill="none" viewBox="0 0 24 24" {
                                circle class="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" stroke-width="4" {}
                                path class="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4zm2 5.291A7.962 7.962 0 014 12H0c0 3.042 1.135 5.824 3 7.938l3-2.647z" {}
                            }
                        }
                    }
                }
            }
        }
    }
}

#[must_use]
pub fn SwitchWithLabel(
    props: &SwitchProps,
    label: impl Into<String>,
    helper_text: impl Into<String>,
) -> Markup {
    let label = label.into();
    let helper_text = helper_text.into();
    let switch_props = props.clone().with_aria_label(&label);
    html! {
        div class="flex items-center justify-between" {
            div class="flex flex-col flex-1 mr-4" {
                @if !label.is_empty() {
                    label
                        for=[shared::optional_attr(&props.id)]
                        class="text-sm font-medium leading-none cursor-pointer peer-disabled:cursor-not-allowed peer-disabled:opacity-70"
                    { (&label) }
                }
                @if !helper_text.is_empty() {
                    p class="text-sm text-muted-foreground mt-1" { (&helper_text) }
                }
            }
            (Switch(&switch_props))
        }
    }
}

#[must_use]
pub fn SwitchWithInlineLabel(props: &SwitchProps, label: impl Into<String>) -> Markup {
    let label = label.into();
    let switch_props = props.clone().with_aria_label(&label);
    html! {
        label class="flex items-center space-x-3 cursor-pointer" {
            (Switch(&switch_props))
            @if !label.is_empty() {
                span class="text-sm font-medium leading-none" { (&label) }
            }
        }
    }
}

#[must_use]
pub fn SwitchCard(
    props: &SwitchProps,
    title: impl Into<String>,
    description: impl Into<String>,
    icon: Option<Markup>,
) -> Markup {
    let title = title.into();
    let description = description.into();
    html! {
        div class="relative flex items-center justify-between rounded-lg border border-border p-4 hover:bg-accent/50 transition-colors duration-150" {
            div class="flex items-start space-x-3 flex-1" {
                @if let Some(icon) = icon {
                    div class="flex items-center justify-center h-10 w-10 rounded-lg bg-primary/10 text-primary" {
                        (icon)
                    }
                }
                div class="flex-1" {
                    @if !title.is_empty() {
                        p class="font-medium text-sm" { (&title) }
                    }
                    @if !description.is_empty() {
                        p class="text-sm text-muted-foreground mt-1" { (&description) }
                    }
                }
            }
            (Switch(props))
        }
    }
}

#[must_use]
pub fn SwitchGroup(
    legend: impl Into<String>,
    helper_text: impl Into<String>,
    children: Markup,
) -> Markup {
    let legend = legend.into();
    let helper_text = helper_text.into();
    html! {
        fieldset class="space-y-3" {
            @if !legend.is_empty() {
                legend class="text-sm font-medium leading-none mb-3" { (&legend) }
            }
            @if !helper_text.is_empty() {
                p class="text-sm text-muted-foreground mb-3" { (&helper_text) }
            }
            div class="space-y-3" { (children) }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_helpers_datastar_and_size_variations_match_go_behavior() {
        let props = SwitchProps::default();
        assert_eq!(props.size, shared::SIZE_MD);
        assert_eq!(props.get_signal_name(), "switchChecked");
        assert_eq!(props.get_data_signals(), "{ switchChecked: false }");
        assert_eq!(camel_to_kebab("requireUppercase"), "require-uppercase");
        assert_eq!(props.get_checkbox_bind_attribute_name(), "data-bind");
        assert_eq!(props.get_checkbox_bind_value(), "switchChecked");

        for (size, container, track, thumb, checked) in [
            (
                shared::SIZE_XS,
                "h-4",
                "w-7 h-4",
                "h-3 w-3",
                "translate-x-3",
            ),
            (
                shared::SIZE_SM,
                "h-5",
                "w-9 h-5",
                "h-4 w-4",
                "translate-x-4",
            ),
            (
                shared::SIZE_MD,
                "h-6",
                "w-11 h-6",
                "h-5 w-5",
                "translate-x-5",
            ),
            (
                shared::SIZE_LG,
                "h-7",
                "w-[52px] h-7",
                "h-7 w-7",
                "translate-x-6",
            ),
            (
                shared::SIZE_XL,
                "h-8",
                "w-[60px] h-8",
                "h-7 w-7",
                "translate-x-7",
            ),
        ] {
            let props = SwitchProps::default().with_size(size).with_checked(true);
            assert!(props.build_container_classes().contains(container));
            assert_eq!(props.get_track_size_classes(), track);
            assert_eq!(props.get_thumb_size_classes(), thumb);
            assert_eq!(props.get_thumb_checked_position(), checked);
            assert!(props.build_track_classes().contains("bg-success"));
            assert!(props.build_thumb_classes().contains(checked));
        }

        let props = SwitchProps::default()
            .with_name("require-uppercase")
            .with_disabled(true)
            .with_loading(true)
            .with_size(shared::SIZE_SM)
            .with_class("switch-extra")
            .with_attribute("data-testid", "switch")
            .with_aria_label("Req")
            .with_aria_described_by("help");
        assert_eq!(props.get_signal_name(), "require_uppercase");
        assert!(props.disabled);
        assert!(props.loading);
        assert!(props.build_container_classes().contains("opacity-50"));
        assert!(props.build_container_classes().contains("switch-extra"));
        assert!(
            props
                .build_track_classes()
                .contains("bg-[var(--switch-track)]")
        );
        assert!(props.build_thumb_classes().contains("translate-x-0.5"));
        assert_eq!(props.get_checkbox_bind_attribute_name(), "data-bind");
        assert_eq!(props.get_checkbox_bind_value(), "require_uppercase");
        assert_eq!(
            props.get_track_data_class(),
            "{ 'bg-success': $require_uppercase, 'bg-[var(--switch-track)]': !$require_uppercase }"
        );
        assert_eq!(
            props.get_thumb_data_class(),
            "{ 'translate-x-4': $require_uppercase, 'translate-x-0.5': !$require_uppercase }"
        );
        let attrs = props.build_attributes();
        assert_eq!(attrs.get("role").map(String::as_str), Some("switch"));
        assert_eq!(
            attrs.get("data-attr:aria-checked").map(String::as_str),
            Some("$require_uppercase")
        );
        assert_eq!(
            attrs.get("data-on:click").map(String::as_str),
            Some("$require_uppercase = !$require_uppercase")
        );
        assert_eq!(attrs.get("aria-label").map(String::as_str), Some("Req"));
        assert_eq!(
            attrs.get("aria-describedby").map(String::as_str),
            Some("help")
        );
        assert_eq!(attrs.get("data-testid").map(String::as_str), Some("switch"));
    }

    #[test]
    fn switch_renders_datastar_colon_attributes_bound_input_track_thumb_and_loading() {
        let html = Switch(
            &SwitchProps::default()
                .with_name("requireUppercase")
                .with_id("require")
                .with_checked(true)
                .with_loading(true)
                .with_attribute("data-testid", "switch"),
        )
        .into_string();
        assert!(html.starts_with("<div data-signals=\"{ requireUppercase: true }\">"));
        assert!(html.contains("id=\"require\""));
        assert!(html.contains("disabled"));
        assert!(html.contains("role=\"switch\""));
        assert!(html.contains("data-attr:aria-checked=\"$requireUppercase\""));
        assert!(html.contains("data-on:click=\"$requireUppercase = !$requireUppercase\""));
        assert!(html.contains("data-testid=\"switch\""));
        assert!(html.contains("name=\"requireUppercase\" value=\"on\" class=\"sr-only\" tabindex=\"-1\" aria-hidden=\"true\" data-bind=\"requireUppercase\""));
        assert!(!html.contains("data-bind:"));
        assert!(html.contains("data-class=\"{ 'bg-success': $requireUppercase, 'bg-[var(--switch-track)]': !$requireUppercase }\""));
        assert!(html.contains("data-class=\"{ 'translate-x-5': $requireUppercase, 'translate-x-0.5': !$requireUppercase }\""));
        assert!(html.contains("animate-spin h-3 w-3 text-primary mx-auto"));
    }

    #[test]
    fn switch_label_inline_card_and_group_render_expected_html() {
        let props = SwitchProps::default().with_name("notify").with_id("notify");
        let labeled = SwitchWithLabel(&props, "<Notify>", "Help").into_string();
        assert!(labeled.starts_with("<div class=\"flex items-center justify-between\">"));
        assert!(labeled.contains("for=\"notify\""));
        assert!(labeled.contains("&lt;Notify&gt;"));
        assert!(labeled.contains("<p class=\"text-sm text-muted-foreground mt-1\">Help</p>"));
        assert!(labeled.contains("aria-label=\"&lt;Notify&gt;\""));

        let inline = SwitchWithInlineLabel(&props, "Inline").into_string();
        assert!(inline.starts_with("<label class=\"flex items-center space-x-3 cursor-pointer\">"));
        assert!(inline.contains("<span class=\"text-sm font-medium leading-none\">Inline</span>"));

        let card =
            SwitchCard(&props, "Title", "Desc", Some(html! { svg class="icon" {} })).into_string();
        assert!(card.contains(
            "relative flex items-center justify-between rounded-lg border border-border p-4"
        ));
        assert!(card.contains("<svg class=\"icon\"></svg>"));
        assert!(card.contains("<p class=\"font-medium text-sm\">Title</p>"));
        assert!(card.contains("<p class=\"text-sm text-muted-foreground mt-1\">Desc</p>"));

        let group = SwitchGroup("Legend", "Help", html! { div { "Child" } }).into_string();
        assert!(group.starts_with("<fieldset class=\"space-y-3\">"));
        assert!(
            group.contains(
                "<legend class=\"text-sm font-medium leading-none mb-3\">Legend</legend>"
            )
        );
        assert!(group.contains("<p class=\"text-sm text-muted-foreground mb-3\">Help</p>"));
        assert!(group.contains("<div class=\"space-y-3\"><div>Child</div></div>"));
    }
}
