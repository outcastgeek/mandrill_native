#![allow(non_snake_case)]

use crate::components::{ComponentInfo, shared};
use maud::{Markup, html};
use std::collections::BTreeMap;

pub const COMPONENTS: &[ComponentInfo] = &[ComponentInfo {
    family: "unstyled",
    module: "mermaid",
    name: "Widget",
}];

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct WidgetProps {
    pub source: String,
    pub class: String,
    pub attributes: BTreeMap<String, String>,
}

impl WidgetProps {
    #[must_use]
    pub fn with_source(mut self, value: impl Into<String>) -> Self {
        self.source = value.into();
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
        shared::build_classes(&["not-prose mermaid-widget", self.class.as_str()])
    }

    #[must_use]
    pub fn build_attributes(&self) -> BTreeMap<String, String> {
        self.attributes.clone()
    }
}

#[must_use]
pub fn Widget(props: &WidgetProps) -> Markup {
    if props.source.is_empty() {
        return html! {};
    }
    let attrs = props.build_attributes();
    html! {
        div
            class=(props.build_classes())
            data-mermaid-widget=""
            id=[shared::attr_value(&attrs, "id")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
        {
            pre class="m-0" {
                code class="language-mermaid" data-mermaid-source="true" {
                    (&props.source)
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn helpers_build_classes_and_copy_attributes() {
        assert_eq!(
            WidgetProps::default().build_classes(),
            "not-prose mermaid-widget"
        );
        assert_eq!(
            WidgetProps::default().with_class("rounded").build_classes(),
            "not-prose mermaid-widget rounded"
        );
        let attrs = WidgetProps::default()
            .with_attribute("id", "chart")
            .with_attribute("data-testid", "mermaid")
            .build_attributes();
        assert_eq!(attrs.get("id").map(String::as_str), Some("chart"));
        assert_eq!(
            attrs.get("data-testid").map(String::as_str),
            Some("mermaid")
        );
    }

    #[test]
    fn widget_renders_source_and_empty_source_renders_nothing() {
        assert_eq!(Widget(&WidgetProps::default()).into_string(), "");
        let rendered = Widget(
            &WidgetProps::default()
                .with_source("graph TD\nA[<Start>] --> B")
                .with_class("p-4")
                .with_attribute("id", "flow")
                .with_attribute("data-testid", "flow-chart"),
        )
        .into_string();
        assert!(rendered.starts_with("<div class=\"not-prose mermaid-widget p-4\""));
        assert!(rendered.contains("data-mermaid-widget=\"\""));
        assert!(rendered.contains("id=\"flow\""));
        assert!(rendered.contains("data-testid=\"flow-chart\""));
        assert!(rendered.contains(
            "<pre class=\"m-0\"><code class=\"language-mermaid\" data-mermaid-source=\"true\">"
        ));
        assert!(rendered.contains("graph TD\nA[&lt;Start&gt;] --&gt; B"));
        assert!(rendered.ends_with("</code></pre></div>"));
    }
}
