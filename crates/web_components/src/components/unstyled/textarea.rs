#![allow(non_snake_case)]

use crate::components::{ComponentInfo, shared};
use maud::{Markup, html};
use std::collections::BTreeMap;

pub const COMPONENTS: &[ComponentInfo] = &[
    ComponentInfo {
        family: "unstyled",
        module: "textarea",
        name: "Textarea",
    },
    ComponentInfo {
        family: "unstyled",
        module: "textarea",
        name: "TextareaWithLabel",
    },
    ComponentInfo {
        family: "unstyled",
        module: "textarea",
        name: "AutoGrowTextarea",
    },
    ComponentInfo {
        family: "unstyled",
        module: "textarea",
        name: "TextareaWithCharCount",
    },
    ComponentInfo {
        family: "unstyled",
        module: "textarea",
        name: "RichTextarea",
    },
];

pub const RESIZE_NONE: &str = "none";
pub const RESIZE_VERTICAL: &str = "vertical";
pub const RESIZE_HORIZONTAL: &str = "horizontal";
pub const RESIZE_BOTH: &str = "both";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TextareaProps {
    pub name: String,
    pub id: String,
    pub placeholder: String,
    pub value: String,
    pub rows: i32,
    pub cols: i32,
    pub resize: &'static str,
    pub max_length: i32,
    pub size: &'static str,
    pub disabled: bool,
    pub read_only: bool,
    pub required: bool,
    pub error: bool,
    pub error_message: String,
    pub show_error: bool,
    pub auto_grow: bool,
    pub show_char_count: bool,
    pub full_width: bool,
    pub class: String,
    pub attributes: BTreeMap<String, String>,
    pub aria_label: String,
    pub aria_described_by: String,
    pub aria_invalid: bool,
}

impl Default for TextareaProps {
    fn default() -> Self {
        Self {
            name: String::new(),
            id: String::new(),
            placeholder: String::new(),
            value: String::new(),
            rows: 4,
            cols: 0,
            resize: RESIZE_VERTICAL,
            max_length: 0,
            size: shared::SIZE_MD,
            disabled: false,
            read_only: false,
            required: false,
            error: false,
            error_message: String::new(),
            show_error: true,
            auto_grow: false,
            show_char_count: false,
            full_width: true,
            class: String::new(),
            attributes: BTreeMap::new(),
            aria_label: String::new(),
            aria_described_by: String::new(),
            aria_invalid: false,
        }
    }
}

impl TextareaProps {
    #[must_use]
    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self
    }

    #[must_use]
    pub fn with_id(mut self, id: impl Into<String>) -> Self {
        self.id = id.into();
        self
    }

    #[must_use]
    pub fn with_placeholder(mut self, placeholder: impl Into<String>) -> Self {
        self.placeholder = placeholder.into();
        self
    }

    #[must_use]
    pub fn with_value(mut self, value: impl Into<String>) -> Self {
        self.value = value.into();
        self
    }

    #[must_use]
    pub const fn with_rows(mut self, rows: i32) -> Self {
        self.rows = rows;
        self
    }

    #[must_use]
    pub const fn with_cols(mut self, cols: i32) -> Self {
        self.cols = cols;
        self
    }

    #[must_use]
    pub const fn with_resize(mut self, resize: &'static str) -> Self {
        self.resize = resize;
        self
    }

    #[must_use]
    pub const fn with_max_length(mut self, max_length: i32) -> Self {
        self.max_length = max_length;
        self
    }

    #[must_use]
    pub const fn with_size(mut self, size: &'static str) -> Self {
        self.size = size;
        self
    }

    #[must_use]
    pub const fn with_disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    #[must_use]
    pub const fn with_read_only(mut self, read_only: bool) -> Self {
        self.read_only = read_only;
        self
    }

    #[must_use]
    pub const fn with_required(mut self, required: bool) -> Self {
        self.required = required;
        self
    }

    #[must_use]
    pub fn with_error(mut self, error: bool, message: impl Into<String>) -> Self {
        self.error = error;
        self.error_message = message.into();
        self.aria_invalid = error;
        self
    }

    #[must_use]
    pub const fn with_show_error(mut self, show_error: bool) -> Self {
        self.show_error = show_error;
        self
    }

    #[must_use]
    pub const fn with_auto_grow(mut self, auto_grow: bool) -> Self {
        self.auto_grow = auto_grow;
        self
    }

    #[must_use]
    pub const fn with_show_char_count(mut self, show: bool) -> Self {
        self.show_char_count = show;
        self
    }

    #[must_use]
    pub const fn with_full_width(mut self, full_width: bool) -> Self {
        self.full_width = full_width;
        self
    }

    #[must_use]
    pub fn with_class(mut self, class: impl Into<String>) -> Self {
        self.class = class.into();
        self
    }

    #[must_use]
    pub fn with_attribute(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.attributes.insert(name.into(), value.into());
        self
    }

    #[must_use]
    pub fn with_aria_label(mut self, label: impl Into<String>) -> Self {
        self.aria_label = label.into();
        self
    }

    #[must_use]
    pub fn with_aria_described_by(mut self, described_by: impl Into<String>) -> Self {
        self.aria_described_by = described_by.into();
        self
    }

    #[must_use]
    pub fn build_classes(&self) -> String {
        shared::build_classes(&[
            "w-full border-0 bg-transparent focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 placeholder:text-muted-foreground disabled:cursor-not-allowed disabled:opacity-50 transition-all duration-150",
            shared::textarea_size_classes(self.size),
            self.get_resize_classes(),
            if self.error {
                "border-destructive focus-visible:ring-destructive"
            } else {
                ""
            },
            if self.full_width { "w-full" } else { "" },
            if self.auto_grow { "min-h-[80px]" } else { "" },
            self.class.as_str(),
        ])
    }

    #[must_use]
    pub fn get_resize_classes(&self) -> &'static str {
        match self.resize {
            RESIZE_NONE => "resize-none",
            RESIZE_HORIZONTAL => "resize-x",
            RESIZE_BOTH => "resize",
            _ => "resize-y",
        }
    }

    #[must_use]
    pub fn build_wrapper_classes(&self) -> String {
        shared::build_classes(&["relative", if self.full_width { "w-full" } else { "" }])
    }

    #[must_use]
    pub fn build_attributes(&self) -> BTreeMap<String, String> {
        let mut attrs = self.attributes.clone();
        if !self.aria_label.is_empty() {
            attrs.insert("aria-label".to_string(), self.aria_label.clone());
        }
        if !self.aria_described_by.is_empty() {
            attrs.insert(
                "aria-describedby".to_string(),
                self.aria_described_by.clone(),
            );
        }
        if self.aria_invalid {
            attrs.insert("aria-invalid".to_string(), "true".to_string());
        }
        attrs
    }

    #[must_use]
    pub fn get_error_id(&self) -> String {
        if self.id.is_empty() {
            String::new()
        } else {
            format!("{}-error", self.id)
        }
    }

    #[must_use]
    pub fn get_char_count_id(&self) -> String {
        if self.id.is_empty() {
            String::new()
        } else {
            format!("{}-char-count", self.id)
        }
    }

    #[must_use]
    pub fn get_current_char_count(&self) -> usize {
        self.value.len()
    }

    #[must_use]
    pub fn is_at_max_length(&self) -> bool {
        self.max_length > 0 && self.get_current_char_count() >= self.max_length as usize
    }
}

fn textarea_described_by(
    props: &TextareaProps,
    attrs: &BTreeMap<String, String>,
) -> Option<String> {
    if props.show_char_count && !props.get_char_count_id().is_empty() {
        Some(props.get_char_count_id())
    } else if props.error && !props.get_error_id().is_empty() {
        Some(props.get_error_id())
    } else {
        shared::attr_value(attrs, "aria-describedby").map(str::to_string)
    }
}

fn textarea_element(props: &TextareaProps, auto_grow_style: bool) -> Markup {
    let attrs = props.build_attributes();
    html! {
        textarea
            name=[shared::optional_attr(&props.name)]
            id=[shared::optional_attr(&props.id)]
            placeholder=[if props.placeholder.is_empty() { None } else { Some(props.placeholder.as_str()) }]
            class=(props.build_classes())
            disabled[props.disabled]
            readonly[props.read_only]
            required[props.required]
            rows=[if props.rows > 0 { Some(props.rows.to_string()) } else { None }]
            cols=[if props.cols > 0 { Some(props.cols.to_string()) } else { None }]
            maxlength=[if props.max_length > 0 { Some(props.max_length.to_string()) } else { None }]
            aria-describedby=[textarea_described_by(props, &attrs)]
            aria-label=[shared::attr_value(&attrs, "aria-label")]
            aria-invalid=[shared::attr_value(&attrs, "aria-invalid")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
            data-bind=[shared::attr_value(&attrs, "data-bind")]
            style=[if auto_grow_style { Some("field-sizing: content; min-height: 80px;") } else { shared::attr_value(&attrs, "style") }]
        {
            (&props.value)
        }
    }
}

#[must_use]
pub fn Textarea(props: &TextareaProps) -> Markup {
    html! {
        div class=(props.build_wrapper_classes()) {
            (textarea_element(props, false))
            @if props.show_char_count && props.max_length > 0 {
                div id=(props.get_char_count_id()) class="absolute bottom-2 right-2 text-xs text-muted-foreground pointer-events-none" {
                    span class=[if props.is_at_max_length() { Some("text-destructive") } else { None }] {
                        (props.get_current_char_count())
                    }
                    "/"
                    (props.max_length)
                }
            }
        }
        @if props.error && props.show_error && !props.error_message.is_empty() {
            p id=(props.get_error_id()) class="mt-1 text-sm text-destructive" role="alert" {
                (&props.error_message)
            }
        }
    }
}

#[must_use]
pub fn TextareaWithLabel(
    props: &TextareaProps,
    label: impl Into<String>,
    helper_text: impl Into<String>,
) -> Markup {
    let label = label.into();
    let helper_text = helper_text.into();
    html! {
        div class="space-y-2" {
            @if !label.is_empty() {
                label
                    for=[shared::optional_attr(&props.id)]
                    class="text-sm font-medium leading-none peer-disabled:cursor-not-allowed peer-disabled:opacity-70"
                {
                    (&label)
                    @if props.required {
                        span class="text-destructive ml-1" aria-label=(shared::default_strings().input_required_aria_label) {
                            (shared::default_strings().input_required_symbol)
                        }
                    }
                    @if props.max_length > 0 && props.show_char_count {
                        span class="text-muted-foreground text-xs font-normal ml-2" {
                            "(" (props.get_current_char_count()) "/" (props.max_length) ")"
                        }
                    }
                }
            }
            (Textarea(props))
            @if !helper_text.is_empty() && !props.error {
                p class="text-sm text-muted-foreground" { (&helper_text) }
            }
        }
    }
}

#[must_use]
pub fn AutoGrowTextarea(props: &TextareaProps) -> Markup {
    html! {
        div class=(props.build_wrapper_classes()) data-auto-grow="true" {
            (textarea_element(props, true))
        }
        @if props.error && props.show_error && !props.error_message.is_empty() {
            p id=(props.get_error_id()) class="mt-1 text-sm text-destructive" role="alert" {
                (&props.error_message)
            }
        }
    }
}

#[must_use]
pub fn TextareaWithCharCount(props: &TextareaProps, label: impl Into<String>) -> Markup {
    let label = label.into();
    html! {
        div class="space-y-2" {
            @if !label.is_empty() {
                div class="flex justify-between items-center" {
                    label
                        for=[shared::optional_attr(&props.id)]
                        class="text-sm font-medium leading-none peer-disabled:cursor-not-allowed peer-disabled:opacity-70"
                    {
                        (&label)
                        @if props.required {
                            span class="text-destructive ml-1" aria-label=(shared::default_strings().input_required_aria_label) {
                                (shared::default_strings().input_required_symbol)
                            }
                        }
                    }
                    @if props.max_length > 0 {
                        span class=(shared::build_classes(&[
                            if props.is_at_max_length() { "text-destructive" } else { "" },
                            "text-xs text-muted-foreground",
                        ])) {
                            (props.get_current_char_count()) " / " (props.max_length)
                        }
                    }
                }
            }
            (Textarea(props))
        }
    }
}

#[must_use]
pub fn RichTextarea(
    props: &TextareaProps,
    label: impl Into<String>,
    toolbar_actions: Option<Markup>,
) -> Markup {
    let label = label.into();
    html! {
        div class="space-y-2" {
            @if !label.is_empty() {
                label
                    for=[shared::optional_attr(&props.id)]
                    class="text-sm font-medium leading-none peer-disabled:cursor-not-allowed peer-disabled:opacity-70"
                {
                    (&label)
                    @if props.required {
                        span class="text-destructive ml-1" aria-label=(shared::default_strings().input_required_aria_label) {
                            (shared::default_strings().input_required_symbol)
                        }
                    }
                }
            }
            div class="border border-border rounded-lg overflow-hidden" {
                @if let Some(toolbar) = toolbar_actions {
                    div class="border-b border-border p-2 bg-muted/50" {
                        (toolbar)
                    }
                }
                div class="p-2" {
                    (Textarea(props))
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_go_defaults() {
        let props = TextareaProps::default();
        assert_eq!(props.size, shared::SIZE_MD);
        assert_eq!(props.rows, 4);
        assert_eq!(props.cols, 0);
        assert_eq!(props.resize, RESIZE_VERTICAL);
        assert!(props.show_error);
        assert!(props.full_width);
        assert!(!props.disabled);
        assert!(!props.read_only);
        assert!(!props.required);
        assert!(!props.error);
    }

    #[test]
    fn helpers_cover_resize_classes_wrapper_attrs_ids_counts_and_builders() {
        for (resize, expected) in [
            (RESIZE_NONE, "resize-none"),
            (RESIZE_VERTICAL, "resize-y"),
            (RESIZE_HORIZONTAL, "resize-x"),
            (RESIZE_BOTH, "resize"),
            ("bad", "resize-y"),
        ] {
            assert_eq!(
                TextareaProps::default()
                    .with_resize(resize)
                    .get_resize_classes(),
                expected
            );
        }

        let props = TextareaProps::default()
            .with_name("body")
            .with_id("body")
            .with_placeholder("Write")
            .with_value("hello")
            .with_rows(5)
            .with_cols(20)
            .with_max_length(5)
            .with_size(shared::SIZE_LG)
            .with_disabled(true)
            .with_read_only(true)
            .with_required(true)
            .with_error(true, "Bad")
            .with_show_error(false)
            .with_auto_grow(true)
            .with_show_char_count(true)
            .with_full_width(false)
            .with_class("extra")
            .with_aria_label("Body")
            .with_aria_described_by("external")
            .with_attribute("data-testid", "textarea");

        assert!(props.build_classes().contains(
            "resize-y border-destructive focus-visible:ring-destructive min-h-[80px] extra"
        ));
        assert_eq!(props.build_wrapper_classes(), "relative");
        let attrs = props.build_attributes();
        assert_eq!(attrs.get("aria-label").map(String::as_str), Some("Body"));
        assert_eq!(
            attrs.get("aria-describedby").map(String::as_str),
            Some("external")
        );
        assert_eq!(attrs.get("aria-invalid").map(String::as_str), Some("true"));
        assert_eq!(props.get_error_id(), "body-error");
        assert_eq!(props.get_char_count_id(), "body-char-count");
        assert_eq!(props.get_current_char_count(), 5);
        assert!(props.is_at_max_length());
        assert_eq!(TextareaProps::default().get_error_id(), "");
        assert_eq!(TextareaProps::default().get_char_count_id(), "");
        let described = TextareaProps::default().with_attribute("aria-describedby", "external");
        assert_eq!(
            textarea_described_by(&described, &described.build_attributes()),
            Some("external".to_string())
        );
    }

    #[test]
    fn textarea_renders_boolean_attrs_char_count_error_and_escaped_value() {
        let html = Textarea(
            &TextareaProps::default()
                .with_name("message")
                .with_id("message")
                .with_placeholder("<Type>")
                .with_value("<hello>")
                .with_rows(2)
                .with_cols(10)
                .with_max_length(5)
                .with_disabled(true)
                .with_read_only(true)
                .with_required(true)
                .with_show_char_count(true)
                .with_error(true, "<Error>")
                .with_attribute("data-bind", "message")
                .with_attribute("data-testid", "textarea"),
        )
        .into_string();

        assert!(html.starts_with("<div class=\"relative w-full\">"));
        assert!(html.contains("name=\"message\""));
        assert!(html.contains("id=\"message\""));
        assert!(html.contains("placeholder=\"&lt;Type&gt;\""));
        assert!(html.contains("disabled"));
        assert!(html.contains("readonly"));
        assert!(html.contains("required"));
        assert!(html.contains("rows=\"2\""));
        assert!(html.contains("cols=\"10\""));
        assert!(html.contains("maxlength=\"5\""));
        assert!(html.contains("aria-describedby=\"message-char-count\""));
        assert!(html.contains("aria-invalid=\"true\""));
        assert!(html.contains("data-bind=\"message\""));
        assert!(html.contains("&lt;hello&gt;</textarea>"));
        assert!(html.contains("id=\"message-char-count\""));
        assert!(html.contains("<span class=\"text-destructive\">7</span>/5"));
        assert!(html.contains("<p id=\"message-error\" class=\"mt-1 text-sm text-destructive\" role=\"alert\">&lt;Error&gt;</p>"));
    }

    #[test]
    fn textarea_variants_render_labels_autogrow_char_count_and_rich_toolbar() {
        let props = TextareaProps::default()
            .with_id("body")
            .with_value("abc")
            .with_max_length(10)
            .with_show_char_count(true)
            .with_required(true);

        let labeled = TextareaWithLabel(&props, "<Body>", "Help").into_string();
        assert!(labeled.starts_with("<div class=\"space-y-2\">"));
        assert!(labeled.contains("<label for=\"body\""));
        assert!(labeled.contains("&lt;Body&gt;"));
        assert!(labeled.contains("(3/10)"));
        assert!(labeled.contains("<p class=\"text-sm text-muted-foreground\">Help</p>"));

        let autogrow = AutoGrowTextarea(&props.clone().with_auto_grow(true)).into_string();
        assert!(autogrow.contains("data-auto-grow=\"true\""));
        assert!(autogrow.contains("style=\"field-sizing: content; min-height: 80px;\""));
        let autogrow_error = AutoGrowTextarea(
            &TextareaProps::default()
                .with_id("auto")
                .with_error(true, "Too long"),
        )
        .into_string();
        assert!(autogrow_error.contains(r#"id="auto-error""#));
        assert!(autogrow_error.contains("Too long"));

        let char_count = TextareaWithCharCount(&props, "Body").into_string();
        assert!(char_count.contains("class=\"flex justify-between items-center\""));
        assert!(char_count.contains("3 / 10"));

        let rich = RichTextarea(&props, "Body", Some(html! { button type="button" { "B" } }))
            .into_string();
        assert!(rich.contains("border border-border rounded-lg overflow-hidden"));
        assert!(rich.contains("border-b border-border p-2 bg-muted/50"));
        assert!(rich.contains("<button type=\"button\">B</button>"));

        let rich_without_toolbar = RichTextarea(&props, "", None).into_string();
        assert!(!rich_without_toolbar.contains("border-b border-border p-2 bg-muted/50"));
    }
}
