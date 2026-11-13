#![allow(non_snake_case)]
#![allow(clippy::needless_pass_by_value)]

use crate::components::ComponentInfo;
use crate::components::unstyled::democard as base;

pub use base::{DemoCard, DemoCardProps};

pub const COMPONENTS: &[ComponentInfo] = &[ComponentInfo {
    family: "neomorphism",
    module: "democard",
    name: "DemoCard",
}];

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn registry_matches_neomorphism_source_components() {
        assert_eq!(COMPONENTS.len(), 1);
        assert!(
            COMPONENTS.iter().all(
                |component| component.family == "neomorphism" && component.module == "democard"
            )
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "DemoCard")
        );
    }
}
