#![allow(non_snake_case)]
#![allow(clippy::needless_pass_by_value)]

use crate::components::ComponentInfo;
use crate::components::shared;
use crate::components::unstyled::selectfield as base;
use maud::Markup;

pub use base::{SelectOption, SelectOptionGroup, SelectProps};

pub const COMPONENTS: &[ComponentInfo] = &[
    ComponentInfo {
        family: "neomorphism",
        module: "selectfield",
        name: "Select",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "selectfield",
        name: "SelectWithGroups",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "selectfield",
        name: "SelectWithLabel",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "selectfield",
        name: "SelectWithGroupsAndLabel",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "selectfield",
        name: "CountrySelect",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "selectfield",
        name: "LanguageSelect",
    },
];

#[must_use]
fn addNeomorphClasses(props: &SelectProps) -> SelectProps {
    let mut props = props.clone();
    props.class = shared::build_classes(&[
        "neomorph-input pr-10",
        if props.error {
            "border-destructive"
        } else {
            ""
        },
        props.class.as_str(),
    ]);
    props
}

#[must_use]
pub fn Select(props: &SelectProps, options: &[SelectOption]) -> Markup {
    let neo_props = addNeomorphClasses(props);
    base::Select(&neo_props, options)
}

#[must_use]
pub fn SelectWithGroups(props: &SelectProps, groups: &[SelectOptionGroup]) -> Markup {
    let neo_props = addNeomorphClasses(props);
    base::SelectWithGroups(&neo_props, groups)
}

#[must_use]
pub fn SelectWithLabel(
    props: &SelectProps,
    options: &[SelectOption],
    label: impl Into<String>,
    helper_text: impl Into<String>,
) -> Markup {
    let neo_props = addNeomorphClasses(props);
    base::SelectWithLabel(&neo_props, options, label, helper_text)
}

#[must_use]
pub fn SelectWithGroupsAndLabel(
    props: &SelectProps,
    groups: &[SelectOptionGroup],
    label: impl Into<String>,
    helper_text: impl Into<String>,
) -> Markup {
    let neo_props = addNeomorphClasses(props);
    base::SelectWithGroupsAndLabel(&neo_props, groups, label, helper_text)
}

#[must_use]
pub fn CountrySelect(id: &str, name: &str, required: bool) -> Markup {
    SelectWithLabel(
        &SelectProps::default()
            .with_id(id)
            .with_name(name)
            .with_required(required)
            .with_size(shared::SIZE_MD)
            .with_placeholder("Select a country"),
        &[
            SelectOption::new("us", "United States"),
            SelectOption::new("ca", "Canada"),
            SelectOption::new("uk", "United Kingdom"),
            SelectOption::new("au", "Australia"),
            SelectOption::new("de", "Germany"),
            SelectOption::new("fr", "France"),
            SelectOption::new("jp", "Japan"),
            SelectOption::new("other", "Other"),
        ],
        "Country",
        "",
    )
}

#[must_use]
pub fn LanguageSelect(id: &str, name: &str, required: bool) -> Markup {
    SelectWithLabel(
        &SelectProps::default()
            .with_id(id)
            .with_name(name)
            .with_required(required)
            .with_size(shared::SIZE_MD)
            .with_placeholder("Select a language"),
        &[
            SelectOption::new("en", "English"),
            SelectOption::new("es", "Spanish"),
            SelectOption::new("fr", "French"),
            SelectOption::new("de", "German"),
            SelectOption::new("ja", "Japanese"),
            SelectOption::new("zh", "Chinese"),
            SelectOption::new("ar", "Arabic"),
            SelectOption::new("pt", "Portuguese"),
        ],
        "Language",
        "",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    fn assert_no_placeholder_markers(html: &str) {
        assert!(
            !html.contains("data-family="),
            "placeholder family marker leaked: {html}"
        );
        assert!(
            !html.contains("data-module="),
            "placeholder module marker leaked: {html}"
        );
        assert!(
            !html.contains("data-component=\""),
            "placeholder component marker leaked: {html}"
        );
    }

    #[test]
    fn registry_matches_neomorphism_source_components() {
        assert_eq!(COMPONENTS.len(), 6);
        assert!(COMPONENTS.iter().all(
            |component| component.family == "neomorphism" && component.module == "selectfield"
        ));
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "Select")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "SelectWithGroups")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "SelectWithLabel")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "SelectWithGroupsAndLabel")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "CountrySelect")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "LanguageSelect")
        );
    }

    #[test]
    fn neomorphism_only_components_render_html_without_generated_stub_markers() {
        let options = vec![
            SelectOption::new("one", "One"),
            SelectOption::new("two", "Two"),
        ];
        let groups = vec![SelectOptionGroup::new("Numbers", options.clone())];
        let error_props = SelectProps::default()
            .with_id("choice")
            .with_name("choice")
            .with_placeholder("Choose")
            .with_error(true, "Required");

        let html = Select(&error_props, &options).into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.contains("<select"));
        assert!(html.contains("id=\"choice\""));
        assert!(html.contains("name=\"choice\""));
        assert!(html.contains("border-destructive"));
        assert!(html.contains("<option value=\"one\">One</option>"));

        let html = SelectWithGroups(&error_props, &groups).into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.contains("<optgroup label=\"Numbers\">"));
        assert!(html.contains("<option value=\"two\">Two</option>"));
        assert!(html.contains("border-destructive"));

        let html =
            SelectWithGroupsAndLabel(&error_props, &groups, "Choice", "Pick one").into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.contains("<label"));
        assert!(html.contains(">Choice"));
        assert!(!html.contains("Pick one"));
        assert!(html.contains("Required"));
        assert!(html.contains("<optgroup label=\"Numbers\">"));

        let html = SelectWithGroupsAndLabel(
            &SelectProps::default()
                .with_id("choice-ok")
                .with_name("choice-ok")
                .with_placeholder("Choose"),
            &groups,
            "Choice",
            "Pick one",
        )
        .into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.contains("Pick one"));
        assert!(html.contains("<optgroup label=\"Numbers\">"));

        let html = CountrySelect("sample", "sample", true).into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.starts_with("<div class=\"space-y-2\""));
        assert!(html.contains("<label for=\"sample\""));
        assert!(html.contains(">Country"));
        assert!(html.contains("<select name=\"sample\" id=\"sample\""));
        assert!(html.contains("required"));
        assert!(html.contains("neomorph-input pr-10"));
        assert!(html.contains("<option value=\"us\">United States</option>"));
        assert!(html.contains("<option value=\"other\">Other</option>"));
        let html = LanguageSelect("sample", "sample", true).into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.starts_with("<div class=\"space-y-2\""));
        assert!(html.contains("<label for=\"sample\""));
        assert!(html.contains(">Language"));
        assert!(html.contains("<select name=\"sample\" id=\"sample\""));
        assert!(html.contains("required"));
        assert!(html.contains("neomorph-input pr-10"));
        assert!(html.contains("<option value=\"en\">English</option>"));
        assert!(html.contains("<option value=\"pt\">Portuguese</option>"));
    }
}
