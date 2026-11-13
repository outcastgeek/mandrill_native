#![allow(non_snake_case)]

use crate::components::ComponentInfo;
use maud::{Markup, html};
use std::collections::BTreeMap;

pub const COMPONENTS: &[ComponentInfo] = &[
    ComponentInfo {
        family: "unstyled",
        module: "learning",
        name: "LoadingBar",
    },
    ComponentInfo {
        family: "unstyled",
        module: "learning",
        name: "SimpleLoadingBar",
    },
    ComponentInfo {
        family: "unstyled",
        module: "learning",
        name: "ProgressLoadingBar",
    },
];

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LoadingBarProps {
    pub progress: i32,
    pub variant: String,
    pub size: String,
    pub animated: bool,
    pub show_label: bool,
    pub class: String,
    pub attrs: BTreeMap<String, String>,
}

impl Default for LoadingBarProps {
    fn default() -> Self {
        Self {
            progress: -1,
            variant: "primary".to_string(),
            size: "md".to_string(),
            animated: true,
            show_label: false,
            class: String::new(),
            attrs: BTreeMap::new(),
        }
    }
}

impl LoadingBarProps {
    #[must_use]
    pub const fn with_progress(mut self, value: i32) -> Self {
        self.progress = value;
        self
    }
    #[must_use]
    pub fn with_variant(mut self, value: impl Into<String>) -> Self {
        self.variant = value.into();
        self
    }
    #[must_use]
    pub fn with_size(mut self, value: impl Into<String>) -> Self {
        self.size = value.into();
        self
    }
    #[must_use]
    pub const fn with_animated(mut self, value: bool) -> Self {
        self.animated = value;
        self
    }
    #[must_use]
    pub const fn with_show_label(mut self, value: bool) -> Self {
        self.show_label = value;
        self
    }
    #[must_use]
    pub fn with_class(mut self, value: impl Into<String>) -> Self {
        self.class = value.into();
        self
    }
    #[must_use]
    pub fn with_attr(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.attrs.insert(name.into(), value.into());
        self
    }

    #[must_use]
    pub fn get_progress_width(&self) -> String {
        if self.progress < 0 || self.progress > 100 {
            "100%".to_string()
        } else {
            format!("{:03}%", self.progress)
        }
    }

    #[must_use]
    pub fn get_size_classes(&self) -> &'static str {
        match self.size.as_str() {
            "xs" => "h-1",
            "sm" => "h-2",
            "md" => "h-3",
            "lg" => "h-4",
            _ => "h-3",
        }
    }

    #[must_use]
    pub fn get_variant_classes(&self) -> &'static str {
        match self.variant.as_str() {
            "success" => "bg-success",
            "warning" => "bg-warning",
            "error" => "bg-destructive",
            _ => "bg-primary",
        }
    }
}

#[must_use]
pub fn buildLoadingBarClasses(props: &LoadingBarProps) -> String {
    let mut classes = format!("loading-bar flex items-center {}", props.get_size_classes());
    if !props.class.is_empty() {
        classes.push(' ');
        classes.push_str(&props.class);
    }
    classes
}

#[must_use]
pub fn LoadingBar(props: &LoadingBarProps) -> Markup {
    html! {
        div class=(buildLoadingBarClasses(props)) {
            div class="relative w-full bg-muted rounded-full overflow-hidden" {
                @if props.progress < 0 {
                    div
                        class=(format!("h-full {} animate-indeterminate", props.get_variant_classes()))
                        style="width: 30%"
                    {}
                } @else {
                    div
                        class=(format!("h-full transition-all duration-300 {}", props.get_variant_classes()))
                        style=(format!("width: {}%", props.progress))
                    {}
                }
            }
            @if props.show_label && props.progress >= 0 {
                span class="ml-2 text-sm text-muted-foreground" {
                    (format!("{}%", props.progress))
                }
            }
        }
    }
}

#[must_use]
pub fn SimpleLoadingBar() -> Markup {
    LoadingBar(&LoadingBarProps::default())
}

#[must_use]
pub fn ProgressLoadingBar(progress: i32, variant: &str) -> Markup {
    LoadingBar(
        &LoadingBarProps::default()
            .with_progress(progress)
            .with_variant(variant)
            .with_size("md")
            .with_show_label(true),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn helpers_cover_defaults_width_size_variant_classes_and_builders() {
        let props = LoadingBarProps::default();
        assert_eq!(props.progress, -1);
        assert_eq!(props.variant, "primary");
        assert_eq!(props.size, "md");
        assert!(props.animated);
        assert!(!props.show_label);
        assert_eq!(props.get_progress_width(), "100%");
        assert_eq!(props.get_size_classes(), "h-3");
        assert_eq!(props.get_variant_classes(), "bg-primary");
        assert_eq!(
            buildLoadingBarClasses(&props),
            "loading-bar flex items-center h-3"
        );

        for (size, expected) in [
            ("xs", "h-1"),
            ("sm", "h-2"),
            ("md", "h-3"),
            ("lg", "h-4"),
            ("bad", "h-3"),
        ] {
            assert_eq!(
                LoadingBarProps::default()
                    .with_size(size)
                    .get_size_classes(),
                expected
            );
        }
        for (variant, expected) in [
            ("primary", "bg-primary"),
            ("success", "bg-success"),
            ("warning", "bg-warning"),
            ("error", "bg-destructive"),
            ("bad", "bg-primary"),
        ] {
            assert_eq!(
                LoadingBarProps::default()
                    .with_variant(variant)
                    .get_variant_classes(),
                expected
            );
        }
        assert_eq!(
            LoadingBarProps::default()
                .with_progress(0)
                .get_progress_width(),
            "000%"
        );
        assert_eq!(
            LoadingBarProps::default()
                .with_progress(5)
                .get_progress_width(),
            "005%"
        );
        assert_eq!(
            LoadingBarProps::default()
                .with_progress(42)
                .get_progress_width(),
            "042%"
        );
        assert_eq!(
            LoadingBarProps::default()
                .with_progress(100)
                .get_progress_width(),
            "100%"
        );
        assert_eq!(
            LoadingBarProps::default()
                .with_progress(101)
                .get_progress_width(),
            "100%"
        );

        let built = LoadingBarProps::default()
            .with_progress(40)
            .with_variant("success")
            .with_size("lg")
            .with_animated(false)
            .with_show_label(true)
            .with_class("custom")
            .with_attr("data-testid", "bar");
        assert_eq!(built.progress, 40);
        assert_eq!(built.variant, "success");
        assert_eq!(built.size, "lg");
        assert!(!built.animated);
        assert!(built.show_label);
        assert_eq!(built.class, "custom");
        assert_eq!(
            built.attrs.get("data-testid").map(String::as_str),
            Some("bar")
        );
        assert_eq!(
            buildLoadingBarClasses(&built),
            "loading-bar flex items-center h-4 custom"
        );
    }

    #[test]
    fn loading_bar_renders_indeterminate_and_determinate_variants() {
        let indeterminate =
            LoadingBar(&LoadingBarProps::default().with_variant("warning")).into_string();
        assert_eq!(
            indeterminate,
            "<div class=\"loading-bar flex items-center h-3\"><div class=\"relative w-full bg-muted rounded-full overflow-hidden\"><div class=\"h-full bg-warning animate-indeterminate\" style=\"width: 30%\"></div></div></div>",
        );

        let determinate = LoadingBar(
            &LoadingBarProps::default()
                .with_progress(75)
                .with_variant("success")
                .with_size("sm")
                .with_show_label(true)
                .with_class("wide"),
        )
        .into_string();
        assert_eq!(
            determinate,
            "<div class=\"loading-bar flex items-center h-2 wide\"><div class=\"relative w-full bg-muted rounded-full overflow-hidden\"><div class=\"h-full transition-all duration-300 bg-success\" style=\"width: 75%\"></div></div><span class=\"ml-2 text-sm text-muted-foreground\">75%</span></div>",
        );

        let overflow = LoadingBar(
            &LoadingBarProps::default()
                .with_progress(150)
                .with_show_label(true),
        )
        .into_string();
        assert!(overflow.contains("style=\"width: 150%\""));
        assert!(overflow.contains(">150%</span>"));
    }

    #[test]
    fn convenience_components_render_expected_html() {
        let simple = SimpleLoadingBar().into_string();
        assert!(simple.contains("animate-indeterminate"));
        assert!(simple.contains("bg-primary"));

        let progress = ProgressLoadingBar(33, "error").into_string();
        assert!(progress.contains("loading-bar flex items-center h-3"));
        assert!(progress.contains("bg-destructive"));
        assert!(progress.contains("style=\"width: 33%\""));
        assert!(progress.contains("<span class=\"ml-2 text-sm text-muted-foreground\">33%</span>"));
    }
}
