#![allow(non_snake_case)]

use crate::components::{ComponentInfo, shared};
use maud::{Markup, html};
use std::collections::BTreeMap;

const STAR_PATH: &str = "M9.049 2.927c.3-.921 1.603-.921 1.902 0l1.07 3.292a1 1 0 00.95.69h3.462c.969 0 1.371 1.24.588 1.81l-2.8 2.034a1 1 0 00-.364 1.118l1.07 3.292c.3.921-.755 1.688-1.54 1.118l-2.8-2.034a1 1 0 00-1.175 0l-2.8 2.034c-.784.57-1.838-.197-1.539-1.118l1.07-3.292a1 1 0 00-.364-1.118L2.98 8.72c-.783-.57-.38-1.81.588-1.81h3.461a1 1 0 00.951-.69l1.07-3.292z";

pub const COMPONENTS: &[ComponentInfo] = &[
    ComponentInfo {
        family: "unstyled",
        module: "starrating",
        name: "StarRating",
    },
    ComponentInfo {
        family: "unstyled",
        module: "starrating",
        name: "SimpleStarRating",
    },
    ComponentInfo {
        family: "unstyled",
        module: "starrating",
        name: "StarRatingWithValue",
    },
];

#[derive(Clone, Debug, PartialEq)]
pub struct StarRatingProps {
    pub rating: f64,
    pub max_rating: i32,
    pub size: &'static str,
    pub color: String,
    pub custom_color: String,
    pub show_value: bool,
    pub value_position: String,
    pub interactive: bool,
    pub on_rating_change: String,
    pub class: String,
    pub attributes: BTreeMap<String, String>,
}

impl Default for StarRatingProps {
    fn default() -> Self {
        Self {
            rating: 5.0,
            max_rating: 5,
            size: shared::SIZE_MD,
            color: "yellow".to_string(),
            custom_color: String::new(),
            show_value: false,
            value_position: String::new(),
            interactive: false,
            on_rating_change: String::new(),
            class: String::new(),
            attributes: BTreeMap::new(),
        }
    }
}

impl StarRatingProps {
    #[must_use]
    pub const fn with_rating(mut self, value: f64) -> Self {
        self.rating = value;
        self
    }

    #[must_use]
    pub const fn with_max_rating(mut self, value: i32) -> Self {
        self.max_rating = value;
        self
    }

    #[must_use]
    pub const fn with_size(mut self, value: &'static str) -> Self {
        self.size = value;
        self
    }

    #[must_use]
    pub fn with_color(mut self, value: impl Into<String>) -> Self {
        self.color = value.into();
        self
    }

    #[must_use]
    pub fn with_custom_color(mut self, value: impl Into<String>) -> Self {
        self.color = "custom".to_string();
        self.custom_color = value.into();
        self
    }

    #[must_use]
    pub fn showing_value(mut self, position: impl Into<String>) -> Self {
        self.show_value = true;
        self.value_position = position.into();
        self
    }

    #[must_use]
    pub fn with_interactive(mut self, on_change: impl Into<String>) -> Self {
        self.interactive = true;
        self.on_rating_change = on_change.into();
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
        let size_class = match self.size {
            shared::SIZE_SM => "w-4 h-4",
            shared::SIZE_LG => "w-6 h-6",
            shared::SIZE_XL => "w-8 h-8",
            _ => "w-5 h-5",
        };
        let color_class = if self.color == "custom" && !self.custom_color.is_empty() {
            self.custom_color.as_str()
        } else {
            match self.color.as_str() {
                "yellow" | "orange" => "text-warning",
                "red" => "text-destructive",
                _ => "",
            }
        };
        shared::build_classes(&[size_class, color_class])
    }

    #[must_use]
    pub fn build_attributes(&self) -> BTreeMap<String, String> {
        self.attributes.clone()
    }
}

#[must_use]
pub fn ratingAriaLabel(props: &StarRatingProps) -> String {
    format!(
        "Rating: {:.1} out of {} stars",
        props.rating, props.max_rating
    )
}

fn full_star(classes: &str) -> Markup {
    html! {
        svg class=(classes) fill="currentColor" viewBox="0 0 20 20" aria-hidden="true" {
            path d=(STAR_PATH) {}
        }
    }
}

fn half_star(classes: &str, index: i32) -> Markup {
    html! {
        svg class=(classes) fill="currentColor" viewBox="0 0 20 20" aria-hidden="true" {
            defs {
                linearGradient id=(format!("half-star-{index}")) {
                    stop offset="50%" stop-color="currentColor" {}
                    stop offset="50%" stop-color="currentColor" stop-opacity="0.3" {}
                }
            }
            path fill=(format!("url(#half-star-{index})")) d=(STAR_PATH) {}
        }
    }
}

fn empty_star(classes: &str) -> Markup {
    html! {
        svg class=(format!("{classes} opacity-30")) fill="currentColor" viewBox="0 0 20 20" aria-hidden="true" {
            path d=(STAR_PATH) {}
        }
    }
}

#[must_use]
pub fn StarRating(props: &StarRatingProps) -> Markup {
    let attrs = props.build_attributes();
    let classes = props.build_classes();
    html! {
        div
            class=(format!("flex items-center gap-1 {}", props.class))
            role="img"
            aria-label=(ratingAriaLabel(props))
            id=[shared::attr_value(&attrs, "id")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
        {
            div class="flex gap-1" {
                @for index in 1..=props.max_rating {
                    @if f64::from(index) <= props.rating {
                        (full_star(&classes))
                    } @else if f64::from(index - 1) < props.rating && props.rating < f64::from(index) {
                        (half_star(&classes, index))
                    } @else {
                        (empty_star(&classes))
                    }
                }
            }
            @if props.show_value {
                span class="text-sm ml-1" {
                    (format!("{:.1}", props.rating))
                }
            }
        }
    }
}

#[must_use]
pub fn SimpleStarRating(rating: f64) -> Markup {
    StarRating(&StarRatingProps::default().with_rating(rating))
}

#[must_use]
pub fn StarRatingWithValue(rating: f64) -> Markup {
    StarRating(
        &StarRatingProps::default()
            .with_rating(rating)
            .showing_value("right"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn helpers_cover_defaults_build_classes_attributes_and_aria_label() {
        let props = StarRatingProps::default();
        assert_eq!(props.rating, 5.0);
        assert_eq!(props.max_rating, 5);
        assert_eq!(props.size, shared::SIZE_MD);
        assert_eq!(props.color, "yellow");
        assert_eq!(props.build_classes(), "w-5 h-5 text-warning");
        assert_eq!(ratingAriaLabel(&props), "Rating: 5.0 out of 5 stars");
        assert_eq!(
            StarRatingProps::default()
                .with_size(shared::SIZE_SM)
                .with_color("red")
                .build_classes(),
            "w-4 h-4 text-destructive"
        );
        assert_eq!(
            StarRatingProps::default()
                .with_size(shared::SIZE_LG)
                .with_color("orange")
                .build_classes(),
            "w-6 h-6 text-warning"
        );
        assert_eq!(
            StarRatingProps::default()
                .with_size(shared::SIZE_XL)
                .with_custom_color("text-brand")
                .build_classes(),
            "w-8 h-8 text-brand"
        );
        assert_eq!(
            StarRatingProps::default()
                .with_color("unknown")
                .build_classes(),
            "w-5 h-5"
        );
        let attrs = StarRatingProps::default()
            .with_interactive("$rating = 4")
            .with_attribute("id", "rating")
            .with_attribute("data-testid", "rating-widget")
            .build_attributes();
        assert_eq!(attrs.get("id").map(String::as_str), Some("rating"));
        assert_eq!(
            attrs.get("data-testid").map(String::as_str),
            Some("rating-widget")
        );
    }

    #[test]
    fn star_rating_renders_full_half_empty_stars_value_attrs_and_escaping() {
        let rendered = StarRating(
            &StarRatingProps::default()
                .with_rating(3.5)
                .with_max_rating(5)
                .with_size(shared::SIZE_SM)
                .showing_value("right")
                .with_class("custom")
                .with_attribute("id", "course-rating")
                .with_attribute("data-testid", "stars"),
        )
        .into_string();
        assert!(rendered.starts_with("<div class=\"flex items-center gap-1 custom\""));
        assert!(rendered.contains("role=\"img\""));
        assert!(rendered.contains("aria-label=\"Rating: 3.5 out of 5 stars\""));
        assert!(rendered.contains("id=\"course-rating\""));
        assert!(rendered.contains("data-testid=\"stars\""));
        assert_eq!(rendered.matches("<svg ").count(), 5);
        assert_eq!(
            rendered.matches("class=\"w-4 h-4 text-warning\"").count(),
            4
        );
        assert_eq!(rendered.matches("opacity-30").count(), 1);
        assert!(rendered.contains("<linearGradient id=\"half-star-4\">"));
        assert!(rendered.contains("fill=\"url(#half-star-4)\""));
        assert!(rendered.contains("<span class=\"text-sm ml-1\">3.5</span>"));
        assert!(rendered.contains(STAR_PATH));
    }

    #[test]
    fn simple_and_value_variants_render_expected_html() {
        let simple = SimpleStarRating(2.0).into_string();
        assert_eq!(simple.matches("<svg ").count(), 5);
        assert_eq!(simple.matches("opacity-30").count(), 3);
        assert!(!simple.contains("text-sm ml-1"));

        let with_value = StarRatingWithValue(4.25).into_string();
        assert_eq!(with_value.matches("<svg ").count(), 5);
        assert!(with_value.contains("aria-label=\"Rating: 4.2 out of 5 stars\""));
        assert!(with_value.contains("<span class=\"text-sm ml-1\">4.2</span>"));

        let empty = StarRating(&StarRatingProps::default().with_max_rating(0)).into_string();
        assert!(!empty.contains("<svg "));
        assert!(empty.contains("<div class=\"flex gap-1\"></div>"));
    }
}
