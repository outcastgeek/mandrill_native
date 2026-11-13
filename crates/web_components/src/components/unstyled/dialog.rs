#![allow(non_snake_case)]

use crate::components::{ComponentInfo, shared};
use maud::{Markup, html};
use std::collections::BTreeMap;

pub const COMPONENTS: &[ComponentInfo] = &[
    ComponentInfo {
        family: "unstyled",
        module: "dialog",
        name: "Dialog",
    },
    ComponentInfo {
        family: "unstyled",
        module: "dialog",
        name: "DialogTrigger",
    },
    ComponentInfo {
        family: "unstyled",
        module: "dialog",
        name: "DialogOverlay",
    },
    ComponentInfo {
        family: "unstyled",
        module: "dialog",
        name: "DialogContent",
    },
    ComponentInfo {
        family: "unstyled",
        module: "dialog",
        name: "DialogHeader",
    },
    ComponentInfo {
        family: "unstyled",
        module: "dialog",
        name: "DialogTitle",
    },
    ComponentInfo {
        family: "unstyled",
        module: "dialog",
        name: "DialogDescription",
    },
    ComponentInfo {
        family: "unstyled",
        module: "dialog",
        name: "DialogFooter",
    },
    ComponentInfo {
        family: "unstyled",
        module: "dialog",
        name: "DialogClose",
    },
    ComponentInfo {
        family: "unstyled",
        module: "dialog",
        name: "SimpleDialog",
    },
    ComponentInfo {
        family: "unstyled",
        module: "dialog",
        name: "DatastarDialog",
    },
    ComponentInfo {
        family: "unstyled",
        module: "dialog",
        name: "ConfirmDialog",
    },
];

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DialogProps {
    pub id: String,
    pub open: bool,
    pub class: String,
    pub attributes: BTreeMap<String, String>,
    pub aria_label: String,
    pub aria_described_by: String,
    pub aria_labelled_by: String,
    pub role: String,
    pub signal: String,
    pub after_close_action: String,
}

impl Default for DialogProps {
    fn default() -> Self {
        Self {
            id: String::new(),
            open: false,
            class: String::new(),
            attributes: BTreeMap::new(),
            aria_label: String::new(),
            aria_described_by: String::new(),
            aria_labelled_by: String::new(),
            role: "dialog".to_string(),
            signal: String::new(),
            after_close_action: String::new(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DialogTriggerProps {
    pub dialog_id: String,
    pub class: String,
    pub attributes: BTreeMap<String, String>,
    pub type_attr: String,
    pub aria_label: String,
    pub signal: String,
}

impl Default for DialogTriggerProps {
    fn default() -> Self {
        Self {
            dialog_id: String::new(),
            class: String::new(),
            attributes: BTreeMap::new(),
            type_attr: "button".to_string(),
            aria_label: String::new(),
            signal: String::new(),
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DialogOverlayProps {
    pub dialog_id: String,
    pub class: String,
    pub attributes: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DialogContentProps {
    pub dialog_id: String,
    pub size: &'static str,
    pub disable_full_width: bool,
    pub class: String,
    pub attributes: BTreeMap<String, String>,
    pub aria_label: String,
    pub aria_described_by: String,
    pub aria_labelled_by: String,
    pub role: String,
}

impl Default for DialogContentProps {
    fn default() -> Self {
        Self {
            dialog_id: String::new(),
            size: shared::SIZE_MD,
            disable_full_width: false,
            class: String::new(),
            attributes: BTreeMap::new(),
            aria_label: String::new(),
            aria_described_by: String::new(),
            aria_labelled_by: String::new(),
            role: "document".to_string(),
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DialogHeaderProps {
    pub class: String,
    pub attributes: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DialogTitleProps {
    pub level: String,
    pub id: String,
    pub class: String,
    pub attributes: BTreeMap<String, String>,
}

impl Default for DialogTitleProps {
    fn default() -> Self {
        Self {
            level: "h2".to_string(),
            id: String::new(),
            class: String::new(),
            attributes: BTreeMap::new(),
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DialogDescriptionProps {
    pub id: String,
    pub class: String,
    pub attributes: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DialogFooterProps {
    pub class: String,
    pub attributes: BTreeMap<String, String>,
    pub align: String,
}

impl Default for DialogFooterProps {
    fn default() -> Self {
        Self {
            class: String::new(),
            attributes: BTreeMap::new(),
            align: "right".to_string(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DialogCloseProps {
    pub dialog_id: String,
    pub class: String,
    pub attributes: BTreeMap<String, String>,
    pub aria_label: String,
    pub show_icon: bool,
    pub signal: String,
    pub after_close_action: String,
}

impl Default for DialogCloseProps {
    fn default() -> Self {
        Self {
            dialog_id: String::new(),
            class: String::new(),
            attributes: BTreeMap::new(),
            aria_label: String::new(),
            show_icon: true,
            signal: String::new(),
            after_close_action: String::new(),
        }
    }
}

impl DialogProps {
    #[must_use]
    pub fn with_id(mut self, value: impl Into<String>) -> Self {
        self.id = value.into();
        self
    }
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
    pub fn with_signal(mut self, value: impl Into<String>) -> Self {
        self.signal = value.into();
        self
    }
    #[must_use]
    pub fn with_after_close_action(mut self, value: impl Into<String>) -> Self {
        self.after_close_action = value.into();
        self
    }
    #[must_use]
    pub fn build_classes(&self) -> String {
        if !self.signal.is_empty() || self.open {
            shared::build_classes(&["fixed inset-0 z-50", self.class.as_str()])
        } else {
            shared::build_classes(&["fixed inset-0 z-50 hidden", self.class.as_str()])
        }
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
        if !self.aria_described_by.is_empty() {
            attrs.insert(
                "aria-describedby".to_string(),
                self.aria_described_by.clone(),
            );
        }
        if !self.aria_labelled_by.is_empty() {
            attrs.insert("aria-labelledby".to_string(), self.aria_labelled_by.clone());
        }
        if !self.role.is_empty() {
            attrs.insert("role".to_string(), self.role.clone());
        }
        attrs.insert("data-dialog".to_string(), "true".to_string());
        attrs.insert("aria-modal".to_string(), "true".to_string());
        if !self.signal.is_empty() {
            attrs.insert("data-show".to_string(), format!("${}", self.signal));
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
                    "if (evt.key === 'Escape' && ${}) {{ {} }}",
                    self.signal,
                    buildDialogCloseAction(&self.signal, &self.after_close_action)
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

impl DialogTriggerProps {
    #[must_use]
    pub fn with_dialog_id(mut self, value: impl Into<String>) -> Self {
        self.dialog_id = value.into();
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
    pub fn with_type(mut self, value: impl Into<String>) -> Self {
        self.type_attr = value.into();
        self
    }
    #[must_use]
    pub fn with_aria_label(mut self, value: impl Into<String>) -> Self {
        self.aria_label = value.into();
        self
    }
    #[must_use]
    pub fn with_signal(mut self, value: impl Into<String>) -> Self {
        self.signal = value.into();
        self
    }
    #[must_use]
    pub fn build_classes(&self) -> String {
        shared::build_classes(&["cursor-pointer", self.class.as_str()])
    }
    #[must_use]
    pub fn build_trigger_attributes(&self) -> BTreeMap<String, String> {
        let mut attrs = self.attributes.clone();
        if !self.type_attr.is_empty() {
            attrs.insert("type".to_string(), self.type_attr.clone());
        }
        attrs.insert("aria-haspopup".to_string(), "dialog".to_string());
        if !self.dialog_id.is_empty() {
            attrs.insert("data-dialog-trigger".to_string(), self.dialog_id.clone());
            attrs.insert("aria-controls".to_string(), self.dialog_id.clone());
        }
        if !self.signal.is_empty() {
            attrs.insert(
                "data-on:click".to_string(),
                format!("${} = true", self.signal),
            );
        }
        attrs
    }
}

impl DialogOverlayProps {
    #[must_use]
    pub fn with_dialog_id(mut self, value: impl Into<String>) -> Self {
        self.dialog_id = value.into();
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
            "fixed inset-0 bg-overlay-bg backdrop-blur-sm animate-overlay-in",
            self.class.as_str(),
        ])
    }
}

impl DialogContentProps {
    #[must_use]
    pub fn with_dialog_id(mut self, value: impl Into<String>) -> Self {
        self.dialog_id = value.into();
        self
    }
    #[must_use]
    pub const fn with_size(mut self, value: &'static str) -> Self {
        self.size = value;
        self
    }
    #[must_use]
    pub const fn with_disable_full_width(mut self, value: bool) -> Self {
        self.disable_full_width = value;
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
    pub fn build_classes(&self) -> String {
        let mut base = "fixed left-1/2 top-1/2 z-50 -translate-x-1/2 -translate-y-1/2 rounded-lg border bg-card text-card-foreground p-6 shadow-lg animate-dialog-in dialog-mobile-sheet".to_string();
        if !self.disable_full_width {
            base.push_str(" w-full");
        }
        let size = match self.size {
            shared::SIZE_SM => "max-w-sm dialog-sm",
            shared::SIZE_LG => "max-w-2xl dialog-lg",
            _ => "max-w-lg dialog-md",
        };
        shared::build_classes(&[base.as_str(), size, self.class.as_str()])
    }
    #[must_use]
    pub fn build_content_attributes(&self) -> BTreeMap<String, String> {
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
        if !self.aria_labelled_by.is_empty() {
            attrs.insert("aria-labelledby".to_string(), self.aria_labelled_by.clone());
        }
        if !self.role.is_empty() {
            attrs.insert("role".to_string(), self.role.clone());
        }
        if !self.dialog_id.is_empty() {
            attrs.insert("data-dialog-content".to_string(), self.dialog_id.clone());
        }
        attrs.insert("tabindex".to_string(), "-1".to_string());
        attrs
    }
}

impl DialogHeaderProps {
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
            "flex flex-col space-y-1.5 text-center sm:text-left mb-4",
            self.class.as_str(),
        ])
    }
}
impl DialogTitleProps {
    #[must_use]
    pub fn with_level(mut self, value: impl Into<String>) -> Self {
        self.level = value.into();
        self
    }
    #[must_use]
    pub fn with_id(mut self, value: impl Into<String>) -> Self {
        self.id = value.into();
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
            "dialog-title text-lg font-semibold leading-none tracking-tight font-heading",
            self.class.as_str(),
        ])
    }
}
impl DialogDescriptionProps {
    #[must_use]
    pub fn with_id(mut self, value: impl Into<String>) -> Self {
        self.id = value.into();
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
        shared::build_classes(&["text-sm text-muted-foreground", self.class.as_str()])
    }
}
impl DialogFooterProps {
    #[must_use]
    pub fn with_class(mut self, value: impl Into<String>) -> Self {
        self.class = value.into();
        self
    }
    #[must_use]
    pub fn with_align(mut self, value: impl Into<String>) -> Self {
        self.align = value.into();
        self
    }
    #[must_use]
    pub fn with_attribute(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.attributes.insert(name.into(), value.into());
        self
    }
    #[must_use]
    pub fn build_classes(&self) -> String {
        let align = match self.align.as_str() {
            "left" => "justify-start",
            "center" => "justify-center",
            "between" => "justify-between",
            "right" => "justify-end",
            _ => "justify-end gap-2",
        };
        shared::build_classes(&["flex mt-6", align, self.class.as_str()])
    }
}

impl DialogCloseProps {
    #[must_use]
    pub fn with_dialog_id(mut self, value: impl Into<String>) -> Self {
        self.dialog_id = value.into();
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
    pub const fn with_show_icon(mut self, value: bool) -> Self {
        self.show_icon = value;
        self
    }
    #[must_use]
    pub fn with_signal(mut self, value: impl Into<String>) -> Self {
        self.signal = value.into();
        self
    }
    #[must_use]
    pub fn with_after_close_action(mut self, value: impl Into<String>) -> Self {
        self.after_close_action = value.into();
        self
    }
    #[must_use]
    pub fn build_classes(&self) -> String {
        shared::build_classes(&[
            "dialog-close absolute right-3 top-3 inline-flex items-center justify-center p-1.5 rounded-md border-none bg-transparent text-muted-foreground cursor-pointer transition-colors duration-150 hover:bg-muted hover:text-foreground focus-visible:outline-2 focus-visible:outline-ring focus-visible:outline-offset-2",
            self.class.as_str(),
        ])
    }
    #[must_use]
    pub fn build_close_attributes(&self) -> BTreeMap<String, String> {
        let mut attrs = self.attributes.clone();
        attrs.insert("type".to_string(), "button".to_string());
        if !self.dialog_id.is_empty() {
            attrs.insert("data-dialog-close".to_string(), self.dialog_id.clone());
        }
        if !self.signal.is_empty() {
            attrs.insert(
                "data-on:click".to_string(),
                buildDialogCloseAction(&self.signal, &self.after_close_action),
            );
        }
        attrs
    }
}

#[must_use]
pub fn appendDisplayNone(existing: Option<&str>) -> String {
    match existing.filter(|value| !value.is_empty()) {
        Some(value) if value.ends_with(';') => format!("{value}display:none"),
        Some(value) => format!("{value}; display:none"),
        None => "display:none".to_string(),
    }
}

#[must_use]
pub fn buildDialogCloseAction(signal: &str, after: &str) -> String {
    let base = format!("${signal} = false");
    let after = after.trim();
    if after.is_empty() {
        base
    } else {
        format!("{base}; {after}")
    }
}

#[must_use]
pub fn resolveDialogTriggerAriaLabel(props: &DialogTriggerProps) -> String {
    if props.aria_label.is_empty() {
        "Open dialog".to_string()
    } else {
        props.aria_label.clone()
    }
}

#[must_use]
pub fn resolveDialogCloseAriaLabel(props: &DialogCloseProps) -> String {
    if props.aria_label.is_empty() {
        shared::default_strings()
            .dialog_close_aria_label
            .to_string()
    } else {
        props.aria_label.clone()
    }
}

#[must_use]
pub fn Dialog(props: &DialogProps, children: Markup) -> Markup {
    let attrs = props.build_attributes();
    html! {
        div
            class=(props.build_classes())
            id=[shared::attr_value(&attrs, "id")]
            aria-label=[shared::attr_value(&attrs, "aria-label")]
            aria-describedby=[shared::attr_value(&attrs, "aria-describedby")]
            aria-labelledby=[shared::attr_value(&attrs, "aria-labelledby")]
            role=[shared::attr_value(&attrs, "role")]
            data-dialog=[shared::attr_value(&attrs, "data-dialog")]
            aria-modal=[shared::attr_value(&attrs, "aria-modal")]
            data-show=[shared::attr_value(&attrs, "data-show")]
            style=[shared::attr_value(&attrs, "style")]
            aria-hidden=[shared::attr_value(&attrs, "aria-hidden")]
            data-on:keydown__window=[shared::attr_value(&attrs, "data-on:keydown__window")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
        { (children) }
    }
}

#[must_use]
pub fn DialogTrigger(props: &DialogTriggerProps, children: Markup) -> Markup {
    let attrs = props.build_trigger_attributes();
    let label = resolveDialogTriggerAriaLabel(props);
    html! {
        button
            class=(props.build_classes())
            aria-label=(label)
            type=[shared::attr_value(&attrs, "type")]
            aria-haspopup=[shared::attr_value(&attrs, "aria-haspopup")]
            data-dialog-trigger=[shared::attr_value(&attrs, "data-dialog-trigger")]
            aria-controls=[shared::attr_value(&attrs, "aria-controls")]
            data-on:click=[shared::attr_value(&attrs, "data-on:click")]
            id=[shared::attr_value(&attrs, "id")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
        { (children) }
    }
}

#[must_use]
pub fn DialogOverlay(props: &DialogOverlayProps) -> Markup {
    html! {
        div
            class=(props.build_classes())
            data-dialog-overlay=(&props.dialog_id)
            id=[shared::attr_value(&props.attributes, "id")]
            data-testid=[shared::attr_value(&props.attributes, "data-testid")]
            style=[shared::attr_value(&props.attributes, "style")]
            data-on:click=[shared::attr_value(&props.attributes, "data-on:click")]
        {}
    }
}

#[must_use]
pub fn DialogContent(props: &DialogContentProps, children: Markup) -> Markup {
    let attrs = props.build_content_attributes();
    html! {
        div
            class=(props.build_classes())
            aria-label=[shared::attr_value(&attrs, "aria-label")]
            aria-describedby=[shared::attr_value(&attrs, "aria-describedby")]
            aria-labelledby=[shared::attr_value(&attrs, "aria-labelledby")]
            role=[shared::attr_value(&attrs, "role")]
            data-dialog-content=[shared::attr_value(&attrs, "data-dialog-content")]
            tabindex=[shared::attr_value(&attrs, "tabindex")]
            id=[shared::attr_value(&attrs, "id")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
            style=[shared::attr_value(&attrs, "style")]
        { (children) }
    }
}

#[must_use]
pub fn DialogHeader(props: &DialogHeaderProps, children: Markup) -> Markup {
    html! { div class=(props.build_classes()) id=[shared::attr_value(&props.attributes, "id")] data-testid=[shared::attr_value(&props.attributes, "data-testid")] { (children) } }
}

#[must_use]
pub fn DialogTitle(props: &DialogTitleProps, children: Markup) -> Markup {
    let class = props.build_classes();
    match props.level.as_str() {
        "h1" => {
            html! { h1 id=[shared::optional_attr(&props.id)] class=(class) { (children) } }
        }
        "h3" => {
            html! { h3 id=[shared::optional_attr(&props.id)] class=(class) { (children) } }
        }
        "h4" => {
            html! { h4 id=[shared::optional_attr(&props.id)] class=(class) { (children) } }
        }
        "h5" => {
            html! { h5 id=[shared::optional_attr(&props.id)] class=(class) { (children) } }
        }
        "h6" => {
            html! { h6 id=[shared::optional_attr(&props.id)] class=(class) { (children) } }
        }
        _ => {
            html! { h2 id=[shared::optional_attr(&props.id)] class=(class) { (children) } }
        }
    }
}

#[must_use]
pub fn DialogDescription(props: &DialogDescriptionProps, children: Markup) -> Markup {
    html! { p id=[shared::optional_attr(&props.id)] class=(props.build_classes()) data-testid=[shared::attr_value(&props.attributes, "data-testid")] { (children) } }
}
#[must_use]
pub fn DialogFooter(props: &DialogFooterProps, children: Markup) -> Markup {
    html! { div class=(props.build_classes()) id=[shared::attr_value(&props.attributes, "id")] data-testid=[shared::attr_value(&props.attributes, "data-testid")] { (children) } }
}

#[must_use]
pub fn DialogClose(props: &DialogCloseProps, children: Markup) -> Markup {
    let attrs = props.build_close_attributes();
    let label = resolveDialogCloseAriaLabel(props);
    html! {
        button
            class=(props.build_classes())
            aria-label=(label)
            type=[shared::attr_value(&attrs, "type")]
            data-dialog-close=[shared::attr_value(&attrs, "data-dialog-close")]
            data-on:click=[shared::attr_value(&attrs, "data-on:click")]
            id=[shared::attr_value(&attrs, "id")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
            style=[shared::attr_value(&attrs, "style")]
        {
            @if props.show_icon {
                svg class="h-4 w-4" fill="none" stroke="currentColor" viewBox="0 0 24 24" xmlns="http://www.w3.org/2000/svg" {
                    path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M6 18L18 6M6 6l12 12" {}
                }
            } @else {
                (children)
            }
        }
    }
}

#[must_use]
pub fn SimpleDialog(dialog_id: &str, title: &str, description: &str, children: Markup) -> Markup {
    Dialog(
        &DialogProps::default()
            .with_id(dialog_id)
            .with_aria_labelled_by(format!("{dialog_id}-title"))
            .with_aria_described_by(format!("{dialog_id}-description"))
            .with_role("dialog"),
        html! {
            (DialogOverlay(&DialogOverlayProps::default().with_dialog_id(dialog_id)))
            (DialogContent(&DialogContentProps::default().with_dialog_id(dialog_id).with_aria_labelled_by(format!("{dialog_id}-title")).with_aria_described_by(format!("{dialog_id}-description")), html! {
                (DialogHeader(&DialogHeaderProps::default(), html! {
                    (DialogTitle(&DialogTitleProps::default().with_level("h2").with_id(format!("{dialog_id}-title")), html! { (title) }))
                    @if !description.is_empty() {
                        (DialogDescription(&DialogDescriptionProps::default().with_id(format!("{dialog_id}-description")), html! { (description) }))
                    }
                }))
                div class="py-4" { (children) }
                (DialogClose(&DialogCloseProps::default().with_dialog_id(dialog_id).with_aria_label(shared::default_strings().dialog_close_aria_label), html! {}))
            }))
        },
    )
}

#[must_use]
pub fn DatastarDialog(
    dialog_id: &str,
    signal: &str,
    title: &str,
    description: &str,
    children: Markup,
) -> Markup {
    Dialog(
        &DialogProps::default()
            .with_id(dialog_id)
            .with_aria_labelled_by(format!("{dialog_id}-title"))
            .with_aria_described_by(format!("{dialog_id}-description"))
            .with_role("dialog")
            .with_signal(signal),
        html! {
            (DialogOverlay(&DialogOverlayProps::default().with_dialog_id(dialog_id)))
            (DialogContent(&DialogContentProps::default().with_dialog_id(dialog_id).with_aria_labelled_by(format!("{dialog_id}-title")).with_aria_described_by(format!("{dialog_id}-description")), html! {
                (DialogHeader(&DialogHeaderProps::default(), html! {
                    (DialogTitle(&DialogTitleProps::default().with_level("h2").with_id(format!("{dialog_id}-title")), html! { (title) }))
                    @if !description.is_empty() {
                        (DialogDescription(&DialogDescriptionProps::default().with_id(format!("{dialog_id}-description")), html! { (description) }))
                    }
                }))
                div class="py-4" { (children) }
                (DialogClose(&DialogCloseProps::default().with_dialog_id(dialog_id).with_aria_label(shared::default_strings().dialog_close_aria_label).with_signal(signal), html! {}))
            }))
        },
    )
}

#[must_use]
pub fn ConfirmDialog(
    dialog_id: &str,
    title: &str,
    description: &str,
    confirm_text: &str,
    cancel_text: &str,
    children: Markup,
) -> Markup {
    Dialog(
        &DialogProps::default()
            .with_id(dialog_id)
            .with_aria_labelled_by(format!("{dialog_id}-title"))
            .with_aria_described_by(format!("{dialog_id}-description"))
            .with_role("alertdialog"),
        html! {
            (DialogOverlay(&DialogOverlayProps::default().with_dialog_id(dialog_id)))
            (DialogContent(&DialogContentProps::default().with_dialog_id(dialog_id).with_aria_labelled_by(format!("{dialog_id}-title")).with_aria_described_by(format!("{dialog_id}-description")).with_role("alertdialog"), html! {
                (DialogHeader(&DialogHeaderProps::default(), html! {
                    (DialogTitle(&DialogTitleProps::default().with_level("h2").with_id(format!("{dialog_id}-title")), html! { (title) }))
                    @if !description.is_empty() {
                        (DialogDescription(&DialogDescriptionProps::default().with_id(format!("{dialog_id}-description")), html! { (description) }))
                    }
                }))
                div class="py-4" { (children) }
                (DialogFooter(&DialogFooterProps::default(), html! {
                    button type="button" class=(shared::build_classes(&[
                        "inline-flex items-center justify-center gap-2 whitespace-nowrap font-medium transition-all duration-150 ease-out cursor-pointer select-none focus-visible:outline-2 focus-visible:outline-ring focus-visible:outline-offset-2 disabled:pointer-events-none disabled:opacity-50 active:scale-[0.98]",
                        shared::button_variant_classes(shared::VARIANT_OUTLINE),
                        shared::button_size_classes(shared::SIZE_SM),
                    ])) data-dialog-close=(dialog_id) { (cancel_text) }
                    button type="button" class=(shared::build_classes(&[
                        "inline-flex items-center justify-center gap-2 whitespace-nowrap font-medium transition-all duration-150 ease-out cursor-pointer select-none focus-visible:outline-2 focus-visible:outline-ring focus-visible:outline-offset-2 disabled:pointer-events-none disabled:opacity-50 active:scale-[0.98]",
                        shared::button_variant_classes(shared::VARIANT_PRIMARY),
                        shared::button_size_classes(shared::SIZE_SM),
                    ])) data-dialog-confirm=(dialog_id) { (confirm_text) }
                }))
                (DialogClose(&DialogCloseProps::default().with_dialog_id(dialog_id).with_aria_label(shared::default_strings().dialog_close_aria_label), html! {}))
            }))
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn helpers_cover_dialog_visibility_signal_and_close_action() {
        assert_eq!(appendDisplayNone(None), "display:none");
        assert_eq!(
            appendDisplayNone(Some("color:red")),
            "color:red; display:none"
        );
        assert_eq!(
            appendDisplayNone(Some("color:red;")),
            "color:red;display:none"
        );
        assert_eq!(buildDialogCloseAction("open", ""), "$open = false");
        assert_eq!(
            buildDialogCloseAction("open", "window.done()"),
            "$open = false; window.done()"
        );
        assert_eq!(
            resolveDialogTriggerAriaLabel(&DialogTriggerProps::default()),
            "Open dialog"
        );
        assert_eq!(
            resolveDialogTriggerAriaLabel(&DialogTriggerProps::default().with_aria_label("Launch")),
            "Launch"
        );
        assert_eq!(
            resolveDialogCloseAriaLabel(&DialogCloseProps::default()),
            "Close dialog"
        );
        assert_eq!(
            resolveDialogCloseAriaLabel(&DialogCloseProps::default().with_aria_label("Dismiss")),
            "Dismiss"
        );

        let closed = DialogProps::default();
        assert_eq!(closed.build_classes(), "fixed inset-0 z-50 hidden");
        assert_eq!(
            closed
                .build_attributes()
                .get("aria-hidden")
                .map(String::as_str),
            Some("true")
        );
        let open = DialogProps::default().with_open(true);
        assert_eq!(open.build_classes(), "fixed inset-0 z-50");
        assert_eq!(
            open.build_attributes()
                .get("aria-hidden")
                .map(String::as_str),
            Some("false")
        );
        let signal = DialogProps::default()
            .with_signal("open")
            .with_attribute("style", "color:red")
            .with_after_close_action("done()");
        let attrs = signal.build_attributes();
        assert_eq!(attrs.get("data-show").map(String::as_str), Some("$open"));
        assert_eq!(
            attrs.get("style").map(String::as_str),
            Some("color:red; display:none")
        );
        assert_eq!(attrs.get("aria-hidden").map(String::as_str), Some("true"));
        assert_eq!(
            attrs.get("data-on:keydown__window").map(String::as_str),
            Some("if (evt.key === 'Escape' && $open) { $open = false; done() }")
        );
    }

    #[test]
    fn helpers_cover_classes_and_attributes_for_all_parts() {
        assert_eq!(
            DialogTriggerProps::default()
                .with_class("trigger")
                .build_classes(),
            "cursor-pointer trigger"
        );
        let trigger_attrs = DialogTriggerProps::default()
            .with_dialog_id("d")
            .with_signal("open")
            .build_trigger_attributes();
        assert_eq!(
            trigger_attrs.get("data-on:click").map(String::as_str),
            Some("$open = true")
        );
        assert_eq!(
            DialogOverlayProps::default()
                .with_class("dark")
                .build_classes(),
            "fixed inset-0 bg-overlay-bg backdrop-blur-sm animate-overlay-in dark"
        );
        assert!(
            DialogContentProps::default()
                .build_classes()
                .contains("w-full max-w-lg dialog-md")
        );
        assert!(
            DialogContentProps::default()
                .with_size(shared::SIZE_SM)
                .build_classes()
                .contains("max-w-sm dialog-sm")
        );
        assert!(
            DialogContentProps::default()
                .with_size(shared::SIZE_LG)
                .with_disable_full_width(true)
                .with_class("content-extra")
                .build_classes()
                .contains("max-w-2xl dialog-lg content-extra")
        );
        assert!(
            !DialogContentProps::default()
                .with_disable_full_width(true)
                .build_classes()
                .contains(" w-full ")
        );
        let content_attrs = DialogContentProps::default()
            .with_dialog_id("d")
            .with_aria_label("Content")
            .build_content_attributes();
        assert_eq!(
            content_attrs.get("role").map(String::as_str),
            Some("document")
        );
        assert_eq!(
            content_attrs.get("data-dialog-content").map(String::as_str),
            Some("d")
        );
        assert_eq!(
            content_attrs.get("tabindex").map(String::as_str),
            Some("-1")
        );
        assert_eq!(
            DialogHeaderProps::default()
                .with_class("head")
                .build_classes(),
            "flex flex-col space-y-1.5 text-center sm:text-left mb-4 head"
        );
        assert_eq!(
            DialogTitleProps::default()
                .with_class("title")
                .build_classes(),
            "dialog-title text-lg font-semibold leading-none tracking-tight font-heading title"
        );
        assert_eq!(
            DialogDescriptionProps::default()
                .with_class("desc")
                .build_classes(),
            "text-sm text-muted-foreground desc"
        );
        assert_eq!(
            DialogFooterProps::default()
                .with_align("left")
                .build_classes(),
            "flex mt-6 justify-start"
        );
        assert_eq!(
            DialogFooterProps::default()
                .with_align("bad")
                .build_classes(),
            "flex mt-6 justify-end gap-2"
        );
        let close_attrs = DialogCloseProps::default()
            .with_dialog_id("d")
            .with_signal("open")
            .with_after_close_action("done()")
            .build_close_attributes();
        assert_eq!(close_attrs.get("type").map(String::as_str), Some("button"));
        assert_eq!(
            close_attrs.get("data-dialog-close").map(String::as_str),
            Some("d")
        );
        assert_eq!(
            close_attrs.get("data-on:click").map(String::as_str),
            Some("$open = false; done()")
        );
    }

    #[test]
    fn parts_render_expected_html_and_title_levels() {
        let dialog = Dialog(
            &DialogProps::default()
                .with_id("d")
                .with_open(true)
                .with_aria_label("Dialog")
                .with_attribute("data-testid", "dialog"),
            html! { "Body" },
        )
        .into_string();
        assert!(dialog.starts_with("<div class=\"fixed inset-0 z-50\" id=\"d\" aria-label=\"Dialog\" role=\"dialog\" data-dialog=\"true\" aria-modal=\"true\" aria-hidden=\"false\" data-testid=\"dialog\">"));
        let trigger = DialogTrigger(
            &DialogTriggerProps::default()
                .with_dialog_id("d")
                .with_signal("open"),
            html! { "Open" },
        )
        .into_string();
        assert!(trigger.contains("aria-label=\"Open dialog\" type=\"button\" aria-haspopup=\"dialog\" data-dialog-trigger=\"d\" aria-controls=\"d\" data-on:click=\"$open = true\""));
        assert_eq!(
            DialogOverlay(&DialogOverlayProps::default().with_dialog_id("d")).into_string(),
            "<div class=\"fixed inset-0 bg-overlay-bg backdrop-blur-sm animate-overlay-in\" data-dialog-overlay=\"d\"></div>"
        );
        let content = DialogContent(
            &DialogContentProps::default().with_dialog_id("d"),
            html! { "Content" },
        )
        .into_string();
        assert!(
            content.contains(
                "role=\"document\" data-dialog-content=\"d\" tabindex=\"-1\">Content</div>"
            )
        );
        assert_eq!(
            DialogHeader(&DialogHeaderProps::default(), html! { "Head" }).into_string(),
            "<div class=\"flex flex-col space-y-1.5 text-center sm:text-left mb-4\">Head</div>"
        );
        assert!(
            DialogTitle(
                &DialogTitleProps::default()
                    .with_level("h1")
                    .with_id("title"),
                html! { "<Title>" }
            )
            .into_string()
            .starts_with("<h1 id=\"title\"")
        );
        assert!(
            DialogTitle(
                &DialogTitleProps::default().with_level("bad"),
                html! { "Title" }
            )
            .into_string()
            .starts_with("<h2")
        );
        assert_eq!(
            DialogDescription(
                &DialogDescriptionProps::default().with_id("desc"),
                html! { "<Desc>" }
            )
            .into_string(),
            "<p id=\"desc\" class=\"text-sm text-muted-foreground\">&lt;Desc&gt;</p>"
        );
        assert_eq!(
            DialogFooter(
                &DialogFooterProps::default().with_align("between"),
                html! { "Foot" }
            )
            .into_string(),
            "<div class=\"flex mt-6 justify-between\">Foot</div>"
        );
    }

    #[test]
    fn close_and_convenience_dialogs_render_expected_html() {
        let close = DialogClose(
            &DialogCloseProps::default()
                .with_dialog_id("d")
                .with_signal("open"),
            html! { "Close" },
        )
        .into_string();
        assert!(close.contains("aria-label=\"Close dialog\" type=\"button\" data-dialog-close=\"d\" data-on:click=\"$open = false\""));
        assert!(close.contains("<svg class=\"h-4 w-4\""));
        let text_close = DialogClose(
            &DialogCloseProps::default().with_show_icon(false),
            html! { "<Close>" },
        )
        .into_string();
        assert!(text_close.ends_with("&lt;Close&gt;</button>"));

        let simple = SimpleDialog("basic", "Title", "Desc", html! { "Inner" }).into_string();
        assert!(simple.contains("id=\"basic-title\""));
        assert!(simple.contains("id=\"basic-description\""));
        assert!(simple.contains("<div class=\"py-4\">Inner</div>"));
        let ds = DatastarDialog("ds", "open", "Title", "", html! {}).into_string();
        assert!(ds.contains("data-show=\"$open\""));
        assert!(ds.contains("data-on:click=\"$open = false\""));
        let confirm = ConfirmDialog(
            "confirm",
            "Delete",
            "Sure?",
            "Yes",
            "No",
            html! { "Content" },
        )
        .into_string();
        assert!(confirm.contains("role=\"alertdialog\""));
        assert!(confirm.contains("data-dialog-close=\"confirm\""));
        assert!(confirm.contains("data-dialog-confirm=\"confirm\""));
        assert!(confirm.contains(">Yes</button>"));
        assert!(confirm.contains(">No</button>"));
    }

    #[test]
    fn builder_methods_and_remaining_dialog_branches_render_expected_html() {
        let open_signal = DialogProps::default()
            .with_id("open")
            .with_signal("visible")
            .with_open(true)
            .with_class("dialog-extra")
            .with_aria_label("Signal dialog")
            .with_aria_described_by("open-desc")
            .with_aria_labelled_by("open-title")
            .with_role("alertdialog");
        let attrs = open_signal.build_attributes();
        assert_eq!(attrs.get("aria-hidden").map(String::as_str), Some("false"));
        assert_eq!(attrs.get("data-show").map(String::as_str), Some("$visible"));
        assert_eq!(attrs.get("role").map(String::as_str), Some("alertdialog"));
        assert!(open_signal.build_classes().contains("dialog-extra"));

        let trigger = DialogTriggerProps::default()
            .with_type("submit")
            .with_aria_label("Launch")
            .with_attribute("data-testid", "trigger");
        let trigger_html = DialogTrigger(&trigger, html! { "Open" }).into_string();
        assert!(trigger_html.contains(r#"aria-label="Launch""#));
        assert!(trigger_html.contains(r#"type="submit""#));
        assert!(trigger_html.contains(r#"data-testid="trigger""#));

        let overlay = DialogOverlay(
            &DialogOverlayProps::default()
                .with_dialog_id("d")
                .with_attribute("id", "overlay")
                .with_attribute("data-testid", "overlay"),
        )
        .into_string();
        assert!(overlay.contains(r#"id="overlay""#));
        assert!(overlay.contains(r#"data-testid="overlay""#));

        let content_attrs = DialogContentProps::default()
            .with_role("region")
            .with_aria_described_by("desc")
            .with_aria_labelled_by("title")
            .with_attribute("id", "content")
            .build_content_attributes();
        assert_eq!(
            content_attrs.get("role").map(String::as_str),
            Some("region")
        );
        assert_eq!(
            content_attrs.get("aria-describedby").map(String::as_str),
            Some("desc")
        );
        assert_eq!(
            content_attrs.get("aria-labelledby").map(String::as_str),
            Some("title")
        );

        assert!(
            DialogHeader(
                &DialogHeaderProps::default()
                    .with_attribute("id", "head")
                    .with_attribute("data-testid", "head"),
                html! { "Head" },
            )
            .into_string()
            .contains(r#"id="head" data-testid="head""#)
        );

        for level in ["h3", "h4", "h5", "h6"] {
            let rendered = DialogTitle(
                &DialogTitleProps::default().with_level(level),
                html! { "Title" },
            )
            .into_string();
            assert!(rendered.starts_with(&format!("<{level} ")));
            assert!(rendered.ends_with(&format!(">Title</{level}>")));
        }
        assert!(
            DialogTitle(
                &DialogTitleProps::default()
                    .with_id("title")
                    .with_attribute("data-testid", "ignored"),
                html! { "Title" },
            )
            .into_string()
            .contains(r#"id="title""#)
        );

        assert!(
            DialogDescription(
                &DialogDescriptionProps::default()
                    .with_class("desc-extra")
                    .with_attribute("data-testid", "desc"),
                html! { "Desc" },
            )
            .into_string()
            .contains(r#"class="text-sm text-muted-foreground desc-extra" data-testid="desc""#)
        );

        for (align, expected) in [
            ("center", "justify-center"),
            ("right", "justify-end"),
            ("between", "justify-between"),
        ] {
            assert!(
                DialogFooter(&DialogFooterProps::default().with_align(align), html! {})
                    .into_string()
                    .contains(expected)
            );
        }
        assert!(
            DialogFooter(
                &DialogFooterProps::default()
                    .with_class("foot-extra")
                    .with_attribute("id", "foot"),
                html! { "Foot" },
            )
            .into_string()
            .contains(r#"class="flex mt-6 justify-end foot-extra" id="foot""#)
        );

        let close = DialogClose(
            &DialogCloseProps::default()
                .with_class("close-extra")
                .with_aria_label("Dismiss")
                .with_attribute("id", "close")
                .with_show_icon(false),
            html! { "Close" },
        )
        .into_string();
        assert!(close.contains(r#"class="dialog-close "#));
        assert!(close.contains("close-extra"));
        assert!(close.contains(r#"aria-label="Dismiss""#));
        assert!(close.contains(r#"id="close""#));
        assert!(close.ends_with(">Close</button>"));

        let datastar_with_description =
            DatastarDialog("ds-desc", "open", "Title", "Description", html! {}).into_string();
        assert!(datastar_with_description.contains(r#"id="ds-desc-description""#));
    }
}
