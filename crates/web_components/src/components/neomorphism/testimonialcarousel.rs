#![allow(non_snake_case)]
#![allow(clippy::needless_pass_by_value)]

use crate::components::ComponentInfo;
use crate::components::unstyled::testimonialcarousel as base;

pub use base::{TestimonialCarousel, TestimonialCarouselProps};

pub const COMPONENTS: &[ComponentInfo] = &[ComponentInfo {
    family: "neomorphism",
    module: "testimonialcarousel",
    name: "TestimonialCarousel",
}];

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
pub fn buildPrevClick(stateVar: &str) -> String {
    format!("${stateVar}Current = (${stateVar}Current - 1 + ${stateVar}Total) % ${stateVar}Total")
}

#[must_use]
pub fn buildNextClick(stateVar: &str) -> String {
    format!("${stateVar}Current = (${stateVar}Current + 1) % ${stateVar}Total")
}

#[must_use]
pub fn buildVisibilityExpression(stateVar: &str, index: usize) -> String {
    format!("${stateVar}Current === {index}")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn registry_matches_neomorphism_source_components() {
        assert_eq!(COMPONENTS.len(), 1);
        assert!(
            COMPONENTS
                .iter()
                .all(|component| component.family == "neomorphism"
                    && component.module == "testimonialcarousel")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "TestimonialCarousel")
        );
    }

    #[test]
    fn datastar_expression_helpers_match_templ_output() {
        let props = TestimonialCarouselProps::default().with_state_var("review");
        assert_eq!(
            buildDatastarSignals(&props),
            "{reviewCurrent: 0, reviewTotal: 0}"
        );
        assert_eq!(
            buildPrevClick("review"),
            "$reviewCurrent = ($reviewCurrent - 1 + $reviewTotal) % $reviewTotal"
        );
        assert_eq!(
            buildNextClick("review"),
            "$reviewCurrent = ($reviewCurrent + 1) % $reviewTotal"
        );
        assert_eq!(
            buildVisibilityExpression("review", 2),
            "$reviewCurrent === 2"
        );
    }
}
