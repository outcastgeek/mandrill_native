#![allow(non_snake_case)]
#![allow(clippy::needless_pass_by_value)]

use crate::components::ComponentInfo;
use crate::components::unstyled::gradienttext as base;

pub use base::{GradientHeading, GradientSpan, GradientText, GradientTextProps};

pub const COMPONENTS: &[ComponentInfo] = &[
    ComponentInfo {
        family: "neomorphism",
        module: "gradienttext",
        name: "GradientText",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "gradienttext",
        name: "GradientHeading",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "gradienttext",
        name: "GradientSpan",
    },
];

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn registry_matches_neomorphism_source_components() {
        assert_eq!(COMPONENTS.len(), 3);
        assert!(COMPONENTS.iter().all(
            |component| component.family == "neomorphism" && component.module == "gradienttext"
        ));
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "GradientText")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "GradientHeading")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "GradientSpan")
        );
    }
}
