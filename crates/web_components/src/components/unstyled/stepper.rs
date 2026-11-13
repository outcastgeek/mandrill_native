#![allow(non_snake_case)]

use crate::components::{ComponentInfo, shared};
use maud::{Markup, html};
use std::collections::BTreeMap;

pub const COMPONENTS: &[ComponentInfo] = &[ComponentInfo {
    family: "unstyled",
    module: "stepper",
    name: "Stepper",
}];

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Item {
    pub label: String,
    pub completed: bool,
    pub current: bool,
    pub class: String,
    pub attributes: BTreeMap<String, String>,
}

impl Item {
    #[must_use]
    pub fn with_label(mut self, value: impl Into<String>) -> Self {
        self.label = value.into();
        self
    }
    #[must_use]
    pub const fn with_completed(mut self, value: bool) -> Self {
        self.completed = value;
        self
    }
    #[must_use]
    pub const fn with_current(mut self, value: bool) -> Self {
        self.current = value;
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
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Props {
    pub items: Vec<Item>,
    pub class: String,
    pub attributes: BTreeMap<String, String>,
}

impl Props {
    #[must_use]
    pub fn with_items(mut self, value: Vec<Item>) -> Self {
        self.items = value;
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
        shared::build_classes(&["inline-flex items-center gap-2", self.class.as_str()])
    }
}

#[must_use]
pub fn DotClasses(item: &Item) -> String {
    let base = "block rounded-full border transition-all duration-150";
    if item.current {
        shared::build_classes(&[
            base,
            "size-2.5 border-primary bg-primary shadow-[0_0_0_4px_var(--dot-active-glow)]",
            item.class.as_str(),
        ])
    } else if item.completed {
        shared::build_classes(&[
            base,
            "size-2.5 border-success bg-success",
            item.class.as_str(),
        ])
    } else {
        shared::build_classes(&[
            base,
            "size-2 border-[var(--dot-inactive-border)] bg-[var(--dot-inactive)]",
            item.class.as_str(),
        ])
    }
}

#[must_use]
pub fn Stepper(props: &Props) -> Markup {
    html! {
        div
            class=(props.build_classes())
            id=[shared::attr_value(&props.attributes, "id")]
            data-testid=[shared::attr_value(&props.attributes, "data-testid")]
        {
            @for item in &props.items {
                span
                    class=(DotClasses(item))
                    aria-label=[if item.label.is_empty() { None } else { Some(item.label.as_str()) }]
                    title=[if item.label.is_empty() { None } else { Some(item.label.as_str()) }]
                    id=[shared::attr_value(&item.attributes, "id")]
                    data-testid=[shared::attr_value(&item.attributes, "data-testid")]
                    r#"data-class:size-2.5"#=[shared::attr_value(&item.attributes, "data-class:size-2.5")]
                    data-class:border-primary=[shared::attr_value(&item.attributes, "data-class:border-primary")]
                    data-class:bg-primary=[shared::attr_value(&item.attributes, "data-class:bg-primary")]
                {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn helpers_cover_defaults_builders_and_all_dot_states() {
        let item = Item::default()
            .with_label("Step 1")
            .with_completed(true)
            .with_current(false)
            .with_class("custom")
            .with_attribute("data-testid", "step");
        assert_eq!(item.label, "Step 1");
        assert!(item.completed);
        assert!(!item.current);
        assert_eq!(item.class, "custom");
        assert_eq!(
            item.attributes.get("data-testid").map(String::as_str),
            Some("step")
        );

        assert_eq!(
            DotClasses(&Item::default()),
            "block rounded-full border transition-all duration-150 size-2 border-[var(--dot-inactive-border)] bg-[var(--dot-inactive)]",
        );
        assert_eq!(
            DotClasses(&Item::default().with_completed(true).with_class("done")),
            "block rounded-full border transition-all duration-150 size-2.5 border-success bg-success done",
        );
        assert_eq!(
            DotClasses(&Item::default().with_completed(true).with_current(true)),
            "block rounded-full border transition-all duration-150 size-2.5 border-primary bg-primary shadow-[0_0_0_4px_var(--dot-active-glow)]",
        );

        let props = Props::default()
            .with_items(vec![item])
            .with_class("steps")
            .with_attribute("id", "stepper");
        assert_eq!(props.items.len(), 1);
        assert_eq!(
            props.build_classes(),
            "inline-flex items-center gap-2 steps"
        );
        assert_eq!(
            props.attributes.get("id").map(String::as_str),
            Some("stepper")
        );
    }

    #[test]
    fn stepper_renders_items_labels_attrs_and_escaping() {
        let rendered = Stepper(
            &Props::default()
                .with_class("progress")
                .with_attribute("data-testid", "stepper")
                .with_items(vec![
                    Item::default()
                        .with_label("<Start>")
                        .with_completed(true)
                        .with_attribute("data-testid", "start"),
                    Item::default().with_label("Current").with_current(true),
                    Item::default(),
                ]),
        )
        .into_string();
        assert!(rendered.starts_with(
            "<div class=\"inline-flex items-center gap-2 progress\" data-testid=\"stepper\">"
        ));
        assert!(rendered.contains("class=\"block rounded-full border transition-all duration-150 size-2.5 border-success bg-success\" aria-label=\"&lt;Start&gt;\" title=\"&lt;Start&gt;\" data-testid=\"start\""));
        assert!(rendered.contains("class=\"block rounded-full border transition-all duration-150 size-2.5 border-primary bg-primary shadow-[0_0_0_4px_var(--dot-active-glow)]\" aria-label=\"Current\" title=\"Current\""));
        assert!(rendered.contains("class=\"block rounded-full border transition-all duration-150 size-2 border-[var(--dot-inactive-border)] bg-[var(--dot-inactive)]\"></span>"));
        assert_eq!(rendered.matches("<span ").count(), 3);
    }

    #[test]
    fn empty_stepper_renders_empty_wrapper() {
        assert_eq!(
            Stepper(&Props::default()).into_string(),
            "<div class=\"inline-flex items-center gap-2\"></div>",
        );
    }
}
