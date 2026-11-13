#![allow(non_snake_case)]

use crate::components::{ComponentInfo, shared};
use maud::{Markup, html};
use std::collections::BTreeMap;

pub const COMPONENTS: &[ComponentInfo] = &[
    ComponentInfo {
        family: "unstyled",
        module: "confirmdialog",
        name: "ConfirmDialog",
    },
    ComponentInfo {
        family: "unstyled",
        module: "confirmdialog",
        name: "ConfirmDialogWithForm",
    },
    ComponentInfo {
        family: "unstyled",
        module: "confirmdialog",
        name: "ConfirmDialogWithDynamicForm",
    },
    ComponentInfo {
        family: "unstyled",
        module: "confirmdialog",
        name: "ConfirmDeleteButton",
    },
];

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConfirmDialogProps {
    pub id: String,
    pub title: String,
    pub message: String,
    pub confirm_text: String,
    pub cancel_text: String,
    pub confirm_variant: &'static str,
    pub class: String,
    pub attributes: BTreeMap<String, String>,
    pub aria_label: String,
    pub aria_described_by: String,
    pub aria_labelled_by: String,
    pub role: String,
}

impl Default for ConfirmDialogProps {
    fn default() -> Self {
        Self {
            id: String::new(),
            title: String::new(),
            message: String::new(),
            confirm_text: String::new(),
            cancel_text: String::new(),
            confirm_variant: shared::VARIANT_DESTRUCTIVE,
            class: String::new(),
            attributes: BTreeMap::new(),
            aria_label: String::new(),
            aria_described_by: String::new(),
            aria_labelled_by: String::new(),
            role: "alertdialog".to_string(),
        }
    }
}

impl ConfirmDialogProps {
    #[must_use]
    pub fn with_id(mut self, value: impl Into<String>) -> Self {
        self.id = value.into();
        self
    }
    #[must_use]
    pub fn with_title(mut self, value: impl Into<String>) -> Self {
        self.title = value.into();
        self
    }
    #[must_use]
    pub fn with_message(mut self, value: impl Into<String>) -> Self {
        self.message = value.into();
        self
    }
    #[must_use]
    pub fn with_confirm_text(mut self, value: impl Into<String>) -> Self {
        self.confirm_text = value.into();
        self
    }
    #[must_use]
    pub fn with_cancel_text(mut self, value: impl Into<String>) -> Self {
        self.cancel_text = value.into();
        self
    }
    #[must_use]
    pub const fn with_confirm_variant(mut self, value: &'static str) -> Self {
        self.confirm_variant = value;
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
    pub fn with_aria_labelled_by(mut self, value: impl Into<String>) -> Self {
        self.aria_labelled_by = value.into();
        self
    }
    #[must_use]
    pub fn with_role(mut self, value: impl Into<String>) -> Self {
        self.role = value.into();
        self
    }

    #[must_use]
    pub fn get_signal_name(&self) -> String {
        if self.id.is_empty() {
            "confirmDialogOpen".to_string()
        } else {
            format!("{}Open", self.id)
        }
    }
    #[must_use]
    pub fn get_action_signal_name(&self) -> String {
        if self.id.is_empty() {
            "confirmDialogAction".to_string()
        } else {
            format!("{}Action", self.id)
        }
    }
    #[must_use]
    pub fn get_data_signals(&self) -> String {
        format!(
            "{{ {}: false, {}: '' }}",
            self.get_signal_name(),
            self.get_action_signal_name()
        )
    }
    #[must_use]
    pub fn build_classes(&self) -> String {
        shared::build_classes(&[
            "relative bg-background rounded-lg shadow-lg p-6 max-w-md w-full mx-4 z-10",
            self.class.as_str(),
        ])
    }
    #[must_use]
    pub const fn build_overlay_classes(&self) -> &'static str {
        "fixed inset-0 bg-overlay-bg transition-opacity"
    }
    #[must_use]
    pub const fn build_container_classes(&self) -> &'static str {
        "fixed inset-0 z-50 flex items-center justify-center"
    }
    #[must_use]
    pub fn build_attributes(&self) -> BTreeMap<String, String> {
        let mut attrs = self.attributes.clone();
        let role = if self.role.is_empty() {
            "alertdialog"
        } else {
            self.role.as_str()
        };
        attrs.insert("role".to_string(), role.to_string());
        attrs.insert("aria-modal".to_string(), "true".to_string());
        if !self.aria_label.is_empty() {
            attrs.insert("aria-label".to_string(), self.aria_label.clone());
        }
        if !self.aria_described_by.is_empty() {
            attrs.insert(
                "aria-describedby".to_string(),
                self.aria_described_by.clone(),
            );
        } else if !self.message.is_empty() {
            attrs.insert("aria-describedby".to_string(), self.get_description_id());
        }
        if !self.aria_labelled_by.is_empty() {
            attrs.insert("aria-labelledby".to_string(), self.aria_labelled_by.clone());
        } else if !self.title.is_empty() {
            attrs.insert("aria-labelledby".to_string(), self.get_title_id());
        }
        attrs
    }
    #[must_use]
    pub fn get_confirm_button_classes(&self) -> String {
        let variant = if self.confirm_variant.is_empty() {
            shared::VARIANT_DESTRUCTIVE
        } else {
            self.confirm_variant
        };
        shared::build_classes(&[
            "px-4 py-2 rounded-md font-medium transition-colors duration-150",
            shared::button_variant_classes(variant),
        ])
    }
    #[must_use]
    pub const fn get_cancel_button_classes(&self) -> &'static str {
        "px-4 py-2 rounded-md border border-input bg-background hover:bg-accent hover:text-accent-foreground transition-colors duration-150"
    }
    #[must_use]
    pub fn get_confirm_text(&self) -> String {
        self.confirm_text.clone()
    }
    #[must_use]
    pub fn get_cancel_text(&self) -> String {
        self.cancel_text.clone()
    }
    #[must_use]
    pub fn get_title_id(&self) -> String {
        if self.id.is_empty() {
            "confirmDialogTitle".to_string()
        } else {
            format!("{}Title", self.id)
        }
    }
    #[must_use]
    pub fn get_description_id(&self) -> String {
        if self.id.is_empty() {
            "confirmDialogDescription".to_string()
        } else {
            format!("{}Description", self.id)
        }
    }
}

fn attr<'a>(attrs: &'a BTreeMap<String, String>, name: &str) -> Option<&'a str> {
    shared::attr_value(attrs, name)
}

fn close_expr(props: &ConfirmDialogProps) -> String {
    format!("${} = false", props.get_signal_name())
}

fn render_shell(props: &ConfirmDialogProps, body: Markup) -> Markup {
    let attrs = props.build_attributes();
    html! {
        div
            class=(props.build_container_classes())
            data-show=(format!("${}", props.get_signal_name()))
            style="display: none"
            role=[attr(&attrs, "role")]
            aria-modal=[attr(&attrs, "aria-modal")]
            aria-label=[attr(&attrs, "aria-label")]
            aria-describedby=[attr(&attrs, "aria-describedby")]
            aria-labelledby=[attr(&attrs, "aria-labelledby")]
            data-testid=[attr(&attrs, "data-testid")]
        {
            div class=(props.build_overlay_classes()) data-on:click=(close_expr(props)) {}
            div class=(props.build_classes()) {
                @if !props.title.is_empty() {
                    h3 id=(props.get_title_id()) class="text-lg font-semibold text-foreground mb-2" { (&props.title) }
                }
                @if !props.message.is_empty() {
                    p id=(props.get_description_id()) class="text-muted-foreground mb-6" { (&props.message) }
                }
                div class="flex justify-end gap-3" {
                    button type="button" class=(props.get_cancel_button_classes()) data-on:click=(close_expr(props)) {
                        (props.get_cancel_text())
                    }
                    (body)
                }
            }
        }
    }
}

#[must_use]
pub fn ConfirmDialog(props: &ConfirmDialogProps, children: Markup) -> Markup {
    render_shell(props, children)
}

#[must_use]
pub fn ConfirmDialogWithForm(
    props: &ConfirmDialogProps,
    form_action: &str,
    csrf_token: &str,
    csrf_field_name: &str,
) -> Markup {
    render_shell(
        props,
        html! {
            form method="POST" action=(form_action) class="inline" {
                input type="hidden" name=(csrf_field_name) value=(csrf_token) {}
                button type="submit" class=(props.get_confirm_button_classes()) {
                    (props.get_confirm_text())
                }
            }
        },
    )
}

#[must_use]
pub fn ConfirmDialogWithDynamicForm(
    props: &ConfirmDialogProps,
    csrf_token: &str,
    csrf_field_name: &str,
) -> Markup {
    render_shell(
        props,
        html! {
            form method="POST" class="inline" data-attr:action=(format!("${}", props.get_action_signal_name())) {
                input type="hidden" name=(csrf_field_name) value=(csrf_token) {}
                button type="submit" class=(props.get_confirm_button_classes()) data-on:click=(close_expr(props)) {
                    (props.get_confirm_text())
                }
            }
        },
    )
}

#[must_use]
pub fn buildDeleteButtonClasses(custom_class: &str) -> String {
    let base = "inline-flex items-center justify-center rounded-md text-sm font-medium transition-colors duration-150 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 disabled:opacity-50 disabled:pointer-events-none bg-destructive text-destructive-foreground hover:bg-destructive/90";
    if custom_class.is_empty() {
        format!("{base} h-9 px-4 py-2")
    } else {
        format!("{base} {custom_class}")
    }
}

#[must_use]
pub fn ConfirmDeleteButton(
    dialog_id: &str,
    action_url: &str,
    button_class: &str,
    button_text: &str,
) -> Markup {
    html! {
        button
            type="button"
            class=(buildDeleteButtonClasses(button_class))
            data-on:click=(format!("${dialog_id}Action = '{action_url}'; ${dialog_id}Open = true"))
        {
            (button_text)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn props() -> ConfirmDialogProps {
        ConfirmDialogProps::default()
            .with_id("deleteUser")
            .with_title("<Delete>")
            .with_message("<Really?>")
            .with_confirm_text("Delete")
            .with_cancel_text("Cancel")
            .with_class("extra")
            .with_attribute("data-testid", "confirm")
    }

    #[test]
    fn confirm_props_helpers_cover_defaults_signals_classes_attrs_and_text() {
        let default_props = ConfirmDialogProps::default();
        assert_eq!(default_props.confirm_variant, shared::VARIANT_DESTRUCTIVE);
        assert_eq!(default_props.role, "alertdialog");
        assert_eq!(default_props.get_signal_name(), "confirmDialogOpen");
        assert_eq!(
            default_props.get_action_signal_name(),
            "confirmDialogAction"
        );
        assert_eq!(
            default_props.get_data_signals(),
            "{ confirmDialogOpen: false, confirmDialogAction: '' }"
        );
        assert_eq!(default_props.get_title_id(), "confirmDialogTitle");
        assert_eq!(
            default_props.get_description_id(),
            "confirmDialogDescription"
        );
        assert_eq!(
            default_props.build_overlay_classes(),
            "fixed inset-0 bg-overlay-bg transition-opacity"
        );
        assert_eq!(
            default_props.build_container_classes(),
            "fixed inset-0 z-50 flex items-center justify-center"
        );
        assert!(
            default_props
                .get_confirm_button_classes()
                .contains("bg-destructive")
        );
        assert_eq!(
            default_props.get_cancel_button_classes(),
            "px-4 py-2 rounded-md border border-input bg-background hover:bg-accent hover:text-accent-foreground transition-colors duration-150"
        );
        assert_eq!(default_props.get_confirm_text(), "");
        assert_eq!(default_props.get_cancel_text(), "");

        let configured = props()
            .with_confirm_variant(shared::VARIANT_PRIMARY)
            .with_aria_label("Confirm")
            .with_aria_described_by("desc")
            .with_aria_labelled_by("title")
            .with_role("dialog");
        assert_eq!(configured.get_signal_name(), "deleteUserOpen");
        assert_eq!(configured.get_action_signal_name(), "deleteUserAction");
        assert_eq!(
            configured.get_data_signals(),
            "{ deleteUserOpen: false, deleteUserAction: '' }"
        );
        assert_eq!(configured.get_title_id(), "deleteUserTitle");
        assert_eq!(configured.get_description_id(), "deleteUserDescription");
        assert!(configured.build_classes().contains("extra"));
        assert!(
            configured
                .get_confirm_button_classes()
                .contains("btn-primary-gradient")
        );
        let attrs = configured.build_attributes();
        assert_eq!(attr(&attrs, "role"), Some("dialog"));
        assert_eq!(attr(&attrs, "aria-modal"), Some("true"));
        assert_eq!(attr(&attrs, "aria-label"), Some("Confirm"));
        assert_eq!(attr(&attrs, "aria-describedby"), Some("desc"));
        assert_eq!(attr(&attrs, "aria-labelledby"), Some("title"));
        assert_eq!(attr(&attrs, "data-testid"), Some("confirm"));
        assert_eq!(
            attr(
                &ConfirmDialogProps::default()
                    .with_role("")
                    .build_attributes(),
                "role"
            ),
            Some("alertdialog")
        );
        assert!(
            ConfirmDialogProps::default()
                .with_confirm_variant("")
                .get_confirm_button_classes()
                .contains("bg-destructive")
        );

        let auto_attrs = props().build_attributes();
        assert_eq!(
            attr(&auto_attrs, "aria-describedby"),
            Some("deleteUserDescription")
        );
        assert_eq!(
            attr(&auto_attrs, "aria-labelledby"),
            Some("deleteUserTitle")
        );
        assert_eq!(close_expr(&props()), "$deleteUserOpen = false");
        assert!(buildDeleteButtonClasses("").ends_with("h-9 px-4 py-2"));
        assert!(buildDeleteButtonClasses("custom").ends_with("custom"));
    }

    #[test]
    fn confirm_dialog_renders_shell_children_and_aria() {
        let html = ConfirmDialog(
            &props(),
            html! {
                button type="button" class="confirm" { "Confirm" }
            },
        )
        .into_string();
        assert!(
            html.starts_with("<div class=\"fixed inset-0 z-50 flex items-center justify-center\"")
        );
        assert!(html.contains("data-show=\"$deleteUserOpen\""));
        assert!(html.contains("style=\"display: none\""));
        assert!(html.contains("role=\"alertdialog\""));
        assert!(html.contains("aria-modal=\"true\""));
        assert!(html.contains("aria-describedby=\"deleteUserDescription\""));
        assert!(html.contains("aria-labelledby=\"deleteUserTitle\""));
        assert!(html.contains("data-testid=\"confirm\""));
        assert!(html.contains("data-on:click=\"$deleteUserOpen = false\""));
        assert!(html.contains("<h3 id=\"deleteUserTitle\" class=\"text-lg font-semibold text-foreground mb-2\">&lt;Delete&gt;</h3>"));
        assert!(html.contains("<p id=\"deleteUserDescription\" class=\"text-muted-foreground mb-6\">&lt;Really?&gt;</p>"));
        assert!(
            html.contains(
                "<button type=\"button\" class=\"px-4 py-2 rounded-md border border-input"
            )
        );
        assert!(html.contains(">Cancel</button>"));
        assert!(html.contains("<button type=\"button\" class=\"confirm\">Confirm</button>"));

        let empty = ConfirmDialog(
            &ConfirmDialogProps::default().with_cancel_text("No"),
            html! {},
        )
        .into_string();
        assert!(!empty.contains("<h3"));
        assert!(!empty.contains("<p id=\"confirmDialogDescription\""));
    }

    #[test]
    fn form_variations_and_delete_trigger_render_expected_datastar_and_html() {
        let form = ConfirmDialogWithForm(&props(), "/delete", "<csrf>", "csrf_token").into_string();
        assert!(form.contains("<form method=\"POST\" action=\"/delete\" class=\"inline\">"));
        assert!(
            form.contains("<input type=\"hidden\" name=\"csrf_token\" value=\"&lt;csrf&gt;\">")
        );
        assert!(form.contains("<button type=\"submit\" class=\"px-4 py-2 rounded-md font-medium"));
        assert!(form.contains(">Delete</button>"));

        let dynamic = ConfirmDialogWithDynamicForm(&props(), "csrf", "csrf_token").into_string();
        assert!(dynamic.contains(
            "<form method=\"POST\" class=\"inline\" data-attr:action=\"$deleteUserAction\">"
        ));
        assert!(dynamic.contains("data-on:click=\"$deleteUserOpen = false\""));
        assert!(dynamic.contains("<input type=\"hidden\" name=\"csrf_token\" value=\"csrf\">"));

        let trigger = ConfirmDeleteButton("deleteUser", "/delete/1", "", "<Delete>").into_string();
        assert!(trigger.starts_with(
            "<button type=\"button\" class=\"inline-flex items-center justify-center rounded-md"
        ));
        assert!(trigger.contains("bg-destructive text-destructive-foreground"));
        assert!(trigger.contains("h-9 px-4 py-2"));
        assert!(trigger.contains("data-on:click="));
        assert!(trigger.contains("$deleteUserAction ="));
        assert!(trigger.contains("/delete/1"));
        assert!(trigger.contains("$deleteUserOpen = true"));
        assert!(trigger.contains("&lt;Delete&gt;</button>"));

        let custom =
            ConfirmDeleteButton("deleteUser", "/delete/1", "custom", "Delete").into_string();
        assert!(custom.contains("custom"));
        assert!(!custom.contains("h-9 px-4 py-2"));
    }
}
