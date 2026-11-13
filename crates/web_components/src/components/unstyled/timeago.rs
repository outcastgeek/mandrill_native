#![allow(non_snake_case)]

use crate::components::{ComponentInfo, shared};
use maud::{Markup, html};
use std::collections::BTreeMap;

pub const COMPONENTS: &[ComponentInfo] = &[
    ComponentInfo {
        family: "unstyled",
        module: "timeago",
        name: "TimeAgo",
    },
    ComponentInfo {
        family: "unstyled",
        module: "timeago",
        name: "SimpleTimeAgo",
    },
];

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct TimeAgoProps {
    pub datetime: String,
    pub display: String,
    pub class: String,
    pub attributes: BTreeMap<String, String>,
}

impl TimeAgoProps {
    #[must_use]
    pub fn with_datetime(mut self, value: impl Into<String>) -> Self {
        self.datetime = value.into();
        self
    }

    #[must_use]
    pub fn with_display(mut self, value: impl Into<String>) -> Self {
        self.display = value.into();
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
        shared::build_classes(&["text-xs text-muted-foreground", self.class.as_str()])
    }

    #[must_use]
    pub fn build_attributes(&self) -> BTreeMap<String, String> {
        self.attributes.clone()
    }
}

#[must_use]
pub fn TimeAgo(props: &TimeAgoProps) -> Markup {
    if props.display.is_empty() {
        return html! {};
    }
    let attrs = props.build_attributes();
    if props.datetime.is_empty() {
        html! {
            span
                class=(props.build_classes())
                id=[shared::attr_value(&attrs, "id")]
                data-testid=[shared::attr_value(&attrs, "data-testid")]
                title=[shared::attr_value(&attrs, "title")]
            {
                (&props.display)
            }
        }
    } else {
        html! {
            time
                datetime=(&props.datetime)
                data-timeago=""
                class=(props.build_classes())
                id=[shared::attr_value(&attrs, "id")]
                data-testid=[shared::attr_value(&attrs, "data-testid")]
                title=[shared::attr_value(&attrs, "title")]
            {
                (&props.display)
            }
        }
    }
}

#[must_use]
pub fn SimpleTimeAgo(datetime: &str, display: &str, class: &str) -> Markup {
    TimeAgo(
        &TimeAgoProps::default()
            .with_datetime(datetime)
            .with_display(display)
            .with_class(class),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn helpers_build_classes_and_copy_attributes() {
        assert_eq!(
            TimeAgoProps::default().build_classes(),
            "text-xs text-muted-foreground"
        );
        assert_eq!(
            TimeAgoProps::default().with_class("inline").build_classes(),
            "text-xs text-muted-foreground inline"
        );
        let attrs = TimeAgoProps::default()
            .with_attribute("id", "created")
            .with_attribute("title", "Created at")
            .build_attributes();
        assert_eq!(attrs.get("id").map(String::as_str), Some("created"));
        assert_eq!(attrs.get("title").map(String::as_str), Some("Created at"));
    }

    #[test]
    fn timeago_renders_time_span_and_empty_variations() {
        assert_eq!(TimeAgo(&TimeAgoProps::default()).into_string(), "");
        let time = TimeAgo(
            &TimeAgoProps::default()
                .with_datetime("2026-01-05T10:00:00Z")
                .with_display("<2 hours ago>")
                .with_class("block")
                .with_attribute("id", "created")
                .with_attribute("data-testid", "relative-time"),
        )
        .into_string();
        assert_eq!(
            time,
            "<time datetime=\"2026-01-05T10:00:00Z\" data-timeago=\"\" class=\"text-xs text-muted-foreground block\" id=\"created\" data-testid=\"relative-time\">&lt;2 hours ago&gt;</time>"
        );

        let span = TimeAgo(
            &TimeAgoProps::default()
                .with_display("Jan 5, 2026")
                .with_attribute("title", "No datetime"),
        )
        .into_string();
        assert_eq!(
            span,
            "<span class=\"text-xs text-muted-foreground\" title=\"No datetime\">Jan 5, 2026</span>"
        );

        let simple = SimpleTimeAgo("2026-01-05T10:00:00Z", "2 hours ago", "inline").into_string();
        assert!(simple.starts_with("<time "));
        assert!(simple.contains("datetime=\"2026-01-05T10:00:00Z\""));
        assert!(simple.contains("class=\"text-xs text-muted-foreground inline\""));
        assert!(simple.contains(">2 hours ago</time>"));
    }
}
