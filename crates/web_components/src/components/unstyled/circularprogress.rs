#![allow(non_snake_case)]

use crate::components::{ComponentInfo, shared};
use maud::{Markup, html};
use std::collections::BTreeMap;

pub const COMPONENTS: &[ComponentInfo] = &[ComponentInfo {
    family: "unstyled",
    module: "circularprogress",
    name: "Ring",
}];

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Props {
    pub radius: i32,
    pub stroke_width: i32,
    pub value: i32,
    pub max: i32,
    pub class: String,
    pub track_class: String,
    pub progress_class: String,
    pub attributes: BTreeMap<String, String>,
}

impl Default for Props {
    fn default() -> Self {
        Self {
            radius: 45,
            stroke_width: 8,
            value: 0,
            max: 100,
            class: String::new(),
            track_class: String::new(),
            progress_class: String::new(),
            attributes: BTreeMap::new(),
        }
    }
}

impl Props {
    #[must_use]
    pub const fn with_radius(mut self, radius: i32) -> Self {
        self.radius = radius;
        self
    }

    #[must_use]
    pub const fn with_stroke_width(mut self, stroke_width: i32) -> Self {
        self.stroke_width = stroke_width;
        self
    }

    #[must_use]
    pub const fn with_value(mut self, value: i32) -> Self {
        self.value = value;
        self
    }

    #[must_use]
    pub const fn with_max(mut self, max: i32) -> Self {
        self.max = max;
        self
    }

    #[must_use]
    pub fn with_class(mut self, class: impl Into<String>) -> Self {
        self.class = class.into();
        self
    }

    #[must_use]
    pub fn with_track_class(mut self, class: impl Into<String>) -> Self {
        self.track_class = class.into();
        self
    }

    #[must_use]
    pub fn with_progress_class(mut self, class: impl Into<String>) -> Self {
        self.progress_class = class.into();
        self
    }

    #[must_use]
    pub fn with_attribute(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.attributes.insert(name.into(), value.into());
        self
    }

    #[must_use]
    pub const fn normalized_radius(&self) -> i32 {
        if self.radius <= 0 { 45 } else { self.radius }
    }

    #[must_use]
    pub const fn normalized_stroke_width(&self) -> i32 {
        if self.stroke_width <= 0 {
            8
        } else {
            self.stroke_width
        }
    }

    #[must_use]
    pub const fn diameter(&self) -> i32 {
        self.normalized_radius() * 2 + 10
    }

    #[must_use]
    pub const fn normalized_value(&self) -> i32 {
        let max = if self.max <= 0 { 100 } else { self.max };
        if self.value < 0 {
            0
        } else if self.value > max {
            max
        } else {
            self.value
        }
    }

    #[must_use]
    pub fn circumference(&self) -> f64 {
        2.0 * std::f64::consts::PI * f64::from(self.normalized_radius())
    }

    #[must_use]
    pub fn dash_array(&self) -> String {
        format!("{:.0}", self.circumference())
    }

    #[must_use]
    pub fn dash_offset(&self) -> String {
        let max = if self.max <= 0 { 100 } else { self.max };
        format!(
            "{:.0}",
            self.circumference()
                - (self.circumference() * f64::from(self.normalized_value()) / f64::from(max))
        )
    }

    #[must_use]
    pub fn build_classes(&self) -> String {
        shared::build_classes(&["w-full h-full", self.class.as_str()])
    }
}

#[must_use]
pub fn Ring(props: &Props) -> Markup {
    let center = (props.diameter() / 2).to_string();
    let radius = props.normalized_radius().to_string();
    let stroke_width = props.normalized_stroke_width().to_string();

    html! {
        svg
            class=(props.build_classes())
            viewBox=(format!("0 0 {} {}", props.diameter(), props.diameter()))
            role="img"
            id=[shared::attr_value(&props.attributes, "id")]
            title=[shared::attr_value(&props.attributes, "title")]
            aria-label=[shared::attr_value(&props.attributes, "aria-label")]
            data-testid=[shared::attr_value(&props.attributes, "data-testid")]
        {
            circle
                cx=(&center)
                cy=(&center)
                r=(&radius)
                fill="none"
                stroke="currentColor"
                stroke-width=(&stroke_width)
                class=(shared::build_classes(&["text-muted", props.track_class.as_str()]))
            {}
            circle
                cx=(&center)
                cy=(&center)
                r=(&radius)
                fill="none"
                stroke="currentColor"
                stroke-width=(&stroke_width)
                stroke-linecap="round"
                stroke-dasharray=(props.dash_array())
                stroke-dashoffset=(props.dash_offset())
                class=(&props.progress_class)
            {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_and_normalizers_match_go_behavior() {
        let props = Props::default();
        assert_eq!(props.radius, 45);
        assert_eq!(props.stroke_width, 8);
        assert_eq!(props.max, 100);
        assert_eq!(props.normalized_radius(), 45);
        assert_eq!(props.normalized_stroke_width(), 8);
        assert_eq!(props.diameter(), 100);
        assert_eq!(props.normalized_value(), 0);

        let invalid = Props::default()
            .with_radius(0)
            .with_stroke_width(0)
            .with_value(-5)
            .with_max(0);
        assert_eq!(invalid.normalized_radius(), 45);
        assert_eq!(invalid.normalized_stroke_width(), 8);
        assert_eq!(invalid.normalized_value(), 0);
        assert_eq!(invalid.dash_offset(), invalid.dash_array());

        assert_eq!(
            Props::default()
                .with_value(250)
                .with_max(100)
                .normalized_value(),
            100
        );
    }

    #[test]
    fn helper_methods_cover_geometry_dash_math_classes_and_attrs() {
        let props = Props::default()
            .with_radius(10)
            .with_stroke_width(4)
            .with_value(25)
            .with_max(50)
            .with_class("extra")
            .with_track_class("track")
            .with_progress_class("progress")
            .with_attribute("title", "Progress");

        assert_eq!(props.diameter(), 30);
        assert_eq!(props.dash_array(), "63");
        assert_eq!(props.dash_offset(), "31");
        assert_eq!(props.build_classes(), "w-full h-full extra");
        assert_eq!(
            props.attributes.get("title").map(String::as_str),
            Some("Progress")
        );
    }

    #[test]
    fn ring_renders_svg_track_progress_and_escaped_attrs() {
        let html = Ring(
            &Props::default()
                .with_radius(10)
                .with_stroke_width(4)
                .with_value(25)
                .with_max(50)
                .with_track_class("track")
                .with_progress_class("progress")
                .with_attribute("aria-label", "<Progress>")
                .with_attribute("data-testid", "ring"),
        )
        .into_string();

        assert!(html.starts_with("<svg "));
        assert!(html.contains("viewBox=\"0 0 30 30\""));
        assert!(html.contains("role=\"img\""));
        assert!(html.contains("aria-label=\"&lt;Progress&gt;\""));
        assert!(html.contains("data-testid=\"ring\""));
        assert!(html.contains("<circle cx=\"15\" cy=\"15\" r=\"10\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"4\" class=\"text-muted track\"></circle>"));
        assert!(html.contains("stroke-linecap=\"round\""));
        assert!(html.contains("stroke-dasharray=\"63\""));
        assert!(html.contains("stroke-dashoffset=\"31\""));
        assert!(html.contains("class=\"progress\""));
    }
}
