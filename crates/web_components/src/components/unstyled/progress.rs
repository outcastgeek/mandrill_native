#![allow(non_snake_case)]

use crate::components::{ComponentInfo, shared};
use maud::{Markup, html};
use std::collections::BTreeMap;

pub const COMPONENTS: &[ComponentInfo] = &[
    ComponentInfo {
        family: "unstyled",
        module: "progress",
        name: "Progress",
    },
    ComponentInfo {
        family: "unstyled",
        module: "progress",
        name: "ProgressWithLabel",
    },
    ComponentInfo {
        family: "unstyled",
        module: "progress",
        name: "CircularProgress",
    },
    ComponentInfo {
        family: "unstyled",
        module: "progress",
        name: "PrimaryProgress",
    },
    ComponentInfo {
        family: "unstyled",
        module: "progress",
        name: "SuccessProgress",
    },
    ComponentInfo {
        family: "unstyled",
        module: "progress",
        name: "WarningProgress",
    },
    ComponentInfo {
        family: "unstyled",
        module: "progress",
        name: "InfoProgress",
    },
    ComponentInfo {
        family: "unstyled",
        module: "progress",
        name: "IndeterminateProgress",
    },
];

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProgressProps {
    pub value: i32,
    pub max: i32,
    pub indeterminate: bool,
    pub size: &'static str,
    pub variant: &'static str,
    pub show_label: bool,
    pub label: String,
    pub animated: bool,
    pub class: String,
    pub attributes: BTreeMap<String, String>,
    pub aria_label: String,
    pub aria_described_by: String,
}

impl Default for ProgressProps {
    fn default() -> Self {
        Self {
            value: 0,
            max: 100,
            indeterminate: false,
            size: shared::SIZE_MD,
            variant: shared::VARIANT_PRIMARY,
            show_label: false,
            label: String::new(),
            animated: false,
            class: String::new(),
            attributes: BTreeMap::new(),
            aria_label: String::new(),
            aria_described_by: String::new(),
        }
    }
}

impl ProgressProps {
    #[must_use]
    pub const fn with_value(mut self, value: i32) -> Self {
        self.value = value;
        self
    }

    #[must_use]
    pub const fn with_max(mut self, max: i32) -> Self {
        self.max = max;
        self
    }

    #[must_use]
    pub const fn with_indeterminate(mut self, indeterminate: bool) -> Self {
        self.indeterminate = indeterminate;
        self
    }

    #[must_use]
    pub fn with_size(mut self, size: &'static str) -> Self {
        self.size = size;
        self
    }

    #[must_use]
    pub fn with_variant(mut self, variant: &'static str) -> Self {
        self.variant = variant;
        self
    }

    #[must_use]
    pub const fn with_show_label(mut self, show_label: bool) -> Self {
        self.show_label = show_label;
        self
    }

    #[must_use]
    pub fn with_label(mut self, label: impl Into<String>) -> Self {
        self.label = label.into();
        self
    }

    #[must_use]
    pub const fn with_animated(mut self, animated: bool) -> Self {
        self.animated = animated;
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
    pub fn with_aria_label(mut self, aria_label: impl Into<String>) -> Self {
        self.aria_label = aria_label.into();
        self
    }

    #[must_use]
    pub fn with_aria_described_by(mut self, aria_described_by: impl Into<String>) -> Self {
        self.aria_described_by = aria_described_by.into();
        self
    }

    #[must_use]
    pub fn build_container_classes(&self) -> String {
        shared::build_classes(&[
            "w-full overflow-hidden bg-muted rounded-full",
            shared::progress_size_classes(self.size),
            self.class.as_str(),
        ])
    }

    #[must_use]
    pub fn build_bar_classes(&self) -> String {
        shared::build_classes(&[
            "h-full transition-all duration-300 ease-in-out",
            shared::progress_variant_classes(self.variant),
            if self.animated { "animate-pulse" } else { "" },
            if self.indeterminate {
                "animate-progress-indeterminate"
            } else {
                ""
            },
        ])
    }

    #[must_use]
    pub fn build_attributes(&self) -> BTreeMap<String, String> {
        let mut attrs = self.attributes.clone();
        attrs.insert("role".to_string(), "progressbar".to_string());
        if !self.indeterminate {
            attrs.insert("aria-valuenow".to_string(), self.value.to_string());
            attrs.insert("aria-valuemin".to_string(), "0".to_string());
            attrs.insert("aria-valuemax".to_string(), self.max.to_string());
        }
        if !self.aria_label.is_empty() {
            attrs.insert("aria-label".to_string(), self.aria_label.clone());
        }
        if !self.aria_described_by.is_empty() {
            attrs.insert(
                "aria-describedby".to_string(),
                self.aria_described_by.clone(),
            );
        }
        attrs
    }

    #[must_use]
    pub fn get_percentage(&self) -> i32 {
        if self.max == 0 {
            return 0;
        }
        ((self.value * 100) / self.max).clamp(0, 100)
    }

    #[must_use]
    pub fn get_label_text(&self) -> String {
        if !self.label.is_empty() {
            return self.label.clone();
        }
        if self.indeterminate {
            return shared::default_strings().loading_text.to_string();
        }
        format!("{}%", self.get_percentage())
    }
}

#[must_use]
pub fn getSizeClass(size: &str) -> &'static str {
    match size {
        shared::SIZE_XS => "h-4 w-4",
        shared::SIZE_SM => "h-6 w-6",
        shared::SIZE_LG => "h-10 w-10",
        shared::SIZE_XL => "h-12 w-12",
        _ => "h-8 w-8",
    }
}

#[must_use]
pub fn Progress(props: &ProgressProps) -> Markup {
    let attrs = props.build_attributes();
    let bar_style = if props.indeterminate {
        "width: 100%".to_string()
    } else {
        format!("width: {}%", props.get_percentage())
    };
    html! {
        div class="w-full" {
            @if props.show_label {
                div class="flex justify-between items-center mb-1" {
                    span class="text-sm font-medium text-foreground" {
                        (props.get_label_text())
                    }
                }
            }
            div
                class=(props.build_container_classes())
                role=(shared::attr_value(&attrs, "role").unwrap_or("progressbar"))
                aria-valuenow=[shared::attr_value(&attrs, "aria-valuenow")]
                aria-valuemin=[shared::attr_value(&attrs, "aria-valuemin")]
                aria-valuemax=[shared::attr_value(&attrs, "aria-valuemax")]
                aria-label=[shared::attr_value(&attrs, "aria-label")]
                aria-describedby=[shared::attr_value(&attrs, "aria-describedby")]
                id=[shared::attr_value(&attrs, "id")]
                title=[shared::attr_value(&attrs, "title")]
            {
                div class=(props.build_bar_classes()) style=(bar_style) {}
            }
        }
    }
}

#[must_use]
pub fn ProgressWithLabel(props: &ProgressProps) -> Markup {
    let mut next = props.clone();
    next.show_label = true;
    Progress(&next)
}

#[must_use]
pub fn CircularProgress(
    size: &'static str,
    variant: &'static str,
    class: impl Into<String>,
) -> Markup {
    let class = class.into();
    html! {
        div
            class=(shared::build_classes(&["inline-flex items-center justify-center", class.as_str()]))
            role="progressbar"
            aria-label=(shared::default_strings().progress_loading_aria_label)
        {
            svg class=(shared::build_classes(&["animate-spin", getSizeClass(size)])) fill="none" viewBox="0 0 24 24" {
                circle class="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" stroke-width="4" {}
                path class=(shared::progress_variant_classes(variant)) fill="currentColor" d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4zm2 5.291A7.962 7.962 0 014 12H0c0 3.042 1.135 5.824 3 7.938l3-2.647z" {}
            }
        }
    }
}

#[must_use]
pub fn PrimaryProgress(value: i32, max: i32, size: &'static str, show_label: bool) -> Markup {
    Progress(
        &ProgressProps::default()
            .with_value(value)
            .with_max(max)
            .with_size(size)
            .with_variant(shared::VARIANT_PRIMARY)
            .with_show_label(show_label),
    )
}

#[must_use]
pub fn SuccessProgress(value: i32, max: i32, size: &'static str, show_label: bool) -> Markup {
    Progress(
        &ProgressProps::default()
            .with_value(value)
            .with_max(max)
            .with_size(size)
            .with_variant(shared::VARIANT_SUCCESS)
            .with_show_label(show_label),
    )
}

#[must_use]
pub fn WarningProgress(value: i32, max: i32, size: &'static str, show_label: bool) -> Markup {
    Progress(
        &ProgressProps::default()
            .with_value(value)
            .with_max(max)
            .with_size(size)
            .with_variant(shared::VARIANT_WARNING)
            .with_show_label(show_label),
    )
}

#[must_use]
pub fn InfoProgress(value: i32, max: i32, size: &'static str, show_label: bool) -> Markup {
    Progress(
        &ProgressProps::default()
            .with_value(value)
            .with_max(max)
            .with_size(size)
            .with_variant(shared::VARIANT_INFO)
            .with_show_label(show_label),
    )
}

#[must_use]
pub fn IndeterminateProgress(size: &'static str, variant: &'static str) -> Markup {
    Progress(
        &ProgressProps::default()
            .with_indeterminate(true)
            .with_size(size)
            .with_variant(variant)
            .with_show_label(false),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const SIZES: &[(&str, &str, &str)] = &[
        (shared::SIZE_XS, "h-2", "h-4 w-4"),
        (shared::SIZE_SM, "h-1", "h-6 w-6"),
        (shared::SIZE_MD, "h-2", "h-8 w-8"),
        (shared::SIZE_LG, "h-4", "h-10 w-10"),
        (shared::SIZE_XL, "h-2", "h-12 w-12"),
    ];
    const VARIANTS: &[&str] = &[
        shared::VARIANT_PRIMARY,
        shared::VARIANT_SECONDARY,
        shared::VARIANT_DESTRUCTIVE,
        shared::VARIANT_SUCCESS,
        shared::VARIANT_WARNING,
        shared::VARIANT_INFO,
        "unknown",
    ];

    #[test]
    fn default_props_match_go_defaults() {
        let props = ProgressProps::default();
        assert_eq!(props.value, 0);
        assert_eq!(props.max, 100);
        assert!(!props.indeterminate);
        assert_eq!(props.size, shared::SIZE_MD);
        assert_eq!(props.variant, shared::VARIANT_PRIMARY);
        assert!(!props.show_label);
    }

    #[test]
    fn helper_methods_cover_sizes_variants_animated_indeterminate_and_attrs() {
        for &(size, container_size, circular_size) in SIZES {
            assert_eq!(getSizeClass(size), circular_size);
            for &variant in VARIANTS {
                for animated in [false, true] {
                    for indeterminate in [false, true] {
                        let props = ProgressProps::default()
                            .with_size(size)
                            .with_variant(variant)
                            .with_animated(animated)
                            .with_indeterminate(indeterminate)
                            .with_class("custom")
                            .with_attribute("title", "Progress")
                            .with_aria_label("Upload")
                            .with_aria_described_by("hint");
                        let container = props.build_container_classes();
                        assert!(container.contains("w-full overflow-hidden bg-muted rounded-full"));
                        assert!(container.contains(container_size));
                        assert!(container.contains("custom"));
                        let bar = props.build_bar_classes();
                        assert!(bar.contains("h-full transition-all duration-300 ease-in-out"));
                        assert!(bar.contains(shared::progress_variant_classes(variant)));
                        assert_eq!(bar.contains("animate-pulse"), animated);
                        assert_eq!(
                            bar.contains("animate-progress-indeterminate"),
                            indeterminate
                        );
                        let attrs = props.build_attributes();
                        assert_eq!(attrs.get("role"), Some(&"progressbar".to_string()));
                        assert_eq!(attrs.get("title"), Some(&"Progress".to_string()));
                        assert_eq!(attrs.get("aria-label"), Some(&"Upload".to_string()));
                        assert_eq!(attrs.get("aria-describedby"), Some(&"hint".to_string()));
                    }
                }
            }
        }
    }

    #[test]
    fn percentage_and_label_text_cover_clamping_zero_max_custom_and_indeterminate() {
        assert_eq!(ProgressProps::default().with_value(50).get_percentage(), 50);
        assert_eq!(
            ProgressProps::default()
                .with_value(150)
                .with_max(100)
                .get_percentage(),
            100
        );
        assert_eq!(
            ProgressProps::default()
                .with_value(-10)
                .with_max(100)
                .get_percentage(),
            0
        );
        assert_eq!(ProgressProps::default().with_max(0).get_percentage(), 0);
        assert_eq!(
            ProgressProps::default().with_value(25).get_label_text(),
            "25%"
        );
        assert_eq!(
            ProgressProps::default()
                .with_label("Uploading")
                .get_label_text(),
            "Uploading"
        );
        assert_eq!(
            ProgressProps::default()
                .with_indeterminate(true)
                .get_label_text(),
            "Loading..."
        );
    }

    #[test]
    fn progress_renders_determinate_and_indeterminate_html() {
        let determinate = Progress(
            &ProgressProps::default()
                .with_value(25)
                .with_max(50)
                .with_show_label(true)
                .with_attribute("title", "\"Progress\" & more"),
        )
        .into_string();
        assert!(determinate.starts_with(r#"<div class="w-full">"#));
        assert!(
            determinate.contains(r#"<span class="text-sm font-medium text-foreground">50%</span>"#)
        );
        assert!(determinate.contains(
            r#"role="progressbar" aria-valuenow="25" aria-valuemin="0" aria-valuemax="50""#
        ));
        assert!(determinate.contains(r#"title="&quot;Progress&quot; &amp; more""#));
        assert!(determinate.contains(r#"style="width: 50%""#));

        let indeterminate = Progress(
            &ProgressProps::default()
                .with_indeterminate(true)
                .with_show_label(true),
        )
        .into_string();
        assert!(indeterminate.contains(">Loading...</span>"));
        assert!(!indeterminate.contains("aria-valuenow"));
        assert!(indeterminate.contains("animate-progress-indeterminate"));
        assert!(indeterminate.contains(r#"style="width: 100%""#));
    }

    #[test]
    fn circular_progress_renders_sizes_variants_and_accessible_label() {
        for &(size, _, circular_size) in SIZES {
            let html = CircularProgress(size, shared::VARIANT_SUCCESS, "spin").into_string();
            assert!(html.contains(r#"role="progressbar" aria-label="Loading""#));
            assert!(html.contains(circular_size));
            assert!(html.contains(shared::progress_variant_classes(shared::VARIANT_SUCCESS)));
            assert!(html.contains("spin"));
            assert!(html.contains(r#"<circle class="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" stroke-width="4"></circle>"#));
        }
    }

    #[test]
    fn convenience_progress_components_render_expected_variants() {
        assert!(
            PrimaryProgress(1, 2, shared::SIZE_MD, true)
                .into_string()
                .contains("bg-primary")
        );
        assert!(
            SuccessProgress(1, 2, shared::SIZE_MD, false)
                .into_string()
                .contains("bg-success")
        );
        assert!(
            WarningProgress(1, 2, shared::SIZE_MD, false)
                .into_string()
                .contains("bg-warning")
        );
        assert!(
            InfoProgress(1, 2, shared::SIZE_MD, false)
                .into_string()
                .contains("bg-info")
        );
        let indeterminate =
            IndeterminateProgress(shared::SIZE_LG, shared::VARIANT_DESTRUCTIVE).into_string();
        assert!(indeterminate.contains("animate-progress-indeterminate"));
        assert!(indeterminate.contains("bg-destructive"));
        assert!(indeterminate.contains("h-4"));
    }

    #[test]
    fn progress_with_label_forces_label_on() {
        let html = ProgressWithLabel(&ProgressProps::default().with_value(10)).into_string();
        assert!(html.contains(">10%</span>"));
    }
}
