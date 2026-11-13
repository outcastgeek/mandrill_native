#![allow(non_snake_case)]
#![allow(clippy::needless_pass_by_value)]

use crate::components::unstyled::toast as base;
use crate::components::{ComponentInfo, shared};
use maud::Markup;

pub use base::{
    ErrorToast, InfoToast, SimpleToast, SuccessToast, ToastClose, ToastCloseProps, ToastContainer,
    ToastContainerProps, ToastDescription, ToastDescriptionProps, ToastIcon, ToastIconProps,
    ToastProps, ToastTitle, ToastTitleProps, ToastWithIcon, WarningToast,
};

pub const COMPONENTS: &[ComponentInfo] = &[
    ComponentInfo {
        family: "neomorphism",
        module: "toast",
        name: "ToastContainer",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "toast",
        name: "Toast",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "toast",
        name: "ToastIcon",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "toast",
        name: "ToastTitle",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "toast",
        name: "ToastDescription",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "toast",
        name: "ToastClose",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "toast",
        name: "SimpleToast",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "toast",
        name: "ToastWithIcon",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "toast",
        name: "InfoToast",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "toast",
        name: "SuccessToast",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "toast",
        name: "WarningToast",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "toast",
        name: "ErrorToast",
    },
];

#[must_use]
fn addNeomorphClasses(props: &ToastProps) -> ToastProps {
    let mut props = props.clone();
    props.class = shared::build_classes(&[
        "neomorph-flat",
        shared::alert_variant_classes(props.variant),
        props.class.as_str(),
    ]);
    props
}

#[must_use]
pub fn Toast(props: &ToastProps, children: Markup) -> Markup {
    let neo_props = addNeomorphClasses(props);
    base::Toast(&neo_props, children)
}

#[cfg(test)]
mod tests {
    use super::*;
    use maud::html;
    #[test]
    fn registry_matches_neomorphism_source_components() {
        assert_eq!(COMPONENTS.len(), 12);
        assert!(
            COMPONENTS
                .iter()
                .all(|component| component.family == "neomorphism" && component.module == "toast")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "ToastContainer")
        );
        assert!(COMPONENTS.iter().any(|component| component.name == "Toast"));
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "ToastIcon")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "ToastTitle")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "ToastDescription")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "ToastClose")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "SimpleToast")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "ToastWithIcon")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "InfoToast")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "SuccessToast")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "WarningToast")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "ErrorToast")
        );
    }

    #[test]
    fn toast_helper_adds_neomorph_classes_and_rendered_html() {
        let props = addNeomorphClasses(
            &ToastProps::default()
                .with_variant(shared::VARIANT_WARNING)
                .with_class("custom"),
        );
        assert!(props.class.contains("neomorph-flat"));
        assert!(props.class.contains("border-warning"));
        assert!(props.class.contains("custom"));

        let rendered = Toast(
            &ToastProps::default()
                .with_variant(shared::VARIANT_WARNING)
                .with_class("custom"),
            html! { "Heads up" },
        )
        .into_string();
        assert!(rendered.contains("neomorph-flat"));
        assert!(rendered.contains("border-warning"));
        assert!(rendered.contains("custom"));
        assert!(rendered.contains("Heads up"));
    }
}
