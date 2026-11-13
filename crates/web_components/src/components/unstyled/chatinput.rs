#![allow(non_snake_case)]

use crate::components::unstyled::button::{Button, ButtonProps, loadingSpinner};
use crate::components::{ComponentInfo, shared};
use maud::{Markup, html};
use std::collections::BTreeMap;

pub const COMPONENTS: &[ComponentInfo] = &[
    ComponentInfo {
        family: "unstyled",
        module: "chatinput",
        name: "ChatInput",
    },
    ComponentInfo {
        family: "unstyled",
        module: "chatinput",
        name: "SimpleChatInput",
    },
];

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChatInputProps {
    pub name: String,
    pub placeholder: String,
    pub rows: i32,
    pub value: String,
    pub form_id: String,
    pub form_action: String,
    pub csrf_token: String,
    pub csrf_field_name: String,
    pub submit_button_text: String,
    pub stop_button_text: String,
    pub data_model: String,
    pub loading_signal: String,
    pub show_loading_state: bool,
    pub sticky_bottom: bool,
    pub required: bool,
    pub size: &'static str,
    pub min_height: String,
    pub class: String,
    pub textarea_class: String,
    pub attributes: BTreeMap<String, String>,
    pub textarea_attributes: BTreeMap<String, String>,
    pub on_submit_extra: String,
    pub disable_enter_submit: bool,
    pub keyboard_hint: String,
    pub max_length: i32,
    pub show_char_count: bool,
}

impl Default for ChatInputProps {
    fn default() -> Self {
        Self {
            name: "message".to_string(),
            placeholder: String::new(),
            rows: 1,
            value: String::new(),
            form_id: String::new(),
            form_action: String::new(),
            csrf_token: String::new(),
            csrf_field_name: "_csrf".to_string(),
            submit_button_text: String::new(),
            stop_button_text: String::new(),
            data_model: "message".to_string(),
            loading_signal: "isLoading".to_string(),
            show_loading_state: true,
            sticky_bottom: true,
            required: true,
            size: shared::SIZE_MD,
            min_height: String::new(),
            class: String::new(),
            textarea_class: String::new(),
            attributes: BTreeMap::new(),
            textarea_attributes: BTreeMap::new(),
            on_submit_extra: String::new(),
            disable_enter_submit: false,
            keyboard_hint: String::new(),
            max_length: 0,
            show_char_count: false,
        }
    }
}

impl ChatInputProps {
    #[must_use]
    pub fn with_name(mut self, value: impl Into<String>) -> Self {
        self.name = value.into();
        self
    }
    #[must_use]
    pub fn with_placeholder(mut self, value: impl Into<String>) -> Self {
        self.placeholder = value.into();
        self
    }
    #[must_use]
    pub const fn with_rows(mut self, value: i32) -> Self {
        self.rows = value;
        self
    }
    #[must_use]
    pub fn with_value(mut self, value: impl Into<String>) -> Self {
        self.value = value.into();
        self
    }
    #[must_use]
    pub fn with_form_id(mut self, value: impl Into<String>) -> Self {
        self.form_id = value.into();
        self
    }
    #[must_use]
    pub fn with_form_action(mut self, value: impl Into<String>) -> Self {
        self.form_action = value.into();
        self
    }
    #[must_use]
    pub fn with_csrf_token(mut self, value: impl Into<String>) -> Self {
        self.csrf_token = value.into();
        self
    }
    #[must_use]
    pub fn with_csrf_field_name(mut self, value: impl Into<String>) -> Self {
        self.csrf_field_name = value.into();
        self
    }
    #[must_use]
    pub fn with_submit_button_text(mut self, value: impl Into<String>) -> Self {
        self.submit_button_text = value.into();
        self
    }
    #[must_use]
    pub fn with_stop_button_text(mut self, value: impl Into<String>) -> Self {
        self.stop_button_text = value.into();
        self
    }
    #[must_use]
    pub fn with_data_model(mut self, value: impl Into<String>) -> Self {
        self.data_model = value.into();
        self
    }
    #[must_use]
    pub fn with_loading_signal(mut self, value: impl Into<String>) -> Self {
        self.loading_signal = value.into();
        self
    }
    #[must_use]
    pub const fn with_sticky_bottom(mut self, value: bool) -> Self {
        self.sticky_bottom = value;
        self
    }
    #[must_use]
    pub const fn with_show_loading_state(mut self, value: bool) -> Self {
        self.show_loading_state = value;
        self
    }
    #[must_use]
    pub const fn with_required(mut self, value: bool) -> Self {
        self.required = value;
        self
    }
    #[must_use]
    pub const fn with_size(mut self, value: &'static str) -> Self {
        self.size = value;
        self
    }
    #[must_use]
    pub fn with_class(mut self, value: impl Into<String>) -> Self {
        self.class = value.into();
        self
    }
    #[must_use]
    pub fn with_textarea_class(mut self, value: impl Into<String>) -> Self {
        self.textarea_class = value.into();
        self
    }
    #[must_use]
    pub fn with_min_height(mut self, value: impl Into<String>) -> Self {
        self.min_height = value.into();
        self
    }
    #[must_use]
    pub fn with_attribute(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.attributes.insert(name.into(), value.into());
        self
    }
    #[must_use]
    pub fn with_textarea_attribute(
        mut self,
        name: impl Into<String>,
        value: impl Into<String>,
    ) -> Self {
        self.textarea_attributes.insert(name.into(), value.into());
        self
    }
    #[must_use]
    pub fn with_on_submit_extra(mut self, value: impl Into<String>) -> Self {
        self.on_submit_extra = value.into();
        self
    }
    #[must_use]
    pub const fn with_disable_enter_submit(mut self, value: bool) -> Self {
        self.disable_enter_submit = value;
        self
    }
    #[must_use]
    pub fn with_keyboard_hint(mut self, value: impl Into<String>) -> Self {
        self.keyboard_hint = value.into();
        self
    }
    #[must_use]
    pub const fn with_max_length(mut self, value: i32) -> Self {
        self.max_length = value;
        self
    }
    #[must_use]
    pub const fn with_show_char_count(mut self, value: bool) -> Self {
        self.show_char_count = value;
        self
    }
    #[must_use]
    pub fn build_container_classes(&self) -> String {
        shared::build_classes(&[
            "flex-shrink-0 bg-background border-t border-border px-3 py-2",
            if self.sticky_bottom {
                "sticky bottom-0"
            } else {
                ""
            },
            self.class.as_str(),
        ])
    }
    #[must_use]
    pub fn build_textarea_style(&self) -> String {
        if self.min_height.is_empty() {
            "max-height: 200px; overflow-y: auto;".to_string()
        } else {
            format!(
                "height: auto; min-height: {}; max-height: 200px; overflow-y: auto;",
                self.min_height
            )
        }
    }
    #[must_use]
    pub fn get_name(&self) -> &str {
        if self.name.is_empty() {
            "message"
        } else {
            self.name.as_str()
        }
    }
    #[must_use]
    pub fn get_data_model(&self) -> &str {
        if self.data_model.is_empty() {
            "message"
        } else {
            self.data_model.as_str()
        }
    }
    #[must_use]
    pub fn get_loading_signal(&self) -> &str {
        if self.loading_signal.is_empty() {
            "isLoading"
        } else {
            self.loading_signal.as_str()
        }
    }
    #[must_use]
    pub fn get_placeholder(&self) -> String {
        if self.placeholder.is_empty() {
            "Type a message...".to_string()
        } else {
            self.placeholder.clone()
        }
    }
    #[must_use]
    pub fn get_submit_button_text(&self) -> String {
        if self.submit_button_text.is_empty() {
            "Send".to_string()
        } else {
            self.submit_button_text.clone()
        }
    }
    #[must_use]
    pub fn get_stop_button_text(&self) -> String {
        if self.stop_button_text.is_empty() {
            "Stop".to_string()
        } else {
            self.stop_button_text.clone()
        }
    }
    #[must_use]
    pub const fn get_rows(&self) -> i32 {
        if self.rows <= 0 { 1 } else { self.rows }
    }
    #[must_use]
    pub fn get_csrf_field_name(&self) -> &str {
        if self.csrf_field_name.is_empty() {
            "_csrf"
        } else {
            self.csrf_field_name.as_str()
        }
    }
    #[must_use]
    pub fn build_datastar_textarea_attrs(&self) -> BTreeMap<String, String> {
        let mut attrs =
            BTreeMap::from([("data-bind".to_string(), self.get_data_model().to_string())]);
        if self.show_loading_state {
            attrs.insert(
                "data-attr:disabled".to_string(),
                format!("${}", self.get_loading_signal()),
            );
        }
        attrs
    }
}

#[must_use]
pub fn autoResizeJS() -> &'static str {
    "evt.target.style.height = 'auto'; evt.target.style.height = Math.min(evt.target.scrollHeight, 200) + 'px'"
}

#[must_use]
pub fn charCountInputJS(char_count_id: &str, max_length: i32) -> String {
    format!(
        "; var cc = document.getElementById('{char_count_id}'); if(cc) cc.textContent = evt.target.value.length + ' / {max_length}'"
    )
}

#[must_use]
pub fn resetTextareaJS(name: &str) -> String {
    format!("; var ta = evt.target.elements.{name}; if(ta) {{ ta.style.height = 'auto'; }}")
}

#[must_use]
pub fn charCountID(props: &ChatInputProps) -> String {
    format!("char-count-{}", props.get_name())
}

#[must_use]
pub fn buildAutoResizeAndCharCountAttrs(props: &ChatInputProps) -> BTreeMap<String, String> {
    let mut js = autoResizeJS().to_string();
    if props.show_char_count && props.max_length > 0 {
        js.push_str(&charCountInputJS(&charCountID(props), props.max_length));
    }
    BTreeMap::from([("data-on:input".to_string(), js)])
}

#[must_use]
pub fn simpleChatInputContainerClass(props: &ChatInputProps) -> String {
    if props.class.is_empty() {
        "w-full".to_string()
    } else {
        props.class.clone()
    }
}

#[must_use]
pub fn buildSubmitHandler(props: &ChatInputProps) -> String {
    let loading_signal = props.get_loading_signal();
    let data_model = props.get_data_model();
    let name = props.get_name();
    let mut base = format!(
        "evt.preventDefault(); var inputVal = evt.target.elements.{name}.value.trim(); if(!inputVal) return;"
    );
    base.push_str(&format!(
        " ${loading_signal} = true; if(window.forceScrollToBottom) window.forceScrollToBottom();"
    ));
    if !props.form_action.is_empty() && !props.csrf_token.is_empty() {
        base.push_str(&format!(
            " @post('{}', {{contentType: 'form', headers: {{'X-CSRF-Token': '{}'}}}});",
            props.form_action, props.csrf_token
        ));
    }
    base.push_str(&format!(
        " evt.target.elements.{name}.value = ''; ${data_model} = ''"
    ));
    base.push_str(&resetTextareaJS(name));
    if props.show_char_count && props.max_length > 0 {
        base.push_str(&format!(
            "; var cc = document.getElementById('{}'); if(cc) cc.textContent = '0 / {}'",
            charCountID(props),
            props.max_length
        ));
    }
    if !props.on_submit_extra.is_empty() {
        base.push_str("; ");
        base.push_str(&props.on_submit_extra);
    }
    base
}

fn arrow_up_icon() -> Markup {
    html! {
        svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true" {
            path d="m5 12 7-7 7 7" {}
            path d="M12 19V5" {}
        }
    }
}

fn icon_button_classes(props: &ButtonProps) -> String {
    let mut icon_props = props.clone();
    icon_props.class = shared::build_classes(&[props.class.as_str(), "aspect-square p-0"]);
    icon_props.build_classes()
}

fn submit_icon_button(props: &ChatInputProps, data_show: Option<String>) -> Markup {
    let button_props = ButtonProps::default()
        .with_type("submit")
        .with_variant(shared::VARIANT_PRIMARY)
        .with_size(shared::SIZE_XS)
        .with_aria_label(props.get_submit_button_text())
        .with_class("absolute right-2.5 bottom-2.5 w-8 h-8 rounded-full");
    html! {
        button
            type="submit"
            class=(icon_button_classes(&button_props))
            aria-label=(props.get_submit_button_text())
            data-show=[data_show.as_deref()]
        {
            (arrow_up_icon())
        }
    }
}

fn stop_icon_button(props: &ChatInputProps) -> Markup {
    let button_props = ButtonProps::default()
        .with_type("button")
        .with_variant(shared::VARIANT_DESTRUCTIVE)
        .with_size(shared::SIZE_XS)
        .with_aria_label(props.get_stop_button_text())
        .with_loading(true)
        .with_class("absolute right-2.5 bottom-2.5 w-8 h-8 rounded-full");
    html! {
        button
            type="button"
            class=(icon_button_classes(&button_props))
            disabled
            aria-label=(props.get_stop_button_text())
            aria-busy="true"
            data-show=(format!("${}", props.get_loading_signal()))
            data-on:click=(format!("${} = false", props.get_loading_signal()))
            style="display:none"
        {
            (loadingSpinner())
        }
    }
}

#[must_use]
pub fn ChatInput(props: &ChatInputProps) -> Markup {
    let form_attrs = &props.attributes;
    let textarea_attrs = &props.textarea_attributes;
    let auto_attrs = buildAutoResizeAndCharCountAttrs(props);
    let ds_attrs = props.build_datastar_textarea_attrs();
    html! {
        div class=(props.build_container_classes()) {
            form
                method="POST"
                class="w-full"
                id=[if props.form_id.is_empty() { None } else { Some(props.form_id.as_str()) }]
                data-chat-form="true"
                data-on:submit=(buildSubmitHandler(props))
                data-testid=[shared::attr_value(form_attrs, "data-testid")]
            {
                @if !props.csrf_token.is_empty() {
                    input type="hidden" name=(props.get_csrf_field_name()) value=(&props.csrf_token);
                }
                div class="relative" {
                    textarea
                        name=(props.get_name())
                        placeholder=(props.get_placeholder())
                        aria-label=(props.get_placeholder())
                        rows=(props.get_rows())
                        class=(shared::build_classes(&[
                            "w-full focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-1 placeholder:text-muted-foreground disabled:cursor-not-allowed disabled:opacity-50 transition-all duration-150 text-sm rounded-[var(--radius)] resize-none border border-input bg-input p-3 pr-12",
                            props.textarea_class.as_str(),
                        ]))
                        style=(props.build_textarea_style())
                        required[props.required]
                        maxlength=[if props.max_length > 0 { Some(props.max_length.to_string()) } else { None }]
                        data-on:keydown=[if props.disable_enter_submit { None } else { Some("if(evt.key === 'Enter' && !evt.shiftKey) { evt.preventDefault(); if(evt.target.value.trim()) evt.target.form.requestSubmit(); }") }]
                        data-on:input=[shared::attr_value(&auto_attrs, "data-on:input")]
                        data-bind=[shared::attr_value(&ds_attrs, "data-bind")]
                        data-attr:disabled=[shared::attr_value(&ds_attrs, "data-attr:disabled")]
                        id=[shared::attr_value(textarea_attrs, "id")]
                        data-testid=[shared::attr_value(textarea_attrs, "data-testid")]
                    {
                        (&props.value)
                    }
                    @if props.show_loading_state {
                        (submit_icon_button(props, Some(format!("!${}", props.get_loading_signal()))))
                        (stop_icon_button(props))
                    } @else {
                        (submit_icon_button(props, None))
                    }
                }
            }
            @if !props.keyboard_hint.is_empty() || (props.show_char_count && props.max_length > 0) {
                div class="flex justify-between items-center mt-1.5 px-1" {
                    @if !props.keyboard_hint.is_empty() && !props.disable_enter_submit {
                        p class="text-[11px] text-muted-foreground" aria-hidden="true" {
                            (&props.keyboard_hint)
                        }
                    } @else {
                        span {}
                    }
                    @if props.show_char_count && props.max_length > 0 {
                        p id=(charCountID(props)) class="text-[11px] text-muted-foreground" {
                            (format!("0 / {}", props.max_length))
                        }
                    }
                }
            }
        }
    }
}

#[must_use]
pub fn SimpleChatInput(props: &ChatInputProps) -> Markup {
    let auto_attrs = buildAutoResizeAndCharCountAttrs(props);
    let ds_attrs = props.build_datastar_textarea_attrs();
    html! {
        div class=(simpleChatInputContainerClass(props)) {
            div class="space-y-3" {
                div class="relative w-full" {
                    textarea
                        name=(props.get_name())
                        placeholder=(props.get_placeholder())
                        rows=(props.get_rows())
                        class=(shared::build_classes(&[
                            "w-full focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-1 placeholder:text-muted-foreground disabled:cursor-not-allowed disabled:opacity-50 transition-all duration-150 text-sm rounded-[var(--radius)] resize-none border border-input bg-input py-2.5 px-3",
                            props.textarea_class.as_str(),
                        ]))
                        style=(props.build_textarea_style())
                        required[props.required]
                        data-on:input=[shared::attr_value(&auto_attrs, "data-on:input")]
                        data-bind=[shared::attr_value(&ds_attrs, "data-bind")]
                        data-attr:disabled=[shared::attr_value(&ds_attrs, "data-attr:disabled")]
                    {
                        (&props.value)
                    }
                }
                (Button(&ButtonProps::default()
                    .with_type("button")
                    .with_variant(shared::VARIANT_PRIMARY)
                    .with_size(shared::SIZE_MD)
                    .with_full_width(true)
                    .with_class("flex items-center justify-center gap-2"),
                    html! { (props.get_submit_button_text()) }))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn helpers_cover_defaults_builders_datastar_and_submit_handler() {
        let props = ChatInputProps::default();
        assert_eq!(props.get_name(), "message");
        assert_eq!(props.get_data_model(), "message");
        assert_eq!(props.get_loading_signal(), "isLoading");
        assert_eq!(props.get_placeholder(), "Type a message...");
        assert_eq!(props.get_submit_button_text(), "Send");
        assert_eq!(props.get_stop_button_text(), "Stop");
        assert_eq!(props.get_rows(), 1);
        assert_eq!(props.get_csrf_field_name(), "_csrf");
        assert_eq!(
            props.build_container_classes(),
            "flex-shrink-0 bg-background border-t border-border px-3 py-2 sticky bottom-0"
        );
        assert_eq!(
            props.build_textarea_style(),
            "max-height: 200px; overflow-y: auto;"
        );
        assert_eq!(
            autoResizeJS(),
            "evt.target.style.height = 'auto'; evt.target.style.height = Math.min(evt.target.scrollHeight, 200) + 'px'"
        );
        assert_eq!(
            charCountInputJS("cc", 100),
            "; var cc = document.getElementById('cc'); if(cc) cc.textContent = evt.target.value.length + ' / 100'"
        );
        assert_eq!(
            resetTextareaJS("body"),
            "; var ta = evt.target.elements.body; if(ta) { ta.style.height = 'auto'; }"
        );
        assert_eq!(charCountID(&props), "char-count-message");

        let datastar = props.build_datastar_textarea_attrs();
        assert_eq!(
            datastar.get("data-bind").map(String::as_str),
            Some("message")
        );
        assert_eq!(
            datastar.get("data-attr:disabled").map(String::as_str),
            Some("$isLoading")
        );

        let built = ChatInputProps::default()
            .with_name("")
            .with_data_model("")
            .with_loading_signal("")
            .with_placeholder("Ask")
            .with_submit_button_text("Post")
            .with_stop_button_text("Cancel")
            .with_rows(0)
            .with_csrf_field_name("")
            .with_sticky_bottom(false)
            .with_size(shared::SIZE_LG)
            .with_class("custom")
            .with_min_height("80px")
            .with_show_loading_state(false);
        assert_eq!(built.get_name(), "message");
        assert_eq!(built.get_data_model(), "message");
        assert_eq!(built.get_loading_signal(), "isLoading");
        assert_eq!(built.get_placeholder(), "Ask");
        assert_eq!(built.get_submit_button_text(), "Post");
        assert_eq!(built.get_stop_button_text(), "Cancel");
        assert_eq!(built.get_rows(), 1);
        assert_eq!(built.get_csrf_field_name(), "_csrf");
        assert_eq!(built.size, shared::SIZE_LG);
        assert_eq!(
            built.build_container_classes(),
            "flex-shrink-0 bg-background border-t border-border px-3 py-2 custom"
        );
        assert_eq!(
            built.build_textarea_style(),
            "height: auto; min-height: 80px; max-height: 200px; overflow-y: auto;"
        );
        assert_eq!(
            built
                .build_datastar_textarea_attrs()
                .get("data-attr:disabled"),
            None
        );
    }

    #[test]
    fn submit_and_autoresize_helpers_cover_csrf_charcount_and_extra_js() {
        let props = ChatInputProps::default()
            .with_name("body")
            .with_data_model("draft")
            .with_loading_signal("busy")
            .with_form_action("/chat")
            .with_csrf_token("token")
            .with_show_char_count(true)
            .with_max_length(280)
            .with_on_submit_extra("window.afterSubmit()");
        let auto = buildAutoResizeAndCharCountAttrs(&props);
        assert_eq!(
            auto.get("data-on:input").map(String::as_str),
            Some(
                "evt.target.style.height = 'auto'; evt.target.style.height = Math.min(evt.target.scrollHeight, 200) + 'px'; var cc = document.getElementById('char-count-body'); if(cc) cc.textContent = evt.target.value.length + ' / 280'"
            )
        );
        assert_eq!(
            buildSubmitHandler(&props),
            "evt.preventDefault(); var inputVal = evt.target.elements.body.value.trim(); if(!inputVal) return; $busy = true; if(window.forceScrollToBottom) window.forceScrollToBottom(); @post('/chat', {contentType: 'form', headers: {'X-CSRF-Token': 'token'}}); evt.target.elements.body.value = ''; $draft = ''; var ta = evt.target.elements.body; if(ta) { ta.style.height = 'auto'; }; var cc = document.getElementById('char-count-body'); if(cc) cc.textContent = '0 / 280'; window.afterSubmit()"
        );
    }

    #[test]
    fn chat_input_renders_form_textarea_datastar_buttons_hint_and_count() {
        let rendered = ChatInput(
            &ChatInputProps::default()
                .with_name("body")
                .with_placeholder("<Ask>")
                .with_value("<Hello>")
                .with_form_id("chat-form")
                .with_form_action("/chat")
                .with_csrf_token("csrf")
                .with_submit_button_text("Send it")
                .with_stop_button_text("Stop it")
                .with_data_model("draft")
                .with_loading_signal("busy")
                .with_class("outer")
                .with_textarea_class("textarea")
                .with_textarea_attribute("data-testid", "textarea")
                .with_keyboard_hint("Enter to send")
                .with_max_length(100)
                .with_show_char_count(true)
                .with_attribute("data-testid", "form"),
        )
        .into_string();
        assert!(rendered.starts_with("<div class=\"flex-shrink-0 bg-background border-t border-border px-3 py-2 sticky bottom-0 outer\">"));
        assert!(rendered.contains("<form method=\"POST\" class=\"w-full\" id=\"chat-form\" data-chat-form=\"true\" data-on:submit=\"evt.preventDefault(); var inputVal = evt.target.elements.body.value.trim();"));
        assert!(rendered.contains("data-testid=\"form\""));
        assert!(rendered.contains("<input type=\"hidden\" name=\"_csrf\" value=\"csrf\">"));
        assert!(rendered.contains(
            "name=\"body\" placeholder=\"&lt;Ask&gt;\" aria-label=\"&lt;Ask&gt;\" rows=\"1\""
        ));
        assert!(rendered.contains("required maxlength=\"100\""));
        assert!(rendered.contains("data-on:keydown=\"if(evt.key === 'Enter' &amp;&amp; !evt.shiftKey) { evt.preventDefault(); if(evt.target.value.trim()) evt.target.form.requestSubmit(); }\""));
        assert!(rendered.contains("data-on:input=\"evt.target.style.height = 'auto';"));
        assert!(rendered.contains("document.getElementById('char-count-body')"));
        assert!(rendered.contains("evt.target.value.length + ' / 100'"));
        assert!(
            rendered.contains(
                "data-bind=\"draft\" data-attr:disabled=\"$busy\" data-testid=\"textarea\""
            )
        );
        assert!(rendered.contains("&lt;Hello&gt;</textarea>"));
        assert!(rendered.contains("data-show=\"!$busy\""));
        assert!(rendered.contains("aria-label=\"Send it\""));
        assert!(rendered.contains("data-show=\"$busy\""));
        assert!(rendered.contains("data-on:click=\"$busy = false\""));
        assert!(rendered.contains("aria-label=\"Stop it\""));
        assert!(rendered.contains(
            "<p class=\"text-[11px] text-muted-foreground\" aria-hidden=\"true\">Enter to send</p>"
        ));
        assert!(rendered.contains(
            "<p id=\"char-count-body\" class=\"text-[11px] text-muted-foreground\">0 / 100</p>"
        ));
    }

    #[test]
    fn chat_input_renders_no_loading_state_and_disabled_enter_variations() {
        let rendered = ChatInput(
            &ChatInputProps::default()
                .with_show_loading_state(false)
                .with_disable_enter_submit(true)
                .with_required(false)
                .with_max_length(0)
                .with_keyboard_hint("Hidden because disabled"),
        )
        .into_string();
        assert!(!rendered.contains("data-attr:disabled"));
        assert!(!rendered.contains("data-on:keydown"));
        assert!(!rendered.contains("required"));
        assert!(!rendered.contains("maxlength"));
        assert!(!rendered.contains("data-show=\"$isLoading\""));
        assert!(rendered.contains("<span></span>"));
        assert!(rendered.contains("aria-label=\"Send\""));
    }

    #[test]
    fn simple_chat_input_renders_minimal_layout_and_button() {
        assert_eq!(
            simpleChatInputContainerClass(&ChatInputProps::default()),
            "w-full"
        );
        assert_eq!(
            simpleChatInputContainerClass(&ChatInputProps::default().with_class("panel")),
            "panel"
        );
        let rendered = SimpleChatInput(
            &ChatInputProps::default()
                .with_name("query")
                .with_placeholder("Ask")
                .with_rows(3)
                .with_value("<draft>")
                .with_submit_button_text("Run")
                .with_class("panel"),
        )
        .into_string();
        assert!(rendered.starts_with(
            "<div class=\"panel\"><div class=\"space-y-3\"><div class=\"relative w-full\">"
        ));
        assert!(rendered.contains("name=\"query\" placeholder=\"Ask\" rows=\"3\""));
        assert!(rendered.contains("data-bind=\"message\" data-attr:disabled=\"$isLoading\""));
        assert!(rendered.contains("&lt;draft&gt;</textarea>"));
        assert!(rendered.contains("w-full"));
        assert!(rendered.contains(">Run</button>"));
    }
}
