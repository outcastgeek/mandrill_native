#![allow(non_snake_case)]

use crate::components::{ComponentInfo, shared};
use maud::{Markup, html};
use std::collections::BTreeMap;

pub const COMPONENTS: &[ComponentInfo] = &[
    ComponentInfo {
        family: "unstyled",
        module: "statcard",
        name: "StatCard",
    },
    ComponentInfo {
        family: "unstyled",
        module: "statcard",
        name: "SimpleStatCard",
    },
];

#[derive(Clone, Debug)]
pub struct StatCardProps {
    pub value: String,
    pub label: String,
    pub gradient_from: String,
    pub gradient_to: String,
    pub via: String,
    pub icon: Option<Markup>,
    pub class: String,
    pub value_class: String,
    pub label_class: String,
    pub attributes: BTreeMap<String, String>,
}

impl Default for StatCardProps {
    fn default() -> Self {
        Self {
            value: String::new(),
            label: String::new(),
            gradient_from: "blue-600".to_string(),
            gradient_to: "purple-600".to_string(),
            via: String::new(),
            icon: None,
            class: String::new(),
            value_class: String::new(),
            label_class: String::new(),
            attributes: BTreeMap::new(),
        }
    }
}

impl StatCardProps {
    #[must_use]
    pub fn with_value(mut self, value: impl Into<String>) -> Self {
        self.value = value.into();
        self
    }

    #[must_use]
    pub fn with_label(mut self, value: impl Into<String>) -> Self {
        self.label = value.into();
        self
    }

    #[must_use]
    pub fn with_gradient(mut self, from: impl Into<String>, to: impl Into<String>) -> Self {
        self.gradient_from = from.into();
        self.gradient_to = to.into();
        self
    }

    #[must_use]
    pub fn with_via(mut self, value: impl Into<String>) -> Self {
        self.via = value.into();
        self
    }

    #[must_use]
    pub fn with_icon(mut self, icon: Markup) -> Self {
        self.icon = Some(icon);
        self
    }

    #[must_use]
    pub fn with_class(mut self, value: impl Into<String>) -> Self {
        self.class = value.into();
        self
    }

    #[must_use]
    pub fn with_value_class(mut self, value: impl Into<String>) -> Self {
        self.value_class = value.into();
        self
    }

    #[must_use]
    pub fn with_label_class(mut self, value: impl Into<String>) -> Self {
        self.label_class = value.into();
        self
    }

    #[must_use]
    pub fn with_attribute(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.attributes.insert(name.into(), value.into());
        self
    }

    #[must_use]
    pub fn build_value_classes(&self) -> String {
        let mut gradient_class = format!(
            "bg-gradient-to-r from-{} to-{}",
            self.gradient_from, self.gradient_to
        );
        if !self.via.is_empty() {
            gradient_class.push_str(&format!(" via-{}", self.via));
        }
        shared::build_classes(&[
            "text-4xl md:text-5xl font-bold bg-clip-text text-transparent",
            gradient_class.as_str(),
            self.value_class.as_str(),
        ])
    }

    #[must_use]
    pub fn build_attributes(&self) -> BTreeMap<String, String> {
        self.attributes.clone()
    }
}

#[must_use]
pub fn StatCard(props: &StatCardProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        div
            class=(format!("stat-card text-center {}", props.class))
            id=[shared::attr_value(&attrs, "id")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
        {
            @if let Some(icon) = &props.icon {
                div class="stat-icon mb-2 flex justify-center" {
                    (icon.clone())
                }
            }
            div class=(props.build_value_classes()) {
                (&props.value)
            }
            div class=(format!("text-sm font-medium mt-2 {}", props.label_class)) {
                (&props.label)
            }
        }
    }
}

#[must_use]
pub fn SimpleStatCard(value: &str, label: &str) -> Markup {
    StatCard(&StatCardProps::default().with_value(value).with_label(label))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn helpers_cover_defaults_gradients_via_classes_and_attrs() {
        let props = StatCardProps::default();
        assert_eq!(props.gradient_from, "blue-600");
        assert_eq!(props.gradient_to, "purple-600");
        assert_eq!(
            props.build_value_classes(),
            "text-4xl md:text-5xl font-bold bg-clip-text text-transparent bg-gradient-to-r from-blue-600 to-purple-600"
        );
        assert_eq!(
            StatCardProps::default()
                .with_gradient("success", "info")
                .with_via("warning")
                .with_value_class("tracking-tight")
                .build_value_classes(),
            "text-4xl md:text-5xl font-bold bg-clip-text text-transparent bg-gradient-to-r from-success to-info via-warning tracking-tight"
        );
        let attrs = StatCardProps::default()
            .with_attribute("id", "users")
            .with_attribute("data-testid", "stat")
            .build_attributes();
        assert_eq!(attrs.get("id").map(String::as_str), Some("users"));
        assert_eq!(attrs.get("data-testid").map(String::as_str), Some("stat"));
    }

    #[test]
    fn stat_card_renders_icon_value_label_attrs_and_escapes_text() {
        let html = StatCard(
            &StatCardProps::default()
                .with_value("<10M+>")
                .with_label("<Conversations>")
                .with_icon(html! { svg data-testid="icon" {} })
                .with_class("p-4")
                .with_value_class("leading-none")
                .with_label_class("text-muted-foreground")
                .with_attribute("id", "conversations")
                .with_attribute("data-testid", "stat-card"),
        )
        .into_string();
        assert!(html.starts_with("<div class=\"stat-card text-center p-4\""));
        assert!(html.contains("id=\"conversations\""));
        assert!(html.contains("data-testid=\"stat-card\""));
        assert!(html.contains("<div class=\"stat-icon mb-2 flex justify-center\"><svg data-testid=\"icon\"></svg></div>"));
        assert!(html.contains("class=\"text-4xl md:text-5xl font-bold bg-clip-text text-transparent bg-gradient-to-r from-blue-600 to-purple-600 leading-none\""));
        assert!(html.contains(">&lt;10M+&gt;</div>"));
        assert!(html.contains("<div class=\"text-sm font-medium mt-2 text-muted-foreground\">&lt;Conversations&gt;</div>"));
    }

    #[test]
    fn simple_stat_card_renders_minimal_configuration_without_icon() {
        let html = SimpleStatCard("95%", "Accuracy").into_string();
        assert!(html.starts_with("<div class=\"stat-card text-center \""));
        assert!(!html.contains("stat-icon"));
        assert!(html.contains(">95%</div>"));
        assert!(html.contains(">Accuracy</div>"));
        assert!(html.contains("from-blue-600 to-purple-600"));
    }
}
