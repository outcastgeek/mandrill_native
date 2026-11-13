#![allow(non_snake_case)]
#![allow(clippy::needless_pass_by_value)]

use crate::components::unstyled::alert as base;
use crate::components::{ComponentInfo, shared};
use maud::Markup;

pub use base::{
    AlertClose, AlertCloseProps, AlertDescription, AlertDescriptionProps, AlertIcon,
    AlertIconProps, AlertProps, AlertTitle, AlertTitleProps, AlertWithIcon, ErrorAlert, InfoAlert,
    SimpleAlert, SuccessAlert, WarningAlert,
};

pub const COMPONENTS: &[ComponentInfo] = &[
    ComponentInfo {
        family: "neomorphism",
        module: "alert",
        name: "Alert",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "alert",
        name: "AlertIcon",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "alert",
        name: "AlertTitle",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "alert",
        name: "AlertDescription",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "alert",
        name: "AlertClose",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "alert",
        name: "SimpleAlert",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "alert",
        name: "AlertWithIcon",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "alert",
        name: "InfoAlert",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "alert",
        name: "SuccessAlert",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "alert",
        name: "WarningAlert",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "alert",
        name: "ErrorAlert",
    },
];

#[must_use]
fn addNeomorphClasses(props: &AlertProps) -> AlertProps {
    let mut props = props.clone();
    props.class = shared::build_classes(&[
        "neomorph-flat",
        shared::alert_variant_classes(props.variant),
        props.class.as_str(),
    ]);
    props
}

#[must_use]
pub fn Alert(props: &AlertProps, children: Markup) -> Markup {
    let neo_props = addNeomorphClasses(props);
    base::Alert(&neo_props, children)
}

#[cfg(test)]
mod tests {
    use super::*;
    use maud::html;
    #[test]
    fn registry_matches_neomorphism_source_components() {
        assert_eq!(COMPONENTS.len(), 11);
        assert!(
            COMPONENTS
                .iter()
                .all(|component| component.family == "neomorphism" && component.module == "alert")
        );
        assert!(COMPONENTS.iter().any(|component| component.name == "Alert"));
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "AlertIcon")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "AlertTitle")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "AlertDescription")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "AlertClose")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "SimpleAlert")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "AlertWithIcon")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "InfoAlert")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "SuccessAlert")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "WarningAlert")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "ErrorAlert")
        );
    }

    #[test]
    fn alert_helper_adds_neomorph_classes_and_rendered_html() {
        let props = addNeomorphClasses(
            &AlertProps::default()
                .with_variant(shared::VARIANT_SUCCESS)
                .with_class("custom"),
        );
        assert!(props.class.contains("neomorph-flat"));
        assert!(props.class.contains("border-success"));
        assert!(props.class.contains("custom"));

        let rendered = Alert(
            &AlertProps::default()
                .with_variant(shared::VARIANT_SUCCESS)
                .with_class("custom"),
            html! { "Saved" },
        )
        .into_string();
        assert!(rendered.contains("neomorph-flat"));
        assert!(rendered.contains("border-success"));
        assert!(rendered.contains("custom"));
        assert!(rendered.contains(">Saved</div>"));
    }
}
