#![allow(non_snake_case)]
#![allow(clippy::needless_pass_by_value)]

use crate::components::ComponentInfo;
use crate::components::unstyled::checkmarkitem as base;

pub use base::{CheckmarkItem, CheckmarkItemProps};

pub const COMPONENTS: &[ComponentInfo] = &[ComponentInfo {
    family: "neomorphism",
    module: "checkmarkitem",
    name: "CheckmarkItem",
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
                    && component.module == "checkmarkitem")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "CheckmarkItem")
        );
    }
}
