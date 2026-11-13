#![allow(non_snake_case)]
#![allow(clippy::needless_pass_by_value)]

use crate::components::ComponentInfo;
use crate::components::shared;
use crate::components::unstyled::textarea as base;
use maud::Markup;

pub use base::{RESIZE_BOTH, RESIZE_HORIZONTAL, RESIZE_NONE, RESIZE_VERTICAL, TextareaProps};

pub const COMPONENTS: &[ComponentInfo] = &[
    ComponentInfo {
        family: "neomorphism",
        module: "textarea",
        name: "Textarea",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "textarea",
        name: "TextareaWithLabel",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "textarea",
        name: "AutoGrowTextarea",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "textarea",
        name: "TextareaWithCharCount",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "textarea",
        name: "RichTextarea",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "textarea",
        name: "CommentTextarea",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "textarea",
        name: "BioTextarea",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "textarea",
        name: "MessageTextarea",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "textarea",
        name: "FeedbackTextarea",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "textarea",
        name: "NotesTextarea",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "textarea",
        name: "CodeTextarea",
    },
];

#[must_use]
fn addNeomorphClasses(props: &TextareaProps) -> TextareaProps {
    let mut props = props.clone();
    props.class = shared::build_classes(&[
        "neomorph-textarea",
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
pub fn Textarea(props: &TextareaProps) -> Markup {
    let neo_props = addNeomorphClasses(props);
    base::Textarea(&neo_props)
}

#[must_use]
pub fn AutoGrowTextarea(props: &TextareaProps) -> Markup {
    let neo_props = addNeomorphClasses(props);
    base::AutoGrowTextarea(&neo_props)
}

#[must_use]
pub fn TextareaWithLabel(
    props: &TextareaProps,
    label: impl Into<String>,
    helper_text: impl Into<String>,
) -> Markup {
    let neo_props = addNeomorphClasses(props);
    base::TextareaWithLabel(&neo_props, label, helper_text)
}

#[must_use]
pub fn TextareaWithCharCount(props: &TextareaProps, label: impl Into<String>) -> Markup {
    let neo_props = addNeomorphClasses(props);
    base::TextareaWithCharCount(&neo_props, label)
}

#[must_use]
pub fn RichTextarea(
    props: &TextareaProps,
    label: impl Into<String>,
    toolbar_actions: Option<Markup>,
) -> Markup {
    let neo_props = addNeomorphClasses(props);
    base::RichTextarea(&neo_props, label, toolbar_actions)
}

#[must_use]
pub fn CommentTextarea(id: &str, name: &str, placeholder: &str) -> Markup {
    TextareaWithLabel(
        &TextareaProps::default()
            .with_id(id)
            .with_name(name)
            .with_placeholder(placeholder)
            .with_rows(3)
            .with_size(shared::SIZE_MD)
            .with_resize(RESIZE_VERTICAL),
        "Comment",
        "",
    )
}

#[must_use]
pub fn BioTextarea(id: &str, name: &str, value: &str, maxLength: i32) -> Markup {
    TextareaWithCharCount(
        &TextareaProps::default()
            .with_id(id)
            .with_name(name)
            .with_value(value)
            .with_placeholder("Tell us about yourself...")
            .with_rows(5)
            .with_max_length(maxLength)
            .with_size(shared::SIZE_MD)
            .with_resize(RESIZE_VERTICAL)
            .with_show_char_count(true),
        "Bio",
    )
}

#[must_use]
pub fn MessageTextarea(id: &str, name: &str, required: bool) -> Markup {
    TextareaWithLabel(
        &TextareaProps::default()
            .with_id(id)
            .with_name(name)
            .with_placeholder("Type your message here...")
            .with_rows(6)
            .with_required(required)
            .with_size(shared::SIZE_MD)
            .with_resize(RESIZE_VERTICAL),
        "Message",
        "",
    )
}

#[must_use]
pub fn FeedbackTextarea(id: &str, name: &str, maxLength: i32) -> Markup {
    TextareaWithCharCount(
        &TextareaProps::default()
            .with_id(id)
            .with_name(name)
            .with_placeholder("Share your feedback...")
            .with_rows(4)
            .with_max_length(maxLength)
            .with_size(shared::SIZE_MD)
            .with_resize(RESIZE_VERTICAL)
            .with_show_char_count(true)
            .with_required(true),
        "Feedback",
    )
}

#[must_use]
pub fn NotesTextarea(id: &str, name: &str, value: &str) -> Markup {
    TextareaWithLabel(
        &TextareaProps::default()
            .with_id(id)
            .with_name(name)
            .with_value(value)
            .with_placeholder("Add your notes here...")
            .with_rows(8)
            .with_size(shared::SIZE_MD)
            .with_resize(RESIZE_BOTH),
        "Notes",
        "These notes are private and only visible to you",
    )
}

#[must_use]
pub fn CodeTextarea(id: &str, name: &str, value: &str) -> Markup {
    TextareaWithLabel(
        &TextareaProps::default()
            .with_id(id)
            .with_name(name)
            .with_value(value)
            .with_placeholder("// Enter code here")
            .with_rows(12)
            .with_size(shared::SIZE_MD)
            .with_resize(RESIZE_BOTH)
            .with_class("font-mono text-sm"),
        "Code",
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
        assert_eq!(COMPONENTS.len(), 11);
        assert!(
            COMPONENTS.iter().all(
                |component| component.family == "neomorphism" && component.module == "textarea"
            )
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "Textarea")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "TextareaWithLabel")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "AutoGrowTextarea")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "TextareaWithCharCount")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "RichTextarea")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "CommentTextarea")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "BioTextarea")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "MessageTextarea")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "FeedbackTextarea")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "NotesTextarea")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "CodeTextarea")
        );
    }

    #[test]
    fn wrapped_components_render_unstyled_html_with_neomorphism_classes() {
        let html = Textarea(&TextareaProps::default()).into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.starts_with("<div class=\"relative w-full\""));
        assert!(html.contains("<textarea"));
        assert!(html.contains("neomorph-textarea"));
        assert!(html.contains("rows=\"4\""));
        assert!(html.contains("resize-y"));
        let html = AutoGrowTextarea(&TextareaProps::default()).into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.starts_with("<div class=\"relative w-full\" data-auto-grow=\"true\""));
        assert!(html.contains("<textarea"));
        assert!(html.contains("data-auto-grow=\"true\""));
        assert!(html.contains("neomorph-textarea"));
        assert!(html.contains("style=\"field-sizing: content; min-height: 80px;\""));
    }

    #[test]
    fn neomorphism_only_components_render_html_without_generated_stub_markers() {
        let error_props = TextareaProps::default()
            .with_id("rich")
            .with_name("rich")
            .with_value("body")
            .with_error(true, "Too long");
        let html = RichTextarea(
            &error_props,
            "Rich body",
            Some(html! { button type="button" { "Bold" } }),
        )
        .into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.contains("<textarea"));
        assert!(html.contains("id=\"rich\""));
        assert!(html.contains("name=\"rich\""));
        assert!(html.contains("neomorph-textarea"));
        assert!(html.contains("border-destructive"));
        assert!(html.contains("Rich body"));
        assert!(html.contains("Bold"));
        assert!(html.contains(">body</textarea>"));

        let html = CommentTextarea("sample", "sample", "sample").into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.starts_with("<div class=\"space-y-2\""));
        assert!(html.contains(">Comment</label>"));
        assert!(html.contains("id=\"sample\""));
        assert!(html.contains("name=\"sample\""));
        assert!(html.contains("placeholder=\"sample\""));
        assert!(html.contains("rows=\"3\""));
        assert!(html.contains("neomorph-textarea"));
        let html = BioTextarea("sample", "sample", "sample", 2).into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.starts_with("<div class=\"space-y-2\""));
        assert!(html.contains(">Bio</label>"));
        assert!(html.contains("maxlength=\"2\""));
        assert!(html.contains(">sample</textarea>"));
        assert!(html.contains("neomorph-textarea"));
        let html = MessageTextarea("sample", "sample", true).into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.starts_with("<div class=\"space-y-2\""));
        assert!(html.contains(">Message"));
        assert!(html.contains("required"));
        assert!(html.contains("Type your message here..."));
        assert!(html.contains("neomorph-textarea"));
        let html = FeedbackTextarea("sample", "sample", 2).into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.starts_with("<div class=\"space-y-2\""));
        assert!(html.contains(">Feedback"));
        assert!(html.contains("maxlength=\"2\""));
        assert!(html.contains("required"));
        assert!(html.contains("Share your feedback..."));
        assert!(html.contains("neomorph-textarea"));
        let html = NotesTextarea("sample", "sample", "sample").into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.starts_with("<div class=\"space-y-2\""));
        assert!(html.contains(">Notes</label>"));
        assert!(html.contains(">sample</textarea>"));
        assert!(html.contains("These notes are private and only visible to you"));
        assert!(html.contains("resize"));
        assert!(html.contains("neomorph-textarea"));
        let html = CodeTextarea("sample", "sample", "sample").into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.starts_with("<div class=\"space-y-2\""));
        assert!(html.contains(">Code</label>"));
        assert!(html.contains(">sample</textarea>"));
        assert!(html.contains("neomorph-textarea font-mono text-sm"));
        assert!(html.contains("// Enter code here"));
    }
}
