#![allow(non_snake_case)]
#![allow(clippy::needless_pass_by_value)]

use crate::components::ComponentInfo;
use crate::components::shared;
use crate::components::unstyled::chatmessage as base;
use maud::Markup;

pub use base::{
    ChatInputProps, ChatMessageProps, ROLE_ASSISTANT, ROLE_SYSTEM, ROLE_USER, SimpleChatInput,
    SimpleChatMessage, TypingDots, TypingIndicatorProps, VARIANT_DOTS, VARIANT_TEXT,
};

pub const COMPONENTS: &[ComponentInfo] = &[
    ComponentInfo {
        family: "neomorphism",
        module: "chatmessage",
        name: "ChatMessage",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "chatmessage",
        name: "ChatInput",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "chatmessage",
        name: "TypingIndicator",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "chatmessage",
        name: "TypingDots",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "chatmessage",
        name: "SimpleChatMessage",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "chatmessage",
        name: "SimpleChatInput",
    },
];

#[must_use]
fn addMessageNeomorphClasses(props: &ChatMessageProps) -> ChatMessageProps {
    let mut props = props.clone();
    props.class = shared::build_classes(&[props.class.as_str(), "neomorph-flat"]);
    props
}

#[must_use]
pub fn ChatMessage(props: &ChatMessageProps) -> Markup {
    let neo_props = addMessageNeomorphClasses(props);
    base::ChatMessage(&neo_props)
}

#[must_use]
fn addInputNeomorphClasses(props: &ChatInputProps) -> ChatInputProps {
    let mut props = props.clone();
    props.class = shared::build_classes(&[props.class.as_str(), "neomorph-flat"]);
    props
}

#[must_use]
pub fn ChatInput(props: &ChatInputProps) -> Markup {
    let neo_props = addInputNeomorphClasses(props);
    base::ChatInput(&neo_props)
}

#[must_use]
fn addTypingNeomorphClasses(props: &TypingIndicatorProps) -> TypingIndicatorProps {
    let mut props = props.clone();
    props.class = shared::build_classes(&[props.class.as_str(), "neomorph-flat"]);
    props
}

#[must_use]
pub fn TypingIndicator(props: &TypingIndicatorProps) -> Markup {
    let neo_props = addTypingNeomorphClasses(props);
    base::TypingIndicator(&neo_props)
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
        assert_eq!(COMPONENTS.len(), 6);
        assert!(COMPONENTS.iter().all(
            |component| component.family == "neomorphism" && component.module == "chatmessage"
        ));
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "ChatMessage")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "ChatInput")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "TypingIndicator")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "TypingDots")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "SimpleChatMessage")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "SimpleChatInput")
        );
    }

    #[test]
    fn wrapped_components_render_unstyled_html_with_neomorphism_classes() {
        let html = ChatMessage(&ChatMessageProps::default()).into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.starts_with("<"), "expected rendered element: {html}");
        assert!(
            html.contains("neomorph-flat"),
            "missing neomorph-flat: {html}"
        );
        let html = ChatInput(&ChatInputProps::default()).into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.starts_with("<"), "expected rendered element: {html}");
        assert!(
            html.contains("neomorph-flat"),
            "missing neomorph-flat: {html}"
        );
        let html = TypingIndicator(&TypingIndicatorProps::default()).into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.starts_with("<"), "expected rendered element: {html}");
        assert!(
            html.contains("neomorph-flat"),
            "missing neomorph-flat: {html}"
        );
    }
}
