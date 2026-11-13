#![allow(non_snake_case)]
#![allow(clippy::needless_pass_by_value)]

use crate::components::ComponentInfo;
use crate::components::shared;
use crate::components::unstyled::dialog as base;
use maud::Markup;

pub use base::{
    ConfirmDialog, Dialog, DialogClose, DialogCloseProps, DialogContentProps, DialogDescription,
    DialogDescriptionProps, DialogFooter, DialogFooterProps, DialogHeader, DialogHeaderProps,
    DialogOverlayProps, DialogProps, DialogTitle, DialogTitleProps, DialogTrigger,
    DialogTriggerProps, SimpleDialog,
};

pub const COMPONENTS: &[ComponentInfo] = &[
    ComponentInfo {
        family: "neomorphism",
        module: "dialog",
        name: "Dialog",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "dialog",
        name: "DialogTrigger",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "dialog",
        name: "DialogOverlay",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "dialog",
        name: "DialogContent",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "dialog",
        name: "DialogHeader",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "dialog",
        name: "DialogTitle",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "dialog",
        name: "DialogDescription",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "dialog",
        name: "DialogFooter",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "dialog",
        name: "DialogClose",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "dialog",
        name: "SimpleDialog",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "dialog",
        name: "ConfirmDialog",
    },
];

#[must_use]
fn addOverlayNeomorphClasses(props: &DialogOverlayProps) -> DialogOverlayProps {
    let mut props = props.clone();
    props.class = shared::build_classes(&[props.class.as_str(), "backdrop-blur-md"]);
    props
}

#[must_use]
pub fn DialogOverlay(props: &DialogOverlayProps) -> Markup {
    let neo_props = addOverlayNeomorphClasses(props);
    base::DialogOverlay(&neo_props)
}

#[must_use]
fn addContentNeomorphClasses(props: &DialogContentProps) -> DialogContentProps {
    let mut props = props.clone();
    props.class = shared::build_classes(&[props.class.as_str(), "neomorph-card-flat"]);
    props
}

#[must_use]
pub fn DialogContent(props: &DialogContentProps, children: Markup) -> Markup {
    let neo_props = addContentNeomorphClasses(props);
    base::DialogContent(&neo_props, children)
}

#[cfg(test)]
mod tests {
    use super::*;
    use maud::html;
    fn assert_no_placeholder_markers(html: &str) {
        assert!(
            !html.contains("data-family="),
            "placeholder family marker leaked: {html}"
        );
        assert!(
            !html.contains("data-module="),
            "placeholder module marker leaked: {html}"
        );
        assert!(
            !html.contains("data-component=\""),
            "placeholder component marker leaked: {html}"
        );
    }

    #[test]
    fn registry_matches_neomorphism_source_components() {
        assert_eq!(COMPONENTS.len(), 11);
        assert!(
            COMPONENTS
                .iter()
                .all(|component| component.family == "neomorphism" && component.module == "dialog")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "Dialog")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "DialogTrigger")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "DialogOverlay")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "DialogContent")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "DialogHeader")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "DialogTitle")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "DialogDescription")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "DialogFooter")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "DialogClose")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "SimpleDialog")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "ConfirmDialog")
        );
    }

    #[test]
    fn wrapped_components_render_unstyled_html_with_neomorphism_classes() {
        let html = DialogOverlay(&DialogOverlayProps::default()).into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.starts_with("<"), "expected rendered element: {html}");
        assert!(
            html.contains("backdrop-blur-md"),
            "missing backdrop-blur-md: {html}"
        );
        let html = DialogContent(&DialogContentProps::default(), html! { "child" }).into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.starts_with("<"), "expected rendered element: {html}");
        assert!(
            html.contains("neomorph-card-flat"),
            "missing neomorph-card-flat: {html}"
        );
    }
}
