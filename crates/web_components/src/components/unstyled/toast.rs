#![allow(non_snake_case)]

use crate::components::{ComponentInfo, shared};
use maud::{Markup, html};
use std::collections::BTreeMap;

pub const COMPONENTS: &[ComponentInfo] = &[
    ComponentInfo {
        family: "unstyled",
        module: "toast",
        name: "ToastContainer",
    },
    ComponentInfo {
        family: "unstyled",
        module: "toast",
        name: "Toast",
    },
    ComponentInfo {
        family: "unstyled",
        module: "toast",
        name: "ToastIcon",
    },
    ComponentInfo {
        family: "unstyled",
        module: "toast",
        name: "ToastTitle",
    },
    ComponentInfo {
        family: "unstyled",
        module: "toast",
        name: "ToastDescription",
    },
    ComponentInfo {
        family: "unstyled",
        module: "toast",
        name: "ToastClose",
    },
    ComponentInfo {
        family: "unstyled",
        module: "toast",
        name: "SimpleToast",
    },
    ComponentInfo {
        family: "unstyled",
        module: "toast",
        name: "ToastWithIcon",
    },
    ComponentInfo {
        family: "unstyled",
        module: "toast",
        name: "InfoToast",
    },
    ComponentInfo {
        family: "unstyled",
        module: "toast",
        name: "SuccessToast",
    },
    ComponentInfo {
        family: "unstyled",
        module: "toast",
        name: "WarningToast",
    },
    ComponentInfo {
        family: "unstyled",
        module: "toast",
        name: "ErrorToast",
    },
    ComponentInfo {
        family: "unstyled",
        module: "toast",
        name: "SSEToast",
    },
    ComponentInfo {
        family: "unstyled",
        module: "toast",
        name: "SSEToastSuccess",
    },
    ComponentInfo {
        family: "unstyled",
        module: "toast",
        name: "SSEToastError",
    },
    ComponentInfo {
        family: "unstyled",
        module: "toast",
        name: "SSEToastWarning",
    },
    ComponentInfo {
        family: "unstyled",
        module: "toast",
        name: "SSEToastInfo",
    },
    ComponentInfo {
        family: "unstyled",
        module: "toast",
        name: "sseToastIcon",
    },
];

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ToastProps {
    pub id: String,
    pub variant: &'static str,
    pub duration: i32,
    pub dismissible: bool,
    pub position: &'static str,
    pub class: String,
    pub attributes: BTreeMap<String, String>,
    pub aria_label: String,
    pub aria_live: String,
    pub aria_described_by: String,
    pub role: String,
}

impl Default for ToastProps {
    fn default() -> Self {
        Self {
            id: String::new(),
            variant: shared::VARIANT_DEFAULT,
            duration: 5000,
            dismissible: true,
            position: shared::POSITION_TOP_RIGHT,
            class: String::new(),
            attributes: BTreeMap::new(),
            aria_label: String::new(),
            aria_live: "polite".to_string(),
            aria_described_by: String::new(),
            role: "status".to_string(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ToastContainerProps {
    pub position: &'static str,
    pub class: String,
    pub attributes: BTreeMap<String, String>,
    pub aria_label: String,
    pub aria_live: String,
    pub role: String,
}

impl Default for ToastContainerProps {
    fn default() -> Self {
        Self {
            position: shared::POSITION_TOP_RIGHT,
            class: String::new(),
            attributes: BTreeMap::new(),
            aria_label: String::new(),
            aria_live: "polite".to_string(),
            role: "region".to_string(),
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ToastTitleProps {
    pub class: String,
    pub attributes: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ToastDescriptionProps {
    pub class: String,
    pub attributes: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ToastIconProps {
    pub class: String,
    pub attributes: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ToastCloseProps {
    pub toast_id: String,
    pub class: String,
    pub attributes: BTreeMap<String, String>,
    pub aria_label: String,
}

impl ToastProps {
    #[must_use]
    pub fn with_id(mut self, value: impl Into<String>) -> Self {
        self.id = value.into();
        self
    }
    #[must_use]
    pub const fn with_variant(mut self, value: &'static str) -> Self {
        self.variant = value;
        self
    }
    #[must_use]
    pub const fn with_duration(mut self, value: i32) -> Self {
        self.duration = value;
        self
    }
    #[must_use]
    pub const fn with_dismissible(mut self, value: bool) -> Self {
        self.dismissible = value;
        self
    }
    #[must_use]
    pub const fn with_position(mut self, value: &'static str) -> Self {
        self.position = value;
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
    pub fn with_aria_live(mut self, value: impl Into<String>) -> Self {
        self.aria_live = value.into();
        self
    }
    #[must_use]
    pub fn with_aria_described_by(mut self, value: impl Into<String>) -> Self {
        self.aria_described_by = value.into();
        self
    }
    #[must_use]
    pub fn with_role(mut self, value: impl Into<String>) -> Self {
        self.role = value.into();
        self
    }
    #[must_use]
    pub fn build_classes(&self) -> String {
        shared::build_classes(&[
            "pointer-events-auto relative flex w-full max-w-md items-center justify-between space-x-4 overflow-hidden rounded-lg border p-4 pr-8 shadow-lg transition-all duration-200",
            shared::toast_slide_in_classes(self.position),
            shared::toast_accent_classes(self.variant),
            self.class.as_str(),
        ])
    }
    #[must_use]
    pub fn build_progress_classes(&self) -> &'static str {
        shared::toast_progress_classes(self.variant)
    }
    #[must_use]
    pub fn build_attributes(&self) -> BTreeMap<String, String> {
        let mut attrs = self.attributes.clone();
        if !self.id.is_empty() {
            attrs.insert("id".to_string(), self.id.clone());
            attrs.insert("data-toast-id".to_string(), self.id.clone());
        }
        if !self.aria_label.is_empty() {
            attrs.insert("aria-label".to_string(), self.aria_label.clone());
        }
        if !self.aria_live.is_empty() {
            attrs.insert("aria-live".to_string(), self.aria_live.clone());
        }
        if !self.aria_described_by.is_empty() {
            attrs.insert(
                "aria-describedby".to_string(),
                self.aria_described_by.clone(),
            );
        }
        if !self.role.is_empty() {
            attrs.insert("role".to_string(), self.role.clone());
        }
        attrs.insert("data-toast".to_string(), "true".to_string());
        if self.duration > 0 {
            attrs.insert("data-toast-duration".to_string(), self.duration.to_string());
        }
        if self.dismissible {
            attrs.insert("data-dismissible".to_string(), "true".to_string());
        }
        attrs
    }
}

impl ToastContainerProps {
    #[must_use]
    pub const fn with_position(mut self, value: &'static str) -> Self {
        self.position = value;
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
    pub fn with_aria_live(mut self, value: impl Into<String>) -> Self {
        self.aria_live = value.into();
        self
    }
    #[must_use]
    pub fn with_role(mut self, value: impl Into<String>) -> Self {
        self.role = value.into();
        self
    }
    #[must_use]
    pub fn build_classes(&self) -> String {
        shared::build_classes(&[
            "fixed z-50 flex max-h-screen flex-col-reverse gap-2 p-4 pointer-events-none",
            shared::toast_position_classes(self.position),
            self.class.as_str(),
        ])
    }
    #[must_use]
    pub fn build_container_attributes(&self) -> BTreeMap<String, String> {
        let mut attrs = self.attributes.clone();
        attrs.insert(
            "aria-label".to_string(),
            resolveToastContainerAriaLabel(self),
        );
        if !self.aria_live.is_empty() {
            attrs.insert("aria-live".to_string(), self.aria_live.clone());
        }
        if !self.role.is_empty() {
            attrs.insert("role".to_string(), self.role.clone());
        }
        attrs.insert(
            "data-toast-container".to_string(),
            self.position.to_string(),
        );
        attrs
    }
}

impl ToastTitleProps {
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
        shared::build_classes(&["text-sm font-semibold", self.class.as_str()])
    }
}

impl ToastDescriptionProps {
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
        shared::build_classes(&["text-sm opacity-90", self.class.as_str()])
    }
}

impl ToastIconProps {
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
        shared::build_classes(&["h-5 w-5 flex-shrink-0", self.class.as_str()])
    }
}

impl ToastCloseProps {
    #[must_use]
    pub fn with_toast_id(mut self, value: impl Into<String>) -> Self {
        self.toast_id = value.into();
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
        shared::build_classes(&[
            "absolute right-2 top-2 rounded-md p-1 opacity-70 transition-opacity hover:opacity-100 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2",
            self.class.as_str(),
        ])
    }
    #[must_use]
    pub fn build_attributes(&self) -> BTreeMap<String, String> {
        let mut attrs = self.attributes.clone();
        attrs.insert("aria-label".to_string(), resolveToastCloseAriaLabel(self));
        if !self.toast_id.is_empty() {
            attrs.insert("data-toast-close".to_string(), self.toast_id.clone());
        }
        attrs
    }
}

#[must_use]
pub fn resolveToastContainerAriaLabel(props: &ToastContainerProps) -> String {
    if props.aria_label.is_empty() {
        "Notifications".to_string()
    } else {
        props.aria_label.clone()
    }
}

#[must_use]
pub fn resolveToastCloseAriaLabel(props: &ToastCloseProps) -> String {
    if props.aria_label.is_empty() {
        "Close".to_string()
    } else {
        props.aria_label.clone()
    }
}

#[must_use]
pub fn ToastContainer(props: &ToastContainerProps, children: Markup) -> Markup {
    let attrs = props.build_container_attributes();
    html! {
        div
            class=(props.build_classes())
            aria-label=[shared::attr_value(&attrs, "aria-label")]
            aria-live=[shared::attr_value(&attrs, "aria-live")]
            role=[shared::attr_value(&attrs, "role")]
            data-toast-container=[shared::attr_value(&attrs, "data-toast-container")]
            id=[shared::attr_value(&attrs, "id")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
        {
            (children)
        }
    }
}

#[must_use]
pub fn Toast(props: &ToastProps, children: Markup) -> Markup {
    let attrs = props.build_attributes();
    html! {
        div
            class=(props.build_classes())
            id=[shared::attr_value(&attrs, "id")]
            aria-label=[shared::attr_value(&attrs, "aria-label")]
            aria-live=[shared::attr_value(&attrs, "aria-live")]
            aria-describedby=[shared::attr_value(&attrs, "aria-describedby")]
            role=[shared::attr_value(&attrs, "role")]
            data-toast=[shared::attr_value(&attrs, "data-toast")]
            data-toast-id=[shared::attr_value(&attrs, "data-toast-id")]
            data-toast-duration=[shared::attr_value(&attrs, "data-toast-duration")]
            data-dismissible=[shared::attr_value(&attrs, "data-dismissible")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
        {
            (children)
        }
    }
}

#[must_use]
pub fn ToastIcon(props: &ToastIconProps, children: Markup) -> Markup {
    html! {
        div
            class=(props.build_classes())
            id=[shared::attr_value(&props.attributes, "id")]
            data-testid=[shared::attr_value(&props.attributes, "data-testid")]
        {
            (children)
        }
    }
}

#[must_use]
pub fn ToastTitle(props: &ToastTitleProps, children: Markup) -> Markup {
    html! {
        div
            class=(props.build_classes())
            id=[shared::attr_value(&props.attributes, "id")]
            data-testid=[shared::attr_value(&props.attributes, "data-testid")]
        {
            (children)
        }
    }
}

#[must_use]
pub fn ToastDescription(props: &ToastDescriptionProps, children: Markup) -> Markup {
    html! {
        div
            class=(props.build_classes())
            id=[shared::attr_value(&props.attributes, "id")]
            data-testid=[shared::attr_value(&props.attributes, "data-testid")]
        {
            (children)
        }
    }
}

#[must_use]
pub fn ToastClose(props: &ToastCloseProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        button
            type="button"
            class=(props.build_classes())
            aria-label=[shared::attr_value(&attrs, "aria-label")]
            data-toast-close=[shared::attr_value(&attrs, "data-toast-close")]
            data-on:click="evt.currentTarget.closest('[role=status]').remove()"
            id=[shared::attr_value(&attrs, "id")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
        {
            svg class="h-4 w-4" fill="none" stroke="currentColor" viewBox="0 0 24 24" xmlns="http://www.w3.org/2000/svg" {
                path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M6 18L18 6M6 6l12 12" {}
            }
        }
    }
}

#[must_use]
pub fn SimpleToast(
    id: &str,
    variant: &'static str,
    title: &str,
    description: &str,
    duration: i32,
    dismissible: bool,
) -> Markup {
    Toast(
        &ToastProps::default()
            .with_id(id)
            .with_variant(variant)
            .with_duration(duration)
            .with_dismissible(dismissible)
            .with_position(shared::POSITION_TOP_RIGHT)
            .with_aria_live("polite")
            .with_role("status"),
        html! {
            div class="flex gap-3 w-full" {
                div class="flex-1" {
                    @if !title.is_empty() {
                        (ToastTitle(&ToastTitleProps::default(), html! { (title) }))
                    }
                    @if !description.is_empty() {
                        (ToastDescription(&ToastDescriptionProps::default(), html! { (description) }))
                    }
                }
                @if dismissible {
                    (ToastClose(&ToastCloseProps::default().with_toast_id(id).with_aria_label("Close")))
                }
            }
        },
    )
}

#[must_use]
pub fn ToastWithIcon(
    id: &str,
    variant: &'static str,
    title: &str,
    description: &str,
    duration: i32,
    dismissible: bool,
    icon: Markup,
) -> Markup {
    Toast(
        &ToastProps::default()
            .with_id(id)
            .with_variant(variant)
            .with_duration(duration)
            .with_dismissible(dismissible)
            .with_position(shared::POSITION_TOP_RIGHT)
            .with_aria_live("polite")
            .with_role("status"),
        html! {
            div class="flex gap-3 w-full" {
                (ToastIcon(&ToastIconProps::default(), icon))
                div class="flex-1" {
                    @if !title.is_empty() {
                        (ToastTitle(&ToastTitleProps::default(), html! { (title) }))
                    }
                    @if !description.is_empty() {
                        (ToastDescription(&ToastDescriptionProps::default(), html! { (description) }))
                    }
                }
                @if dismissible {
                    (ToastClose(&ToastCloseProps::default().with_toast_id(id).with_aria_label("Close")))
                }
            }
        },
    )
}

#[must_use]
pub fn InfoToast(id: &str, title: &str, description: &str, duration: i32) -> Markup {
    SimpleToast(id, shared::VARIANT_INFO, title, description, duration, true)
}
#[must_use]
pub fn SuccessToast(id: &str, title: &str, description: &str, duration: i32) -> Markup {
    SimpleToast(
        id,
        shared::VARIANT_SUCCESS,
        title,
        description,
        duration,
        true,
    )
}
#[must_use]
pub fn WarningToast(id: &str, title: &str, description: &str, duration: i32) -> Markup {
    SimpleToast(
        id,
        shared::VARIANT_WARNING,
        title,
        description,
        duration,
        true,
    )
}
#[must_use]
pub fn ErrorToast(id: &str, title: &str, description: &str, duration: i32) -> Markup {
    SimpleToast(
        id,
        shared::VARIANT_DESTRUCTIVE,
        title,
        description,
        duration,
        true,
    )
}

#[must_use]
pub fn sseToastDismissAction() -> &'static str {
    "var toast = evt.currentTarget.closest('[role=status]'); if (toast) toast.remove()"
}

#[must_use]
pub fn sseToastAttributes(auto_dismiss_seconds: i32) -> BTreeMap<String, String> {
    let mut attrs = BTreeMap::new();
    if auto_dismiss_seconds > 0 {
        attrs.insert(
            "data-toast-duration-ms".to_string(),
            (auto_dismiss_seconds * 1000).to_string(),
        );
    }
    attrs
}

#[must_use]
pub fn sseToastProgressClass(variant: &str) -> &'static str {
    match variant {
        shared::VARIANT_SUCCESS => "toast-progress-success",
        shared::VARIANT_DESTRUCTIVE => "toast-progress-destructive",
        shared::VARIANT_WARNING => "toast-progress-warning",
        shared::VARIANT_INFO => "toast-progress-info",
        _ => "toast-progress-default",
    }
}

#[must_use]
pub fn sseToastVariantClasses(variant: &str) -> &'static str {
    match variant {
        shared::VARIANT_SUCCESS => "bg-success text-success-foreground",
        shared::VARIANT_DESTRUCTIVE => "bg-destructive text-destructive-foreground",
        shared::VARIANT_WARNING => "bg-warning text-warning-foreground",
        shared::VARIANT_INFO => "bg-info text-info-foreground",
        _ => "bg-muted text-foreground",
    }
}

#[must_use]
pub fn sseToastIcon(variant: &str) -> Markup {
    let label = match variant {
        shared::VARIANT_SUCCESS => "check-circle",
        shared::VARIANT_DESTRUCTIVE => "x-circle",
        shared::VARIANT_WARNING => "alert-triangle",
        shared::VARIANT_INFO => "info",
        _ => "message-circle",
    };
    html! {
        svg
            class="h-4 w-4"
            data-icon=(label)
            fill="none"
            stroke="currentColor"
            viewBox="0 0 24 24"
            xmlns="http://www.w3.org/2000/svg"
            aria-hidden="true"
        {
            circle cx="12" cy="12" r="10" {}
            @match variant {
                shared::VARIANT_SUCCESS => {
                    path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M8 12l2.5 2.5L16 9" {}
                }
                shared::VARIANT_DESTRUCTIVE => {
                    path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M15 9l-6 6M9 9l6 6" {}
                }
                shared::VARIANT_WARNING => {
                    path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M12 8v4M12 16h.01" {}
                }
                shared::VARIANT_INFO => {
                    path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M12 11v5M12 8h.01" {}
                }
                _ => {
                    path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M8 12h8M8 16h5" {}
                }
            }
        }
    }
}

#[must_use]
pub fn SSEToast(variant: &'static str, message: &str, auto_dismiss_seconds: i32) -> Markup {
    let attrs = sseToastAttributes(auto_dismiss_seconds);
    html! {
        div
            class=(shared::build_classes(&[
                "pointer-events-auto animate-in slide-in-from-top-2 rounded-lg mb-2 relative overflow-hidden",
                sseToastVariantClasses(variant),
            ]))
            role="status"
            aria-live="polite"
            data-toast-duration-ms=[shared::attr_value(&attrs, "data-toast-duration-ms")]
        {
            div class="flex items-center gap-3 p-3" {
                span class="text-sm" {
                    (sseToastIcon(variant))
                }
                p class="text-sm font-medium flex-1" { (message) }
                button
                    type="button"
                    class="opacity-70 hover:opacity-100 text-sm font-bold cursor-pointer"
                    aria-label="Dismiss"
                    data-toast-dismiss-button="true"
                    data-on:click=(sseToastDismissAction())
                {
                    "×"
                }
            }
            @if auto_dismiss_seconds > 0 {
                div
                    class=(format!("toast-progress-bar {}", sseToastProgressClass(variant)))
                    style=(format!("animation: toast-progress {auto_dismiss_seconds}s linear forwards"))
                    data-on:animationend=(sseToastDismissAction())
                    aria-hidden="true"
                {}
            }
        }
    }
}

#[must_use]
pub fn SSEToastSuccess(message: &str) -> Markup {
    SSEToast(shared::VARIANT_SUCCESS, message, 5)
}
#[must_use]
pub fn SSEToastError(message: &str) -> Markup {
    SSEToast(shared::VARIANT_DESTRUCTIVE, message, 8)
}
#[must_use]
pub fn SSEToastWarning(message: &str) -> Markup {
    SSEToast(shared::VARIANT_WARNING, message, 5)
}
#[must_use]
pub fn SSEToastInfo(message: &str) -> Markup {
    SSEToast(shared::VARIANT_INFO, message, 5)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn helpers_cover_defaults_variants_positions_and_attributes() {
        let props = ToastProps::default();
        assert_eq!(props.variant, shared::VARIANT_DEFAULT);
        assert_eq!(props.duration, 5000);
        assert!(props.dismissible);
        assert_eq!(props.position, shared::POSITION_TOP_RIGHT);
        assert!(
            props
                .build_classes()
                .contains(shared::toast_slide_in_classes(shared::POSITION_TOP_RIGHT))
        );
        assert!(
            props
                .build_classes()
                .contains(shared::toast_accent_classes(shared::VARIANT_DEFAULT))
        );
        assert_eq!(
            props.build_progress_classes(),
            shared::toast_progress_classes(shared::VARIANT_DEFAULT)
        );

        for variant in [
            shared::VARIANT_DEFAULT,
            shared::VARIANT_INFO,
            shared::VARIANT_SUCCESS,
            shared::VARIANT_WARNING,
            shared::VARIANT_DESTRUCTIVE,
        ] {
            assert!(
                ToastProps::default()
                    .with_variant(variant)
                    .build_classes()
                    .contains(shared::toast_accent_classes(variant))
            );
            assert_eq!(
                ToastProps::default()
                    .with_variant(variant)
                    .build_progress_classes(),
                shared::toast_progress_classes(variant)
            );
            assert_eq!(
                sseToastProgressClass(variant),
                match variant {
                    shared::VARIANT_SUCCESS => "toast-progress-success",
                    shared::VARIANT_DESTRUCTIVE => "toast-progress-destructive",
                    shared::VARIANT_WARNING => "toast-progress-warning",
                    shared::VARIANT_INFO => "toast-progress-info",
                    _ => "toast-progress-default",
                }
            );
            assert_eq!(
                sseToastVariantClasses(variant),
                match variant {
                    shared::VARIANT_SUCCESS => "bg-success text-success-foreground",
                    shared::VARIANT_DESTRUCTIVE => "bg-destructive text-destructive-foreground",
                    shared::VARIANT_WARNING => "bg-warning text-warning-foreground",
                    shared::VARIANT_INFO => "bg-info text-info-foreground",
                    _ => "bg-muted text-foreground",
                }
            );
        }

        for position in [
            shared::POSITION_TOP_LEFT,
            shared::POSITION_TOP_RIGHT,
            shared::POSITION_BOTTOM_LEFT,
            shared::POSITION_BOTTOM_RIGHT,
            shared::POSITION_TOP_CENTER,
            shared::POSITION_BOTTOM_CENTER,
        ] {
            assert!(
                ToastContainerProps::default()
                    .with_position(position)
                    .build_classes()
                    .contains(shared::toast_position_classes(position))
            );
            assert!(
                ToastProps::default()
                    .with_position(position)
                    .build_classes()
                    .contains(shared::toast_slide_in_classes(position))
            );
        }

        let attrs = ToastProps::default()
            .with_id("toast-1")
            .with_duration(0)
            .with_dismissible(false)
            .with_aria_label("Saved")
            .with_aria_live("assertive")
            .with_aria_described_by("desc")
            .with_role("alert")
            .with_attribute("data-testid", "toast")
            .build_attributes();
        assert_eq!(attrs.get("id").map(String::as_str), Some("toast-1"));
        assert_eq!(
            attrs.get("data-toast-id").map(String::as_str),
            Some("toast-1")
        );
        assert_eq!(attrs.get("data-toast-duration"), None);
        assert_eq!(attrs.get("data-dismissible"), None);
        assert_eq!(attrs.get("aria-label").map(String::as_str), Some("Saved"));
        assert_eq!(
            attrs.get("aria-live").map(String::as_str),
            Some("assertive")
        );
        assert_eq!(
            attrs.get("aria-describedby").map(String::as_str),
            Some("desc")
        );
        assert_eq!(attrs.get("role").map(String::as_str), Some("alert"));
        assert_eq!(attrs.get("data-testid").map(String::as_str), Some("toast"));

        let container_attrs = ToastContainerProps::default()
            .with_position(shared::POSITION_BOTTOM_CENTER)
            .build_container_attributes();
        assert_eq!(
            container_attrs.get("aria-label").map(String::as_str),
            Some("Notifications")
        );
        assert_eq!(
            resolveToastContainerAriaLabel(&ToastContainerProps::default()),
            "Notifications"
        );
        assert_eq!(
            resolveToastContainerAriaLabel(
                &ToastContainerProps::default().with_aria_label("Alerts")
            ),
            "Alerts"
        );
        assert_eq!(
            container_attrs
                .get("data-toast-container")
                .map(String::as_str),
            Some("bottom-center")
        );
    }

    #[test]
    fn part_helpers_and_simple_parts_render_html() {
        assert_eq!(
            ToastTitleProps::default()
                .with_class("title")
                .build_classes(),
            "text-sm font-semibold title"
        );
        assert_eq!(
            ToastDescriptionProps::default()
                .with_class("desc")
                .build_classes(),
            "text-sm opacity-90 desc"
        );
        assert_eq!(
            ToastIconProps::default().with_class("icon").build_classes(),
            "h-5 w-5 flex-shrink-0 icon"
        );
        assert!(
            ToastCloseProps::default()
                .with_class("close")
                .build_classes()
                .contains("close")
        );
        assert_eq!(
            ToastCloseProps::default()
                .build_attributes()
                .get("aria-label")
                .map(String::as_str),
            Some("Close")
        );
        assert_eq!(
            resolveToastCloseAriaLabel(&ToastCloseProps::default()),
            "Close"
        );
        assert_eq!(
            resolveToastCloseAriaLabel(&ToastCloseProps::default().with_aria_label("Dismiss")),
            "Dismiss"
        );

        assert_eq!(
            ToastTitle(
                &ToastTitleProps::default().with_attribute("data-testid", "title"),
                html! { "<Title>" }
            )
            .into_string(),
            "<div class=\"text-sm font-semibold\" data-testid=\"title\">&lt;Title&gt;</div>",
        );
        assert_eq!(
            ToastDescription(&ToastDescriptionProps::default(), html! { "Details" }).into_string(),
            "<div class=\"text-sm opacity-90\">Details</div>",
        );
        assert_eq!(
            ToastIcon(
                &ToastIconProps::default().with_class("text-info"),
                html! { svg {} }
            )
            .into_string(),
            "<div class=\"h-5 w-5 flex-shrink-0 text-info\"><svg></svg></div>",
        );
    }

    #[test]
    fn container_toast_and_close_render_real_attributes() {
        let container = ToastContainer(
            &ToastContainerProps::default()
                .with_position(shared::POSITION_BOTTOM_LEFT)
                .with_aria_label("Alerts")
                .with_attribute("data-testid", "container"),
            html! { "Items" },
        )
        .into_string();
        assert!(
            container.starts_with("<div class=\"fixed z-50 flex max-h-screen flex-col-reverse")
        );
        assert!(container.contains("bottom-4 left-4"));
        assert!(container.contains("aria-label=\"Alerts\" aria-live=\"polite\" role=\"region\""));
        assert!(
            container.contains("data-toast-container=\"bottom-left\" data-testid=\"container\"")
        );
        assert!(container.ends_with(">Items</div>"));

        let toast = Toast(
            &ToastProps::default()
                .with_id("t1")
                .with_variant(shared::VARIANT_SUCCESS)
                .with_duration(2500)
                .with_aria_described_by("desc"),
            html! { "Saved" },
        )
        .into_string();
        assert!(toast.contains("toast-accent-success"));
        assert!(
            toast.contains(
                "id=\"t1\" aria-live=\"polite\" aria-describedby=\"desc\" role=\"status\""
            )
        );
        assert!(toast.contains("data-toast=\"true\" data-toast-id=\"t1\" data-toast-duration=\"2500\" data-dismissible=\"true\""));
        assert!(toast.ends_with(">Saved</div>"));

        let close = ToastClose(
            &ToastCloseProps::default()
                .with_toast_id("t1")
                .with_attribute("data-testid", "close"),
        )
        .into_string();
        assert!(close.contains("aria-label=\"Close\" data-toast-close=\"t1\""));
        assert!(
            close.contains("data-on:click=\"evt.currentTarget.closest('[role=status]').remove()\"")
        );
        assert!(close.contains("data-testid=\"close\""));
        assert!(close.contains("<svg class=\"h-4 w-4\""));
    }

    #[test]
    fn convenience_toasts_render_variants_icons_and_dismissible_states() {
        let simple = SimpleToast(
            "t1",
            shared::VARIANT_WARNING,
            "Warn",
            "<Details>",
            4000,
            true,
        )
        .into_string();
        assert!(simple.contains("toast-accent-warning"));
        assert!(simple.contains("<div class=\"text-sm font-semibold\">Warn</div>"));
        assert!(simple.contains("<div class=\"text-sm opacity-90\">&lt;Details&gt;</div>"));
        assert!(simple.contains("data-toast-close=\"t1\""));

        let no_close =
            SimpleToast("t2", shared::VARIANT_INFO, "", "Only details", 0, false).into_string();
        assert!(no_close.contains("toast-accent-info"));
        assert!(!no_close.contains("data-toast-duration"));
        assert!(!no_close.contains("data-dismissible"));
        assert!(!no_close.contains("data-toast-close"));
        assert!(!no_close.contains("text-sm font-semibold"));
        assert!(no_close.contains("Only details"));

        let with_icon = ToastWithIcon(
            "t3",
            shared::VARIANT_SUCCESS,
            "Done",
            "",
            1,
            true,
            html! { svg data-testid="ok" {} },
        )
        .into_string();
        assert!(
            with_icon.contains(
                "<div class=\"h-5 w-5 flex-shrink-0\"><svg data-testid=\"ok\"></svg></div>"
            )
        );
        assert!(with_icon.contains("toast-accent-success"));

        assert!(
            InfoToast("info", "Info", "Body", 5)
                .into_string()
                .contains("toast-accent-info")
        );
        assert!(
            SuccessToast("success", "Success", "Body", 5)
                .into_string()
                .contains("toast-accent-success")
        );
        assert!(
            WarningToast("warning", "Warning", "Body", 5)
                .into_string()
                .contains("toast-accent-warning")
        );
        assert!(
            ErrorToast("error", "Error", "Body", 5)
                .into_string()
                .contains("toast-accent-destructive")
        );
    }

    #[test]
    fn sse_toasts_render_datastar_and_auto_dismiss_variations() {
        assert_eq!(
            sseToastDismissAction(),
            "var toast = evt.currentTarget.closest('[role=status]'); if (toast) toast.remove()"
        );
        assert_eq!(sseToastAttributes(0).get("data-toast-duration-ms"), None);
        assert_eq!(
            sseToastAttributes(5)
                .get("data-toast-duration-ms")
                .map(String::as_str),
            Some("5000")
        );

        for (variant, icon_name) in [
            (shared::VARIANT_SUCCESS, "check-circle"),
            (shared::VARIANT_DESTRUCTIVE, "x-circle"),
            (shared::VARIANT_WARNING, "alert-triangle"),
            (shared::VARIANT_INFO, "info"),
            (shared::VARIANT_DEFAULT, "message-circle"),
        ] {
            let icon = sseToastIcon(variant).into_string();
            assert!(icon.contains(&format!("data-icon=\"{icon_name}\"")));
            assert!(icon.contains("class=\"h-4 w-4\""));
        }

        let toast = SSEToast(shared::VARIANT_SUCCESS, "<Saved>", 5).into_string();
        assert!(toast.contains("bg-success text-success-foreground"));
        assert!(
            toast.contains("role=\"status\" aria-live=\"polite\" data-toast-duration-ms=\"5000\"")
        );
        assert!(toast.contains("data-toast-dismiss-button=\"true\" data-on:click=\"var toast = evt.currentTarget.closest('[role=status]'); if (toast) toast.remove()\""));
        assert!(toast.contains("&lt;Saved&gt;"));
        assert!(toast.contains("style=\"animation: toast-progress 5s linear forwards\""));
        assert!(toast.contains("data-on:animationend=\"var toast = evt.currentTarget.closest('[role=status]'); if (toast) toast.remove()\""));

        let persistent = SSEToast(shared::VARIANT_INFO, "Info", 0).into_string();
        assert!(!persistent.contains("data-toast-duration-ms"));
        assert!(!persistent.contains("toast-progress-bar toast-progress-info"));

        assert!(
            SSEToastSuccess("ok")
                .into_string()
                .contains("toast-progress-success")
        );
        assert!(
            SSEToastError("bad")
                .into_string()
                .contains("toast-progress-destructive")
        );
        assert!(
            SSEToastWarning("warn")
                .into_string()
                .contains("toast-progress-warning")
        );
        assert!(
            SSEToastInfo("note")
                .into_string()
                .contains("toast-progress-info")
        );
    }

    #[test]
    fn remaining_builders_and_icon_toast_description_validate_html() {
        let toast = Toast(
            &ToastProps::default()
                .with_id("custom")
                .with_variant(shared::VARIANT_INFO)
                .with_duration(100)
                .with_dismissible(true)
                .with_position(shared::POSITION_BOTTOM_RIGHT)
                .with_class("toast-extra")
                .with_attribute("data-testid", "toast")
                .with_aria_label("Custom toast")
                .with_aria_live("assertive")
                .with_aria_described_by("toast-desc")
                .with_role("alert"),
            html! { "Custom" },
        )
        .into_string();
        assert!(toast.contains("toast-extra"));
        assert!(toast.contains(r#"id="custom" aria-label="Custom toast" aria-live="assertive" aria-describedby="toast-desc" role="alert""#));
        assert!(toast.contains(r#"data-testid="toast""#));

        let container = ToastContainer(
            &ToastContainerProps::default()
                .with_position(shared::POSITION_TOP_CENTER)
                .with_class("container-extra")
                .with_attribute("id", "container")
                .with_aria_label("Toasts")
                .with_aria_live("assertive")
                .with_role("log"),
            html! {},
        )
        .into_string();
        assert!(container.contains("top-4 left-1/2"));
        assert!(container.contains("container-extra"));
        assert!(container.contains(r#"aria-label="Toasts" aria-live="assertive" role="log""#));
        assert!(container.contains(r#"id="container""#));

        let title = ToastTitle(
            &ToastTitleProps::default()
                .with_class("title-extra")
                .with_attribute("id", "title"),
            html! { "Title" },
        )
        .into_string();
        assert!(title.contains(r#"class="text-sm font-semibold title-extra" id="title""#));

        let description = ToastDescription(
            &ToastDescriptionProps::default()
                .with_class("description-extra")
                .with_attribute("data-testid", "description"),
            html! { "<Description>" },
        )
        .into_string();
        assert!(description.contains("description-extra"));
        assert!(description.contains(r#"data-testid="description""#));
        assert!(description.contains("&lt;Description&gt;"));

        let icon = ToastIcon(
            &ToastIconProps::default()
                .with_class("icon-extra")
                .with_attribute("id", "icon"),
            html! { svg {} },
        )
        .into_string();
        assert!(icon.contains(r#"class="h-5 w-5 flex-shrink-0 icon-extra" id="icon""#));

        let close = ToastClose(
            &ToastCloseProps::default()
                .with_toast_id("custom")
                .with_class("close-extra")
                .with_attribute("id", "close")
                .with_aria_label("Dismiss"),
        )
        .into_string();
        assert!(close.contains("close-extra"));
        assert!(close.contains(r#"aria-label="Dismiss""#));
        assert!(close.contains(r#"data-toast-close="custom""#));
        assert!(
            close
                .contains(r#"data-on:click="evt.currentTarget.closest('[role=status]').remove()""#)
        );
        assert!(close.contains(r#"id="close""#));

        let with_icon = ToastWithIcon(
            "with-icon",
            shared::VARIANT_WARNING,
            "Warn",
            "Detailed warning",
            10,
            true,
            html! { svg data-testid="warn" {} },
        )
        .into_string();
        assert!(with_icon.contains("<div class=\"text-sm opacity-90\">Detailed warning</div>"));
        assert!(with_icon.contains(r#"data-testid="warn""#));
    }
}
