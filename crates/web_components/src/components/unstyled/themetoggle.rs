#![allow(non_snake_case)]

use crate::components::{ComponentInfo, shared};
use maud::{Markup, html};
use std::collections::BTreeMap;

const TOGGLE_SCRIPT: &str = "document.documentElement.classList.toggle('dark'); localStorage.setItem('theme', document.documentElement.classList.contains('dark') ? 'dark' : 'light')";

pub const COMPONENTS: &[ComponentInfo] = &[
    ComponentInfo {
        family: "unstyled",
        module: "themetoggle",
        name: "ThemeToggleButton",
    },
    ComponentInfo {
        family: "unstyled",
        module: "themetoggle",
        name: "SimpleThemeToggle",
    },
    ComponentInfo {
        family: "unstyled",
        module: "themetoggle",
        name: "ThemeToggleWithLabel",
    },
];

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ThemeToggleButtonProps {
    pub current_theme: String,
    pub show_label: bool,
    pub size: &'static str,
    pub class: String,
    pub attributes: BTreeMap<String, String>,
    pub label_toggle: String,
    pub label_switch_light: String,
    pub label_switch_dark: String,
}

impl Default for ThemeToggleButtonProps {
    fn default() -> Self {
        Self {
            current_theme: "system".to_string(),
            show_label: false,
            size: shared::SIZE_MD,
            class: String::new(),
            attributes: BTreeMap::new(),
            label_toggle: String::new(),
            label_switch_light: String::new(),
            label_switch_dark: String::new(),
        }
    }
}

impl ThemeToggleButtonProps {
    #[must_use]
    pub fn with_current_theme(mut self, value: impl Into<String>) -> Self {
        self.current_theme = value.into();
        self
    }

    #[must_use]
    pub const fn with_show_label(mut self, value: bool) -> Self {
        self.show_label = value;
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
    pub fn with_attribute(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.attributes.insert(name.into(), value.into());
        self
    }

    #[must_use]
    pub fn with_label_toggle(mut self, value: impl Into<String>) -> Self {
        self.label_toggle = value.into();
        self
    }

    #[must_use]
    pub fn with_label_switch_light(mut self, value: impl Into<String>) -> Self {
        self.label_switch_light = value.into();
        self
    }

    #[must_use]
    pub fn with_label_switch_dark(mut self, value: impl Into<String>) -> Self {
        self.label_switch_dark = value.into();
        self
    }

    #[must_use]
    pub fn build_classes(&self) -> String {
        let size_class = match self.size {
            shared::SIZE_SM => "h-8 w-8",
            shared::SIZE_LG => "h-12 w-12",
            _ => "h-10 w-10",
        };
        shared::build_classes(&[
            "inline-flex items-center justify-center rounded-md transition-colors duration-150 hover:bg-accent hover:text-accent-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring",
            size_class,
            self.class.as_str(),
        ])
    }

    #[must_use]
    pub fn build_attributes(&self) -> BTreeMap<String, String> {
        self.attributes.clone()
    }

    #[must_use]
    pub fn is_dark(&self) -> bool {
        self.current_theme == "dark"
    }

    #[must_use]
    pub fn is_light(&self) -> bool {
        self.current_theme == "light"
    }
}

#[must_use]
pub fn getSunIconClass(is_dark: bool) -> &'static str {
    if is_dark {
        "rotate-0 scale-100 transition-all dark:rotate-0 dark:scale-100"
    } else {
        "rotate-90 scale-0 absolute transition-all dark:rotate-0 dark:scale-100"
    }
}

#[must_use]
pub fn getMoonIconClass(_is_light: bool) -> &'static str {
    "rotate-0 scale-100 transition-all dark:rotate-90 dark:scale-0 dark:absolute"
}

#[must_use]
pub fn resolveAriaLabel(props: &ThemeToggleButtonProps) -> String {
    match props.current_theme.as_str() {
        "dark" => {
            if props.label_switch_light.is_empty() {
                "Switch to light mode".to_string()
            } else {
                props.label_switch_light.clone()
            }
        }
        "light" => {
            if props.label_switch_dark.is_empty() {
                "Switch to dark mode".to_string()
            } else {
                props.label_switch_dark.clone()
            }
        }
        _ => {
            if props.label_toggle.is_empty() {
                "Toggle theme".to_string()
            } else {
                props.label_toggle.clone()
            }
        }
    }
}

#[must_use]
pub fn resolveThemeLabel(props: &ThemeToggleButtonProps) -> &'static str {
    match props.current_theme.as_str() {
        "dark" => "Dark",
        "light" => "Light",
        _ => "System",
    }
}

fn sun_icon(props: &ThemeToggleButtonProps) -> Markup {
    html! {
        svg xmlns="http://www.w3.org/2000/svg" width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" class=(getSunIconClass(props.is_dark())) {
            circle cx="12" cy="12" r="4" {}
            path d="M12 2v2" {}
            path d="M12 20v2" {}
            path d="m4.93 4.93 1.41 1.41" {}
            path d="m17.66 17.66 1.41 1.41" {}
            path d="M2 12h2" {}
            path d="M20 12h2" {}
            path d="m6.34 17.66-1.41 1.41" {}
            path d="m19.07 4.93-1.41 1.41" {}
        }
    }
}

fn moon_icon(props: &ThemeToggleButtonProps) -> Markup {
    html! {
        svg xmlns="http://www.w3.org/2000/svg" width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" class=(getMoonIconClass(props.is_light())) {
            path d="M12 3a6 6 0 0 0 9 9 9 9 0 1 1-9-9Z" {}
        }
    }
}

#[must_use]
pub fn ThemeToggleButton(props: &ThemeToggleButtonProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        button
            type="button"
            class=(props.build_classes())
            aria-label=(resolveAriaLabel(props))
            data-theme-toggle=""
            data-current-theme=(&props.current_theme)
            data-on:click=(TOGGLE_SCRIPT)
            id=[shared::attr_value(&attrs, "id")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
        {
            (sun_icon(props))
            (moon_icon(props))
            @if props.show_label {
                span class="ml-2 text-sm" {
                    (resolveThemeLabel(props))
                }
            }
        }
    }
}

#[must_use]
pub fn SimpleThemeToggle(class: &str) -> Markup {
    ThemeToggleButton(&ThemeToggleButtonProps::default().with_class(class))
}

#[must_use]
pub fn ThemeToggleWithLabel(current_theme: &str, class: &str) -> Markup {
    ThemeToggleButton(
        &ThemeToggleButtonProps::default()
            .with_current_theme(current_theme)
            .with_show_label(true)
            .with_class(class),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn helpers_cover_defaults_sizes_icon_classes_labels_and_attrs() {
        let props = ThemeToggleButtonProps::default();
        assert_eq!(props.current_theme, "system");
        assert!(!props.show_label);
        assert_eq!(props.size, shared::SIZE_MD);
        assert_eq!(
            props.build_classes(),
            "inline-flex items-center justify-center rounded-md transition-colors duration-150 hover:bg-accent hover:text-accent-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring h-10 w-10"
        );
        assert_eq!(
            ThemeToggleButtonProps::default()
                .with_size(shared::SIZE_SM)
                .with_class("custom")
                .build_classes(),
            "inline-flex items-center justify-center rounded-md transition-colors duration-150 hover:bg-accent hover:text-accent-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring h-8 w-8 custom"
        );
        assert!(
            ThemeToggleButtonProps::default()
                .with_current_theme("dark")
                .is_dark()
        );
        assert!(
            ThemeToggleButtonProps::default()
                .with_current_theme("light")
                .is_light()
        );
        assert_eq!(
            getSunIconClass(true),
            "rotate-0 scale-100 transition-all dark:rotate-0 dark:scale-100"
        );
        assert_eq!(
            getSunIconClass(false),
            "rotate-90 scale-0 absolute transition-all dark:rotate-0 dark:scale-100"
        );
        assert_eq!(
            getMoonIconClass(true),
            "rotate-0 scale-100 transition-all dark:rotate-90 dark:scale-0 dark:absolute"
        );
        assert_eq!(resolveAriaLabel(&props), "Toggle theme");
        assert_eq!(
            resolveAriaLabel(&ThemeToggleButtonProps::default().with_current_theme("dark")),
            "Switch to light mode"
        );
        assert_eq!(
            resolveAriaLabel(&ThemeToggleButtonProps::default().with_current_theme("light")),
            "Switch to dark mode"
        );
        assert_eq!(
            resolveAriaLabel(&ThemeToggleButtonProps::default().with_label_toggle("Toggle")),
            "Toggle"
        );
        assert_eq!(
            resolveAriaLabel(
                &ThemeToggleButtonProps::default()
                    .with_current_theme("dark")
                    .with_label_switch_light("Light please")
            ),
            "Light please"
        );
        assert_eq!(
            resolveAriaLabel(
                &ThemeToggleButtonProps::default()
                    .with_current_theme("light")
                    .with_label_switch_dark("Dark please")
            ),
            "Dark please"
        );
        assert_eq!(
            ThemeToggleButtonProps::default()
                .with_attribute("id", "theme")
                .build_attributes()
                .get("id")
                .map(String::as_str),
            Some("theme")
        );
    }

    #[test]
    fn theme_toggle_button_renders_datastar_attrs_icons_label_and_escaping() {
        let html = ThemeToggleButton(
            &ThemeToggleButtonProps::default()
                .with_current_theme("dark")
                .with_show_label(true)
                .with_size(shared::SIZE_LG)
                .with_class("custom")
                .with_attribute("data-testid", "theme-toggle"),
        )
        .into_string();
        assert!(html.starts_with(
            "<button type=\"button\" class=\"inline-flex items-center justify-center"
        ));
        assert!(html.contains("h-12 w-12 custom"));
        assert!(html.contains("aria-label=\"Switch to light mode\""));
        assert!(html.contains("data-theme-toggle=\"\""));
        assert!(html.contains("data-current-theme=\"dark\""));
        assert!(html.contains("data-on:click=\"document.documentElement.classList.toggle('dark'); localStorage.setItem('theme', document.documentElement.classList.contains('dark') ? 'dark' : 'light')\""));
        assert!(html.contains("data-testid=\"theme-toggle\""));
        assert_eq!(html.matches("<svg ").count(), 2);
        assert!(html.contains("<circle cx=\"12\" cy=\"12\" r=\"4\"></circle>"));
        assert!(html.contains("<path d=\"M12 3a6 6 0 0 0 9 9 9 9 0 1 1-9-9Z\"></path>"));
        assert!(html.contains("<span class=\"ml-2 text-sm\">Dark</span>"));
    }

    #[test]
    fn simple_and_label_variants_render_expected_html() {
        let simple = SimpleThemeToggle("compact").into_string();
        assert!(simple.contains("data-current-theme=\"system\""));
        assert!(simple.contains("compact"));
        assert!(!simple.contains("ml-2 text-sm"));

        let light = ThemeToggleWithLabel("light", "wide").into_string();
        assert!(light.contains("aria-label=\"Switch to dark mode\""));
        assert!(light.contains("data-current-theme=\"light\""));
        assert!(light.contains("<span class=\"ml-2 text-sm\">Light</span>"));

        let system = ThemeToggleWithLabel("system", "").into_string();
        assert!(system.contains("<span class=\"ml-2 text-sm\">System</span>"));
    }
}
