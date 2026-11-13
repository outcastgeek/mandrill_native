#![allow(non_snake_case)]
#![allow(clippy::needless_pass_by_value)]

use crate::components::ComponentInfo;
use crate::components::shared;
use crate::components::unstyled::radio as base;
use maud::Markup;

pub use base::{RadioGroupProps, RadioOption, RadioProps};

pub const COMPONENTS: &[ComponentInfo] = &[
    ComponentInfo {
        family: "neomorphism",
        module: "radio",
        name: "Radio",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "radio",
        name: "RadioWithLabel",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "radio",
        name: "RadioGroup",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "radio",
        name: "RadioCard",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "radio",
        name: "RadioGroupCards",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "radio",
        name: "PaymentMethodRadio",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "radio",
        name: "ShippingMethodRadio",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "radio",
        name: "NotificationPreferenceRadio",
    },
];

#[must_use]
fn addNeomorphClasses(props: &RadioProps) -> RadioProps {
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
pub fn Radio(props: &RadioProps) -> Markup {
    let neo_props = addNeomorphClasses(props);
    base::Radio(&neo_props)
}

#[must_use]
pub fn RadioWithLabel(
    props: &RadioProps,
    label: impl Into<String>,
    helper_text: impl Into<String>,
) -> Markup {
    let neo_props = addNeomorphClasses(props);
    base::RadioWithLabel(&neo_props, label, helper_text)
}

#[must_use]
pub fn RadioGroup(
    group: &RadioGroupProps,
    legend: impl Into<String>,
    helper_text: impl Into<String>,
    options: &[RadioOption],
) -> Markup {
    base::RadioGroup(group, legend, helper_text, options)
}

#[must_use]
pub fn RadioCard(
    props: &RadioProps,
    title: impl Into<String>,
    description: impl Into<String>,
) -> Markup {
    let neo_props = addNeomorphClasses(props);
    base::RadioCard(&neo_props, title, description)
}

#[must_use]
pub fn RadioGroupCards(
    group: &RadioGroupProps,
    legend: impl Into<String>,
    helper_text: impl Into<String>,
    options: &[RadioOption],
) -> Markup {
    base::RadioGroupCards(group, legend, helper_text, options)
}

#[must_use]
pub fn PaymentMethodRadio(name: &str, required: bool) -> Markup {
    RadioGroup(
        &RadioGroupProps::default()
            .with_name(name)
            .with_required(required)
            .with_orientation("vertical")
            .with_size(shared::SIZE_MD),
        "Payment Method",
        "Select how you would like to pay",
        &[
            RadioOption::default()
                .with_value("card")
                .with_label("Credit/Debit Card")
                .with_help_text("Visa, Mastercard, Amex"),
            RadioOption::default()
                .with_value("paypal")
                .with_label("PayPal")
                .with_help_text("Fast and secure"),
            RadioOption::default()
                .with_value("bank")
                .with_label("Bank Transfer")
                .with_help_text("Direct from your bank account"),
        ],
    )
}

#[must_use]
pub fn ShippingMethodRadio(name: &str, required: bool) -> Markup {
    RadioGroupCards(
        &RadioGroupProps::default()
            .with_name(name)
            .with_required(required)
            .with_size(shared::SIZE_MD),
        "Shipping Method",
        "Choose your preferred delivery option",
        &[
            RadioOption::default()
                .with_value("standard")
                .with_label("Standard Shipping")
                .with_help_text("5-7 business days - Free"),
            RadioOption::default()
                .with_value("express")
                .with_label("Express Shipping")
                .with_help_text("2-3 business days - $9.99"),
            RadioOption::default()
                .with_value("overnight")
                .with_label("Overnight Shipping")
                .with_help_text("Next business day - $19.99"),
        ],
    )
}

#[must_use]
pub fn NotificationPreferenceRadio(name: &str, required: bool) -> Markup {
    RadioGroup(
        &RadioGroupProps::default()
            .with_name(name)
            .with_required(required)
            .with_orientation("vertical")
            .with_size(shared::SIZE_MD),
        "Notification Preferences",
        "How would you like to receive notifications?",
        &[
            RadioOption::default()
                .with_value("email")
                .with_label("Email")
                .with_help_text("Receive notifications via email"),
            RadioOption::default()
                .with_value("sms")
                .with_label("SMS")
                .with_help_text("Receive notifications via text message"),
            RadioOption::default()
                .with_value("both")
                .with_label("Both")
                .with_help_text("Receive via email and SMS"),
            RadioOption::default()
                .with_value("none")
                .with_label("None")
                .with_help_text("Don't send me notifications"),
        ],
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
        assert_eq!(COMPONENTS.len(), 8);
        assert!(
            COMPONENTS
                .iter()
                .all(|component| component.family == "neomorphism" && component.module == "radio")
        );
        assert!(COMPONENTS.iter().any(|component| component.name == "Radio"));
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "RadioWithLabel")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "RadioGroup")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "RadioCard")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "RadioGroupCards")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "PaymentMethodRadio")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "ShippingMethodRadio")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "NotificationPreferenceRadio")
        );
    }

    #[test]
    fn wrapped_components_render_unstyled_html_with_neomorphism_classes() {
        let html = Radio(&RadioProps::default()).into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.starts_with("<"), "expected rendered element: {html}");
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
        let error_props = RadioProps::default()
            .with_id("plan-basic")
            .with_name("plan")
            .with_value("basic")
            .with_error(true);
        let html = RadioWithLabel(&error_props, "Basic", "Starter plan").into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.contains("type=\"radio\""));
        assert!(html.contains("id=\"plan-basic\""));
        assert!(html.contains("name=\"plan\""));
        assert!(html.contains("value=\"basic\""));
        assert!(html.contains("border-destructive"));
        assert!(html.contains(">Basic</label>"));
        assert!(html.contains("Starter plan"));

        let html = RadioCard(&error_props, "Card plan", "Card description").into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.contains("type=\"radio\""));
        assert!(html.contains("border-destructive"));
        assert!(html.contains("Card plan"));
        assert!(html.contains("Card description"));

        let html = PaymentMethodRadio("sample", true).into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.contains("<fieldset"));
        assert!(html.contains("Payment Method"));
        assert!(html.contains("Credit/Debit Card"));
        let html = ShippingMethodRadio("sample", true).into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.contains("Shipping Method"));
        assert!(html.contains("Overnight Shipping"));
        let html = NotificationPreferenceRadio("sample", true).into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.contains("Notification Preferences"));
        assert!(html.contains("Receive via email and SMS"));
    }
}
