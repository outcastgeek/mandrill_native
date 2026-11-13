#![allow(non_snake_case)]
#![allow(clippy::needless_pass_by_value)]

use crate::components::ComponentInfo;
use crate::components::unstyled::testimonialcard as base;

pub use base::{TestimonialCard, TestimonialCardProps};

pub const COMPONENTS: &[ComponentInfo] = &[ComponentInfo {
    family: "neomorphism",
    module: "testimonialcard",
    name: "TestimonialCard",
}];

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
                    && component.module == "testimonialcard")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "TestimonialCard")
        );
    }
}
