#![allow(non_snake_case)]

use crate::components::ComponentInfo;
use maud::{Markup, html};
use std::collections::BTreeMap;

pub const COMPONENTS: &[ComponentInfo] = &[
    ComponentInfo {
        family: "unstyled",
        module: "languageselector",
        name: "LanguageSelector",
    },
    ComponentInfo {
        family: "unstyled",
        module: "languageselector",
        name: "SimpleLanguageSelector",
    },
    ComponentInfo {
        family: "unstyled",
        module: "languageselector",
        name: "CompactLanguageSelector",
    },
];

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Language {
    pub code: String,
    pub name: String,
    pub flag: String,
    pub default: bool,
}

impl Language {
    #[must_use]
    pub fn new(code: impl Into<String>, name: impl Into<String>, flag: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            name: name.into(),
            flag: flag.into(),
            default: false,
        }
    }

    #[must_use]
    pub const fn with_default(mut self, value: bool) -> Self {
        self.default = value;
        self
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LanguageSelectorProps {
    pub languages: Vec<Language>,
    pub current_language: String,
    pub on_change: String,
    pub show_flags: bool,
    pub compact: bool,
    pub drop_up: bool,
    pub class: String,
    pub attrs: BTreeMap<String, String>,
    pub id: String,
}

impl Default for LanguageSelectorProps {
    fn default() -> Self {
        Self {
            languages: vec![
                Language::new("en", "English", "\u{1F1FA}\u{1F1F8}").with_default(true),
                Language::new("es", "Espa\u{00F1}ol", "\u{1F1EA}\u{1F1F8}"),
                Language::new("pt", "Portugu\u{00EA}s", "\u{1F1E7}\u{1F1F7}"),
            ],
            current_language: "en".to_string(),
            on_change: String::new(),
            show_flags: true,
            compact: false,
            drop_up: false,
            class: String::new(),
            attrs: BTreeMap::new(),
            id: "langSel".to_string(),
        }
    }
}

impl LanguageSelectorProps {
    #[must_use]
    pub fn with_id(mut self, value: impl Into<String>) -> Self {
        self.id = value.into();
        self
    }

    #[must_use]
    pub fn with_languages(mut self, value: Vec<Language>) -> Self {
        self.languages = value;
        self
    }

    #[must_use]
    pub fn with_current_language(mut self, value: impl Into<String>) -> Self {
        self.current_language = value.into();
        self
    }

    #[must_use]
    pub fn with_on_change(mut self, value: impl Into<String>) -> Self {
        self.on_change = value.into();
        self
    }

    #[must_use]
    pub const fn with_show_flags(mut self, value: bool) -> Self {
        self.show_flags = value;
        self
    }

    #[must_use]
    pub const fn with_compact(mut self, value: bool) -> Self {
        self.compact = value;
        self
    }

    #[must_use]
    pub const fn with_drop_up(mut self, value: bool) -> Self {
        self.drop_up = value;
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
    pub fn get_current_language(&self) -> Language {
        for language in &self.languages {
            if language.code == self.current_language {
                return language.clone();
            }
        }
        if let Some(language) = self.languages.first() {
            return language.clone();
        }
        Language::new("en", "English", "")
    }
}

#[must_use]
pub fn buildLanguageSelectorClasses(props: &LanguageSelectorProps) -> String {
    if props.class.is_empty() {
        "language-selector relative inline-block".to_string()
    } else {
        format!("language-selector relative inline-block {}", props.class)
    }
}

#[must_use]
pub fn getLanguageItemClasses(code: &str, current_code: &str) -> String {
    let base = "flex items-center gap-2 w-full px-3 py-2 text-left hover:bg-muted transition-colors duration-150";
    if code == current_code {
        format!("{base} bg-muted/50")
    } else {
        base.to_string()
    }
}

fn signal_id(props: &LanguageSelectorProps) -> &str {
    if props.id.is_empty() {
        "langSel"
    } else {
        &props.id
    }
}

#[must_use]
pub fn buildSignals(props: &LanguageSelectorProps) -> String {
    format!("{{{}Open: false}}", signal_id(props))
}

#[must_use]
pub fn buildOpenExpression(props: &LanguageSelectorProps) -> String {
    format!("${}Open", signal_id(props))
}

#[must_use]
pub fn buildToggleExpression(props: &LanguageSelectorProps) -> String {
    let id = signal_id(props);
    format!("${id}Open = !${id}Open")
}

#[must_use]
pub fn buildCloseExpression(props: &LanguageSelectorProps) -> String {
    format!("${}Open = false", signal_id(props))
}

#[must_use]
pub fn dropdownClasses(props: &LanguageSelectorProps) -> String {
    let base = "absolute z-50 min-w-[150px] rounded-md border border-border bg-background text-foreground shadow-lg transition-all duration-200";
    if props.drop_up {
        format!("{base} bottom-full mb-1")
    } else {
        format!("{base} mt-1")
    }
}

#[must_use]
pub fn buildRotateExpression(props: &LanguageSelectorProps) -> String {
    format!("${}Open ? 'rotate-180' : ''", signal_id(props))
}

fn chevron(props: &LanguageSelectorProps) -> Markup {
    html! {
        svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" class="transition-transform" data-class=(buildRotateExpression(props)) {
            polyline points="6 9 12 15 18 9" {}
        }
    }
}

fn checkmark() -> Markup {
    html! {
        svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" class="ml-auto" {
            polyline points="20 6 9 17 4 12" {}
        }
    }
}

#[must_use]
pub fn LanguageSelector(props: &LanguageSelectorProps) -> Markup {
    let current = props.get_current_language();
    html! {
        div
            class=(buildLanguageSelectorClasses(props))
            data-signals=(buildSignals(props))
            data-on:click__outside=(buildCloseExpression(props))
        {
            button
                type="button"
                class="inline-flex items-center gap-2 px-3 py-2 rounded-md border border-border bg-background text-foreground hover:bg-muted transition-colors duration-150"
                data-on:click=(buildToggleExpression(props))
                aria-haspopup="listbox"
                data-attr:aria-expanded=(buildOpenExpression(props))
            {
                @if props.show_flags {
                    span class="text-base" { (&current.flag) }
                }
                @if !props.compact {
                    span class="text-sm" { (&current.name) }
                }
                (chevron(props))
            }
            div
                data-show=(buildOpenExpression(props))
                style="display: none"
                class=(dropdownClasses(props))
                role="listbox"
            {
                @for language in &props.languages {
                    button
                        type="button"
                        class=(getLanguageItemClasses(&language.code, &props.current_language))
                        role="option"
                        aria-selected=(if language.code == props.current_language { "true" } else { "false" })
                        data-on:click=(if props.on_change.is_empty() {
                            buildCloseExpression(props)
                        } else {
                            format!("{}('{}'); {}", props.on_change, language.code, buildCloseExpression(props))
                        })
                        data-lang=(&language.code)
                    {
                        @if props.show_flags {
                            span class="text-base" { (&language.flag) }
                        }
                        span class="text-sm" { (&language.name) }
                        @if language.code == props.current_language {
                            (checkmark())
                        }
                    }
                }
            }
        }
    }
}

#[must_use]
pub fn SimpleLanguageSelector() -> Markup {
    LanguageSelector(&LanguageSelectorProps::default())
}

#[must_use]
pub fn CompactLanguageSelector() -> Markup {
    LanguageSelector(
        &LanguageSelectorProps::default()
            .with_compact(true)
            .with_id("langSelCompact"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn helpers_cover_defaults_current_language_classes_and_datastar_expressions() {
        let props = LanguageSelectorProps::default();
        assert_eq!(props.current_language, "en");
        assert!(props.show_flags);
        assert!(!props.compact);
        assert_eq!(props.id, "langSel");
        assert_eq!(props.get_current_language().name, "English");
        assert_eq!(
            props
                .clone()
                .with_current_language("missing")
                .get_current_language()
                .code,
            "en"
        );
        assert_eq!(
            props
                .clone()
                .with_languages(Vec::new())
                .get_current_language(),
            Language::new("en", "English", "")
        );
        assert_eq!(
            buildLanguageSelectorClasses(&props),
            "language-selector relative inline-block"
        );
        assert_eq!(
            buildLanguageSelectorClasses(&props.clone().with_class("ml-2")),
            "language-selector relative inline-block ml-2"
        );
        assert_eq!(
            getLanguageItemClasses("en", "en"),
            "flex items-center gap-2 w-full px-3 py-2 text-left hover:bg-muted transition-colors duration-150 bg-muted/50"
        );
        assert_eq!(
            getLanguageItemClasses("es", "en"),
            "flex items-center gap-2 w-full px-3 py-2 text-left hover:bg-muted transition-colors duration-150"
        );
        assert_eq!(buildSignals(&props), "{langSelOpen: false}");
        assert_eq!(buildOpenExpression(&props), "$langSelOpen");
        assert_eq!(
            buildToggleExpression(&props),
            "$langSelOpen = !$langSelOpen"
        );
        assert_eq!(buildCloseExpression(&props), "$langSelOpen = false");
        assert_eq!(
            buildRotateExpression(&props),
            "$langSelOpen ? 'rotate-180' : ''"
        );
        assert_eq!(
            props
                .clone()
                .with_attr("data-testid", "language")
                .attrs
                .get("data-testid")
                .map(String::as_str),
            Some("language")
        );
        assert!(dropdownClasses(&props).ends_with(" mt-1"));
        assert!(dropdownClasses(&props.with_drop_up(true)).ends_with(" bottom-full mb-1"));
    }

    #[test]
    fn language_selector_renders_trigger_menu_flags_current_check_and_onchange() {
        let props = LanguageSelectorProps::default()
            .with_id("locale")
            .with_current_language("es")
            .with_on_change("setLocale")
            .with_class("custom");
        let html = LanguageSelector(&props).into_string();
        assert!(html.starts_with("<div class=\"language-selector relative inline-block custom\""));
        assert!(html.contains("data-signals=\"{localeOpen: false}\""));
        assert!(html.contains("data-on:click__outside=\"$localeOpen = false\""));
        assert!(html.contains("data-on:click=\"$localeOpen = !$localeOpen\""));
        assert!(html.contains("data-attr:aria-expanded=\"$localeOpen\""));
        assert!(html.contains("data-class=\"$localeOpen ? 'rotate-180' : ''\""));
        assert!(html.contains("<span class=\"text-sm\">Espa\u{00F1}ol</span>"));
        assert!(html.contains("data-show=\"$localeOpen\""));
        assert!(html.contains("role=\"listbox\""));
        assert_eq!(html.matches("role=\"option\"").count(), 3);
        assert!(html.contains("aria-selected=\"true\" data-on:click=\"setLocale('es'); $localeOpen = false\" data-lang=\"es\""));
        assert!(html.contains("<polyline points=\"20 6 9 17 4 12\"></polyline>"));
    }

    #[test]
    fn compact_no_flags_empty_id_and_simple_variants_render_expected_html() {
        let custom = LanguageSelector(
            &LanguageSelectorProps::default()
                .with_id("")
                .with_show_flags(false)
                .with_compact(true)
                .with_languages(vec![Language::new("fr", "Fran\u{00E7}ais", "")])
                .with_current_language("fr"),
        )
        .into_string();
        assert!(custom.contains("data-signals=\"{langSelOpen: false}\""));
        assert!(!custom.contains("🇺🇸"));
        assert!(!custom.contains("Fran\u{00E7}ais</span></button><svg"));
        assert!(custom.contains("data-on:click=\"$langSelOpen = false\" data-lang=\"fr\""));
        assert!(custom.contains("<span class=\"text-sm\">Fran\u{00E7}ais</span>"));

        let simple = SimpleLanguageSelector().into_string();
        assert!(simple.contains("data-signals=\"{langSelOpen: false}\""));
        assert!(simple.contains("<span class=\"text-sm\">English</span>"));

        let compact = CompactLanguageSelector().into_string();
        assert!(compact.contains("data-signals=\"{langSelCompactOpen: false}\""));
        assert!(!compact.contains("<span class=\"text-sm\">English</span></button>"));
    }
}
