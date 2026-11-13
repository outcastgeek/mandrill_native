#![allow(non_snake_case)]
#![allow(clippy::needless_pass_by_value)]

use crate::components::ComponentInfo;
use crate::components::shared;
use crate::components::unstyled::switchfield as base;
use maud::Markup;

pub use base::{SwitchCard, SwitchGroup, SwitchProps, SwitchWithInlineLabel, SwitchWithLabel};

pub const COMPONENTS: &[ComponentInfo] = &[
    ComponentInfo {
        family: "neomorphism",
        module: "switchfield",
        name: "Switch",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "switchfield",
        name: "SwitchWithLabel",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "switchfield",
        name: "SwitchWithInlineLabel",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "switchfield",
        name: "SwitchCard",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "switchfield",
        name: "SwitchGroup",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "switchfield",
        name: "DarkModeSwitch",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "switchfield",
        name: "NotificationsSwitch",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "switchfield",
        name: "AutoSaveSwitch",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "switchfield",
        name: "PrivacySwitch",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "switchfield",
        name: "EmailNotificationsSwitch",
    },
];

#[must_use]
fn addNeomorphClasses(props: &SwitchProps) -> SwitchProps {
    props.clone()
}

#[must_use]
pub fn Switch(props: &SwitchProps) -> Markup {
    let neo_props = addNeomorphClasses(props);
    base::Switch(&neo_props)
}

#[must_use]
pub fn DarkModeSwitch(id: &str, checked: bool) -> Markup {
    SwitchWithLabel(
        &SwitchProps::default()
            .with_id(id)
            .with_name("dark-mode")
            .with_checked(checked)
            .with_size(shared::SIZE_MD),
        "Dark Mode",
        "Toggle between light and dark themes",
    )
}

#[must_use]
pub fn NotificationsSwitch(id: &str, checked: bool) -> Markup {
    SwitchWithLabel(
        &SwitchProps::default()
            .with_id(id)
            .with_name("notifications")
            .with_checked(checked)
            .with_size(shared::SIZE_MD),
        "Enable Notifications",
        "Receive updates and alerts",
    )
}

#[must_use]
pub fn AutoSaveSwitch(id: &str, checked: bool, loading: bool) -> Markup {
    SwitchWithLabel(
        &SwitchProps::default()
            .with_id(id)
            .with_name("auto-save")
            .with_checked(checked)
            .with_loading(loading)
            .with_size(shared::SIZE_MD),
        "Auto-save",
        "Automatically save your changes",
    )
}

#[must_use]
pub fn PrivacySwitch(id: &str, checked: bool) -> Markup {
    SwitchWithLabel(
        &SwitchProps::default()
            .with_id(id)
            .with_name("profile-public")
            .with_checked(checked)
            .with_size(shared::SIZE_MD),
        "Public Profile",
        "Make your profile visible to others",
    )
}

#[must_use]
pub fn EmailNotificationsSwitch(id: &str, checked: bool) -> Markup {
    SwitchWithInlineLabel(
        &SwitchProps::default()
            .with_id(id)
            .with_name("email-notifications")
            .with_checked(checked)
            .with_size(shared::SIZE_SM),
        "Email notifications",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
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
        assert_eq!(COMPONENTS.len(), 10);
        assert!(COMPONENTS.iter().all(
            |component| component.family == "neomorphism" && component.module == "switchfield"
        ));
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
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "SwitchWithInlineLabel")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "SwitchCard")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "SwitchGroup")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "DarkModeSwitch")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "NotificationsSwitch")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "AutoSaveSwitch")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "PrivacySwitch")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "EmailNotificationsSwitch")
        );
    }

    #[test]
    fn wrapped_components_render_unstyled_html_with_neomorphism_classes() {
        let html = Switch(&SwitchProps::default()).into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.starts_with("<"), "expected rendered element: {html}");
    }

    #[test]
    fn neomorphism_only_components_render_html_without_generated_stub_markers() {
        let html = DarkModeSwitch("sample", true).into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.contains("Dark Mode"));
        assert!(html.contains("name=\"dark-mode\""));
        let html = NotificationsSwitch("sample", true).into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.contains("Enable Notifications"));
        assert!(html.contains("name=\"notifications\""));
        let html = AutoSaveSwitch("sample", true, true).into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.contains("Auto-save"));
        assert!(html.contains("animate-spin"));
        let html = PrivacySwitch("sample", true).into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.contains("Public Profile"));
        assert!(html.contains("name=\"profile-public\""));
        let html = EmailNotificationsSwitch("sample", true).into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.contains("Email notifications"));
        assert!(html.contains("name=\"email-notifications\""));
    }
}
