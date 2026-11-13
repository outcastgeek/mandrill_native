#![allow(non_snake_case)]

use crate::components::unstyled::navigationarrow::{NavigationArrow, NavigationArrowProps};
use crate::components::unstyled::paginationdots::{PaginationDots, PaginationDotsProps};
use crate::components::unstyled::testimonialcard::{TestimonialCard, TestimonialCardProps};
use crate::components::{ComponentInfo, shared};
use maud::{Markup, html};
use std::collections::BTreeMap;

pub const COMPONENTS: &[ComponentInfo] = &[
    ComponentInfo {
        family: "unstyled",
        module: "testimonialcarousel",
        name: "TestimonialCarousel",
    },
    ComponentInfo {
        family: "unstyled",
        module: "testimonialcarousel",
        name: "SimpleTestimonialCarousel",
    },
];

#[derive(Clone, Debug, PartialEq)]
pub struct TestimonialCarouselProps {
    pub testimonials: Vec<TestimonialCardProps>,
    pub auto_play: bool,
    pub auto_play_delay: i32,
    pub show_pagination: bool,
    pub show_arrows: bool,
    pub state_var: String,
    pub class: String,
    pub attributes: BTreeMap<String, String>,
}

impl Default for TestimonialCarouselProps {
    fn default() -> Self {
        Self {
            testimonials: Vec::new(),
            auto_play: false,
            auto_play_delay: 5000,
            show_pagination: true,
            show_arrows: true,
            state_var: "testimonial".to_string(),
            class: String::new(),
            attributes: BTreeMap::new(),
        }
    }
}

impl TestimonialCarouselProps {
    #[must_use]
    pub fn with_testimonials(mut self, value: Vec<TestimonialCardProps>) -> Self {
        self.testimonials = value;
        self
    }
    #[must_use]
    pub const fn with_auto_play(mut self, delay: i32) -> Self {
        self.auto_play = true;
        self.auto_play_delay = delay;
        self
    }
    #[must_use]
    pub const fn without_auto_play(mut self) -> Self {
        self.auto_play = false;
        self
    }
    #[must_use]
    pub const fn with_pagination(mut self, show: bool) -> Self {
        self.show_pagination = show;
        self
    }
    #[must_use]
    pub const fn with_arrows(mut self, show: bool) -> Self {
        self.show_arrows = show;
        self
    }
    #[must_use]
    pub fn with_state_var(mut self, value: impl Into<String>) -> Self {
        self.state_var = value.into();
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
    pub fn build_attributes(&self) -> BTreeMap<String, String> {
        self.attributes.clone()
    }
}

#[must_use]
pub fn buildDatastarSignals(props: &TestimonialCarouselProps) -> String {
    format!(
        "{{{}Current: 0, {}Total: {}}}",
        props.state_var,
        props.state_var,
        props.testimonials.len()
    )
}

#[must_use]
pub fn buildPrevClick(state_var: &str) -> String {
    format!(
        "${state_var}Current = (${state_var}Current - 1 + ${state_var}Total) % ${state_var}Total"
    )
}

#[must_use]
pub fn buildNextClick(state_var: &str) -> String {
    format!("${state_var}Current = (${state_var}Current + 1) % ${state_var}Total")
}

#[must_use]
pub fn buildVisibilityExpression(state_var: &str, index: usize) -> String {
    format!("${state_var}Current === {index}")
}

#[must_use]
pub fn TestimonialCarousel(props: &TestimonialCarouselProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        div
            class=(format!("testimonial-carousel {}", props.class))
            data-signals=(buildDatastarSignals(props))
            id=[shared::attr_value(&attrs, "id")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
        {
            div class="relative" {
                div class="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-8" {
                    @for (index, testimonial) in props.testimonials.iter().enumerate() {
                        div
                            data-show=(buildVisibilityExpression(&props.state_var, index))
                            style="display: none"
                            class="transition-opacity duration-300"
                        {
                            (TestimonialCard(testimonial))
                        }
                    }
                }
            }
            @if props.show_arrows {
                div class="flex justify-center gap-4 mt-8" {
                    (NavigationArrow(&NavigationArrowProps::default()
                        .with_direction(shared::POSITION_LEFT)
                        .with_size(shared::SIZE_MD)
                        .with_on_click(buildPrevClick(&props.state_var))))
                    (NavigationArrow(&NavigationArrowProps::default()
                        .with_direction(shared::POSITION_RIGHT)
                        .with_size(shared::SIZE_MD)
                        .with_on_click(buildNextClick(&props.state_var))))
                }
            }
            @if props.show_pagination {
                (PaginationDots(&PaginationDotsProps::default()
                    .with_total(props.testimonials.len() as i32)
                    .with_current(0)
                    .with_state_var(format!("{}Current", props.state_var))))
            }
        }
    }
}

#[must_use]
pub fn SimpleTestimonialCarousel(testimonials: Vec<TestimonialCardProps>) -> Markup {
    TestimonialCarousel(&TestimonialCarouselProps::default().with_testimonials(testimonials))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_testimonials() -> Vec<TestimonialCardProps> {
        vec![
            TestimonialCardProps::default()
                .with_quote("First")
                .with_author("Ada", "CTO", "Acme")
                .with_rating(5.0),
            TestimonialCardProps::default()
                .with_quote("Second")
                .with_author("Grace", "Engineer", "Labs")
                .with_rating(4.0),
        ]
    }

    #[test]
    fn helpers_cover_defaults_builders_signals_and_expressions() {
        let props = TestimonialCarouselProps::default();
        assert!(!props.auto_play);
        assert_eq!(props.auto_play_delay, 5000);
        assert!(props.show_pagination);
        assert!(props.show_arrows);
        assert_eq!(props.state_var, "testimonial");
        assert_eq!(
            buildDatastarSignals(&props),
            "{testimonialCurrent: 0, testimonialTotal: 0}"
        );
        assert_eq!(
            buildPrevClick("testimonial"),
            "$testimonialCurrent = ($testimonialCurrent - 1 + $testimonialTotal) % $testimonialTotal"
        );
        assert_eq!(
            buildNextClick("testimonial"),
            "$testimonialCurrent = ($testimonialCurrent + 1) % $testimonialTotal"
        );
        assert_eq!(
            buildVisibilityExpression("testimonial", 2),
            "$testimonialCurrent === 2"
        );

        let built = TestimonialCarouselProps::default()
            .with_testimonials(sample_testimonials())
            .with_auto_play(7000)
            .without_auto_play()
            .with_pagination(false)
            .with_arrows(false)
            .with_state_var("review")
            .with_class("wide")
            .with_attribute("id", "reviews");
        assert_eq!(built.testimonials.len(), 2);
        assert!(!built.auto_play);
        assert_eq!(built.auto_play_delay, 7000);
        assert!(!built.show_pagination);
        assert!(!built.show_arrows);
        assert_eq!(built.state_var, "review");
        assert_eq!(built.class, "wide");
        assert_eq!(
            built.build_attributes().get("id").map(String::as_str),
            Some("reviews")
        );
        assert_eq!(
            buildDatastarSignals(&built),
            "{reviewCurrent: 0, reviewTotal: 2}"
        );
    }

    #[test]
    fn carousel_renders_cards_arrows_dots_and_datastar_attributes() {
        let rendered = TestimonialCarousel(
            &TestimonialCarouselProps::default()
                .with_testimonials(sample_testimonials())
                .with_state_var("review")
                .with_class("featured")
                .with_attribute("data-testid", "reviews"),
        )
        .into_string();
        assert!(rendered.starts_with("<div class=\"testimonial-carousel featured\" data-signals=\"{reviewCurrent: 0, reviewTotal: 2}\""));
        assert!(rendered.contains("data-testid=\"reviews\""));
        assert!(
            rendered
                .contains("<div class=\"grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-8\">")
        );
        assert!(rendered.contains("data-show=\"$reviewCurrent === 0\" style=\"display: none\" class=\"transition-opacity duration-300\""));
        assert!(rendered.contains("data-show=\"$reviewCurrent === 1\" style=\"display: none\" class=\"transition-opacity duration-300\""));
        assert!(rendered.contains("&quot;First&quot;"));
        assert!(rendered.contains("&quot;Second&quot;"));
        assert!(rendered.contains("aria-label=\"Previous\""));
        assert!(rendered.contains(
            "data-on:click=\"$reviewCurrent = ($reviewCurrent - 1 + $reviewTotal) % $reviewTotal\""
        ));
        assert!(rendered.contains("aria-label=\"Next\""));
        assert!(
            rendered
                .contains("data-on:click=\"$reviewCurrent = ($reviewCurrent + 1) % $reviewTotal\"")
        );
        assert!(rendered.contains("role=\"tablist\" aria-label=\"Pagination\""));
        assert!(rendered.contains("data-on:click=\"$reviewCurrent = 0\""));
        assert!(rendered.contains("data-class:slide-dot-active=\"$reviewCurrent === 1\""));
    }

    #[test]
    fn carousel_can_hide_arrows_and_pagination_and_escape_attrs() {
        let rendered = TestimonialCarousel(
            &TestimonialCarouselProps::default()
                .with_testimonials(sample_testimonials())
                .with_arrows(false)
                .with_pagination(false)
                .with_attribute("id", "carousel"),
        )
        .into_string();
        assert!(rendered.contains("id=\"carousel\""));
        assert!(!rendered.contains("aria-label=\"Previous\""));
        assert!(!rendered.contains("role=\"tablist\""));
        assert!(rendered.contains("data-signals=\"{testimonialCurrent: 0, testimonialTotal: 2}\""));
    }

    #[test]
    fn simple_carousel_uses_defaults() {
        let rendered = SimpleTestimonialCarousel(sample_testimonials()).into_string();
        assert!(rendered.contains("class=\"testimonial-carousel \""));
        assert!(rendered.contains("data-signals=\"{testimonialCurrent: 0, testimonialTotal: 2}\""));
        assert!(rendered.contains("data-show=\"$testimonialCurrent === 0\""));
        assert!(rendered.contains(
            "data-on:click=\"$testimonialCurrent = ($testimonialCurrent + 1) % $testimonialTotal\""
        ));
        assert!(rendered.contains("data-on:click=\"$testimonialCurrent = 0\""));
    }
}
