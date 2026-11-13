#![allow(non_snake_case)]

use crate::components::{ComponentInfo, shared};
use maud::{Markup, html};
use std::collections::BTreeMap;

pub const COMPONENTS: &[ComponentInfo] = &[
    ComponentInfo {
        family: "unstyled",
        module: "image",
        name: "Image",
    },
    ComponentInfo {
        family: "unstyled",
        module: "image",
        name: "ImageGallery",
    },
];

pub const OBJECT_FIT_COVER: &str = "cover";
pub const OBJECT_FIT_CONTAIN: &str = "contain";
pub const OBJECT_FIT_FILL: &str = "fill";
pub const OBJECT_FIT_NONE: &str = "none";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ImageProps {
    pub src: String,
    pub alt: String,
    pub width: String,
    pub height: String,
    pub loading: String,
    pub object_fit: &'static str,
    pub aspect_ratio: &'static str,
    pub fallback: String,
    pub class: String,
    pub attributes: BTreeMap<String, String>,
}

impl Default for ImageProps {
    fn default() -> Self {
        Self {
            src: String::new(),
            alt: String::new(),
            width: String::new(),
            height: String::new(),
            loading: "lazy".to_string(),
            object_fit: OBJECT_FIT_COVER,
            aspect_ratio: "",
            fallback: String::new(),
            class: String::new(),
            attributes: BTreeMap::new(),
        }
    }
}

impl ImageProps {
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
    pub fn with_width(mut self, width: impl Into<String>) -> Self {
        self.width = width.into();
        self
    }

    #[must_use]
    pub fn with_height(mut self, height: impl Into<String>) -> Self {
        self.height = height.into();
        self
    }

    #[must_use]
    pub fn with_loading(mut self, loading: impl Into<String>) -> Self {
        self.loading = loading.into();
        self
    }

    #[must_use]
    pub const fn with_object_fit(mut self, fit: &'static str) -> Self {
        self.object_fit = fit;
        self
    }

    #[must_use]
    pub const fn with_aspect_ratio(mut self, ratio: &'static str) -> Self {
        self.aspect_ratio = ratio;
        self
    }

    #[must_use]
    pub fn with_fallback(mut self, fallback: impl Into<String>) -> Self {
        self.fallback = fallback.into();
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
    pub fn object_fit_class(&self) -> &'static str {
        match self.object_fit {
            OBJECT_FIT_COVER => "object-cover",
            OBJECT_FIT_CONTAIN => "object-contain",
            OBJECT_FIT_FILL => "object-fill",
            OBJECT_FIT_NONE => "object-none",
            _ => "object-cover",
        }
    }

    #[must_use]
    pub fn aspect_ratio_class(&self) -> &'static str {
        match self.aspect_ratio {
            shared::ASPECT_RATIO_16X9 => "aspect-video",
            shared::ASPECT_RATIO_4X3 => "aspect-[4/3]",
            shared::ASPECT_RATIO_1X1 => "aspect-square",
            shared::ASPECT_RATIO_9X16 => "aspect-[9/16]",
            _ => "",
        }
    }

    #[must_use]
    pub fn build_classes(&self) -> String {
        shared::build_classes(&[
            "max-w-full",
            self.object_fit_class(),
            self.aspect_ratio_class(),
            self.class.as_str(),
        ])
    }

    #[must_use]
    pub fn build_attributes(&self) -> BTreeMap<String, String> {
        let mut attrs = self.attributes.clone();
        attrs.insert("src".to_string(), self.src.clone());
        attrs.insert("alt".to_string(), self.alt.clone());
        if !self.loading.is_empty() {
            attrs.insert("loading".to_string(), self.loading.clone());
        }
        if !self.width.is_empty() {
            attrs.insert("width".to_string(), self.width.clone());
        }
        if !self.height.is_empty() {
            attrs.insert("height".to_string(), self.height.clone());
        }
        attrs
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GalleryImage {
    pub src: String,
    pub alt: String,
    pub caption: String,
}

impl GalleryImage {
    #[must_use]
    pub fn new(src: impl Into<String>, alt: impl Into<String>, caption: impl Into<String>) -> Self {
        Self {
            src: src.into(),
            alt: alt.into(),
            caption: caption.into(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ImageGalleryProps {
    pub images: Vec<GalleryImage>,
    pub cols: i32,
    pub gap: i32,
    pub lightbox: bool,
    pub class: String,
    pub attributes: BTreeMap<String, String>,
}

impl Default for ImageGalleryProps {
    fn default() -> Self {
        Self {
            images: Vec::new(),
            cols: 3,
            gap: 4,
            lightbox: false,
            class: String::new(),
            attributes: BTreeMap::new(),
        }
    }
}

impl ImageGalleryProps {
    #[must_use]
    pub fn with_images(mut self, images: Vec<GalleryImage>) -> Self {
        self.images = images;
        self
    }

    #[must_use]
    pub const fn with_cols(mut self, cols: i32) -> Self {
        self.cols = cols;
        self
    }

    #[must_use]
    pub const fn with_gap(mut self, gap: i32) -> Self {
        self.gap = gap;
        self
    }

    #[must_use]
    pub const fn with_lightbox(mut self, lightbox: bool) -> Self {
        self.lightbox = lightbox;
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
    pub fn build_classes(&self) -> String {
        shared::build_classes(&["grid", self.class.as_str()])
    }

    #[must_use]
    pub fn build_attributes(&self) -> BTreeMap<String, String> {
        self.attributes.clone()
    }

    #[must_use]
    pub fn style(&self) -> String {
        format!(
            "grid-template-columns: repeat({}, minmax(0, 1fr)); gap: {}rem;",
            self.cols, self.gap
        )
    }
}

#[must_use]
pub fn Image(props: &ImageProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        img
            class=(props.build_classes())
            src=[shared::attr_value(&attrs, "src")]
            alt=[shared::attr_value(&attrs, "alt")]
            loading=[shared::attr_value(&attrs, "loading")]
            width=[shared::attr_value(&attrs, "width")]
            height=[shared::attr_value(&attrs, "height")]
            id=[shared::attr_value(&attrs, "id")]
            title=[shared::attr_value(&attrs, "title")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
        {}
    }
}

#[must_use]
pub fn ImageGallery(props: &ImageGalleryProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        div
            class=(props.build_classes())
            style=(props.style())
            id=[shared::attr_value(&attrs, "id")]
            title=[shared::attr_value(&attrs, "title")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
        {
            @for img in &props.images {
                figure class="relative group" {
                    (Image(&ImageProps::default().with_src(&img.src).with_alt(&img.alt).with_class("rounded-lg")))
                    @if !img.caption.is_empty() {
                        figcaption class="mt-2 text-sm text-muted-foreground text-center" {
                            (&img.caption)
                        }
                    }
                    @if props.lightbox {
                        button
                            type="button"
                            class="absolute inset-0 bg-foreground/0 group-hover:bg-foreground/20 transition-colors duration-150 rounded-lg flex items-center justify-center"
                            aria-label=(shared::default_strings().image_view_full_size)
                        {
                            svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" class="opacity-0 group-hover:opacity-100 text-background transition-opacity" {
                                path d="M15 3h6v6" {}
                                path d="M9 21H3v-6" {}
                                path d="M21 3l-7 7" {}
                                path d="M3 21l7-7" {}
                            }
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn image_default_props_match_go_defaults() {
        let props = ImageProps::default();
        assert_eq!(props.loading, "lazy");
        assert_eq!(props.object_fit, OBJECT_FIT_COVER);
        assert!(props.aspect_ratio.is_empty());
    }

    #[test]
    fn image_helpers_cover_fit_ratio_classes_and_attributes() {
        for (fit, expected) in [
            (OBJECT_FIT_COVER, "object-cover"),
            (OBJECT_FIT_CONTAIN, "object-contain"),
            (OBJECT_FIT_FILL, "object-fill"),
            (OBJECT_FIT_NONE, "object-none"),
            ("unknown", "object-cover"),
        ] {
            assert_eq!(
                ImageProps::default()
                    .with_object_fit(fit)
                    .object_fit_class(),
                expected
            );
        }

        for (ratio, expected) in [
            (shared::ASPECT_RATIO_16X9, "aspect-video"),
            (shared::ASPECT_RATIO_4X3, "aspect-[4/3]"),
            (shared::ASPECT_RATIO_1X1, "aspect-square"),
            (shared::ASPECT_RATIO_9X16, "aspect-[9/16]"),
            ("", ""),
        ] {
            assert_eq!(
                ImageProps::default()
                    .with_aspect_ratio(ratio)
                    .aspect_ratio_class(),
                expected
            );
        }

        let props = ImageProps::default()
            .with_src("/x.png")
            .with_alt("X")
            .with_width("320")
            .with_height("180")
            .with_loading("eager")
            .with_aspect_ratio(shared::ASPECT_RATIO_16X9)
            .with_fallback("/fallback.png")
            .with_class("extra")
            .with_attribute("title", "Image");
        assert_eq!(props.fallback, "/fallback.png");
        assert_eq!(
            props.build_classes(),
            "max-w-full object-cover aspect-video extra"
        );
        let attrs = props.build_attributes();
        assert_eq!(attrs.get("src").map(String::as_str), Some("/x.png"));
        assert_eq!(attrs.get("alt").map(String::as_str), Some("X"));
        assert_eq!(attrs.get("loading").map(String::as_str), Some("eager"));
        assert_eq!(attrs.get("width").map(String::as_str), Some("320"));
        assert_eq!(attrs.get("height").map(String::as_str), Some("180"));
        assert_eq!(attrs.get("title").map(String::as_str), Some("Image"));
    }

    #[test]
    fn image_renders_html_with_escaped_attributes_and_optional_dimensions() {
        let html = Image(
            &ImageProps::default()
                .with_src("/photo?<x>")
                .with_alt("<Alt>")
                .with_width("100")
                .with_attribute("data-testid", "hero"),
        )
        .into_string();

        assert!(html.starts_with("<img "));
        assert!(html.contains("class=\"max-w-full object-cover\""));
        assert!(html.contains("src=\"/photo?&lt;x&gt;\""));
        assert!(html.contains("alt=\"&lt;Alt&gt;\""));
        assert!(html.contains("loading=\"lazy\""));
        assert!(html.contains("width=\"100\""));
        assert!(!html.contains("height="));
        assert!(html.contains("data-testid=\"hero\""));
    }

    #[test]
    fn gallery_default_props_and_helpers_match_go_defaults() {
        let props = ImageGalleryProps::default();
        assert_eq!(props.cols, 3);
        assert_eq!(props.gap, 4);
        assert_eq!(props.build_classes(), "grid");
        assert_eq!(
            props.style(),
            "grid-template-columns: repeat(3, minmax(0, 1fr)); gap: 4rem;"
        );

        let customized = props
            .with_cols(2)
            .with_gap(1)
            .with_class("extra")
            .with_attribute("title", "Gallery");
        assert_eq!(customized.build_classes(), "grid extra");
        assert_eq!(
            customized.style(),
            "grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 1rem;"
        );
        assert_eq!(
            customized
                .build_attributes()
                .get("title")
                .map(String::as_str),
            Some("Gallery")
        );
    }

    #[test]
    fn gallery_renders_images_captions_and_lightbox_variations() {
        let images = vec![
            GalleryImage::new("/a.png", "A", "Caption <A>"),
            GalleryImage::new("/b.png", "B", ""),
        ];
        let html = ImageGallery(
            &ImageGalleryProps::default()
                .with_images(images)
                .with_lightbox(true)
                .with_attribute("data-testid", "gallery"),
        )
        .into_string();

        assert!(
            html.contains("style=\"grid-template-columns: repeat(3, minmax(0, 1fr)); gap: 4rem;\"")
        );
        assert!(html.contains("data-testid=\"gallery\""));
        assert_eq!(html.matches("<figure class=\"relative group\">").count(), 2);
        assert!(html.contains("src=\"/a.png\""));
        assert!(html.contains("Caption &lt;A&gt;"));
        assert!(html.contains("aria-label=\"View full size\""));
        assert_eq!(html.matches("class=\"absolute inset-0").count(), 2);

        let no_lightbox = ImageGallery(
            &ImageGalleryProps::default().with_images(vec![GalleryImage::new("/c.png", "C", "")]),
        )
        .into_string();
        assert!(!no_lightbox.contains("View full size"));
    }
}
