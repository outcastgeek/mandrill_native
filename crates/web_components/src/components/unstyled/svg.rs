#![allow(non_snake_case)]

use crate::components::{ComponentInfo, shared};
use maud::{Markup, html};
use std::collections::BTreeMap;

pub const COMPONENTS: &[ComponentInfo] = &[
    ComponentInfo {
        family: "unstyled",
        module: "svg",
        name: "SVG",
    },
    ComponentInfo {
        family: "unstyled",
        module: "svg",
        name: "SVGWidget",
    },
];

pub const LOADING_AUTO: &str = "";
pub const LOADING_LAZY: &str = "lazy";
pub const LOADING_EAGER: &str = "eager";

pub const DECODING_AUTO: &str = "auto";
pub const DECODING_ASYNC: &str = "async";
pub const DECODING_SYNC: &str = "sync";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SVGProps {
    pub src: String,
    pub alt: String,
    pub title: String,
    pub decorative: bool,
    pub width: String,
    pub height: String,
    pub loading: &'static str,
    pub decoding: &'static str,
    pub fetch_priority: String,
    pub container_class: String,
    pub class: String,
    pub container_attributes: BTreeMap<String, String>,
    pub attributes: BTreeMap<String, String>,
}

impl Default for SVGProps {
    fn default() -> Self {
        Self {
            src: String::new(),
            alt: String::new(),
            title: String::new(),
            decorative: false,
            width: String::new(),
            height: String::new(),
            loading: LOADING_LAZY,
            decoding: DECODING_ASYNC,
            fetch_priority: String::new(),
            container_class: String::new(),
            class: String::new(),
            container_attributes: BTreeMap::new(),
            attributes: BTreeMap::new(),
        }
    }
}

impl SVGProps {
    #[must_use]
    pub fn with_src(mut self, src: impl Into<String>) -> Self {
        self.src = src.into();
        self
    }

    #[must_use]
    pub fn with_alt(mut self, alt: impl Into<String>) -> Self {
        self.alt = alt.into();
        self
    }

    #[must_use]
    pub fn with_title(mut self, title: impl Into<String>) -> Self {
        self.title = title.into();
        self
    }

    #[must_use]
    pub const fn with_decorative(mut self, decorative: bool) -> Self {
        self.decorative = decorative;
        self
    }

    #[must_use]
    pub fn with_dimensions(mut self, width: impl Into<String>, height: impl Into<String>) -> Self {
        self.width = width.into();
        self.height = height.into();
        self
    }

    #[must_use]
    pub const fn with_loading(mut self, loading: &'static str) -> Self {
        self.loading = loading;
        self
    }

    #[must_use]
    pub const fn with_decoding(mut self, decoding: &'static str) -> Self {
        self.decoding = decoding;
        self
    }

    #[must_use]
    pub fn with_fetch_priority(mut self, fetch_priority: impl Into<String>) -> Self {
        self.fetch_priority = fetch_priority.into();
        self
    }

    #[must_use]
    pub fn with_container_class(mut self, class: impl Into<String>) -> Self {
        self.container_class = class.into();
        self
    }

    #[must_use]
    pub fn with_class(mut self, class: impl Into<String>) -> Self {
        self.class = class.into();
        self
    }

    #[must_use]
    pub fn with_container_attribute(
        mut self,
        name: impl Into<String>,
        value: impl Into<String>,
    ) -> Self {
        self.container_attributes.insert(name.into(), value.into());
        self
    }

    #[must_use]
    pub fn with_attribute(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.attributes.insert(name.into(), value.into());
        self
    }

    #[must_use]
    pub fn should_render(&self) -> bool {
        !self.src.trim().is_empty()
    }

    #[must_use]
    pub fn resolved_src(&self) -> String {
        self.src.trim().to_string()
    }

    #[must_use]
    pub fn resolved_alt(&self) -> String {
        if self.decorative {
            String::new()
        } else if !self.alt.trim().is_empty() {
            self.alt.trim().to_string()
        } else {
            self.title.trim().to_string()
        }
    }

    #[must_use]
    pub fn resolved_loading(&self) -> &'static str {
        if self.loading.is_empty() {
            LOADING_LAZY
        } else {
            self.loading
        }
    }

    #[must_use]
    pub fn resolved_decoding(&self) -> &'static str {
        if self.decoding.is_empty() {
            DECODING_ASYNC
        } else {
            self.decoding
        }
    }

    #[must_use]
    pub fn build_container_classes(&self) -> String {
        shared::build_classes(&[
            "inline-flex shrink-0 items-center justify-center overflow-hidden",
            self.container_class.as_str(),
        ])
    }

    #[must_use]
    pub fn build_image_classes(&self) -> String {
        shared::build_classes(&["block shrink-0 object-contain", self.class.as_str()])
    }

    #[must_use]
    pub fn build_container_attributes(&self) -> BTreeMap<String, String> {
        let mut attrs = self.container_attributes.clone();
        if self.decorative {
            attrs.insert("aria-hidden".to_string(), "true".to_string());
        }
        attrs
    }

    #[must_use]
    pub fn build_image_attributes(&self) -> BTreeMap<String, String> {
        let mut attrs = self.attributes.clone();
        attrs.insert("src".to_string(), self.resolved_src());
        attrs.insert("alt".to_string(), self.resolved_alt());
        if !self.width.is_empty() {
            attrs.insert("width".to_string(), self.width.clone());
        }
        if !self.height.is_empty() {
            attrs.insert("height".to_string(), self.height.clone());
        }
        let loading = self.resolved_loading();
        attrs.insert("loading".to_string(), loading.to_string());
        let decoding = self.resolved_decoding();
        attrs.insert("decoding".to_string(), decoding.to_string());
        if !self.fetch_priority.trim().is_empty() {
            attrs.insert(
                "fetchpriority".to_string(),
                self.fetch_priority.trim().to_string(),
            );
        }
        if !self.title.trim().is_empty() && !self.decorative {
            attrs.insert("title".to_string(), self.title.trim().to_string());
        }
        if self.decorative {
            attrs.insert("aria-hidden".to_string(), "true".to_string());
        }
        attrs
    }
}

#[must_use]
pub fn DefaultSVGProps(src: impl Into<String>) -> SVGProps {
    SVGProps::default().with_src(src)
}

#[must_use]
pub fn DecorativeSVGProps(src: impl Into<String>) -> SVGProps {
    DefaultSVGProps(src).with_decorative(true)
}

#[must_use]
fn attr<'a>(attrs: &'a BTreeMap<String, String>, name: &str) -> Option<&'a str> {
    shared::attr_value(attrs, name)
}

#[must_use]
pub fn SVG(props: &SVGProps) -> Markup {
    if !props.should_render() {
        return html! {};
    }
    let container_attrs = props.build_container_attributes();
    let image_attrs = props.build_image_attributes();
    html! {
        span
            class=(props.build_container_classes())
            aria-hidden=[attr(&container_attrs, "aria-hidden")]
            id=[attr(&container_attrs, "id")]
            data-testid=[attr(&container_attrs, "data-testid")]
        {
            img
                class=(props.build_image_classes())
                src=[attr(&image_attrs, "src")]
                alt=(props.resolved_alt())
                title=[attr(&image_attrs, "title")]
                width=[attr(&image_attrs, "width")]
                height=[attr(&image_attrs, "height")]
                loading=[attr(&image_attrs, "loading")]
                decoding=[attr(&image_attrs, "decoding")]
                fetchpriority=[attr(&image_attrs, "fetchpriority")]
                aria-hidden=[attr(&image_attrs, "aria-hidden")]
                id=[attr(&image_attrs, "id")]
                data-testid=[attr(&image_attrs, "data-testid")]
                data-icon=[attr(&image_attrs, "data-icon")]
            {}
        }
    }
}

#[must_use]
pub fn SVGWidget(props: &SVGProps) -> Markup {
    SVG(props)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_contains(html: &str, expected: &str) {
        assert!(html.contains(expected), "missing `{expected}` in `{html}`");
    }

    #[test]
    fn svg_helpers_cover_defaults_decorative_accessible_and_attributes() {
        let decorative = DecorativeSVGProps(" /static/img/app.svg ");
        assert!(decorative.should_render());
        assert_eq!(decorative.resolved_src(), "/static/img/app.svg");
        assert_eq!(decorative.resolved_alt(), "");
        assert_eq!(decorative.resolved_loading(), "lazy");
        assert_eq!(decorative.resolved_decoding(), "async");
        assert_contains(&decorative.build_container_classes(), "inline-flex");
        assert_contains(&decorative.build_image_classes(), "object-contain");
        assert_eq!(
            decorative
                .build_container_attributes()
                .get("aria-hidden")
                .map(String::as_str),
            Some("true")
        );
        assert_eq!(
            decorative
                .build_image_attributes()
                .get("aria-hidden")
                .map(String::as_str),
            Some("true")
        );

        let accessible = DefaultSVGProps("/icon.svg")
            .with_title("App icon")
            .with_alt("Ubuntu TechHive")
            .with_dimensions("32", "32")
            .with_loading(LOADING_EAGER)
            .with_decoding(DECODING_SYNC)
            .with_fetch_priority(" high ")
            .with_container_class("h-8 w-8")
            .with_class("dark:invert")
            .with_container_attribute("id", "shell")
            .with_attribute("data-icon", "brand");
        assert_eq!(accessible.resolved_alt(), "Ubuntu TechHive");
        assert_eq!(accessible.resolved_loading(), "eager");
        assert_eq!(accessible.resolved_decoding(), "sync");
        assert_eq!(
            accessible
                .build_image_attributes()
                .get("fetchpriority")
                .map(String::as_str),
            Some("high")
        );

        let title_alt = DefaultSVGProps("/icon.svg").with_title("Only title");
        assert_eq!(title_alt.resolved_alt(), "Only title");
        let empty_modes = DefaultSVGProps("/icon.svg")
            .with_loading(LOADING_AUTO)
            .with_decoding(LOADING_AUTO);
        assert_eq!(
            empty_modes
                .build_image_attributes()
                .get("loading")
                .map(String::as_str),
            Some("lazy")
        );
        assert_eq!(
            empty_modes
                .build_image_attributes()
                .get("decoding")
                .map(String::as_str),
            Some("async")
        );
        assert_eq!(
            DefaultSVGProps("/icon.svg")
                .with_decoding(DECODING_AUTO)
                .build_image_attributes()
                .get("decoding")
                .map(String::as_str),
            Some("auto")
        );
        assert!(!DefaultSVGProps(" ").should_render());
    }

    #[test]
    fn svg_renders_decorative_accessible_alias_and_empty_variations() {
        let decorative = SVG(&DecorativeSVGProps("/static/img/app.svg")).into_string();
        assert_contains(&decorative, "<span");
        assert_contains(&decorative, r#"aria-hidden="true""#);
        assert_contains(&decorative, "<img");
        assert_contains(&decorative, r#"src="/static/img/app.svg""#);
        assert_contains(&decorative, r#"alt="""#);
        assert_contains(&decorative, r#"loading="lazy""#);
        assert_contains(&decorative, r#"decoding="async""#);
        assert_contains(&decorative, "object-contain");

        let accessible = DefaultSVGProps("/static/img/app.svg")
            .with_alt("Ubuntu TechHive")
            .with_title("App icon")
            .with_dimensions("32", "32")
            .with_loading(LOADING_EAGER)
            .with_fetch_priority("high")
            .with_container_class("h-8 w-8 bg-muted")
            .with_class("dark:invert")
            .with_container_attribute("data-testid", "svg-shell")
            .with_attribute("data-icon", "brand");
        let html = SVGWidget(&accessible).into_string();
        assert_contains(&html, r#"data-testid="svg-shell""#);
        assert_contains(&html, "h-8 w-8 bg-muted");
        assert_contains(&html, r#"alt="Ubuntu TechHive""#);
        assert_contains(&html, r#"title="App icon""#);
        assert_contains(&html, r#"width="32""#);
        assert_contains(&html, r#"height="32""#);
        assert_contains(&html, r#"loading="eager""#);
        assert_contains(&html, r#"fetchpriority="high""#);
        assert_contains(&html, r#"data-icon="brand""#);
        assert_contains(&html, "dark:invert");
        assert!(!html.contains(r#"aria-hidden="true""#));

        assert_eq!(SVG(&DefaultSVGProps(" ")).into_string(), "");
    }
}
