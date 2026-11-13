#![allow(non_snake_case)]
#![allow(clippy::needless_pass_by_value)]

use crate::components::ComponentInfo;
use crate::components::shared;
use crate::components::unstyled::checkbox as base;
use maud::Markup;

pub use base::CheckboxProps;

pub const COMPONENTS: &[ComponentInfo] = &[
    ComponentInfo {
        family: "neomorphism",
        module: "checkbox",
        name: "Checkbox",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "checkbox",
        name: "CheckboxWithLabel",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "checkbox",
        name: "CheckboxGroup",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "checkbox",
        name: "CheckboxCard",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "checkbox",
        name: "TermsCheckbox",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "checkbox",
        name: "NewsletterCheckbox",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "checkbox",
        name: "RememberMeCheckbox",
    },
];

#[must_use]
fn addNeomorphClasses(props: &CheckboxProps) -> CheckboxProps {
    let mut props = props.clone();
    props.class = shared::build_classes(&[
        "neomorph-concave-sm checked:neomorph-flat-sm",
        if props.error {
            "border-destructive"
        } else {
            ""
        },
        props.class.as_str(),
    ]);
    props
}

#[must_use]
pub fn Checkbox(props: &CheckboxProps) -> Markup {
    let neo_props = addNeomorphClasses(props);
    base::Checkbox(&neo_props)
}

#[must_use]
pub fn CheckboxWithLabel(
    props: &CheckboxProps,
    label: impl Into<String>,
    helper_text: impl Into<String>,
) -> Markup {
    let neo_props = addNeomorphClasses(props);
    base::CheckboxWithLabel(&neo_props, label, helper_text)
}

#[must_use]
pub fn CheckboxGroup(
    legend: impl Into<String>,
    helper_text: impl Into<String>,
    required: bool,
    has_error: bool,
    error_message: impl Into<String>,
    children: Markup,
) -> Markup {
    base::CheckboxGroup(
        legend,
        helper_text,
        required,
        has_error,
        error_message,
        children,
    )
}

#[must_use]
pub fn CheckboxCard(
    props: &CheckboxProps,
    title: impl Into<String>,
    description: impl Into<String>,
) -> Markup {
    let neo_props = addNeomorphClasses(props);
    base::CheckboxCard(&neo_props, title, description)
}

#[must_use]
pub fn TermsCheckbox(id: &str, name: &str, required: bool) -> Markup {
    CheckboxWithLabel(
        &CheckboxProps::default()
            .with_id(id)
            .with_name(name)
            .with_required(required)
            .with_size(shared::SIZE_MD),
        "I agree to the terms and conditions",
        "",
    )
}

#[must_use]
pub fn NewsletterCheckbox(id: &str, name: &str, checked: bool) -> Markup {
    CheckboxWithLabel(
        &CheckboxProps::default()
            .with_id(id)
            .with_name(name)
            .with_checked(checked)
            .with_size(shared::SIZE_MD),
        "Subscribe to newsletter",
        "Receive updates and news about our products",
    )
}

#[must_use]
pub fn RememberMeCheckbox(id: &str, name: &str) -> Markup {
    CheckboxWithLabel(
        &CheckboxProps::default()
            .with_id(id)
            .with_name(name)
            .with_size(shared::SIZE_SM),
        "Remember me",
        "",
    )
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
        assert_eq!(COMPONENTS.len(), 7);
        assert!(
            COMPONENTS.iter().all(
                |component| component.family == "neomorphism" && component.module == "checkbox"
            )
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "Checkbox")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "CheckboxWithLabel")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "CheckboxGroup")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "CheckboxCard")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "TermsCheckbox")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "NewsletterCheckbox")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "RememberMeCheckbox")
        );
    }

    #[test]
    fn wrapped_components_render_unstyled_html_with_neomorphism_classes() {
        let html = Checkbox(&CheckboxProps::default()).into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.starts_with("<input"));
        assert!(html.contains("type=\"checkbox\""));
        assert!(
            html.contains("neomorph-concave-sm"),
            "missing neomorph-concave-sm: {html}"
        );
        assert!(
            html.contains("checked:neomorph-flat-sm"),
            "missing checked:neomorph-flat-sm: {html}"
        );
    }

    #[test]
    fn neomorphism_only_components_render_html_without_generated_stub_markers() {
        let error_props = CheckboxProps::default()
            .with_id("agree")
            .with_name("agree")
            .with_error(true, "Required");
        let html = CheckboxWithLabel(&error_props, "Agree", "Required to continue").into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.contains("type=\"checkbox\""));
        assert!(html.contains("id=\"agree\""));
        assert!(html.contains("name=\"agree\""));
        assert!(html.contains("border-destructive"));
        assert!(html.contains(">Agree</label>"));
        assert!(html.contains(">Required</p>"));
        assert!(!html.contains("Required to continue"));

        let html = CheckboxGroup(
            "Choices",
            "Pick at least one",
            true,
            true,
            "Choose one",
            html! { input type="checkbox" name="choice" {} },
        )
        .into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.contains("<fieldset"));
        assert!(html.contains(">Choices"));
        assert!(!html.contains("Pick at least one"));
        assert!(html.contains("Choose one"));
        assert!(html.contains("name=\"choice\""));

        let html = CheckboxGroup(
            "Choices",
            "Pick at least one",
            true,
            false,
            "",
            html! { input type="checkbox" name="choice" {} },
        )
        .into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.contains("Pick at least one"));

        let html = CheckboxCard(&error_props, "Card title", "Card description").into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.contains("type=\"checkbox\""));
        assert!(html.contains("border-destructive"));
        assert!(html.contains("Card title"));
        assert!(html.contains("Card description"));

        let html = TermsCheckbox("sample", "sample", true).into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.starts_with("<div class=\"flex items-start space-x-2\""));
        assert!(html.contains("type=\"checkbox\""));
        assert!(html.contains("id=\"sample\""));
        assert!(html.contains("name=\"sample\""));
        assert!(html.contains("required"));
        assert!(html.contains("neomorph-concave-sm checked:neomorph-flat-sm"));
        assert!(html.contains("I agree to the terms and conditions"));
        let html = NewsletterCheckbox("sample", "sample", true).into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.starts_with("<div class=\"flex items-start space-x-2\""));
        assert!(html.contains("checked"));
        assert!(html.contains("neomorph-concave-sm checked:neomorph-flat-sm"));
        assert!(html.contains("Subscribe to newsletter"));
        assert!(html.contains("Receive updates and news about our products"));
        let html = RememberMeCheckbox("sample", "sample").into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.starts_with("<div class=\"flex items-start space-x-2\""));
        assert!(html.contains("neomorph-concave-sm checked:neomorph-flat-sm"));
        assert!(html.contains("h-4 w-4"));
        assert!(html.contains("Remember me"));
    }
}
