#![allow(non_snake_case)]
#![allow(clippy::needless_pass_by_value)]

use crate::components::ComponentInfo;
use crate::components::unstyled::carousel as base;

pub use base::{Carousel, CarouselProps};

pub const COMPONENTS: &[ComponentInfo] = &[ComponentInfo {
    family: "neomorphism",
    module: "carousel",
    name: "Carousel",
}];

#[must_use]
pub fn buildDatastarSignals(stateVar: &str, total: i32) -> String {
    format!("{{{stateVar}Current: 0, {stateVar}Total: {total}}}")
}

#[must_use]
pub fn buildPrevClick(stateVar: &str) -> String {
    format!("${stateVar}Current = (${stateVar}Current - 1 + ${stateVar}Total) % ${stateVar}Total")
}

#[must_use]
pub fn buildNextClick(stateVar: &str) -> String {
    format!("${stateVar}Current = (${stateVar}Current + 1) % ${stateVar}Total")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn registry_matches_neomorphism_source_components() {
        assert_eq!(COMPONENTS.len(), 1);
        assert!(
            COMPONENTS.iter().all(
                |component| component.family == "neomorphism" && component.module == "carousel"
            )
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "Carousel")
        );
    }

    #[test]
    fn datastar_expression_helpers_match_templ_output() {
        assert_eq!(
            buildDatastarSignals("slides", 3),
            "{slidesCurrent: 0, slidesTotal: 3}"
        );
        assert_eq!(
            buildPrevClick("slides"),
            "$slidesCurrent = ($slidesCurrent - 1 + $slidesTotal) % $slidesTotal"
        );
        assert_eq!(
            buildNextClick("slides"),
            "$slidesCurrent = ($slidesCurrent + 1) % $slidesTotal"
        );
    }
}
