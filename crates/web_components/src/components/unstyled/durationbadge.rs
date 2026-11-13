#![allow(non_snake_case)]

use crate::components::{ComponentInfo, shared};
use maud::{Markup, html};
use std::collections::BTreeMap;

pub const COMPONENTS: &[ComponentInfo] = &[
    ComponentInfo {
        family: "unstyled",
        module: "durationbadge",
        name: "DurationBadge",
    },
    ComponentInfo {
        family: "unstyled",
        module: "durationbadge",
        name: "SimpleDurationBadge",
    },
];

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DurationBadgeProps {
    pub duration: String,
    pub size: &'static str,
    pub show_icon: bool,
    pub class: String,
    pub attributes: BTreeMap<String, String>,
}

impl Default for DurationBadgeProps {
    fn default() -> Self {
        Self {
            duration: String::new(),
            size: shared::SIZE_SM,
            show_icon: true,
            class: String::new(),
            attributes: BTreeMap::new(),
        }
    }
}

impl DurationBadgeProps {
    #[must_use]
    pub fn with_duration(mut self, duration: impl Into<String>) -> Self {
        self.duration = duration.into();
        self
    }

    #[must_use]
    pub fn with_size(mut self, size: &'static str) -> Self {
        self.size = size;
        self
    }

    #[must_use]
    pub const fn with_icon(mut self, show: bool) -> Self {
        self.show_icon = show;
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
        shared::build_classes(&[
            "inline-flex items-center gap-1 px-2 py-1 text-xs font-medium rounded",
            self.class.as_str(),
        ])
    }

    #[must_use]
    pub fn build_attributes(&self) -> BTreeMap<String, String> {
        self.attributes.clone()
    }
}

#[must_use]
pub fn DurationBadge(props: &DurationBadgeProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        span
            class=(props.build_classes())
            id=[shared::attr_value(&attrs, "id")]
            title=[shared::attr_value(&attrs, "title")]
            data-show=[shared::attr_value(&attrs, "data-show")]
        {
            @if props.show_icon {
                svg class="w-3 h-3" fill="currentColor" viewBox="0 0 20 20" aria-hidden="true" {
                    path fill-rule="evenodd" d="M10 18a8 8 0 100-16 8 8 0 000 16zm1-12a1 1 0 10-2 0v4a1 1 0 00.293.707l2.828 2.829a1 1 0 101.415-1.415L11 9.586V6z" clip-rule="evenodd" {}
                }
            }
            span { (props.duration) }
        }
    }
}

#[must_use]
pub fn SimpleDurationBadge(duration: impl Into<String>) -> Markup {
    DurationBadge(&DurationBadgeProps::default().with_duration(duration))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_props_match_go_defaults() {
        let props = DurationBadgeProps::default();
        assert_eq!(props.size, shared::SIZE_SM);
        assert!(props.show_icon);
        assert!(props.duration.is_empty());
    }

    #[test]
    fn helper_methods_build_classes_and_attributes() {
        let props = DurationBadgeProps::default()
            .with_duration("3:45")
            .with_size(shared::SIZE_LG)
            .with_icon(false)
            .with_class("custom")
            .with_attribute("title", "Duration");
        assert_eq!(props.duration, "3:45");
        assert_eq!(props.size, shared::SIZE_LG);
        assert!(!props.show_icon);
        assert_eq!(
            props.build_classes(),
            "inline-flex items-center gap-1 px-2 py-1 text-xs font-medium rounded custom"
        );
        assert_eq!(
            props.build_attributes().get("title"),
            Some(&"Duration".to_string())
        );
    }

    #[test]
    fn duration_badge_renders_icon_optional_and_escaped_duration() {
        for show_icon in [false, true] {
            let html = DurationBadge(
                &DurationBadgeProps::default()
                    .with_duration("<3:45>")
                    .with_icon(show_icon)
                    .with_attribute("title", "\"Length\" & time"),
            )
            .into_string();

            assert!(html.starts_with(r#"<span class="inline-flex items-center gap-1 px-2 py-1 text-xs font-medium rounded""#));
            assert!(html.contains(r#"title="&quot;Length&quot; &amp; time""#));
            assert_eq!(html.contains("<svg "), show_icon);
            assert!(html.contains(r#"<span>&lt;3:45&gt;</span>"#));
        }
    }

    #[test]
    fn simple_duration_badge_uses_default_props() {
        let html = SimpleDurationBadge("1:23").into_string();
        assert!(html.contains("<svg "));
        assert!(html.contains("<span>1:23</span>"));
    }
}
