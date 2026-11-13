#![allow(non_snake_case)]
#![allow(clippy::needless_pass_by_value)]

use crate::components::unstyled::switch as base;
use crate::components::{ComponentInfo, shared};

pub use base::{SwitchProps, SwitchWithLabel};

pub const COMPONENTS: &[ComponentInfo] = &[
    ComponentInfo {
        family: "neomorphism",
        module: "switch",
        name: "Switch",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "switch",
        name: "SwitchWithLabel",
    },
];

#[must_use]
fn addNeomorphClasses(props: &SwitchProps) -> SwitchProps {
    let mut props = props.clone();
    props.class = shared::build_classes(&["neomorph-switch", props.class.as_str()]);
    props
}

#[must_use]
pub fn Switch(props: &SwitchProps) -> maud::Markup {
    let neo_props = addNeomorphClasses(props);
    base::Switch(&neo_props)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn registry_matches_neomorphism_source_components() {
        assert_eq!(COMPONENTS.len(), 2);
        assert!(
            COMPONENTS
                .iter()
                .all(|component| component.family == "neomorphism" && component.module == "switch")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "Switch")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "SwitchWithLabel")
        );
    }

    #[test]
    fn switch_helper_adds_neomorph_class_and_rendered_html() {
        let props = addNeomorphClasses(&SwitchProps::default().with_class("custom"));
        assert_eq!(props.class, "neomorph-switch custom");

        let rendered = Switch(&SwitchProps::default().with_class("custom")).into_string();
        assert!(rendered.contains("neomorph-switch"));
        assert!(rendered.contains("custom"));
        assert!(rendered.contains("role=\"switch\""));
    }
}
