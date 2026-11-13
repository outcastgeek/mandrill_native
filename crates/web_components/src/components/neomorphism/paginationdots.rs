#![allow(non_snake_case)]
#![allow(clippy::needless_pass_by_value)]

use crate::components::ComponentInfo;
use crate::components::unstyled::paginationdots as base;

pub use base::{PaginationDots, PaginationDotsProps};

pub const COMPONENTS: &[ComponentInfo] = &[ComponentInfo {
    family: "neomorphism",
    module: "paginationdots",
    name: "PaginationDots",
}];

#[must_use]
pub fn ensureSignalPrefix(stateVar: &str) -> String {
    if stateVar.starts_with('$') {
        stateVar.to_string()
    } else {
        format!("${stateVar}")
    }
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
                    && component.module == "paginationdots")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "PaginationDots")
        );
    }

    #[test]
    fn ensure_signal_prefix_matches_templ_helper() {
        assert_eq!(ensureSignalPrefix("page"), "$page");
        assert_eq!(ensureSignalPrefix("$page"), "$page");
    }
}
