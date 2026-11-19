#![allow(non_snake_case)]

use crate::components::{ComponentInfo, shared};
use maud::{Markup, html};
use std::collections::BTreeMap;

pub const COMPONENTS: &[ComponentInfo] = &[
    ComponentInfo { family: "unstyled", module: "form", name: "Fieldset" },
    ComponentInfo {
        family: "unstyled",
        module: "form",
        name: "Form",
    },
    ComponentInfo {
        family: "unstyled",
        module: "form",
        name: "FormWithCSRF",
    },
    ComponentInfo {
        family: "unstyled",
        module: "form",
        name: "DatastarForm",
    },
    ComponentInfo {
        family: "unstyled",
        module: "form",
        name: "FormDescription",
    },
    ComponentInfo {
        family: "unstyled",
        module: "form",
        name: "ProgressSection",
    },
];

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FormProps {
    pub action: String,
    pub method: String,
    pub datastar_post: String,
    pub datastar_get: String,
    pub datastar_put: String,
    pub datastar_delete: String,
    pub datastar_target: String,
    pub enc_type: String,
    pub no_validate: bool,
    pub autocomplete: bool,
    pub class: String,
    pub attributes: BTreeMap<String, String>,
    pub aria_labelled_by: String,
    pub aria_described_by: String,
}

impl Default for FormProps {
    fn default() -> Self {
        Self {
            action: String::new(),
            method: "POST".to_string(),
            datastar_post: String::new(),
            datastar_get: String::new(),
            datastar_put: String::new(),
            datastar_delete: String::new(),
            datastar_target: String::new(),
            enc_type: String::new(),
            no_validate: false,
            autocomplete: true,
            class: String::new(),
            attributes: BTreeMap::new(),
            aria_labelled_by: String::new(),
            aria_described_by: String::new(),
        }
    }
}

impl FormProps {
    #[must_use]
    pub fn with_action(mut self, action: impl Into<String>) -> Self {
        self.action = action.into();
        self
    }

    #[must_use]
    pub fn with_method(mut self, method: impl Into<String>) -> Self {
        self.method = method.into();
        self
    }

    #[must_use]
    pub fn with_datastar_post(mut self, url: impl Into<String>) -> Self {
        self.datastar_post = url.into();
        self
    }

    #[must_use]
    pub fn with_datastar_get(mut self, url: impl Into<String>) -> Self {
        self.datastar_get = url.into();
        self
    }

    #[must_use]
    pub fn with_datastar_put(mut self, url: impl Into<String>) -> Self {
        self.datastar_put = url.into();
        self
    }

    #[must_use]
    pub fn with_datastar_delete(mut self, url: impl Into<String>) -> Self {
        self.datastar_delete = url.into();
        self
    }

    #[must_use]
    pub fn with_datastar_target(mut self, target: impl Into<String>) -> Self {
        self.datastar_target = target.into();
        self
    }

    #[must_use]
    pub fn with_enc_type(mut self, enc_type: impl Into<String>) -> Self {
        self.enc_type = enc_type.into();
        self
    }

    #[must_use]
    pub const fn with_no_validate(mut self, no_validate: bool) -> Self {
        self.no_validate = no_validate;
        self
    }

    #[must_use]
    pub const fn with_autocomplete(mut self, autocomplete: bool) -> Self {
        self.autocomplete = autocomplete;
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
    pub fn with_aria_labelled_by(mut self, value: impl Into<String>) -> Self {
        self.aria_labelled_by = value.into();
        self
    }

    #[must_use]
    pub fn with_aria_described_by(mut self, value: impl Into<String>) -> Self {
        self.aria_described_by = value.into();
        self
    }

    #[must_use]
    pub fn build_classes(&self) -> String {
        shared::build_classes(&["form", self.class.as_str()])
    }

    #[must_use]
    pub fn build_attributes(&self) -> BTreeMap<String, String> {
        let mut attrs = self.attributes.clone();
        if !self.datastar_post.is_empty() {
            attrs.insert(
                "data-on:submit".to_string(),
                buildDatastarAction("post", &self.datastar_post, &self.datastar_target),
            );
        }
        if !self.datastar_get.is_empty() {
            attrs.insert(
                "data-on:submit".to_string(),
                buildDatastarAction("get", &self.datastar_get, &self.datastar_target),
            );
        }
        if !self.datastar_put.is_empty() {
            attrs.insert(
                "data-on:submit".to_string(),
                buildDatastarAction("put", &self.datastar_put, &self.datastar_target),
            );
        }
        if !self.datastar_delete.is_empty() {
            attrs.insert(
                "data-on:submit".to_string(),
                buildDatastarAction("delete", &self.datastar_delete, &self.datastar_target),
            );
        }
        if !self.enc_type.is_empty() {
            attrs.insert("enctype".to_string(), self.enc_type.clone());
        }
        if self.no_validate {
            attrs.insert("novalidate".to_string(), "true".to_string());
        }
        attrs.insert(
            "autocomplete".to_string(),
            if self.autocomplete { "on" } else { "off" }.to_string(),
        );
        if !self.aria_labelled_by.is_empty() {
            attrs.insert("aria-labelledby".to_string(), self.aria_labelled_by.clone());
        }
        if !self.aria_described_by.is_empty() {
            attrs.insert(
                "aria-describedby".to_string(),
                self.aria_described_by.clone(),
            );
        }
        attrs
    }
}

#[must_use]
pub fn buildDatastarAction(method: &str, url: &str, target: &str) -> String {
    if target.is_empty() {
        format!("@{method}('{url}', {{contentType: 'form'}})")
    } else {
        format!("@{method}('{url}', {{contentType: 'form', selector: '{target}'}})")
    }
}

fn render_form(props: &FormProps, csrf: Option<(&str, &str)>, children: Markup) -> Markup {
    let attrs = props.build_attributes();
    crate::html! {
        form
            action=[if props.action.is_empty() { None } else { Some(props.action.as_str()) }]
            method=(&props.method)
            class=(props.build_classes())
            novalidate[props.no_validate]
            autocomplete=(if props.autocomplete { "on" } else { "off" })
                ..(&attrs)
        {
            @if let Some((token, field_name)) = csrf {
                @if !token.is_empty() && !field_name.is_empty() {
                    input type="hidden" name=(field_name) value=(token);
                }
            }
            (children)
        }
    }
}

#[must_use]
pub fn Form(props: &FormProps, children: Markup) -> Markup {
    render_form(props, None, children)
}

#[must_use]
pub fn FormWithCSRF(
    props: &FormProps,
    csrf_token: &str,
    csrf_field_name: &str,
    children: Markup,
) -> Markup {
    render_form(props, Some((csrf_token, csrf_field_name)), children)
}

#[must_use]
pub fn DatastarForm(
    method: &str,
    url: &str,
    target: &str,
    class: &str,
    children: Markup,
) -> Markup {
    let props = match method {
        "POST" => FormProps::default()
            .with_method("POST")
            .with_datastar_post(url)
            .with_datastar_target(target)
            .with_class(class),
        "GET" => FormProps::default()
            .with_method("GET")
            .with_datastar_get(url)
            .with_datastar_target(target)
            .with_class(class),
        "PUT" => FormProps::default()
            .with_method("POST")
            .with_datastar_put(url)
            .with_datastar_target(target)
            .with_class(class),
        "DELETE" => FormProps::default()
            .with_method("POST")
            .with_datastar_delete(url)
            .with_datastar_target(target)
            .with_class(class),
        _ => return html! {},
    };
    Form(&props, children)
}

#[must_use]
pub fn FormDescription(id: &str, class: &str, children: Markup) -> Markup {
    html! {
        p id=[shared::optional_attr(id)]
          class=(format!("text-sm text-muted-foreground {class}")) {
            (children)
        }
    }
}

#[must_use]
pub fn getStepClass(step: i32, current: i32) -> String {
    let base = "h-2 flex-1 rounded-full transition-colors duration-150";
    if step <= current {
        format!("{base} bg-primary")
    } else {
        format!("{base} bg-muted")
    }
}

#[must_use]
pub fn formatInt(n: i32) -> String {
    if (0..10).contains(&n) {
        char::from(b'0' + u8::try_from(n).unwrap_or(0)).to_string()
    } else {
        n.to_string()
    }
}

#[must_use]
pub fn ProgressSection(current_step: i32, total_steps: i32, class: &str) -> Markup {
    html! {
        div class=(format!("flex items-center gap-2 mb-6 {class}")) {
            @for step in 1..=total_steps {
                div class=(getStepClass(step, current_step)) {}
            }
            span class="ml-2 text-sm text-muted-foreground" {
                (format!("Step {} of {}", current_step, total_steps))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn helpers_build_classes_attributes_actions_and_numbers() {
        assert_eq!(FormProps::default().build_classes(), "form");
        assert_eq!(
            FormProps::default().with_class("space-y-4").build_classes(),
            "form space-y-4"
        );
        assert_eq!(
            buildDatastarAction("post", "/save", ""),
            "@post('/save', {contentType: 'form'})"
        );
        assert_eq!(
            buildDatastarAction("get", "/search", "#results"),
            "@get('/search', {contentType: 'form', selector: '#results'})"
        );
        assert_eq!(
            getStepClass(1, 2),
            "h-2 flex-1 rounded-full transition-colors duration-150 bg-primary"
        );
        assert_eq!(
            getStepClass(3, 2),
            "h-2 flex-1 rounded-full transition-colors duration-150 bg-muted"
        );
        assert_eq!(formatInt(0), "0");
        assert_eq!(formatInt(9), "9");
        assert_eq!(formatInt(10), "10");
        assert_eq!(formatInt(-1), "-1");

        let attrs = FormProps::default()
            .with_datastar_post("/post")
            .with_datastar_get("/get")
            .with_datastar_put("/put")
            .with_datastar_delete("/delete")
            .with_datastar_target("#target")
            .with_enc_type("multipart/form-data")
            .with_no_validate(true)
            .with_autocomplete(false)
            .with_aria_labelled_by("title")
            .with_aria_described_by("desc")
            .with_attribute("data-testid", "form")
            .build_attributes();
        assert_eq!(
            attrs.get("data-on:submit").map(String::as_str),
            Some("@delete('/delete', {contentType: 'form', selector: '#target'})")
        );
        assert_eq!(
            attrs.get("enctype").map(String::as_str),
            Some("multipart/form-data")
        );
        assert_eq!(attrs.get("novalidate").map(String::as_str), Some("true"));
        assert_eq!(attrs.get("autocomplete").map(String::as_str), Some("off"));
        assert_eq!(
            attrs.get("aria-labelledby").map(String::as_str),
            Some("title")
        );
        assert_eq!(
            attrs.get("aria-describedby").map(String::as_str),
            Some("desc")
        );
        assert_eq!(attrs.get("data-testid").map(String::as_str), Some("form"));
    }

    #[test]
    fn form_renders_action_method_attrs_datastar_and_children() {
        let props = FormProps::default()
            .with_action("/save")
            .with_method("PATCH")
            .with_class("space-y-4")
            .with_datastar_post("/api/save")
            .with_datastar_target("#panel")
            .with_enc_type("multipart/form-data")
            .with_no_validate(true)
            .with_autocomplete(false)
            .with_aria_labelled_by("form-title")
            .with_aria_described_by("form-desc")
            .with_attribute("id", "profile-form")
            .with_attribute("data-testid", "profile");
        let rendered = Form(&props, html! { button type="submit" { "Save" } }).into_string();
        assert!(rendered.starts_with("<form "));
        assert!(rendered.contains("action=\"/save\""));
        assert!(rendered.contains("method=\"PATCH\""));
        assert!(rendered.contains("class=\"form space-y-4\""));
        assert!(rendered.contains(
            "data-on:submit=\"@post('/api/save', {contentType: 'form', selector: '#panel'})\""
        ));
        assert!(rendered.contains("enctype=\"multipart/form-data\""));
        assert!(rendered.contains("novalidate"));
        assert!(rendered.contains("autocomplete=\"off\""));
        assert!(rendered.contains("aria-labelledby=\"form-title\""));
        assert!(rendered.contains("aria-describedby=\"form-desc\""));
        assert!(rendered.contains("id=\"profile-form\""));
        assert!(rendered.contains("data-testid=\"profile\""));
        assert!(rendered.contains("<button type=\"submit\">Save</button>"));
    }

    #[test]
    fn csrf_datastar_description_and_progress_variations_render_html() {
        let props = FormProps::default().with_action("/secure");
        let with_csrf = FormWithCSRF(
            &props,
            "<token>",
            "csrf",
            html! { input type="text" name="name"; },
        )
        .into_string();
        assert!(
            with_csrf.contains("<input type=\"hidden\" name=\"csrf\" value=\"&lt;token&gt;\">")
        );
        assert!(with_csrf.contains("<input type=\"text\" name=\"name\">"));
        let without_csrf =
            FormWithCSRF(&props, "", "csrf", html! { span { "Body" } }).into_string();
        assert!(!without_csrf.contains("type=\"hidden\""));
        assert!(without_csrf.contains("<span>Body</span>"));

        let post = DatastarForm("POST", "/save", "", "compact", html! { "Body" }).into_string();
        assert!(post.contains("method=\"POST\""));
        assert!(post.contains("class=\"form compact\""));
        assert!(post.contains("data-on:submit=\"@post('/save', {contentType: 'form'})\""));
        let get = DatastarForm("GET", "/find", "#out", "", html! {}).into_string();
        assert!(get.contains("method=\"GET\""));
        assert!(
            get.contains(
                "data-on:submit=\"@get('/find', {contentType: 'form', selector: '#out'})\""
            )
        );
        let put = DatastarForm("PUT", "/item", "", "", html! {}).into_string();
        assert!(put.contains("method=\"POST\""));
        assert!(put.contains("data-on:submit=\"@put('/item', {contentType: 'form'})\""));
        let delete = DatastarForm("DELETE", "/item", "", "", html! {}).into_string();
        assert!(delete.contains("data-on:submit=\"@delete('/item', {contentType: 'form'})\""));
        assert_eq!(
            DatastarForm("PATCH", "/item", "", "", html! {}).into_string(),
            ""
        );

        let description = FormDescription("help", "extra", html! { "<Help>" }).into_string();
        assert_eq!(
            description,
            "<p id=\"help\" class=\"text-sm text-muted-foreground extra\">&lt;Help&gt;</p>"
        );
        let progress = ProgressSection(2, 4, "mt-2").into_string();
        assert!(progress.starts_with("<div class=\"flex items-center gap-2 mb-6 mt-2\">"));
        assert_eq!(progress.matches("bg-primary").count(), 2);
        assert_eq!(progress.matches("bg-muted").count(), 2);
        assert!(
            progress
                .contains("<span class=\"ml-2 text-sm text-muted-foreground\">Step 2 of 4</span>")
        );
    }
}

/// Field grouping with caller attributes retained by the shared Maud renderer.
#[derive(Clone,Debug,Default)]
pub struct FieldsetProps{pub disabled:bool,pub aria_invalid:String,pub attributes:BTreeMap<String,String>}
pub fn Fieldset(props:&FieldsetProps,children:Markup)->Markup{
    crate::html!{fieldset disabled[props.disabled] aria-invalid=[shared::optional_attr(&props.aria_invalid)] ..(&props.attributes) {(children)}}
}
