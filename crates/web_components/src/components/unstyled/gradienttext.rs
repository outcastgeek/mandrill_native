#![allow(non_snake_case)]

use crate::components::{ComponentInfo, shared};
use maud::{Markup, html};
use std::collections::BTreeMap;

pub const COMPONENTS: &[ComponentInfo] = &[
    ComponentInfo {
        family: "unstyled",
        module: "gradienttext",
        name: "GradientText",
    },
    ComponentInfo {
        family: "unstyled",
        module: "gradienttext",
        name: "GradientHeading",
    },
    ComponentInfo {
        family: "unstyled",
        module: "gradienttext",
        name: "GradientSpan",
    },
];

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GradientTextProps {
    pub text: String,
    pub level: &'static str,
    pub from: String,
    pub to: String,
    pub via: String,
    pub direction: String,
    pub class: String,
    pub attributes: BTreeMap<String, String>,
}

impl Default for GradientTextProps {
    fn default() -> Self {
        Self {
            text: String::new(),
            level: "span",
            from: "blue-600".to_string(),
            to: "purple-600".to_string(),
            via: String::new(),
            direction: "to-r".to_string(),
            class: String::new(),
            attributes: BTreeMap::new(),
        }
    }
}

impl GradientTextProps {
    #[must_use]
    pub fn with_text(mut self, text: impl Into<String>) -> Self {
        self.text = text.into();
        self
    }

    #[must_use]
    pub const fn as_heading(mut self, level: &'static str) -> Self {
        self.level = level;
        self
    }

    #[must_use]
    pub fn with_colors(mut self, from: impl Into<String>, to: impl Into<String>) -> Self {
        self.from = from.into();
        self.to = to.into();
        self
    }

    #[must_use]
    pub fn with_via(mut self, via: impl Into<String>) -> Self {
        self.via = via.into();
        self
    }

    #[must_use]
    pub fn with_direction(mut self, direction: impl Into<String>) -> Self {
        self.direction = direction.into();
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
        let mut gradient_class = format!(
            "bg-gradient-{} from-{} to-{}",
            self.direction, self.from, self.to
        );
        if !self.via.is_empty() {
            gradient_class.push_str(" via-");
            gradient_class.push_str(&self.via);
        }

        shared::build_classes(&[
            "bg-clip-text text-transparent",
            gradient_class.as_str(),
            self.class.as_str(),
        ])
    }

    #[must_use]
    pub fn build_attributes(&self) -> BTreeMap<String, String> {
        self.attributes.clone()
    }
}

fn body(props: &GradientTextProps, children: Markup) -> Markup {
    html! {
        @if !props.text.is_empty() {
            (&props.text)
        } @else {
            (children)
        }
    }
}

#[must_use]
pub fn GradientText(props: &GradientTextProps, children: Markup) -> Markup {
    let attrs = props.build_attributes();
    let class = props.build_classes();
    match props.level {
        "h1" => {
            html! { h1 class=(class) id=[shared::attr_value(&attrs, "id")] title=[shared::attr_value(&attrs, "title")] data-testid=[shared::attr_value(&attrs, "data-testid")] { (body(props, children)) } }
        }
        "h2" => {
            html! { h2 class=(class) id=[shared::attr_value(&attrs, "id")] title=[shared::attr_value(&attrs, "title")] data-testid=[shared::attr_value(&attrs, "data-testid")] { (body(props, children)) } }
        }
        "h3" => {
            html! { h3 class=(class) id=[shared::attr_value(&attrs, "id")] title=[shared::attr_value(&attrs, "title")] data-testid=[shared::attr_value(&attrs, "data-testid")] { (body(props, children)) } }
        }
        "h4" => {
            html! { h4 class=(class) id=[shared::attr_value(&attrs, "id")] title=[shared::attr_value(&attrs, "title")] data-testid=[shared::attr_value(&attrs, "data-testid")] { (body(props, children)) } }
        }
        "h5" => {
            html! { h5 class=(class) id=[shared::attr_value(&attrs, "id")] title=[shared::attr_value(&attrs, "title")] data-testid=[shared::attr_value(&attrs, "data-testid")] { (body(props, children)) } }
        }
        "h6" => {
            html! { h6 class=(class) id=[shared::attr_value(&attrs, "id")] title=[shared::attr_value(&attrs, "title")] data-testid=[shared::attr_value(&attrs, "data-testid")] { (body(props, children)) } }
        }
        "p" => {
            html! { p class=(class) id=[shared::attr_value(&attrs, "id")] title=[shared::attr_value(&attrs, "title")] data-testid=[shared::attr_value(&attrs, "data-testid")] { (body(props, children)) } }
        }
        _ => {
            html! { span class=(class) id=[shared::attr_value(&attrs, "id")] title=[shared::attr_value(&attrs, "title")] data-testid=[shared::attr_value(&attrs, "data-testid")] { (body(props, children)) } }
        }
    }
}

#[must_use]
pub fn GradientHeading(
    level: &'static str,
    text: impl Into<String>,
    from: impl Into<String>,
    to: impl Into<String>,
) -> Markup {
    GradientText(
        &GradientTextProps::default()
            .as_heading(level)
            .with_text(text)
            .with_colors(from, to)
            .with_direction("to-r"),
        html! {},
    )
}

#[must_use]
pub fn GradientSpan(
    text: impl Into<String>,
    from: impl Into<String>,
    to: impl Into<String>,
) -> Markup {
    GradientText(
        &GradientTextProps::default()
            .as_heading("span")
            .with_text(text)
            .with_colors(from, to)
            .with_direction("to-r"),
        html! {},
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_props_match_go_defaults() {
        let props = GradientTextProps::default();
        assert_eq!(props.level, "span");
        assert_eq!(props.from, "blue-600");
        assert_eq!(props.to, "purple-600");
        assert_eq!(props.direction, "to-r");
    }

    #[test]
    fn helper_methods_build_gradient_classes_and_attrs() {
        let props = GradientTextProps::default()
            .with_colors("red-500", "green-500")
            .with_via("yellow-500")
            .with_direction("to-br")
            .with_class("font-bold")
            .with_attribute("title", "Gradient");

        assert_eq!(
            props.build_classes(),
            "bg-clip-text text-transparent bg-gradient-to-br from-red-500 to-green-500 via-yellow-500 font-bold"
        );
        assert_eq!(
            props.build_attributes().get("title").map(String::as_str),
            Some("Gradient")
        );
    }

    #[test]
    fn gradient_text_renders_all_supported_tags_and_escapes_text_or_children() {
        for level in ["h1", "h2", "h3", "h4", "h5", "h6", "p", "span", "bad"] {
            let html = GradientText(
                &GradientTextProps::default()
                    .as_heading(level)
                    .with_text("<Text>"),
                html! { "child" },
            )
            .into_string();
            let tag = if level == "bad" { "span" } else { level };
            assert!(html.starts_with(&format!("<{tag} ")));
            assert!(html.contains("&lt;Text&gt;"));
            assert!(!html.contains("child"));
            assert!(html.ends_with(&format!("</{tag}>")));
        }

        let child_html = GradientText(
            &GradientTextProps::default().with_attribute("data-testid", "gradient"),
            html! { "<Child>" },
        )
        .into_string();
        assert!(child_html.contains("data-testid=\"gradient\""));
        assert!(child_html.contains("&lt;Child&gt;"));
    }

    #[test]
    fn convenience_components_render_expected_levels_and_colors() {
        let heading = GradientHeading("h2", "Title", "cyan-500", "emerald-500").into_string();
        assert!(heading.starts_with("<h2 "));
        assert!(heading.contains("from-cyan-500 to-emerald-500"));
        assert!(heading.contains(">Title</h2>"));

        let span = GradientSpan("Inline", "a", "b").into_string();
        assert!(span.starts_with("<span "));
        assert!(span.contains("from-a to-b"));
    }
}
