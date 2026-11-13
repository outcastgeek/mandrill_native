#![allow(non_snake_case)]

use crate::components::{ComponentInfo, shared};
use maud::{Markup, html};
use std::collections::BTreeMap;

pub const COMPONENTS: &[ComponentInfo] = &[
    ComponentInfo {
        family: "unstyled",
        module: "stack",
        name: "Stack",
    },
    ComponentInfo {
        family: "unstyled",
        module: "stack",
        name: "VStack",
    },
    ComponentInfo {
        family: "unstyled",
        module: "stack",
        name: "HStack",
    },
    ComponentInfo {
        family: "unstyled",
        module: "stack",
        name: "Spacer",
    },
];

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StackProps {
    pub direction: &'static str,
    pub gap: &'static str,
    pub align: &'static str,
    pub justify: &'static str,
    pub wrap: bool,
    pub class: String,
    pub attributes: BTreeMap<String, String>,
}

impl Default for StackProps {
    fn default() -> Self {
        Self {
            direction: shared::STACK_VERTICAL,
            gap: shared::SIZE_MD,
            align: shared::STACK_ALIGN_STRETCH,
            justify: shared::STACK_JUSTIFY_START,
            wrap: false,
            class: String::new(),
            attributes: BTreeMap::new(),
        }
    }
}

impl StackProps {
    #[must_use]
    pub fn with_direction(mut self, direction: &'static str) -> Self {
        self.direction = direction;
        self
    }

    #[must_use]
    pub fn with_gap(mut self, gap: &'static str) -> Self {
        self.gap = gap;
        self
    }

    #[must_use]
    pub fn with_align(mut self, align: &'static str) -> Self {
        self.align = align;
        self
    }

    #[must_use]
    pub fn with_justify(mut self, justify: &'static str) -> Self {
        self.justify = justify;
        self
    }

    #[must_use]
    pub const fn with_wrap(mut self, wrap: bool) -> Self {
        self.wrap = wrap;
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
    pub fn build_classes(&self) -> String {
        let direction_class = match self.direction {
            shared::STACK_HORIZONTAL => "flex-row",
            shared::STACK_VERTICAL => "flex-col",
            _ => "flex-col",
        };
        let gap_class = match self.gap {
            "none" => "gap-0",
            shared::SIZE_XS => "gap-1",
            shared::SIZE_SM => "gap-2",
            shared::SIZE_MD => "gap-4",
            shared::SIZE_LG => "gap-6",
            shared::SIZE_XL => "gap-8",
            _ => "gap-4",
        };
        let align_class = match self.align {
            shared::STACK_ALIGN_START => "items-start",
            shared::STACK_ALIGN_CENTER => "items-center",
            shared::STACK_ALIGN_END => "items-end",
            shared::STACK_ALIGN_STRETCH => "items-stretch",
            _ => "items-stretch",
        };
        let justify_class = match self.justify {
            shared::STACK_JUSTIFY_START => "justify-start",
            shared::STACK_JUSTIFY_CENTER => "justify-center",
            shared::STACK_JUSTIFY_END => "justify-end",
            shared::STACK_JUSTIFY_BETWEEN => "justify-between",
            shared::STACK_JUSTIFY_AROUND => "justify-around",
            "evenly" => "justify-evenly",
            _ => "justify-start",
        };
        shared::build_classes(&[
            "flex",
            direction_class,
            gap_class,
            align_class,
            justify_class,
            if self.wrap { "flex-wrap" } else { "" },
            self.class.as_str(),
        ])
    }

    #[must_use]
    pub fn build_attributes(&self) -> BTreeMap<String, String> {
        self.attributes.clone()
    }
}

#[must_use]
pub fn Stack(props: &StackProps, children: Markup) -> Markup {
    let attrs = props.build_attributes();
    html! {
        div
            class=(props.build_classes())
            id=[shared::attr_value(&attrs, "id")]
            title=[shared::attr_value(&attrs, "title")]
            role=[shared::attr_value(&attrs, "role")]
            data-on:click=[shared::attr_value(&attrs, "data-on:click")]
            data-show=[shared::attr_value(&attrs, "data-show")]
            data-bind=[shared::attr_value(&attrs, "data-bind")]
        {
            (children)
        }
    }
}

#[must_use]
pub fn VStack(
    gap: &'static str,
    align: &'static str,
    class: impl Into<String>,
    children: Markup,
) -> Markup {
    Stack(
        &StackProps::default()
            .with_direction(shared::STACK_VERTICAL)
            .with_gap(gap)
            .with_align(align)
            .with_justify(shared::STACK_JUSTIFY_START)
            .with_class(class),
        children,
    )
}

#[must_use]
pub fn HStack(
    gap: &'static str,
    align: &'static str,
    class: impl Into<String>,
    children: Markup,
) -> Markup {
    Stack(
        &StackProps::default()
            .with_direction(shared::STACK_HORIZONTAL)
            .with_gap(gap)
            .with_align(align)
            .with_justify(shared::STACK_JUSTIFY_START)
            .with_class(class),
        children,
    )
}

#[must_use]
pub fn Spacer() -> Markup {
    html! {
        div class="flex-1" {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DIRECTIONS: &[(&str, &str)] = &[
        (shared::STACK_VERTICAL, "flex-col"),
        (shared::STACK_HORIZONTAL, "flex-row"),
        ("unknown", "flex-col"),
    ];
    const GAPS: &[(&str, &str)] = &[
        ("none", "gap-0"),
        (shared::SIZE_XS, "gap-1"),
        (shared::SIZE_SM, "gap-2"),
        (shared::SIZE_MD, "gap-4"),
        (shared::SIZE_LG, "gap-6"),
        (shared::SIZE_XL, "gap-8"),
        ("unknown", "gap-4"),
    ];
    const ALIGNS: &[(&str, &str)] = &[
        (shared::STACK_ALIGN_START, "items-start"),
        (shared::STACK_ALIGN_CENTER, "items-center"),
        (shared::STACK_ALIGN_END, "items-end"),
        (shared::STACK_ALIGN_STRETCH, "items-stretch"),
        ("unknown", "items-stretch"),
    ];
    const JUSTIFIES: &[(&str, &str)] = &[
        (shared::STACK_JUSTIFY_START, "justify-start"),
        (shared::STACK_JUSTIFY_CENTER, "justify-center"),
        (shared::STACK_JUSTIFY_END, "justify-end"),
        (shared::STACK_JUSTIFY_BETWEEN, "justify-between"),
        (shared::STACK_JUSTIFY_AROUND, "justify-around"),
        ("evenly", "justify-evenly"),
        ("unknown", "justify-start"),
    ];

    #[test]
    fn default_props_match_go_defaults() {
        let props = StackProps::default();
        assert_eq!(props.direction, shared::STACK_VERTICAL);
        assert_eq!(props.gap, shared::SIZE_MD);
        assert_eq!(props.align, shared::STACK_ALIGN_STRETCH);
        assert_eq!(props.justify, shared::STACK_JUSTIFY_START);
        assert!(!props.wrap);
    }

    #[test]
    fn build_classes_covers_all_direction_gap_align_justify_wrap_combinations() {
        for &(direction, direction_class) in DIRECTIONS {
            for &(gap, gap_class) in GAPS {
                for &(align, align_class) in ALIGNS {
                    for &(justify, justify_class) in JUSTIFIES {
                        for wrap in [false, true] {
                            let classes = StackProps::default()
                                .with_direction(direction)
                                .with_gap(gap)
                                .with_align(align)
                                .with_justify(justify)
                                .with_wrap(wrap)
                                .with_class("custom")
                                .build_classes();

                            assert!(classes.contains("flex"));
                            assert!(classes.contains(direction_class));
                            assert!(classes.contains(gap_class));
                            assert!(classes.contains(align_class));
                            assert!(classes.contains(justify_class));
                            assert_eq!(classes.contains("flex-wrap"), wrap);
                            assert!(classes.contains("custom"));
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn build_attributes_returns_custom_attributes() {
        let attrs = StackProps::default()
            .with_attribute("data-on:click", "$open = true")
            .with_attribute("title", "Stack")
            .build_attributes();

        assert_eq!(
            attrs.get("data-on:click"),
            Some(&"$open = true".to_string())
        );
        assert_eq!(attrs.get("title"), Some(&"Stack".to_string()));
    }

    #[test]
    fn stack_renders_actual_html_attributes_children_and_escaping() {
        let html = Stack(
            &StackProps::default()
                .with_attribute("title", "\"Stack\" & more")
                .with_attribute("data-on:click", "$clicked = true")
                .with_wrap(true),
            html! { "<Item>" },
        )
        .into_string();

        assert!(html.starts_with(
            r#"<div class="flex flex-col gap-4 items-stretch justify-start flex-wrap""#
        ));
        assert!(html.contains(r#"title="&quot;Stack&quot; &amp; more""#));
        assert!(html.contains(r#"data-on:click="$clicked = true""#));
        assert!(html.contains("&lt;Item&gt;"));
        assert!(!html.contains("data-on-click"));
    }

    #[test]
    fn convenience_stacks_and_spacer_render_expected_html() {
        let vstack = VStack(
            shared::SIZE_SM,
            shared::STACK_ALIGN_CENTER,
            "v",
            html! { "V" },
        )
        .into_string();
        assert!(vstack.contains("flex-col"));
        assert!(vstack.contains("gap-2"));
        assert!(vstack.contains("items-center"));
        assert!(vstack.contains("v"));
        assert!(vstack.ends_with(">V</div>"));

        let hstack =
            HStack(shared::SIZE_LG, shared::STACK_ALIGN_END, "h", html! { "H" }).into_string();
        assert!(hstack.contains("flex-row"));
        assert!(hstack.contains("gap-6"));
        assert!(hstack.contains("items-end"));
        assert!(hstack.contains("h"));
        assert!(hstack.ends_with(">H</div>"));

        assert_eq!(Spacer().into_string(), r#"<div class="flex-1"></div>"#);
    }
}
