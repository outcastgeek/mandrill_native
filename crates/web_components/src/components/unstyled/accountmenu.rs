#![allow(non_snake_case)]

use crate::components::unstyled::{avatar, badge, text};
use crate::components::{ComponentInfo, shared};
use maud::{Markup, html};
use std::collections::BTreeMap;

pub const COMPONENTS: &[ComponentInfo] = &[ComponentInfo {
    family: "unstyled",
    module: "accountmenu",
    name: "TriggerRow",
}];

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TriggerRowProps {
    pub user_name: String,
    pub role_label: String,
    pub initials: String,
    pub menu_signal: String,
    pub aria_label: String,
    pub name_class: String,
    pub avatar_class: String,
    pub button_class: String,
    pub role_badge_class: String,
    pub chevron_class: String,
    pub attributes: BTreeMap<String, String>,
}

impl Default for TriggerRowProps {
    fn default() -> Self {
        Self {
            user_name: String::new(),
            role_label: String::new(),
            initials: String::new(),
            menu_signal: String::new(),
            aria_label: String::new(),
            name_class: "text-foreground".to_string(),
            avatar_class: String::new(),
            button_class: String::new(),
            role_badge_class: "text-xs uppercase tracking-wide".to_string(),
            chevron_class: "text-muted-foreground".to_string(),
            attributes: BTreeMap::new(),
        }
    }
}

impl TriggerRowProps {
    #[must_use]
    pub fn with_user_name(mut self, value: impl Into<String>) -> Self {
        self.user_name = value.into();
        self
    }

    #[must_use]
    pub fn with_role_label(mut self, value: impl Into<String>) -> Self {
        self.role_label = value.into();
        self
    }

    #[must_use]
    pub fn with_initials(mut self, value: impl Into<String>) -> Self {
        self.initials = value.into();
        self
    }

    #[must_use]
    pub fn with_menu_signal(mut self, value: impl Into<String>) -> Self {
        self.menu_signal = value.into();
        self
    }

    #[must_use]
    pub fn with_aria_label(mut self, value: impl Into<String>) -> Self {
        self.aria_label = value.into();
        self
    }

    #[must_use]
    pub fn with_name_class(mut self, value: impl Into<String>) -> Self {
        self.name_class = value.into();
        self
    }

    #[must_use]
    pub fn with_avatar_class(mut self, value: impl Into<String>) -> Self {
        self.avatar_class = value.into();
        self
    }

    #[must_use]
    pub fn with_button_class(mut self, value: impl Into<String>) -> Self {
        self.button_class = value.into();
        self
    }

    #[must_use]
    pub fn with_role_badge_class(mut self, value: impl Into<String>) -> Self {
        self.role_badge_class = value.into();
        self
    }

    #[must_use]
    pub fn with_chevron_class(mut self, value: impl Into<String>) -> Self {
        self.chevron_class = value.into();
        self
    }

    #[must_use]
    pub fn with_attribute(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.attributes.insert(name.into(), value.into());
        self
    }

    #[must_use]
    pub fn build_button_classes(&self) -> String {
        shared::build_classes(&[
            "account-menu-trigger-base relative inline-flex w-full items-center justify-start gap-3 text-left appearance-none cursor-pointer select-none transition-colors duration-150 focus-visible:outline-none",
            self.button_class.as_str(),
        ])
    }

    #[must_use]
    pub fn build_attributes(&self) -> BTreeMap<String, String> {
        let mut attrs = self.attributes.clone();
        if !self.menu_signal.is_empty() {
            attrs.insert(
                "data-on:click".to_string(),
                format!("${} = !${}", self.menu_signal, self.menu_signal),
            );
        }
        if !self.aria_label.is_empty() {
            attrs.insert("aria-label".to_string(), self.aria_label.clone());
        }
        attrs
    }
}

fn chevron_down() -> Markup {
    html! {
        svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" {
            polyline points="6 9 12 15 18 9" {}
        }
    }
}

fn chevron_up() -> Markup {
    html! {
        svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" {
            polyline points="18 15 12 9 6 15" {}
        }
    }
}

#[must_use]
pub fn TriggerRow(props: &TriggerRowProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        button
            type="button"
            class=(props.build_button_classes())
            data-on:click=[shared::attr_value(&attrs, "data-on:click")]
            aria-label=[shared::attr_value(&attrs, "aria-label")]
            id=[shared::attr_value(&attrs, "id")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
        {
            div class="flex w-full items-center gap-3" {
                (avatar::Avatar(
                    &avatar::AvatarProps::default()
                        .with_initials(&props.initials)
                        .with_size(shared::SIZE_MD)
                        .with_class(shared::build_classes(&[props.avatar_class.as_str(), "shrink-0"])),
                    html! {},
                ))
                div class="account-menu-trigger-copy min-w-0 flex-1 text-left" {
                    (text::Text(
                        &text::TextProps::default()
                            .with_as("div")
                            .with_size(shared::SIZE_SM)
                            .with_weight(shared::TEXT_WEIGHT_MEDIUM)
                            .with_class(shared::build_classes(&[
                                "account-menu-trigger-name truncate",
                                props.name_class.as_str(),
                            ])),
                        html! { (&props.user_name) },
                    ))
                    @if !props.role_label.is_empty() {
                        (badge::Badge(
                            &badge::BadgeProps::default()
                                .with_variant(shared::VARIANT_SECONDARY)
                                .with_size(shared::SIZE_XS)
                                .with_pill(false)
                                .with_class(shared::build_classes(&[
                                    "account-menu-trigger-role",
                                    props.role_badge_class.as_str(),
                                ])),
                            html! { (&props.role_label) },
                        ))
                    }
                }
                div class=(shared::build_classes(&["account-menu-trigger-chevron", props.chevron_class.as_str()])) {
                    @if !props.menu_signal.is_empty() {
                        span
                            class="account-menu-trigger-chevron-closed"
                            data-show=(format!("!${}", props.menu_signal))
                        {
                            (chevron_down())
                        }
                        span
                            class="account-menu-trigger-chevron-open"
                            data-show=(format!("${}", props.menu_signal))
                            style="display:none"
                        {
                            (chevron_up())
                        }
                    } @else {
                        (chevron_down())
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn helpers_cover_defaults_button_classes_and_attrs() {
        let props = TriggerRowProps::default();
        assert_eq!(props.name_class, "text-foreground");
        assert_eq!(props.role_badge_class, "text-xs uppercase tracking-wide");
        assert_eq!(props.chevron_class, "text-muted-foreground");
        assert_eq!(
            props.build_button_classes(),
            "account-menu-trigger-base relative inline-flex w-full items-center justify-start gap-3 text-left appearance-none cursor-pointer select-none transition-colors duration-150 focus-visible:outline-none"
        );
        let attrs = TriggerRowProps::default()
            .with_menu_signal("accountOpen")
            .with_aria_label("Open account menu")
            .with_attribute("id", "account-trigger")
            .build_attributes();
        assert_eq!(
            attrs.get("data-on:click").map(String::as_str),
            Some("$accountOpen = !$accountOpen")
        );
        assert_eq!(
            attrs.get("aria-label").map(String::as_str),
            Some("Open account menu")
        );
        assert_eq!(attrs.get("id").map(String::as_str), Some("account-trigger"));
    }

    #[test]
    fn trigger_row_renders_avatar_text_badge_datastar_chevrons_and_escaping() {
        let html = TriggerRow(
            &TriggerRowProps::default()
                .with_user_name("<Ada Lovelace>")
                .with_role_label("<Admin>")
                .with_initials("<AL>")
                .with_menu_signal("accountOpen")
                .with_aria_label("Account")
                .with_name_class("name-extra")
                .with_avatar_class("ring-2")
                .with_button_class("px-2")
                .with_role_badge_class("badge-extra")
                .with_chevron_class("chev-extra")
                .with_attribute("data-testid", "account-trigger"),
        )
        .into_string();
        assert!(html.starts_with("<button type=\"button\" class=\"account-menu-trigger-base"));
        assert!(html.contains("px-2\""));
        assert!(html.contains("data-on:click=\"$accountOpen = !$accountOpen\""));
        assert!(html.contains("aria-label=\"Account\""));
        assert!(html.contains("data-testid=\"account-trigger\""));
        assert!(html.contains("ring-2 shrink-0"));
        assert!(html.contains("&lt;AL&gt;"));
        assert!(html.contains("account-menu-trigger-name truncate name-extra"));
        assert!(html.contains("&lt;Ada Lovelace&gt;"));
        assert!(html.contains("account-menu-trigger-role badge-extra"));
        assert!(html.contains("&lt;Admin&gt;"));
        assert!(html.contains("class=\"account-menu-trigger-chevron chev-extra\""));
        assert!(
            html.contains(
                "class=\"account-menu-trigger-chevron-closed\" data-show=\"!$accountOpen\""
            )
        );
        assert!(html.contains("class=\"account-menu-trigger-chevron-open\" data-show=\"$accountOpen\" style=\"display:none\""));
        assert!(html.contains("<polyline points=\"6 9 12 15 18 9\"></polyline>"));
        assert!(html.contains("<polyline points=\"18 15 12 9 6 15\"></polyline>"));
    }

    #[test]
    fn trigger_row_without_signal_or_role_renders_single_chevron() {
        let html = TriggerRow(
            &TriggerRowProps::default()
                .with_user_name("Ada")
                .with_initials("AL"),
        )
        .into_string();
        assert!(!html.contains("data-on:click"));
        assert!(!html.contains("account-menu-trigger-role"));
        assert!(!html.contains("account-menu-trigger-chevron-open"));
        assert_eq!(
            html.matches("<polyline points=\"6 9 12 15 18 9\"></polyline>")
                .count(),
            1
        );
    }
}
