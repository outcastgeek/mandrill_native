#![allow(non_snake_case)]
#![allow(clippy::needless_pass_by_value)]

use crate::components::ComponentInfo;
use crate::components::unstyled::statsbar as base;

pub use base::{StatsBar, StatsBarProps};

pub const COMPONENTS: &[ComponentInfo] = &[ComponentInfo {
    family: "neomorphism",
    module: "statsbar",
    name: "StatsBar",
}];

#[must_use]
pub fn intToString(i: i32) -> String {
    i.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn registry_matches_neomorphism_source_components() {
        assert_eq!(COMPONENTS.len(), 1);
        assert!(
            COMPONENTS.iter().all(
                |component| component.family == "neomorphism" && component.module == "statsbar"
            )
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "StatsBar")
        );
    }

    #[test]
    fn int_to_string_matches_go_helper() {
        assert_eq!(intToString(0), "0");
        assert_eq!(intToString(6), "6");
    }
}
