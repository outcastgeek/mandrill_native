#![allow(non_snake_case)]

use crate::components::{ComponentInfo, shared};
use maud::{Markup, html};
use std::collections::BTreeMap;

pub const COMPONENTS: &[ComponentInfo] = &[
    ComponentInfo {
        family: "unstyled",
        module: "tooltip",
        name: "Tooltip",
    },
    ComponentInfo {
        family: "unstyled",
        module: "tooltip",
        name: "TooltipTrigger",
    },
    ComponentInfo {
        family: "unstyled",
        module: "tooltip",
        name: "TooltipContent",
    },
    ComponentInfo {
        family: "unstyled",
        module: "tooltip",
        name: "SimpleTooltip",
    },
    ComponentInfo {
        family: "unstyled",
        module: "tooltip",
        name: "TooltipWithTrigger",
    },
    ComponentInfo {
        family: "unstyled",
        module: "tooltip",
        name: "TopTooltip",
    },
    ComponentInfo {
        family: "unstyled",
        module: "tooltip",
        name: "RightTooltip",
    },
    ComponentInfo {
        family: "unstyled",
        module: "tooltip",
        name: "BottomTooltip",
    },
    ComponentInfo {
        family: "unstyled",
        module: "tooltip",
        name: "LeftTooltip",
    },
];

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct TooltipProps {
    pub id: String,
    pub class: String,
    pub attributes: BTreeMap<String, String>,
}

impl TooltipProps {
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

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct TooltipTriggerProps {
    pub tooltip_id: String,
    pub class: String,
    pub attributes: BTreeMap<String, String>,
    pub aria_label: String,
    pub aria_described_by: String,
}

impl TooltipTriggerProps {
    #[must_use]
    pub fn with_tooltip_id(mut self, value: impl Into<String>) -> Self {
        self.tooltip_id = value.into();
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
    pub fn build_classes(&self) -> String {
        shared::build_classes(&["cursor-help", self.class.as_str()])
    }

    #[must_use]
    pub fn build_trigger_attributes(&self) -> BTreeMap<String, String> {
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
        if !self.tooltip_id.is_empty() {
            attrs.insert("data-tooltip-trigger".to_string(), self.tooltip_id.clone());
            attrs.insert("aria-describedby".to_string(), self.tooltip_id.clone());
        }
        attrs
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TooltipContentProps {
    pub tooltip_id: String,
    pub content: String,
    pub position: &'static str,
    pub delay: i32,
    pub class: String,
    pub attributes: BTreeMap<String, String>,
    pub role: String,
}

impl Default for TooltipContentProps {
    fn default() -> Self {
        Self {
            tooltip_id: String::new(),
            content: String::new(),
            position: shared::POSITION_TOP,
            delay: 200,
            class: String::new(),
            attributes: BTreeMap::new(),
            role: "tooltip".to_string(),
        }
    }
}

impl TooltipContentProps {
    #[must_use]
    pub fn with_tooltip_id(mut self, value: impl Into<String>) -> Self {
        self.tooltip_id = value.into();
        self
    }

    #[must_use]
    pub fn with_content(mut self, value: impl Into<String>) -> Self {
        self.content = value.into();
        self
    }

    #[must_use]
    pub const fn with_position(mut self, value: &'static str) -> Self {
        self.position = value;
        self
    }

    #[must_use]
    pub const fn with_delay(mut self, value: i32) -> Self {
        self.delay = value;
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
    pub fn with_role(mut self, value: impl Into<String>) -> Self {
        self.role = value.into();
        self
    }

    #[must_use]
    pub fn build_classes(&self) -> String {
        shared::build_classes(&[
            "absolute z-50 hidden px-3 py-2 text-sm text-popover-foreground bg-popover rounded-md shadow-sm whitespace-nowrap pointer-events-none",
            shared::tooltip_position_classes(self.position),
            shared::tooltip_animation_classes(self.position),
            self.class.as_str(),
        ])
    }

    #[must_use]
    pub fn build_content_attributes(&self) -> BTreeMap<String, String> {
        let mut attrs = self.attributes.clone();
        if !self.tooltip_id.is_empty() {
            attrs.insert("id".to_string(), self.tooltip_id.clone());
        }
        if !self.role.is_empty() {
            attrs.insert("role".to_string(), self.role.clone());
        }
        attrs.insert("data-tooltip-content".to_string(), self.tooltip_id.clone());
        if !self.position.is_empty() {
            attrs.insert(
                "data-tooltip-position".to_string(),
                self.position.to_string(),
            );
        }
        if self.delay > 0 {
            attrs.insert("data-tooltip-delay".to_string(), self.delay.to_string());
        }
        attrs
    }
}

#[must_use]
pub fn Tooltip(props: &TooltipProps, children: Markup) -> Markup {
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
pub fn TooltipTrigger(props: &TooltipTriggerProps, children: Markup) -> Markup {
    let attrs = props.build_trigger_attributes();
    html! {
        div
            class=(props.build_classes())
            aria-label=[shared::attr_value(&attrs, "aria-label")]
            aria-describedby=[shared::attr_value(&attrs, "aria-describedby")]
            data-tooltip-trigger=[shared::attr_value(&attrs, "data-tooltip-trigger")]
            id=[shared::attr_value(&attrs, "id")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
        {
            (children)
        }
    }
}

#[must_use]
pub fn TooltipContent(props: &TooltipContentProps, children: Markup) -> Markup {
    let attrs = props.build_content_attributes();
    html! {
        div
            class=(props.build_classes())
            id=[shared::attr_value(&attrs, "id")]
            role=[shared::attr_value(&attrs, "role")]
            data-tooltip-content=[shared::attr_value(&attrs, "data-tooltip-content")]
            data-tooltip-position=[shared::attr_value(&attrs, "data-tooltip-position")]
            data-tooltip-delay=[shared::attr_value(&attrs, "data-tooltip-delay")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
        {
            @if props.content.is_empty() {
                (children)
            } @else {
                (&props.content)
            }
        }
    }
}

#[must_use]
pub fn SimpleTooltip(
    tooltip_id: &str,
    content: &str,
    position: &'static str,
    delay: i32,
    children: Markup,
) -> Markup {
    Tooltip(
        &TooltipProps::default().with_id(tooltip_id),
        html! {
            (children)
            (TooltipContent(
                &TooltipContentProps::default()
                    .with_tooltip_id(tooltip_id)
                    .with_content(content)
                    .with_position(position)
                    .with_delay(delay)
                    .with_role("tooltip"),
                html! {},
            ))
        },
    )
}

#[must_use]
pub fn TooltipWithTrigger(
    tooltip_id: &str,
    content: &str,
    position: &'static str,
    delay: i32,
    children: Markup,
) -> Markup {
    Tooltip(
        &TooltipProps::default().with_id(tooltip_id),
        html! {
            (TooltipTrigger(
                &TooltipTriggerProps::default()
                    .with_tooltip_id(tooltip_id)
                    .with_aria_described_by(tooltip_id),
                children,
            ))
            (TooltipContent(
                &TooltipContentProps::default()
                    .with_tooltip_id(tooltip_id)
                    .with_content(content)
                    .with_position(position)
                    .with_delay(delay)
                    .with_role("tooltip"),
                html! {},
            ))
        },
    )
}

#[must_use]
pub fn TopTooltip(tooltip_id: &str, content: &str, children: Markup) -> Markup {
    SimpleTooltip(tooltip_id, content, shared::POSITION_TOP, 200, children)
}

#[must_use]
pub fn RightTooltip(tooltip_id: &str, content: &str, children: Markup) -> Markup {
    SimpleTooltip(tooltip_id, content, shared::POSITION_RIGHT, 200, children)
}

#[must_use]
pub fn BottomTooltip(tooltip_id: &str, content: &str, children: Markup) -> Markup {
    SimpleTooltip(tooltip_id, content, shared::POSITION_BOTTOM, 200, children)
}

#[must_use]
pub fn LeftTooltip(tooltip_id: &str, content: &str, children: Markup) -> Markup {
    SimpleTooltip(tooltip_id, content, shared::POSITION_LEFT, 200, children)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn helpers_build_classes_and_attributes_for_all_parts() {
        assert_eq!(
            TooltipProps::default().build_classes(),
            "relative inline-flex"
        );
        assert_eq!(
            TooltipProps::default().with_class("group").build_classes(),
            "relative inline-flex group"
        );
        let wrapper_attrs = TooltipProps::default()
            .with_id("tip")
            .with_attribute("data-testid", "wrapper")
            .build_attributes();
        assert_eq!(wrapper_attrs.get("id").map(String::as_str), Some("tip"));
        assert_eq!(
            wrapper_attrs.get("data-testid").map(String::as_str),
            Some("wrapper")
        );

        assert_eq!(
            TooltipTriggerProps::default().build_classes(),
            "cursor-help"
        );
        let trigger_attrs = TooltipTriggerProps::default()
            .with_tooltip_id("tip")
            .with_aria_label("More info")
            .with_aria_described_by("caller")
            .with_attribute("id", "trigger")
            .build_trigger_attributes();
        assert_eq!(
            trigger_attrs.get("aria-label").map(String::as_str),
            Some("More info")
        );
        assert_eq!(
            trigger_attrs.get("aria-describedby").map(String::as_str),
            Some("tip")
        );
        assert_eq!(
            trigger_attrs
                .get("data-tooltip-trigger")
                .map(String::as_str),
            Some("tip")
        );
        assert_eq!(trigger_attrs.get("id").map(String::as_str), Some("trigger"));
    }

    #[test]
    fn content_helpers_cover_defaults_positions_delay_and_role() {
        let props = TooltipContentProps::default();
        assert_eq!(props.position, shared::POSITION_TOP);
        assert_eq!(props.delay, 200);
        assert_eq!(props.role, "tooltip");
        assert!(
            props
                .build_classes()
                .contains(shared::tooltip_position_classes(shared::POSITION_TOP))
        );
        assert!(
            props
                .build_classes()
                .contains(shared::tooltip_animation_classes(shared::POSITION_TOP))
        );
        for position in [
            shared::POSITION_TOP,
            shared::POSITION_RIGHT,
            shared::POSITION_BOTTOM,
            shared::POSITION_LEFT,
        ] {
            let classes = TooltipContentProps::default()
                .with_position(position)
                .build_classes();
            assert!(classes.contains(shared::tooltip_position_classes(position)));
            assert!(classes.contains(shared::tooltip_animation_classes(position)));
        }
        let attrs = TooltipContentProps::default()
            .with_tooltip_id("tip")
            .with_position(shared::POSITION_LEFT)
            .with_delay(0)
            .with_role("")
            .with_attribute("data-testid", "content")
            .build_content_attributes();
        assert_eq!(attrs.get("id").map(String::as_str), Some("tip"));
        assert_eq!(attrs.get("role"), None);
        assert_eq!(
            attrs.get("data-tooltip-content").map(String::as_str),
            Some("tip")
        );
        assert_eq!(
            attrs.get("data-tooltip-position").map(String::as_str),
            Some("left")
        );
        assert_eq!(attrs.get("data-tooltip-delay"), None);
        assert_eq!(
            attrs.get("data-testid").map(String::as_str),
            Some("content")
        );
    }

    #[test]
    fn tooltip_parts_render_attributes_content_children_and_escaping() {
        let wrapper = Tooltip(
            &TooltipProps::default()
                .with_id("wrap")
                .with_class("group")
                .with_attribute("data-testid", "wrapper"),
            html! { span { "Trigger" } },
        )
        .into_string();
        assert_eq!(
            wrapper,
            "<div class=\"relative inline-flex group\" id=\"wrap\" data-testid=\"wrapper\"><span>Trigger</span></div>"
        );

        let trigger = TooltipTrigger(
            &TooltipTriggerProps::default()
                .with_tooltip_id("tip")
                .with_class("underline")
                .with_attribute("id", "trig"),
            html! { "<Trigger>" },
        )
        .into_string();
        assert!(trigger.starts_with("<div class=\"cursor-help underline\""));
        assert!(trigger.contains("aria-describedby=\"tip\""));
        assert!(trigger.contains("data-tooltip-trigger=\"tip\""));
        assert!(trigger.contains("id=\"trig\""));
        assert!(trigger.contains("&lt;Trigger&gt;"));

        let content = TooltipContent(
            &TooltipContentProps::default()
                .with_tooltip_id("tip")
                .with_content("<Copy>")
                .with_position(shared::POSITION_BOTTOM)
                .with_delay(350)
                .with_class("tip-extra")
                .with_attribute("data-testid", "tip-content"),
            html! { "Ignored" },
        )
        .into_string();
        assert!(content.contains("tip-extra"));
        assert!(content.contains("role=\"tooltip\""));
        assert!(content.contains("data-tooltip-content=\"tip\""));
        assert!(content.contains("data-tooltip-position=\"bottom\""));
        assert!(content.contains("data-tooltip-delay=\"350\""));
        assert!(content.contains("data-testid=\"tip-content\""));
        assert!(content.contains("&lt;Copy&gt;"));
        assert!(!content.contains("Ignored"));

        let fallback = TooltipContent(&TooltipContentProps::default(), html! { b { "Fallback" } })
            .into_string();
        assert!(fallback.contains("<b>Fallback</b>"));
    }

    #[test]
    fn convenience_tooltips_render_expected_variations() {
        let simple = SimpleTooltip(
            "tip",
            "Info",
            shared::POSITION_RIGHT,
            100,
            html! { button { "?" } },
        )
        .into_string();
        assert!(simple.starts_with("<div class=\"relative inline-flex\" id=\"tip\">"));
        assert!(simple.contains("<button>?</button>"));
        assert!(simple.contains("data-tooltip-position=\"right\""));
        assert!(simple.contains("data-tooltip-delay=\"100\""));
        assert!(simple.contains(">Info</div>"));

        let with_trigger = TooltipWithTrigger(
            "tip2",
            "Details",
            shared::POSITION_LEFT,
            50,
            html! { span { "Hover" } },
        )
        .into_string();
        assert!(with_trigger.contains("data-tooltip-trigger=\"tip2\""));
        assert!(with_trigger.contains("<span>Hover</span>"));
        assert!(with_trigger.contains("data-tooltip-position=\"left\""));
        assert!(with_trigger.contains(">Details</div>"));

        assert!(
            TopTooltip("top", "Top", html! {})
                .into_string()
                .contains("data-tooltip-position=\"top\"")
        );
        assert!(
            RightTooltip("right", "Right", html! {})
                .into_string()
                .contains("data-tooltip-position=\"right\"")
        );
        assert!(
            BottomTooltip("bottom", "Bottom", html! {})
                .into_string()
                .contains("data-tooltip-position=\"bottom\"")
        );
        assert!(
            LeftTooltip("left", "Left", html! {})
                .into_string()
                .contains("data-tooltip-position=\"left\"")
        );
    }
}
