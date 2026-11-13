#![allow(non_snake_case)]

use crate::components::{ComponentInfo, shared};
use maud::{Markup, html};
use std::collections::BTreeMap;

pub const COMPONENTS: &[ComponentInfo] = &[
    ComponentInfo {
        family: "unstyled",
        module: "list",
        name: "List",
    },
    ComponentInfo {
        family: "unstyled",
        module: "list",
        name: "ListItem",
    },
    ComponentInfo {
        family: "unstyled",
        module: "list",
        name: "DescriptionList",
    },
    ComponentInfo {
        family: "unstyled",
        module: "list",
        name: "DescriptionTerm",
    },
    ComponentInfo {
        family: "unstyled",
        module: "list",
        name: "DescriptionDetails",
    },
    ComponentInfo {
        family: "unstyled",
        module: "list",
        name: "UnorderedList",
    },
    ComponentInfo {
        family: "unstyled",
        module: "list",
        name: "OrderedList",
    },
    ComponentInfo {
        family: "unstyled",
        module: "list",
        name: "IconList",
    },
];

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ListProps {
    pub class: String,
    pub attributes: BTreeMap<String, String>,
    pub ordered: bool,
    pub size: &'static str,
    pub spacing: &'static str,
    pub aria_label: String,
    pub aria_described_by: String,
}

impl Default for ListProps {
    fn default() -> Self {
        Self {
            class: String::new(),
            attributes: BTreeMap::new(),
            ordered: false,
            size: shared::SIZE_MD,
            spacing: "md",
            aria_label: String::new(),
            aria_described_by: String::new(),
        }
    }
}

impl ListProps {
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
    pub const fn with_ordered(mut self, value: bool) -> Self {
        self.ordered = value;
        self
    }

    #[must_use]
    pub const fn with_size(mut self, value: &'static str) -> Self {
        self.size = value;
        self
    }

    #[must_use]
    pub const fn with_spacing(mut self, value: &'static str) -> Self {
        self.spacing = value;
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
        let list_style = if self.ordered {
            "list-decimal"
        } else {
            "list-disc"
        };
        let spacing = match self.spacing {
            "none" => "space-y-0",
            "sm" => "space-y-1",
            "lg" => "space-y-4",
            _ => "space-y-2",
        };
        let size = match self.size {
            shared::SIZE_SM => "text-sm",
            shared::SIZE_LG => "text-lg",
            shared::SIZE_XL => "text-xl",
            _ => "text-base",
        };
        shared::build_classes(&["pl-8", list_style, spacing, size, self.class.as_str()])
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
        attrs
    }
}

#[derive(Clone, Debug, Default)]
pub struct ListItemProps {
    pub class: String,
    pub attributes: BTreeMap<String, String>,
    pub icon: Option<Markup>,
}

impl ListItemProps {
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
    pub fn with_icon(mut self, icon: Markup) -> Self {
        self.icon = Some(icon);
        self
    }

    #[must_use]
    pub fn build_classes(&self) -> String {
        shared::build_classes(&[
            if self.icon.is_some() {
                "flex items-start gap-2"
            } else {
                ""
            },
            self.class.as_str(),
        ])
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DescriptionListProps {
    pub class: String,
    pub attributes: BTreeMap<String, String>,
    pub horizontal: bool,
    pub size: &'static str,
}

impl Default for DescriptionListProps {
    fn default() -> Self {
        Self {
            class: String::new(),
            attributes: BTreeMap::new(),
            horizontal: false,
            size: shared::SIZE_MD,
        }
    }
}

impl DescriptionListProps {
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
    pub const fn with_horizontal(mut self, value: bool) -> Self {
        self.horizontal = value;
        self
    }

    #[must_use]
    pub const fn with_size(mut self, value: &'static str) -> Self {
        self.size = value;
        self
    }

    #[must_use]
    pub fn build_classes(&self) -> String {
        let layout = if self.horizontal {
            "grid grid-cols-[auto_1fr] gap-x-4 gap-y-2"
        } else {
            "space-y-4"
        };
        let size = match self.size {
            shared::SIZE_SM => "text-sm",
            shared::SIZE_LG => "text-lg",
            _ => "text-base",
        };
        shared::build_classes(&[layout, size, self.class.as_str()])
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DescriptionTermProps {
    pub class: String,
    pub attributes: BTreeMap<String, String>,
}

impl DescriptionTermProps {
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
        shared::build_classes(&["font-semibold text-foreground", self.class.as_str()])
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DescriptionDetailsProps {
    pub class: String,
    pub attributes: BTreeMap<String, String>,
}

impl DescriptionDetailsProps {
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
        shared::build_classes(&["text-muted-foreground", self.class.as_str()])
    }
}

#[must_use]
pub fn List(props: &ListProps, children: Markup) -> Markup {
    let attrs = props.build_attributes();
    if props.ordered {
        html! {
            ol
                class=(props.build_classes())
                aria-label=[shared::attr_value(&attrs, "aria-label")]
                aria-describedby=[shared::attr_value(&attrs, "aria-describedby")]
                id=[shared::attr_value(&attrs, "id")]
                data-testid=[shared::attr_value(&attrs, "data-testid")]
            {
                (children)
            }
        }
    } else {
        html! {
            ul
                class=(props.build_classes())
                aria-label=[shared::attr_value(&attrs, "aria-label")]
                aria-describedby=[shared::attr_value(&attrs, "aria-describedby")]
                id=[shared::attr_value(&attrs, "id")]
                data-testid=[shared::attr_value(&attrs, "data-testid")]
            {
                (children)
            }
        }
    }
}

#[must_use]
pub fn ListItem(props: &ListItemProps, children: Markup) -> Markup {
    let attrs = props.attributes.clone();
    html! {
        li
            class=(props.build_classes())
            id=[shared::attr_value(&attrs, "id")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
        {
            @if let Some(icon) = &props.icon {
                span class="flex-shrink-0 mt-0.5" aria-hidden="true" {
                    (icon.clone())
                }
                span class="flex-1" {
                    (children)
                }
            } @else {
                (children)
            }
        }
    }
}

#[must_use]
pub fn DescriptionList(props: &DescriptionListProps, children: Markup) -> Markup {
    let attrs = props.attributes.clone();
    html! {
        dl
            class=(props.build_classes())
            id=[shared::attr_value(&attrs, "id")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
        {
            (children)
        }
    }
}

#[must_use]
pub fn DescriptionTerm(props: &DescriptionTermProps, children: Markup) -> Markup {
    let attrs = props.attributes.clone();
    html! {
        dt
            class=(props.build_classes())
            id=[shared::attr_value(&attrs, "id")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
        {
            (children)
        }
    }
}

#[must_use]
pub fn DescriptionDetails(props: &DescriptionDetailsProps, children: Markup) -> Markup {
    let attrs = props.attributes.clone();
    html! {
        dd
            class=(props.build_classes())
            id=[shared::attr_value(&attrs, "id")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
        {
            (children)
        }
    }
}

#[must_use]
pub fn UnorderedList(props: &ListProps, children: Markup) -> Markup {
    let mut list_props = props.clone();
    list_props.ordered = false;
    List(&list_props, children)
}

#[must_use]
pub fn OrderedList(props: &ListProps, children: Markup) -> Markup {
    let mut list_props = props.clone();
    list_props.ordered = true;
    List(&list_props, children)
}

#[must_use]
pub fn IconList(props: &ListProps, _icon: Markup, children: Markup) -> Markup {
    List(props, children)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn helper_classes_cover_list_spacing_sizes_and_attributes() {
        assert_eq!(
            ListProps::default().build_classes(),
            "pl-8 list-disc space-y-2 text-base"
        );
        assert_eq!(
            ListProps::default()
                .with_ordered(true)
                .with_spacing("none")
                .with_size(shared::SIZE_SM)
                .with_class("custom")
                .build_classes(),
            "pl-8 list-decimal space-y-0 text-sm custom"
        );
        assert_eq!(
            ListProps::default()
                .with_spacing("sm")
                .with_size(shared::SIZE_LG)
                .build_classes(),
            "pl-8 list-disc space-y-1 text-lg"
        );
        assert_eq!(
            ListProps::default()
                .with_spacing("lg")
                .with_size(shared::SIZE_XL)
                .build_classes(),
            "pl-8 list-disc space-y-4 text-xl"
        );
        let attrs = ListProps::default()
            .with_attribute("id", "features")
            .with_aria_label("Feature list")
            .with_aria_described_by("features-help")
            .build_attributes();
        assert_eq!(attrs.get("id").map(String::as_str), Some("features"));
        assert_eq!(
            attrs.get("aria-label").map(String::as_str),
            Some("Feature list")
        );
        assert_eq!(
            attrs.get("aria-describedby").map(String::as_str),
            Some("features-help")
        );
    }

    #[test]
    fn helper_classes_cover_items_and_description_list_parts() {
        assert_eq!(ListItemProps::default().build_classes(), "");
        assert_eq!(
            ListItemProps::default()
                .with_icon(html! { svg {} })
                .with_class("item")
                .build_classes(),
            "flex items-start gap-2 item"
        );
        assert_eq!(
            DescriptionListProps::default().build_classes(),
            "space-y-4 text-base"
        );
        assert_eq!(
            DescriptionListProps::default()
                .with_horizontal(true)
                .with_size(shared::SIZE_SM)
                .with_class("dl")
                .build_classes(),
            "grid grid-cols-[auto_1fr] gap-x-4 gap-y-2 text-sm dl"
        );
        assert_eq!(
            DescriptionListProps::default()
                .with_size(shared::SIZE_LG)
                .build_classes(),
            "space-y-4 text-lg"
        );
        assert_eq!(
            DescriptionTermProps::default()
                .with_class("term")
                .build_classes(),
            "font-semibold text-foreground term"
        );
        assert_eq!(
            DescriptionDetailsProps::default()
                .with_class("details")
                .build_classes(),
            "text-muted-foreground details"
        );
    }

    #[test]
    fn list_and_list_item_render_all_variations_and_escape_content() {
        let child = ListItem(
            &ListItemProps::default().with_attribute("data-testid", "one"),
            html! { "<One>" },
        );
        let unordered = List(
            &ListProps::default()
                .with_aria_label("Items")
                .with_attribute("id", "items"),
            child,
        )
        .into_string();
        assert!(unordered.starts_with("<ul "));
        assert!(unordered.contains("class=\"pl-8 list-disc space-y-2 text-base\""));
        assert!(unordered.contains("aria-label=\"Items\""));
        assert!(unordered.contains("id=\"items\""));
        assert!(unordered.contains("<li class=\"\" data-testid=\"one\">&lt;One&gt;</li>"));

        let ordered = OrderedList(
            &ListProps::default().with_spacing("lg"),
            ListItem(
                &ListItemProps::default().with_icon(html! { svg data-testid="icon" {} }),
                html! { "Item" },
            ),
        )
        .into_string();
        assert!(ordered.starts_with("<ol "));
        assert!(ordered.contains("list-decimal space-y-4"));
        assert!(ordered.contains("<span class=\"flex-shrink-0 mt-0.5\" aria-hidden=\"true\"><svg data-testid=\"icon\"></svg></span>"));
        assert!(ordered.contains("<span class=\"flex-1\">Item</span>"));

        let forced_unordered =
            UnorderedList(&ListProps::default().with_ordered(true), html! {}).into_string();
        assert!(forced_unordered.starts_with("<ul "));
        let icon_list = IconList(
            &ListProps::default(),
            html! { svg {} },
            html! { li { "Body" } },
        )
        .into_string();
        assert!(icon_list.starts_with("<ul "));
        assert!(icon_list.contains("<li>Body</li>"));
    }

    #[test]
    fn description_list_renders_terms_and_details() {
        let rendered = DescriptionList(
            &DescriptionListProps::default()
                .with_horizontal(true)
                .with_attribute("data-testid", "metadata"),
            html! {
                (DescriptionTerm(&DescriptionTermProps::default().with_attribute("id", "term"), html! { "Author" }))
                (DescriptionDetails(&DescriptionDetailsProps::default().with_attribute("id", "details"), html! { "<Ada>" }))
            },
        )
        .into_string();
        assert!(rendered.starts_with("<dl "));
        assert!(rendered.contains("grid grid-cols-[auto_1fr] gap-x-4 gap-y-2 text-base"));
        assert!(rendered.contains("data-testid=\"metadata\""));
        assert!(
            rendered
                .contains("<dt class=\"font-semibold text-foreground\" id=\"term\">Author</dt>")
        );
        assert!(
            rendered
                .contains("<dd class=\"text-muted-foreground\" id=\"details\">&lt;Ada&gt;</dd>")
        );
    }
}
