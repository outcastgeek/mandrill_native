#![allow(non_snake_case)]
#![allow(clippy::needless_pass_by_value)]

use crate::components::ComponentInfo;
use crate::components::unstyled::durationbadge as base;

pub use base::{DurationBadge, DurationBadgeProps};

pub const COMPONENTS: &[ComponentInfo] = &[ComponentInfo {
    family: "neomorphism",
    module: "durationbadge",
    name: "DurationBadge",
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
                    && component.module == "durationbadge")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "DurationBadge")
        );
    }
}
