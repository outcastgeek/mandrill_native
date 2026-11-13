#![allow(non_snake_case)]

use crate::components::{ComponentInfo, shared};
use maud::{Markup, html};
use std::collections::BTreeMap;

pub const COMPONENTS: &[ComponentInfo] = &[
    ComponentInfo {
        family: "unstyled",
        module: "copybutton",
        name: "CopyButtonStyles",
    },
    ComponentInfo {
        family: "unstyled",
        module: "copybutton",
        name: "CopyButton",
    },
];

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CopyButtonProps {
    pub target_id: String,
    pub aria_label: String,
    pub class: String,
    pub attributes: BTreeMap<String, String>,
}

impl Default for CopyButtonProps {
    fn default() -> Self {
        Self {
            target_id: String::new(),
            aria_label: String::new(),
            class: String::new(),
            attributes: BTreeMap::new(),
        }
    }
}

impl CopyButtonProps {
    #[must_use]
    pub fn with_target_id(mut self, target_id: impl Into<String>) -> Self {
        self.target_id = target_id.into();
        self
    }

    #[must_use]
    pub fn with_aria_label(mut self, aria_label: impl Into<String>) -> Self {
        self.aria_label = aria_label.into();
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
    pub fn get_aria_label(&self) -> &str {
        if self.aria_label.is_empty() {
            shared::default_strings().code_copy_aria_label
        } else {
            self.aria_label.as_str()
        }
    }

    #[must_use]
    pub fn get_copy_on_click_expr(&self) -> String {
        if self.target_id.is_empty() {
            return String::new();
        }

        let sanitized = self.target_id.replace('\'', "\\'");
        format!(
            "navigator.clipboard.writeText(document.getElementById('{sanitized}').innerText).then(()=>{{evt.target.closest('[data-copied]').dataset.copied='true';setTimeout(()=>evt.target.closest('[data-copied]').dataset.copied='false',2000)}})"
        )
    }

    #[must_use]
    pub fn build_classes(&self) -> String {
        shared::build_classes(&[
            "absolute top-1.5 right-1.5 w-6 h-6 flex items-center justify-center rounded-md opacity-40 hover:opacity-100 transition-all duration-150 focus-visible:opacity-100 focus-visible:outline-none focus-visible:ring-1",
            self.class.as_str(),
        ])
    }

    #[must_use]
    pub fn build_attributes(&self) -> BTreeMap<String, String> {
        self.attributes.clone()
    }
}

#[must_use]
pub fn CopyButtonStyles() -> Markup {
    html! {
        style id="copybutton-styles" {
            "[data-copied=false] .icon-copied, [data-copied=true] .icon-clipboard { display: none; }"
        }
    }
}

#[must_use]
pub fn CopyButton(props: &CopyButtonProps) -> Markup {
    if props.target_id.is_empty() {
        return html! {};
    }

    let attrs = props.build_attributes();
    html! {
        button
            type="button"
            data-copied="false"
            data-on:click=(props.get_copy_on_click_expr())
            aria-label=(props.get_aria_label())
            class=(props.build_classes())
            id=[shared::attr_value(&attrs, "id")]
            title=[shared::attr_value(&attrs, "title")]
            data-show=[shared::attr_value(&attrs, "data-show")]
            data-bind=[shared::attr_value(&attrs, "data-bind")]
            data-class:active=[shared::attr_value(&attrs, "data-class:active")]
        {
            svg class="icon-clipboard w-3.5 h-3.5" xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" {
                rect width="8" height="4" x="8" y="2" rx="1" ry="1" {}
                path d="M16 4h2a2 2 0 0 1 2 2v14a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2V6a2 2 0 0 1 2-2h2" {}
            }
            svg class="icon-copied w-3.5 h-3.5 text-success" xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" {
                path d="M20 6 9 17l-5-5" {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_props_match_go_defaults() {
        let props = CopyButtonProps::default();

        assert!(props.target_id.is_empty());
        assert!(props.aria_label.is_empty());
        assert!(props.class.is_empty());
        assert!(props.attributes.is_empty());
        assert_eq!(props.get_aria_label(), "Copy code");
        assert!(props.get_copy_on_click_expr().is_empty());
    }

    #[test]
    fn helper_methods_cover_custom_label_class_attributes_and_target_sanitizing() {
        let props = CopyButtonProps::default()
            .with_target_id("quote'id")
            .with_aria_label("Copy message")
            .with_class("text-info hover:bg-info-muted")
            .with_attribute("title", "\"Copy\" & go")
            .with_attribute("data-show", "$ready");

        assert_eq!(props.get_aria_label(), "Copy message");
        assert!(props.build_classes().contains("absolute top-1.5 right-1.5"));
        assert!(
            props
                .build_classes()
                .contains("text-info hover:bg-info-muted")
        );
        assert_eq!(
            props.build_attributes().get("title"),
            Some(&"\"Copy\" & go".to_string())
        );
        assert!(
            props
                .get_copy_on_click_expr()
                .contains("document.getElementById('quote\\'id')")
        );
        assert!(
            props
                .get_copy_on_click_expr()
                .contains("evt.target.closest('[data-copied]').dataset.copied='true'")
        );
    }

    #[test]
    fn styles_render_actual_css_toggle_selectors() {
        let html = CopyButtonStyles().into_string();

        assert_eq!(
            html,
            r#"<style id="copybutton-styles">[data-copied=false] .icon-copied, [data-copied=true] .icon-clipboard { display: none; }</style>"#
        );
    }

    #[test]
    fn copy_button_renders_nothing_without_target_id() {
        assert_eq!(CopyButton(&CopyButtonProps::default()).into_string(), "");
    }

    #[test]
    fn copy_button_renders_datastar_attributes_svg_icons_and_escaped_values() {
        let html = CopyButton(
            &CopyButtonProps::default()
                .with_target_id("message-body")
                .with_class("text-muted")
                .with_attribute("title", "\"Copy\" & paste")
                .with_attribute("data-show", "$ready"),
        )
        .into_string();

        assert!(html.starts_with(r#"<button type="button" data-copied="false" data-on:click=""#));
        assert!(html.contains("document.getElementById('message-body').innerText"));
        assert!(html.contains(r#"aria-label="Copy code""#));
        assert!(html.contains("absolute top-1.5 right-1.5"));
        assert!(html.contains("text-muted"));
        assert!(html.contains(r#"title="&quot;Copy&quot; &amp; paste""#));
        assert!(html.contains(r#"data-show="$ready""#));
        assert!(html.contains(r#"<svg class="icon-clipboard w-3.5 h-3.5""#));
        assert!(html.contains(r#"<rect width="8" height="4" x="8" y="2" rx="1" ry="1"></rect>"#));
        assert!(html.contains(r#"<svg class="icon-copied w-3.5 h-3.5 text-success""#));
        assert!(html.contains(r#"<path d="M20 6 9 17l-5-5"></path>"#));
        assert!(!html.contains("data-on-click"));
    }
}
