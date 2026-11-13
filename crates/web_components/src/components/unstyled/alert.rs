#![allow(non_snake_case)]

use crate::components::{ComponentInfo, shared};
use maud::{Markup, html};
use std::collections::BTreeMap;

const ALERT_CLOSE_ON_CLICK: &str = "const el = evt.currentTarget.closest('[role=alert]'); el.classList.add('animate-alert-dismiss'); setTimeout(() => el.remove(), 200)";

pub const COMPONENTS: &[ComponentInfo] = &[
    ComponentInfo {
        family: "unstyled",
        module: "alert",
        name: "Alert",
    },
    ComponentInfo {
        family: "unstyled",
        module: "alert",
        name: "AlertIcon",
    },
    ComponentInfo {
        family: "unstyled",
        module: "alert",
        name: "AlertTitle",
    },
    ComponentInfo {
        family: "unstyled",
        module: "alert",
        name: "AlertDescription",
    },
    ComponentInfo {
        family: "unstyled",
        module: "alert",
        name: "AlertClose",
    },
    ComponentInfo {
        family: "unstyled",
        module: "alert",
        name: "SimpleAlert",
    },
    ComponentInfo {
        family: "unstyled",
        module: "alert",
        name: "AlertWithIcon",
    },
    ComponentInfo {
        family: "unstyled",
        module: "alert",
        name: "InfoAlert",
    },
    ComponentInfo {
        family: "unstyled",
        module: "alert",
        name: "SuccessAlert",
    },
    ComponentInfo {
        family: "unstyled",
        module: "alert",
        name: "WarningAlert",
    },
    ComponentInfo {
        family: "unstyled",
        module: "alert",
        name: "ErrorAlert",
    },
];

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AlertProps {
    pub variant: &'static str,
    pub dismissible: bool,
    pub id: String,
    pub class: String,
    pub attributes: BTreeMap<String, String>,
    pub aria_label: String,
    pub aria_live: &'static str,
    pub aria_described_by: String,
    pub role: &'static str,
}

impl Default for AlertProps {
    fn default() -> Self {
        Self {
            variant: shared::VARIANT_DEFAULT,
            dismissible: false,
            id: String::new(),
            class: String::new(),
            attributes: BTreeMap::new(),
            aria_label: String::new(),
            aria_live: "polite",
            aria_described_by: String::new(),
            role: "alert",
        }
    }
}

impl AlertProps {
    #[must_use]
    pub fn with_variant(mut self, variant: &'static str) -> Self {
        self.variant = variant;
        self
    }

    #[must_use]
    pub const fn with_dismissible(mut self, dismissible: bool) -> Self {
        self.dismissible = dismissible;
        self
    }

    #[must_use]
    pub fn with_id(mut self, id: impl Into<String>) -> Self {
        self.id = id.into();
        self
    }

    #[must_use]
    pub fn with_class(mut self, class: impl Into<String>) -> Self {
        self.class = class.into();
        self
    }

    #[must_use]
    pub fn with_attribute(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.attributes.insert(name.into(), value.into());
        self
    }

    #[must_use]
    pub fn with_aria_label(mut self, aria_label: impl Into<String>) -> Self {
        self.aria_label = aria_label.into();
        self
    }

    #[must_use]
    pub fn with_aria_live(mut self, aria_live: &'static str) -> Self {
        self.aria_live = aria_live;
        self
    }

    #[must_use]
    pub fn with_aria_described_by(mut self, aria_described_by: impl Into<String>) -> Self {
        self.aria_described_by = aria_described_by.into();
        self
    }

    #[must_use]
    pub fn with_role(mut self, role: &'static str) -> Self {
        self.role = role;
        self
    }

    #[must_use]
    pub fn build_classes(&self) -> String {
        shared::build_classes(&[
            "relative w-full rounded-lg p-4 transition-all duration-200",
            shared::alert_variant_classes(self.variant),
            self.class.as_str(),
        ])
    }

    #[must_use]
    pub fn build_attributes(&self) -> BTreeMap<String, String> {
        let mut attrs = self.attributes.clone();
        if !self.id.is_empty() {
            attrs.insert("id".to_string(), self.id.clone());
        }
        if !self.aria_label.is_empty() {
            attrs.insert("aria-label".to_string(), self.aria_label.clone());
        }
        if !self.aria_live.is_empty() {
            attrs.insert("aria-live".to_string(), self.aria_live.to_string());
        }
        if !self.aria_described_by.is_empty() {
            attrs.insert(
                "aria-describedby".to_string(),
                self.aria_described_by.clone(),
            );
        }
        if !self.role.is_empty() {
            attrs.insert("role".to_string(), self.role.to_string());
        }
        if self.dismissible {
            attrs.insert("data-dismissible".to_string(), "true".to_string());
        }
        attrs
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Default)]
pub struct AlertTitleProps {
    pub class: String,
    pub attributes: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Default)]
pub struct AlertDescriptionProps {
    pub class: String,
    pub attributes: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Default)]
pub struct AlertIconProps {
    pub class: String,
    pub attributes: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Default)]
pub struct AlertCloseProps {
    pub class: String,
    pub attributes: BTreeMap<String, String>,
    pub aria_label: String,
}

impl AlertTitleProps {
    #[must_use]
    pub fn with_class(mut self, class: impl Into<String>) -> Self {
        self.class = class.into();
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
            "font-medium leading-tight tracking-tight text-foreground",
            self.class.as_str(),
        ])
    }
}

impl AlertDescriptionProps {
    #[must_use]
    pub fn with_class(mut self, class: impl Into<String>) -> Self {
        self.class = class.into();
        self
    }
    #[must_use]
    pub fn with_attribute(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.attributes.insert(name.into(), value.into());
        self
    }
    #[must_use]
    pub fn build_classes(&self) -> String {
        shared::build_classes(&["mt-1 text-sm text-muted-foreground", self.class.as_str()])
    }
}

impl AlertIconProps {
    #[must_use]
    pub fn with_class(mut self, class: impl Into<String>) -> Self {
        self.class = class.into();
        self
    }
    #[must_use]
    pub fn with_attribute(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.attributes.insert(name.into(), value.into());
        self
    }
    #[must_use]
    pub fn build_classes(&self) -> String {
        shared::build_classes(&["flex-shrink-0", self.class.as_str()])
    }
}

impl AlertCloseProps {
    #[must_use]
    pub fn with_class(mut self, class: impl Into<String>) -> Self {
        self.class = class.into();
        self
    }
    #[must_use]
    pub fn with_attribute(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.attributes.insert(name.into(), value.into());
        self
    }
    #[must_use]
    pub fn with_aria_label(mut self, aria_label: impl Into<String>) -> Self {
        self.aria_label = aria_label.into();
        self
    }
    #[must_use]
    pub fn build_classes(&self) -> String {
        shared::build_classes(&[
            "flex-shrink-0 -mt-1 -mr-1 p-1.5 rounded-md text-muted-foreground hover:text-foreground hover:bg-muted transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2",
            self.class.as_str(),
        ])
    }
}

#[must_use]
pub fn resolveAlertCloseAriaLabel(props: &AlertCloseProps) -> &str {
    if props.aria_label.is_empty() {
        "Close alert"
    } else {
        props.aria_label.as_str()
    }
}

#[must_use]
pub fn Alert(props: &AlertProps, children: Markup) -> Markup {
    let attrs = props.build_attributes();
    html! {
        div
            class=(props.build_classes())
            id=[shared::attr_value(&attrs, "id")]
            role=[shared::attr_value(&attrs, "role")]
            aria-label=[shared::attr_value(&attrs, "aria-label")]
            aria-live=[shared::attr_value(&attrs, "aria-live")]
            aria-describedby=[shared::attr_value(&attrs, "aria-describedby")]
            data-dismissible=[shared::attr_value(&attrs, "data-dismissible")]
            title=[shared::attr_value(&attrs, "title")]
        {
            (children)
        }
    }
}

#[must_use]
pub fn AlertIcon(props: &AlertIconProps, children: Markup) -> Markup {
    html! {
        div class=(props.build_classes()) title=[shared::attr_value(&props.attributes, "title")] {
            (children)
        }
    }
}

#[must_use]
pub fn AlertTitle(props: &AlertTitleProps, children: Markup) -> Markup {
    html! {
        h5 class=(props.build_classes()) id=[shared::attr_value(&props.attributes, "id")] title=[shared::attr_value(&props.attributes, "title")] {
            (children)
        }
    }
}

#[must_use]
pub fn AlertDescription(props: &AlertDescriptionProps, children: Markup) -> Markup {
    html! {
        div class=(props.build_classes()) id=[shared::attr_value(&props.attributes, "id")] title=[shared::attr_value(&props.attributes, "title")] {
            (children)
        }
    }
}

#[must_use]
pub fn AlertClose(props: &AlertCloseProps) -> Markup {
    html! {
        button
            type="button"
            class=(props.build_classes())
            aria-label=(resolveAlertCloseAriaLabel(props))
            data-alert-close
            data-on:click=(ALERT_CLOSE_ON_CLICK)
            title=[shared::attr_value(&props.attributes, "title")]
        {
            svg class="h-4 w-4" fill="none" stroke="currentColor" viewBox="0 0 24 24" xmlns="http://www.w3.org/2000/svg" {
                path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M6 18L18 6M6 6l12 12" {}
            }
        }
    }
}

#[must_use]
pub fn SimpleAlert(
    variant: &'static str,
    title: &str,
    description: &str,
    dismissible: bool,
) -> Markup {
    Alert(
        &AlertProps::default()
            .with_variant(variant)
            .with_dismissible(dismissible)
            .with_aria_live("polite")
            .with_role("alert"),
        html! {
            div class="flex gap-3" {
                @if !title.is_empty() {
                    div class="flex-1" {
                        (AlertTitle(&AlertTitleProps::default(), html! { (title) }))
                        @if !description.is_empty() {
                            (AlertDescription(&AlertDescriptionProps::default(), html! { (description) }))
                        }
                    }
                } @else if !description.is_empty() {
                    div class="flex-1" {
                        (AlertDescription(&AlertDescriptionProps::default(), html! { (description) }))
                    }
                }
                @if dismissible {
                    (AlertClose(&AlertCloseProps::default()))
                }
            }
        },
    )
}

#[must_use]
pub fn AlertWithIcon(
    variant: &'static str,
    title: &str,
    description: &str,
    dismissible: bool,
    icon: Markup,
) -> Markup {
    Alert(
        &AlertProps::default()
            .with_variant(variant)
            .with_dismissible(dismissible)
            .with_aria_live("polite")
            .with_role("alert"),
        html! {
            div class="flex gap-3" {
                (AlertIcon(&AlertIconProps::default(), icon))
                div class="flex-1" {
                    @if !title.is_empty() {
                        (AlertTitle(&AlertTitleProps::default(), html! { (title) }))
                    }
                    @if !description.is_empty() {
                        (AlertDescription(&AlertDescriptionProps::default(), html! { (description) }))
                    }
                }
                @if dismissible {
                    (AlertClose(&AlertCloseProps::default()))
                }
            }
        },
    )
}

#[must_use]
pub fn InfoAlert(title: &str, description: &str, dismissible: bool) -> Markup {
    SimpleAlert(shared::VARIANT_INFO, title, description, dismissible)
}

#[must_use]
pub fn SuccessAlert(title: &str, description: &str, dismissible: bool) -> Markup {
    SimpleAlert(shared::VARIANT_SUCCESS, title, description, dismissible)
}

#[must_use]
pub fn WarningAlert(title: &str, description: &str, dismissible: bool) -> Markup {
    SimpleAlert(shared::VARIANT_WARNING, title, description, dismissible)
}

#[must_use]
pub fn ErrorAlert(title: &str, description: &str, dismissible: bool) -> Markup {
    SimpleAlert(shared::VARIANT_DESTRUCTIVE, title, description, dismissible)
}

#[cfg(test)]
mod tests {
    use super::*;

    const VARIANTS: &[&str] = &[
        shared::VARIANT_DEFAULT,
        shared::VARIANT_INFO,
        shared::VARIANT_SUCCESS,
        shared::VARIANT_WARNING,
        shared::VARIANT_DESTRUCTIVE,
        "unknown",
    ];

    #[test]
    fn default_props_match_go_defaults() {
        let props = AlertProps::default();
        assert_eq!(props.variant, shared::VARIANT_DEFAULT);
        assert!(!props.dismissible);
        assert_eq!(props.aria_live, "polite");
        assert_eq!(props.role, "alert");
    }

    #[test]
    fn helper_classes_and_attributes_cover_variants_and_dismissible_state() {
        for &variant in VARIANTS {
            let props = AlertProps::default()
                .with_variant(variant)
                .with_dismissible(true)
                .with_id("notice")
                .with_aria_label("Notice")
                .with_aria_described_by("desc")
                .with_attribute("title", "Alert")
                .with_class("custom");
            let classes = props.build_classes();
            assert!(classes.contains("relative w-full rounded-lg p-4"));
            assert!(classes.contains(shared::alert_variant_classes(variant)));
            assert!(classes.contains("custom"));
            let attrs = props.build_attributes();
            assert_eq!(attrs.get("id"), Some(&"notice".to_string()));
            assert_eq!(attrs.get("aria-label"), Some(&"Notice".to_string()));
            assert_eq!(attrs.get("aria-describedby"), Some(&"desc".to_string()));
            assert_eq!(attrs.get("role"), Some(&"alert".to_string()));
            assert_eq!(attrs.get("data-dismissible"), Some(&"true".to_string()));
            assert_eq!(attrs.get("title"), Some(&"Alert".to_string()));
        }
    }

    #[test]
    fn section_helpers_render_expected_html_and_custom_classes() {
        assert_eq!(
            AlertIcon(&AlertIconProps::default().with_class("icon"), html! { "!" }).into_string(),
            r#"<div class="flex-shrink-0 icon">!</div>"#
        );
        let icon_props = AlertIconProps::default()
            .with_class("icon")
            .with_attribute("id", "alert-icon");
        assert_eq!(
            icon_props.attributes.get("id").map(String::as_str),
            Some("alert-icon")
        );
        assert_eq!(
            AlertTitle(
                &AlertTitleProps::default()
                    .with_class("title")
                    .with_attribute("id", "alert-title"),
                html! { "Title" }
            )
            .into_string(),
            r#"<h5 class="font-medium leading-tight tracking-tight text-foreground title" id="alert-title">Title</h5>"#
        );
        assert_eq!(
            AlertDescription(
                &AlertDescriptionProps::default()
                    .with_class("desc")
                    .with_attribute("id", "alert-desc"),
                html! { "Desc" }
            )
            .into_string(),
            r#"<div class="mt-1 text-sm text-muted-foreground desc" id="alert-desc">Desc</div>"#
        );
    }

    #[test]
    fn alert_close_renders_datastar_colon_attribute_icon_and_label_variations() {
        assert_eq!(
            resolveAlertCloseAriaLabel(&AlertCloseProps::default()),
            "Close alert"
        );
        assert_eq!(
            resolveAlertCloseAriaLabel(&AlertCloseProps::default().with_aria_label("Dismiss")),
            "Dismiss"
        );
        let html = AlertClose(
            &AlertCloseProps::default()
                .with_class("close")
                .with_aria_label("Dismiss")
                .with_attribute("title", "\"Close\" & now"),
        )
        .into_string();
        assert!(html.starts_with(r#"<button type="button" class="flex-shrink-0"#));
        assert!(html.contains("close"));
        assert!(html.contains(r#"aria-label="Dismiss""#));
        assert!(html.contains("data-alert-close"));
        assert!(
            html.contains(r#"data-on:click="const el = evt.currentTarget.closest('[role=alert]')"#)
        );
        assert!(html.contains(r#"title="&quot;Close&quot; &amp; now""#));
        assert!(html.contains(r#"<path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M6 18L18 6M6 6l12 12"></path>"#));
        assert!(!html.contains("data-on-click"));
    }

    #[test]
    fn alert_renders_actual_attributes_children_and_escaping() {
        let html = Alert(
            &AlertProps::default()
                .with_dismissible(true)
                .with_attribute("title", "\"Alert\" & more"),
            html! { "<Body>" },
        )
        .into_string();
        assert!(html.starts_with(r#"<div class="relative w-full rounded-lg p-4"#));
        assert!(html.contains(r#"role="alert""#));
        assert!(html.contains(r#"aria-live="polite""#));
        assert!(html.contains(r#"data-dismissible="true""#));
        assert!(html.contains(r#"title="&quot;Alert&quot; &amp; more""#));
        assert!(html.contains("&lt;Body&gt;"));
    }

    #[test]
    fn simple_alert_renders_title_description_and_dismissible_combinations() {
        for dismissible in [false, true] {
            let html =
                SimpleAlert(shared::VARIANT_INFO, "Title", "Desc", dismissible).into_string();
            assert!(html.contains(shared::alert_variant_classes(shared::VARIANT_INFO)));
            assert!(html.contains(
                r#"<h5 class="font-medium leading-tight tracking-tight text-foreground">Title</h5>"#
            ));
            assert!(html.contains(r#"<div class="mt-1 text-sm text-muted-foreground">Desc</div>"#));
            assert_eq!(html.contains("data-alert-close"), dismissible);
        }

        let description_only =
            SimpleAlert(shared::VARIANT_WARNING, "", "Desc", false).into_string();
        assert!(!description_only.contains("<h5 "));
        assert!(description_only.contains(">Desc</div>"));
    }

    #[test]
    fn alert_with_icon_and_variant_wrappers_render_expected_variants() {
        let with_icon = AlertWithIcon(
            shared::VARIANT_SUCCESS,
            "Done",
            "Saved",
            true,
            html! { svg class="ok" {} },
        )
        .into_string();
        assert!(with_icon.contains(r#"<div class="flex-shrink-0"><svg class="ok"></svg></div>"#));
        assert!(with_icon.contains("Done"));
        assert!(with_icon.contains("Saved"));
        assert!(with_icon.contains("data-alert-close"));

        assert!(
            InfoAlert("Info", "", false)
                .into_string()
                .contains("border-info")
        );
        assert!(
            SuccessAlert("Success", "", false)
                .into_string()
                .contains("border-success")
        );
        assert!(
            WarningAlert("Warning", "", false)
                .into_string()
                .contains("border-warning")
        );
        assert!(
            ErrorAlert("Error", "", false)
                .into_string()
                .contains("border-destructive")
        );
    }
}
