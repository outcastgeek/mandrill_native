#![allow(non_snake_case)]

use crate::components::{ComponentInfo, shared};
use maud::{Markup, html};
use std::collections::BTreeMap;

pub const COMPONENTS: &[ComponentInfo] = &[
    ComponentInfo {
        family: "unstyled",
        module: "popover",
        name: "Popover",
    },
    ComponentInfo {
        family: "unstyled",
        module: "popover",
        name: "PopoverTrigger",
    },
    ComponentInfo {
        family: "unstyled",
        module: "popover",
        name: "PopoverContent",
    },
    ComponentInfo {
        family: "unstyled",
        module: "popover",
        name: "PopoverHeader",
    },
    ComponentInfo {
        family: "unstyled",
        module: "popover",
        name: "PopoverTitle",
    },
    ComponentInfo {
        family: "unstyled",
        module: "popover",
        name: "PopoverDescription",
    },
    ComponentInfo {
        family: "unstyled",
        module: "popover",
        name: "PopoverClose",
    },
    ComponentInfo {
        family: "unstyled",
        module: "popover",
        name: "SimplePopover",
    },
    ComponentInfo {
        family: "unstyled",
        module: "popover",
        name: "PopoverWithDescription",
    },
    ComponentInfo {
        family: "unstyled",
        module: "popover",
        name: "TopPopover",
    },
    ComponentInfo {
        family: "unstyled",
        module: "popover",
        name: "RightPopover",
    },
    ComponentInfo {
        family: "unstyled",
        module: "popover",
        name: "BottomPopover",
    },
    ComponentInfo {
        family: "unstyled",
        module: "popover",
        name: "LeftPopover",
    },
];

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PopoverProps {
    pub id: String,
    pub class: String,
    pub attributes: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PopoverTriggerProps {
    pub popover_id: String,
    pub class: String,
    pub attributes: BTreeMap<String, String>,
    pub type_attr: String,
    pub aria_label: String,
    pub aria_expanded: bool,
}

impl Default for PopoverTriggerProps {
    fn default() -> Self {
        Self {
            popover_id: String::new(),
            class: String::new(),
            attributes: BTreeMap::new(),
            type_attr: "button".to_string(),
            aria_label: String::new(),
            aria_expanded: false,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PopoverContentProps {
    pub popover_id: String,
    pub position: &'static str,
    pub class: String,
    pub attributes: BTreeMap<String, String>,
    pub aria_label: String,
    pub aria_labelled_by: String,
    pub role: String,
}

impl Default for PopoverContentProps {
    fn default() -> Self {
        Self {
            popover_id: String::new(),
            position: shared::POSITION_BOTTOM,
            class: String::new(),
            attributes: BTreeMap::new(),
            aria_label: String::new(),
            aria_labelled_by: String::new(),
            role: "dialog".to_string(),
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PopoverHeaderProps {
    pub class: String,
    pub attributes: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PopoverTitleProps {
    pub id: String,
    pub class: String,
    pub attributes: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PopoverDescriptionProps {
    pub class: String,
    pub attributes: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PopoverCloseProps {
    pub popover_id: String,
    pub class: String,
    pub attributes: BTreeMap<String, String>,
    pub aria_label: String,
    pub show_icon: bool,
}

impl Default for PopoverCloseProps {
    fn default() -> Self {
        Self {
            popover_id: String::new(),
            class: String::new(),
            attributes: BTreeMap::new(),
            aria_label: String::new(),
            show_icon: true,
        }
    }
}

impl PopoverProps {
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
        shared::build_classes(&["relative inline-flex", self.class.as_str()])
    }
    #[must_use]
    pub fn build_attributes(&self) -> BTreeMap<String, String> {
        let mut attrs = self.attributes.clone();
        if !self.id.is_empty() {
            attrs.insert("id".to_string(), self.id.clone());
        }
        attrs
    }
}

impl PopoverTriggerProps {
    #[must_use]
    pub fn with_popover_id(mut self, value: impl Into<String>) -> Self {
        self.popover_id = value.into();
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
    pub const fn with_aria_expanded(mut self, value: bool) -> Self {
        self.aria_expanded = value;
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
        if !self.aria_label.is_empty() {
            attrs.insert("aria-label".to_string(), self.aria_label.clone());
        }
        attrs.insert("aria-expanded".to_string(), self.aria_expanded.to_string());
        attrs.insert("aria-haspopup".to_string(), "dialog".to_string());
        if !self.popover_id.is_empty() {
            attrs.insert("data-popover-trigger".to_string(), self.popover_id.clone());
            attrs.insert("aria-controls".to_string(), self.popover_id.clone());
        }
        attrs
    }
}

impl PopoverContentProps {
    #[must_use]
    pub fn with_popover_id(mut self, value: impl Into<String>) -> Self {
        self.popover_id = value.into();
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
    pub fn build_content_classes(&self) -> String {
        shared::build_classes(&[
            "absolute z-50 hidden w-72 rounded-md border bg-popover p-4 text-popover-foreground shadow-md outline-none animate-popover-in",
            shared::tooltip_position_classes(self.position),
            self.class.as_str(),
        ])
    }
    #[must_use]
    pub fn build_content_attributes(&self) -> BTreeMap<String, String> {
        let mut attrs = self.attributes.clone();
        if !self.popover_id.is_empty() {
            attrs.insert("id".to_string(), self.popover_id.clone());
            attrs.insert("data-popover-content".to_string(), self.popover_id.clone());
        }
        if !self.aria_label.is_empty() {
            attrs.insert("aria-label".to_string(), self.aria_label.clone());
        }
        if !self.aria_labelled_by.is_empty() {
            attrs.insert("aria-labelledby".to_string(), self.aria_labelled_by.clone());
        }
        if !self.role.is_empty() {
            attrs.insert("role".to_string(), self.role.clone());
        }
        if !self.position.is_empty() {
            attrs.insert(
                "data-popover-position".to_string(),
                self.position.to_string(),
            );
        }
        attrs
    }
}

impl PopoverHeaderProps {
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
        shared::build_classes(&["mb-4", self.class.as_str()])
    }
}

impl PopoverTitleProps {
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
            "font-semibold leading-none tracking-tight",
            self.class.as_str(),
        ])
    }
    #[must_use]
    pub fn build_attributes(&self) -> BTreeMap<String, String> {
        let mut attrs = self.attributes.clone();
        if !self.id.is_empty() {
            attrs.insert("id".to_string(), self.id.clone());
        }
        attrs
    }
}

impl PopoverDescriptionProps {
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
        shared::build_classes(&["text-sm text-muted-foreground mt-2", self.class.as_str()])
    }
}

impl PopoverCloseProps {
    #[must_use]
    pub fn with_popover_id(mut self, value: impl Into<String>) -> Self {
        self.popover_id = value.into();
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
    pub fn build_close_classes(&self) -> String {
        shared::build_classes(&[
            "absolute right-2 top-2 rounded-sm opacity-70 transition-opacity hover:opacity-100 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2",
            self.class.as_str(),
        ])
    }
    #[must_use]
    pub fn build_attributes(&self) -> BTreeMap<String, String> {
        let mut attrs = self.attributes.clone();
        attrs.insert("aria-label".to_string(), resolvePopoverCloseAriaLabel(self));
        if !self.popover_id.is_empty() {
            attrs.insert("data-popover-close".to_string(), self.popover_id.clone());
        }
        attrs
    }
}

#[must_use]
pub fn resolvePopoverCloseAriaLabel(props: &PopoverCloseProps) -> String {
    if props.aria_label.is_empty() {
        "Close popover".to_string()
    } else {
        props.aria_label.clone()
    }
}

#[must_use]
pub fn Popover(props: &PopoverProps, children: Markup) -> Markup {
    let attrs = props.build_attributes();
    html! {
        div
            class=(props.build_classes())
            id=[shared::attr_value(&attrs, "id")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
        {
            (children)
        }
    }
}

#[must_use]
pub fn PopoverTrigger(props: &PopoverTriggerProps, children: Markup) -> Markup {
    let attrs = props.build_trigger_attributes();
    html! {
        button
            class=(props.build_classes())
            type=[shared::attr_value(&attrs, "type")]
            aria-label=[shared::attr_value(&attrs, "aria-label")]
            aria-expanded=[shared::attr_value(&attrs, "aria-expanded")]
            aria-haspopup=[shared::attr_value(&attrs, "aria-haspopup")]
            data-popover-trigger=[shared::attr_value(&attrs, "data-popover-trigger")]
            aria-controls=[shared::attr_value(&attrs, "aria-controls")]
            id=[shared::attr_value(&attrs, "id")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
        {
            (children)
        }
    }
}

#[must_use]
pub fn PopoverContent(props: &PopoverContentProps, children: Markup) -> Markup {
    let attrs = props.build_content_attributes();
    html! {
        div
            class=(props.build_content_classes())
            id=[shared::attr_value(&attrs, "id")]
            aria-label=[shared::attr_value(&attrs, "aria-label")]
            aria-labelledby=[shared::attr_value(&attrs, "aria-labelledby")]
            role=[shared::attr_value(&attrs, "role")]
            data-popover-content=[shared::attr_value(&attrs, "data-popover-content")]
            data-popover-position=[shared::attr_value(&attrs, "data-popover-position")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
        {
            (children)
        }
    }
}

#[must_use]
pub fn PopoverHeader(props: &PopoverHeaderProps, children: Markup) -> Markup {
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
pub fn PopoverTitle(props: &PopoverTitleProps, children: Markup) -> Markup {
    let attrs = props.build_attributes();
    html! {
        h4
            id=[shared::attr_value(&attrs, "id")]
            class=(props.build_classes())
            data-testid=[shared::attr_value(&attrs, "data-testid")]
        {
            (children)
        }
    }
}

#[must_use]
pub fn PopoverDescription(props: &PopoverDescriptionProps, children: Markup) -> Markup {
    html! {
        p
            class=(props.build_classes())
            id=[shared::attr_value(&props.attributes, "id")]
            data-testid=[shared::attr_value(&props.attributes, "data-testid")]
        {
            (children)
        }
    }
}

#[must_use]
pub fn PopoverClose(props: &PopoverCloseProps, children: Markup) -> Markup {
    let attrs = props.build_attributes();
    html! {
        button
            type="button"
            class=(props.build_close_classes())
            aria-label=[shared::attr_value(&attrs, "aria-label")]
            data-popover-close=[shared::attr_value(&attrs, "data-popover-close")]
            id=[shared::attr_value(&attrs, "id")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
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
pub fn SimplePopover(
    popover_id: &str,
    title: &str,
    position: &'static str,
    trigger: Markup,
    children: Markup,
) -> Markup {
    Popover(
        &PopoverProps::default().with_id(popover_id),
        html! {
            (PopoverTrigger(&PopoverTriggerProps::default().with_popover_id(popover_id), trigger))
            (PopoverContent(
                &PopoverContentProps::default()
                    .with_popover_id(popover_id)
                    .with_position(position)
                    .with_aria_labelled_by(format!("{popover_id}-title"))
                    .with_role("dialog"),
                html! {
                    @if !title.is_empty() {
                        (PopoverHeader(&PopoverHeaderProps::default(), html! {
                            (PopoverTitle(&PopoverTitleProps::default().with_id(format!("{popover_id}-title")), html! { (title) }))
                        }))
                    }
                    (children)
                    (PopoverClose(&PopoverCloseProps::default().with_popover_id(popover_id).with_aria_label("Close popover"), html! {}))
                },
            ))
        },
    )
}

#[must_use]
pub fn PopoverWithDescription(
    popover_id: &str,
    title: &str,
    description: &str,
    position: &'static str,
    trigger: Markup,
) -> Markup {
    Popover(
        &PopoverProps::default().with_id(popover_id),
        html! {
            (PopoverTrigger(&PopoverTriggerProps::default().with_popover_id(popover_id), trigger))
            (PopoverContent(
                &PopoverContentProps::default()
                    .with_popover_id(popover_id)
                    .with_position(position)
                    .with_aria_labelled_by(format!("{popover_id}-title"))
                    .with_role("dialog"),
                html! {
                    (PopoverHeader(&PopoverHeaderProps::default(), html! {
                        (PopoverTitle(&PopoverTitleProps::default().with_id(format!("{popover_id}-title")), html! { (title) }))
                        @if !description.is_empty() {
                            (PopoverDescription(&PopoverDescriptionProps::default(), html! { (description) }))
                        }
                    }))
                    (PopoverClose(&PopoverCloseProps::default().with_popover_id(popover_id).with_aria_label("Close popover"), html! {}))
                },
            ))
        },
    )
}

#[must_use]
pub fn TopPopover(popover_id: &str, title: &str, trigger: Markup, children: Markup) -> Markup {
    SimplePopover(popover_id, title, shared::POSITION_TOP, trigger, children)
}

#[must_use]
pub fn RightPopover(popover_id: &str, title: &str, trigger: Markup, children: Markup) -> Markup {
    SimplePopover(popover_id, title, shared::POSITION_RIGHT, trigger, children)
}

#[must_use]
pub fn BottomPopover(popover_id: &str, title: &str, trigger: Markup, children: Markup) -> Markup {
    SimplePopover(
        popover_id,
        title,
        shared::POSITION_BOTTOM,
        trigger,
        children,
    )
}

#[must_use]
pub fn LeftPopover(popover_id: &str, title: &str, trigger: Markup, children: Markup) -> Markup {
    SimplePopover(popover_id, title, shared::POSITION_LEFT, trigger, children)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn helpers_cover_defaults_classes_and_attributes() {
        assert_eq!(
            PopoverProps::default().build_classes(),
            "relative inline-flex"
        );
        assert_eq!(
            PopoverProps::default().with_class("group").build_classes(),
            "relative inline-flex group"
        );
        assert_eq!(
            PopoverProps::default()
                .with_id("pop")
                .with_attribute("data-testid", "root")
                .build_attributes()
                .get("id")
                .map(String::as_str),
            Some("pop"),
        );

        let trigger = PopoverTriggerProps::default()
            .with_popover_id("pop")
            .with_aria_label("Open details")
            .with_aria_expanded(true)
            .with_type("submit")
            .with_attribute("id", "trigger");
        assert_eq!(trigger.build_classes(), "cursor-pointer");
        let trigger_attrs = trigger.build_trigger_attributes();
        assert_eq!(
            trigger_attrs.get("type").map(String::as_str),
            Some("submit")
        );
        assert_eq!(
            trigger_attrs.get("aria-label").map(String::as_str),
            Some("Open details")
        );
        assert_eq!(
            trigger_attrs.get("aria-expanded").map(String::as_str),
            Some("true")
        );
        assert_eq!(
            trigger_attrs.get("aria-haspopup").map(String::as_str),
            Some("dialog")
        );
        assert_eq!(
            trigger_attrs
                .get("data-popover-trigger")
                .map(String::as_str),
            Some("pop")
        );
        assert_eq!(
            trigger_attrs.get("aria-controls").map(String::as_str),
            Some("pop")
        );

        for position in [
            shared::POSITION_TOP,
            shared::POSITION_RIGHT,
            shared::POSITION_BOTTOM,
            shared::POSITION_LEFT,
        ] {
            let classes = PopoverContentProps::default()
                .with_position(position)
                .build_content_classes();
            assert!(classes.contains(shared::tooltip_position_classes(position)));
        }
        let content_attrs = PopoverContentProps::default()
            .with_popover_id("pop")
            .with_aria_label("Popover")
            .with_aria_labelled_by("title")
            .with_position(shared::POSITION_LEFT)
            .with_role("")
            .with_attribute("data-testid", "content")
            .build_content_attributes();
        assert_eq!(content_attrs.get("id").map(String::as_str), Some("pop"));
        assert_eq!(content_attrs.get("role"), None);
        assert_eq!(
            content_attrs
                .get("data-popover-content")
                .map(String::as_str),
            Some("pop")
        );
        assert_eq!(
            content_attrs
                .get("data-popover-position")
                .map(String::as_str),
            Some("left")
        );
        assert_eq!(
            content_attrs.get("data-testid").map(String::as_str),
            Some("content")
        );

        assert_eq!(
            PopoverHeaderProps::default()
                .with_class("head")
                .build_classes(),
            "mb-4 head"
        );
        assert_eq!(
            PopoverTitleProps::default()
                .with_class("title")
                .build_classes(),
            "font-semibold leading-none tracking-tight title"
        );
        assert_eq!(
            PopoverDescriptionProps::default()
                .with_class("desc")
                .build_classes(),
            "text-sm text-muted-foreground mt-2 desc"
        );
        assert!(
            PopoverCloseProps::default()
                .build_close_classes()
                .contains("absolute right-2 top-2")
        );
        assert_eq!(
            PopoverCloseProps::default()
                .build_attributes()
                .get("aria-label")
                .map(String::as_str),
            Some("Close popover")
        );
        assert_eq!(
            resolvePopoverCloseAriaLabel(&PopoverCloseProps::default()),
            "Close popover"
        );
        assert_eq!(
            resolvePopoverCloseAriaLabel(&PopoverCloseProps::default().with_aria_label("Dismiss")),
            "Dismiss"
        );
    }

    #[test]
    fn parts_render_expected_html_and_escape_text() {
        let root = Popover(
            &PopoverProps::default()
                .with_id("pop")
                .with_class("wrap")
                .with_attribute("data-testid", "root"),
            html! { "<body>" },
        )
        .into_string();
        assert_eq!(
            root,
            "<div class=\"relative inline-flex wrap\" id=\"pop\" data-testid=\"root\">&lt;body&gt;</div>"
        );

        let trigger = PopoverTrigger(
            &PopoverTriggerProps::default()
                .with_popover_id("pop")
                .with_aria_expanded(true),
            html! { "Open" },
        )
        .into_string();
        assert!(trigger.starts_with("<button class=\"cursor-pointer\" type=\"button\""));
        assert!(trigger.contains("aria-expanded=\"true\""));
        assert!(trigger.contains("aria-haspopup=\"dialog\""));
        assert!(trigger.contains("data-popover-trigger=\"pop\" aria-controls=\"pop\""));
        assert!(trigger.ends_with(">Open</button>"));

        let content = PopoverContent(
            &PopoverContentProps::default()
                .with_popover_id("pop")
                .with_position(shared::POSITION_TOP)
                .with_aria_label("More")
                .with_aria_labelled_by("pop-title")
                .with_attribute("data-testid", "content"),
            html! { span { "Details" } },
        )
        .into_string();
        assert!(content.contains("class=\"absolute z-50 hidden w-72"));
        assert!(content.contains("bottom-full left-1/2 -translate-x-1/2 mb-2"));
        assert!(content.contains(
            "id=\"pop\" aria-label=\"More\" aria-labelledby=\"pop-title\" role=\"dialog\""
        ));
        assert!(content.contains(
            "data-popover-content=\"pop\" data-popover-position=\"top\" data-testid=\"content\""
        ));
        assert!(content.contains("<span>Details</span>"));

        assert_eq!(
            PopoverHeader(
                &PopoverHeaderProps::default().with_class("section"),
                html! { "Head" }
            )
            .into_string(),
            "<div class=\"mb-4 section\">Head</div>",
        );
        assert_eq!(
            PopoverTitle(
                &PopoverTitleProps::default().with_id("t").with_class("lead"),
                html! { "<Title>" }
            )
            .into_string(),
            "<h4 id=\"t\" class=\"font-semibold leading-none tracking-tight lead\">&lt;Title&gt;</h4>",
        );
        assert_eq!(
            PopoverDescription(&PopoverDescriptionProps::default(), html! { "<Desc>" })
                .into_string(),
            "<p class=\"text-sm text-muted-foreground mt-2\">&lt;Desc&gt;</p>",
        );
    }

    #[test]
    fn close_button_renders_icon_or_children() {
        let icon = PopoverClose(
            &PopoverCloseProps::default()
                .with_popover_id("pop")
                .with_attribute("data-testid", "close"),
            html! { "Close" },
        )
        .into_string();
        assert!(icon.starts_with("<button type=\"button\" class=\"absolute right-2 top-2"));
        assert!(icon.contains(
            "aria-label=\"Close popover\" data-popover-close=\"pop\" data-testid=\"close\""
        ));
        assert!(icon.contains("<svg class=\"h-4 w-4\""));
        assert!(icon.contains("d=\"M6 18L18 6M6 6l12 12\""));
        assert!(!icon.contains(">Close</button>"));

        let text = PopoverClose(
            &PopoverCloseProps::default()
                .with_popover_id("pop")
                .with_show_icon(false)
                .with_aria_label("Dismiss"),
            html! { "<Dismiss>" },
        )
        .into_string();
        assert!(text.contains("aria-label=\"Dismiss\" data-popover-close=\"pop\""));
        assert!(text.ends_with("&lt;Dismiss&gt;</button>"));
    }

    #[test]
    fn convenience_popovers_render_all_variations() {
        let simple = SimplePopover(
            "pop",
            "Title",
            shared::POSITION_BOTTOM,
            html! { button { "Trigger" } },
            html! { "Body" },
        )
        .into_string();
        assert!(simple.starts_with("<div class=\"relative inline-flex\" id=\"pop\">"));
        assert!(simple.contains("<button>Trigger</button>"));
        assert!(simple.contains(
            "<h4 id=\"pop-title\" class=\"font-semibold leading-none tracking-tight\">Title</h4>"
        ));
        assert!(simple.contains("data-popover-position=\"bottom\""));
        assert!(simple.contains("Body"));
        assert!(simple.contains("data-popover-close=\"pop\""));

        let with_description = PopoverWithDescription(
            "info",
            "Info",
            "<Description>",
            shared::POSITION_RIGHT,
            html! { "Open" },
        )
        .into_string();
        assert!(with_description.contains("data-popover-position=\"right\""));
        assert!(with_description.contains(
            "<h4 id=\"info-title\" class=\"font-semibold leading-none tracking-tight\">Info</h4>"
        ));
        assert!(
            with_description.contains(
                "<p class=\"text-sm text-muted-foreground mt-2\">&lt;Description&gt;</p>"
            )
        );

        assert!(
            TopPopover("top", "Top", html! {}, html! {})
                .into_string()
                .contains("data-popover-position=\"top\"")
        );
        assert!(
            RightPopover("right", "Right", html! {}, html! {})
                .into_string()
                .contains("data-popover-position=\"right\"")
        );
        assert!(
            BottomPopover("bottom", "Bottom", html! {}, html! {})
                .into_string()
                .contains("data-popover-position=\"bottom\"")
        );
        assert!(
            LeftPopover("left", "Left", html! {}, html! {})
                .into_string()
                .contains("data-popover-position=\"left\"")
        );
    }

    #[test]
    fn remaining_builder_methods_preserve_attrs_and_classes() {
        let trigger = PopoverTrigger(
            &PopoverTriggerProps::default()
                .with_popover_id("pop")
                .with_class("trigger-extra")
                .with_attribute("data-testid", "trigger")
                .with_type("submit")
                .with_aria_label("Launch")
                .with_aria_expanded(false),
            html! { "Launch" },
        )
        .into_string();
        assert!(trigger.contains("cursor-pointer trigger-extra"));
        assert!(trigger.contains(r#"type="submit" aria-label="Launch" aria-expanded="false""#));
        assert!(trigger.contains(r#"data-testid="trigger""#));

        let content = PopoverContent(
            &PopoverContentProps::default()
                .with_popover_id("pop")
                .with_position(shared::POSITION_RIGHT)
                .with_class("content-extra")
                .with_attribute("data-testid", "content")
                .with_aria_label("More")
                .with_aria_labelled_by("title")
                .with_role("tooltip"),
            html! { "Content" },
        )
        .into_string();
        assert!(content.contains("content-extra"));
        assert!(content.contains(r#"aria-label="More" aria-labelledby="title" role="tooltip""#));
        assert!(content.contains(r#"data-popover-position="right" data-testid="content""#));

        let header = PopoverHeader(
            &PopoverHeaderProps::default()
                .with_class("header-extra")
                .with_attribute("id", "header"),
            html! { "Header" },
        )
        .into_string();
        assert!(header.contains(r#"class="mb-4 header-extra" id="header""#));

        let title = PopoverTitle(
            &PopoverTitleProps::default()
                .with_id("title")
                .with_class("title-extra")
                .with_attribute("data-testid", "title"),
            html! { "Title" },
        )
        .into_string();
        assert!(title.contains(r#"id="title" class="font-semibold leading-none tracking-tight title-extra" data-testid="title""#));

        let description = PopoverDescription(
            &PopoverDescriptionProps::default()
                .with_class("description-extra")
                .with_attribute("id", "description"),
            html! { "<Description>" },
        )
        .into_string();
        assert!(description.contains(
            r#"class="text-sm text-muted-foreground mt-2 description-extra" id="description""#
        ));
        assert!(description.contains("&lt;Description&gt;"));

        let close = PopoverClose(
            &PopoverCloseProps::default()
                .with_popover_id("pop")
                .with_class("close-extra")
                .with_attribute("id", "close")
                .with_aria_label("Dismiss")
                .with_show_icon(false),
            html! { "Dismiss" },
        )
        .into_string();
        assert!(close.contains("close-extra"));
        assert!(close.contains(r#"aria-label="Dismiss" data-popover-close="pop" id="close""#));
        assert!(close.ends_with(">Dismiss</button>"));
    }
}
