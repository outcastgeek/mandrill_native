#![allow(non_snake_case)]

use crate::components::{ComponentInfo, shared};
use maud::{Markup, Render, html};
use std::collections::BTreeMap;

pub const COMPONENTS: &[ComponentInfo] = &[
    ComponentInfo {
        family: "unstyled",
        module: "radio",
        name: "Radio",
    },
    ComponentInfo {
        family: "unstyled",
        module: "radio",
        name: "RadioWithLabel",
    },
    ComponentInfo {
        family: "unstyled",
        module: "radio",
        name: "RadioGroup",
    },
    ComponentInfo {
        family: "unstyled",
        module: "radio",
        name: "RadioCard",
    },
    ComponentInfo {
        family: "unstyled",
        module: "radio",
        name: "RadioCardRich",
    },
    ComponentInfo {
        family: "unstyled",
        module: "radio",
        name: "RadioGroupCards",
    },
];

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RadioOption {
    pub value: String,
    pub label: String,
    pub label_html: String,
    pub checked: bool,
    pub disabled: bool,
    pub help_text: String,
    pub attributes: BTreeMap<String, String>,
}

impl RadioOption {
    #[must_use]
    pub fn with_value(mut self, value: impl Into<String>) -> Self {
        self.value = value.into();
        self
    }
    #[must_use]
    pub fn with_label(mut self, label: impl Into<String>) -> Self {
        self.label = label.into();
        self
    }
    #[must_use]
    pub fn with_label_html(mut self, html: impl Into<String>) -> Self {
        self.label_html = html.into();
        self
    }
    #[must_use]
    pub const fn with_checked(mut self, checked: bool) -> Self {
        self.checked = checked;
        self
    }
    #[must_use]
    pub const fn with_disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }
    #[must_use]
    pub fn with_help_text(mut self, text: impl Into<String>) -> Self {
        self.help_text = text.into();
        self
    }
    #[must_use]
    pub fn with_attribute(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.attributes.insert(name.into(), value.into());
        self
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RadioProps {
    pub name: String,
    pub id: String,
    pub value: String,
    pub checked: bool,
    pub disabled: bool,
    pub required: bool,
    pub size: &'static str,
    pub error: bool,
    pub class: String,
    pub attributes: BTreeMap<String, String>,
    pub aria_label: String,
    pub aria_described_by: String,
    pub aria_invalid: bool,
}

impl Default for RadioProps {
    fn default() -> Self {
        Self {
            name: String::new(),
            id: String::new(),
            value: String::new(),
            checked: false,
            disabled: false,
            required: false,
            size: shared::SIZE_MD,
            error: false,
            class: String::new(),
            attributes: BTreeMap::new(),
            aria_label: String::new(),
            aria_described_by: String::new(),
            aria_invalid: false,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RadioGroupProps {
    pub name: String,
    pub required: bool,
    pub error: bool,
    pub error_message: String,
    pub orientation: &'static str,
    pub size: &'static str,
    pub class: String,
}

impl Default for RadioGroupProps {
    fn default() -> Self {
        Self {
            name: String::new(),
            required: false,
            error: false,
            error_message: String::new(),
            orientation: "vertical",
            size: shared::SIZE_MD,
            class: String::new(),
        }
    }
}

impl RadioProps {
    #[must_use]
    pub fn with_name(mut self, value: impl Into<String>) -> Self {
        self.name = value.into();
        self
    }
    #[must_use]
    pub fn with_id(mut self, value: impl Into<String>) -> Self {
        self.id = value.into();
        self
    }
    #[must_use]
    pub fn with_value(mut self, value: impl Into<String>) -> Self {
        self.value = value.into();
        self
    }
    #[must_use]
    pub const fn with_checked(mut self, checked: bool) -> Self {
        self.checked = checked;
        self
    }
    #[must_use]
    pub const fn with_disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }
    #[must_use]
    pub const fn with_required(mut self, required: bool) -> Self {
        self.required = required;
        self
    }
    #[must_use]
    pub const fn with_size(mut self, size: &'static str) -> Self {
        self.size = size;
        self
    }
    #[must_use]
    pub const fn with_error(mut self, error: bool) -> Self {
        self.error = error;
        self.aria_invalid = error;
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
    pub fn with_aria_label(mut self, value: impl Into<String>) -> Self {
        self.aria_label = value.into();
        self
    }
    #[must_use]
    pub fn with_aria_described_by(mut self, value: impl Into<String>) -> Self {
        self.aria_described_by = value.into();
        self
    }

    #[must_use]
    pub fn build_classes(&self) -> String {
        shared::build_classes(&[
            "rounded-full border-2 border-border text-primary focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 disabled:cursor-not-allowed disabled:opacity-50 transition-all duration-150 cursor-pointer",
            self.get_size_classes(),
            if self.error {
                "border-destructive focus-visible:ring-destructive"
            } else {
                ""
            },
            self.class.as_str(),
        ])
    }

    #[must_use]
    pub fn get_size_classes(&self) -> &'static str {
        match self.size {
            shared::SIZE_SM => "h-4 w-4",
            shared::SIZE_LG => "h-6 w-6",
            _ => "h-5 w-5",
        }
    }

    #[must_use]
    pub fn build_attributes(&self) -> BTreeMap<String, String> {
        let mut attrs = self.attributes.clone();
        if !self.aria_label.is_empty() {
            attrs.insert("aria-label".to_string(), self.aria_label.clone());
        }
        if !self.aria_described_by.is_empty() {
            attrs.insert(
                "aria-describedby".to_string(),
                self.aria_described_by.clone(),
            );
        }
        if self.aria_invalid {
            attrs.insert("aria-invalid".to_string(), "true".to_string());
        }
        attrs
    }
}

impl RadioGroupProps {
    #[must_use]
    pub fn with_name(mut self, value: impl Into<String>) -> Self {
        self.name = value.into();
        self
    }
    #[must_use]
    pub const fn with_required(mut self, required: bool) -> Self {
        self.required = required;
        self
    }
    #[must_use]
    pub fn with_error(mut self, error: bool, message: impl Into<String>) -> Self {
        self.error = error;
        self.error_message = message.into();
        self
    }
    #[must_use]
    pub const fn with_orientation(mut self, orientation: &'static str) -> Self {
        self.orientation = orientation;
        self
    }
    #[must_use]
    pub const fn with_size(mut self, size: &'static str) -> Self {
        self.size = size;
        self
    }
    #[must_use]
    pub fn with_class(mut self, class: impl Into<String>) -> Self {
        self.class = class.into();
        self
    }
    #[must_use]
    pub fn get_orientation_classes(&self) -> &'static str {
        if self.orientation == "horizontal" {
            "flex flex-wrap gap-4"
        } else {
            "space-y-2"
        }
    }
}

#[must_use]
pub fn Radio(props: &RadioProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        input
            type="radio"
            name=[shared::optional_attr(&props.name)]
            id=[shared::optional_attr(&props.id)]
            value=[shared::optional_attr(&props.value)]
            class=(props.build_classes())
            checked[props.checked]
            disabled[props.disabled]
            required[props.required]
            aria-label=[shared::attr_value(&attrs, "aria-label")]
            aria-describedby=[shared::attr_value(&attrs, "aria-describedby")]
            aria-invalid=[shared::attr_value(&attrs, "aria-invalid")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
            data-on:change=[shared::attr_value(&attrs, "data-on:change")]
        {}
    }
}

#[must_use]
pub fn RadioWithLabel(
    props: &RadioProps,
    label: impl Into<String>,
    helper_text: impl Into<String>,
) -> Markup {
    let label = label.into();
    let helper_text = helper_text.into();
    html! {
        div class="flex items-start space-x-2" {
            div class="flex items-center h-5" { (Radio(props)) }
            div class="flex flex-col" {
                @if !label.is_empty() {
                    label
                        for=[shared::optional_attr(&props.id)]
                        class="text-sm font-medium leading-none peer-disabled:cursor-not-allowed peer-disabled:opacity-70 cursor-pointer"
                    {
                        (&label)
                        @if props.required {
                            span class="text-destructive ml-1" aria-label=(shared::default_strings().input_required_aria_label) {
                                (shared::default_strings().input_required_symbol)
                            }
                        }
                    }
                }
                @if !helper_text.is_empty() {
                    p class="text-sm text-muted-foreground mt-1" { (&helper_text) }
                }
            }
        }
    }
}

fn option_props(group: &RadioGroupProps, option: &RadioOption) -> RadioProps {
    RadioProps::default()
        .with_name(&group.name)
        .with_id(format!("{}-{}", group.name, option.value))
        .with_value(&option.value)
        .with_checked(option.checked)
        .with_disabled(option.disabled)
        .with_required(group.required)
        .with_size(group.size)
        .with_error(group.error)
        .with_attribute(
            "data-on:change",
            shared::attr_value(&option.attributes, "data-on:change").unwrap_or(""),
        )
}

#[must_use]
pub fn RadioGroup(
    group: &RadioGroupProps,
    legend: impl Into<String>,
    helper_text: impl Into<String>,
    options: &[RadioOption],
) -> Markup {
    let legend = legend.into();
    let helper_text = helper_text.into();
    html! {
        fieldset class="space-y-3" {
            (group_heading(group.required, &legend, &helper_text, group.error))
            div class=(group.get_orientation_classes()) {
                @for option in options {
                    (RadioWithLabel(&option_props(group, option), &option.label, &option.help_text))
                }
            }
            (group_error(group))
        }
    }
}

fn group_heading(required: bool, legend: &str, helper_text: &str, has_error: bool) -> Markup {
    html! {
        @if !legend.is_empty() {
            legend class="text-sm font-medium leading-none mb-3" {
                (legend)
                @if required {
                    span class="text-destructive ml-1" aria-label=(shared::default_strings().input_required_aria_label) {
                        (shared::default_strings().input_required_symbol)
                    }
                }
            }
        }
        @if !helper_text.is_empty() && !has_error {
            p class="text-sm text-muted-foreground mb-3" { (helper_text) }
        }
    }
}

fn group_error(group: &RadioGroupProps) -> Markup {
    html! {
        @if group.error && !group.error_message.is_empty() {
            p class="text-sm text-destructive mt-2" role="alert" { (&group.error_message) }
        }
    }
}

#[must_use]
pub fn RadioCard(
    props: &RadioProps,
    title: impl Into<String>,
    description: impl Into<String>,
) -> Markup {
    let title = title.into();
    let description = description.into();
    html! {
        label
            for=[shared::optional_attr(&props.id)]
            class="relative flex cursor-pointer rounded-lg border border-border p-4 hover:bg-accent/50 transition-colors duration-150"
        {
            div class="flex items-start space-x-3 w-full" {
                div class="flex items-center h-5 mt-0.5" { (Radio(props)) }
                div class="flex-1" {
                    @if !title.is_empty() {
                        p class="font-medium text-sm" {
                            (&title)
                            @if props.required {
                                span class="text-destructive ml-1" aria-label=(shared::default_strings().input_required_aria_label) {
                                    (shared::default_strings().input_required_symbol)
                                }
                            }
                        }
                    }
                    @if !description.is_empty() {
                        p class="text-sm text-muted-foreground mt-1" { (&description) }
                    }
                }
            }
        }
    }
}

struct RawHtml<'a>(&'a str);

impl Render for RawHtml<'_> {
    fn render_to(&self, output: &mut String) {
        output.push_str(self.0);
    }
}

#[must_use]
pub fn RadioCardRich(
    props: &RadioProps,
    label_html: impl Into<String>,
    description_html: impl Into<String>,
) -> Markup {
    let label_html = label_html.into();
    let description_html = description_html.into();
    html! {
        label
            for=[shared::optional_attr(&props.id)]
            class="relative flex cursor-pointer rounded-lg border border-border p-4 hover:bg-accent/50 transition-colors duration-150 has-[:checked]:border-primary has-[:checked]:bg-primary/5"
        {
            div class="flex items-start space-x-3 w-full" {
                div class="flex items-center h-5 mt-0.5" { (Radio(props)) }
                div class="flex-1 prose prose-sm dark:prose-invert max-w-none rich-content [&>p]:m-0 [&>pre]:my-1" data-rich-content="true" {
                    @if !label_html.is_empty() {
                        (RawHtml(&label_html))
                    }
                    @if !description_html.is_empty() {
                        div class="text-muted-foreground mt-1" {
                            (RawHtml(&description_html))
                        }
                    }
                }
            }
        }
    }
}

#[must_use]
pub fn RadioGroupCards(
    group: &RadioGroupProps,
    legend: impl Into<String>,
    helper_text: impl Into<String>,
    options: &[RadioOption],
) -> Markup {
    let legend = legend.into();
    let helper_text = helper_text.into();
    html! {
        fieldset class="space-y-3" {
            (group_heading(group.required, &legend, &helper_text, group.error))
            div class="space-y-2" {
                @for option in options {
                    @if !option.label_html.is_empty() {
                        (RadioCardRich(&option_props(group, option), &option.label_html, &option.help_text))
                    } @else {
                        (RadioCard(&option_props(group, option), &option.label, &option.help_text))
                    }
                }
            }
            (group_error(group))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_helpers_and_attrs_match_go_behavior() {
        assert_eq!(RadioProps::default().size, shared::SIZE_MD);
        assert_eq!(RadioGroupProps::default().orientation, "vertical");
        assert_eq!(
            RadioGroupProps::default().get_orientation_classes(),
            "space-y-2"
        );
        assert_eq!(
            RadioGroupProps::default()
                .with_orientation("horizontal")
                .get_orientation_classes(),
            "flex flex-wrap gap-4"
        );
        let group_builder = RadioGroupProps::default()
            .with_size(shared::SIZE_LG)
            .with_class("group-extra");
        assert_eq!(group_builder.size, shared::SIZE_LG);
        assert_eq!(group_builder.class, "group-extra");
        assert_eq!(RadioProps::default().get_size_classes(), "h-5 w-5");
        assert_eq!(
            RadioProps::default()
                .with_size(shared::SIZE_SM)
                .get_size_classes(),
            "h-4 w-4"
        );
        assert_eq!(
            RadioProps::default()
                .with_size(shared::SIZE_LG)
                .get_size_classes(),
            "h-6 w-6"
        );

        let props = RadioProps::default()
            .with_name("plan")
            .with_id("plan-basic")
            .with_value("basic")
            .with_checked(true)
            .with_disabled(true)
            .with_required(true)
            .with_error(true)
            .with_class("extra")
            .with_aria_label("Plan")
            .with_aria_described_by("help")
            .with_attribute("data-on:change", "$plan = 'basic'");
        assert!(
            props
                .build_classes()
                .contains("border-destructive focus-visible:ring-destructive extra")
        );
        let attrs = props.build_attributes();
        assert_eq!(attrs.get("aria-label").map(String::as_str), Some("Plan"));
        assert_eq!(
            attrs.get("aria-describedby").map(String::as_str),
            Some("help")
        );
        assert_eq!(attrs.get("aria-invalid").map(String::as_str), Some("true"));
        assert_eq!(
            attrs.get("data-on:change").map(String::as_str),
            Some("$plan = 'basic'")
        );
    }

    #[test]
    fn radio_renders_input_and_label_variations() {
        let props = RadioProps::default()
            .with_name("plan")
            .with_id("plan-basic")
            .with_value("<basic>")
            .with_checked(true)
            .with_disabled(true)
            .with_required(true)
            .with_error(true)
            .with_attribute("data-testid", "radio")
            .with_attribute("data-on:change", "$plan = 'basic'");
        let html = Radio(&props).into_string();
        assert!(html.starts_with("<input type=\"radio\""));
        assert!(html.contains("name=\"plan\""));
        assert!(html.contains("id=\"plan-basic\""));
        assert!(html.contains("value=\"&lt;basic&gt;\""));
        assert!(html.contains("checked"));
        assert!(html.contains("disabled"));
        assert!(html.contains("required"));
        assert!(html.contains("aria-invalid=\"true\""));
        assert!(html.contains("data-on:change=\"$plan = 'basic'\""));

        let labeled = RadioWithLabel(&props, "<Basic>", "Help").into_string();
        assert!(labeled.contains("for=\"plan-basic\""));
        assert!(labeled.contains("&lt;Basic&gt;"));
        assert!(labeled.contains("aria-label=\"required\">*</span>"));
        assert!(labeled.contains("<p class=\"text-sm text-muted-foreground mt-1\">Help</p>"));
    }

    #[test]
    fn groups_and_cards_render_options_errors_raw_html_and_datastar_attrs() {
        let group = RadioGroupProps::default()
            .with_name("plan")
            .with_required(true)
            .with_error(true, "<Required>")
            .with_orientation("horizontal");
        let options = vec![
            RadioOption::default()
                .with_value("a")
                .with_label("A")
                .with_checked(true)
                .with_attribute("data-on:change", "$plan = 'a'"),
            RadioOption::default()
                .with_value("b")
                .with_label("B")
                .with_disabled(true)
                .with_help_text("Help B"),
        ];
        let html = RadioGroup(&group, "Legend", "Hidden help", &options).into_string();
        assert!(html.starts_with("<fieldset class=\"space-y-3\">"));
        assert!(html.contains("Legend"));
        assert!(!html.contains("Hidden help"));
        assert!(html.contains("class=\"flex flex-wrap gap-4\""));
        assert!(html.contains("id=\"plan-a\""));
        assert!(html.contains("data-on:change=\"$plan = 'a'\""));
        assert!(html.contains("role=\"alert\">&lt;Required&gt;</p>"));

        let card = RadioCard(
            &RadioProps::default().with_id("x").with_required(true),
            "Title",
            "Desc",
        )
        .into_string();
        assert!(card.contains("relative flex cursor-pointer rounded-lg border border-border p-4"));
        assert!(card.contains("<p class=\"font-medium text-sm\">Title"));
        assert!(card.contains("<p class=\"text-sm text-muted-foreground mt-1\">Desc</p>"));

        let rich = RadioCardRich(
            &RadioProps::default().with_id("rich"),
            "<strong>Raw</strong>",
            "<em>Desc</em>",
        )
        .into_string();
        assert!(rich.contains("data-rich-content=\"true\""));
        assert!(rich.contains("<strong>Raw</strong>"));
        assert!(rich.contains("<div class=\"text-muted-foreground mt-1\"><em>Desc</em></div>"));

        let card_group = RadioGroupCards(
            &RadioGroupProps::default().with_name("card"),
            "Cards",
            "Pick one",
            &[RadioOption::default()
                .with_value("x")
                .with_label_html("<strong>X</strong>")
                .with_help_text("<em>Y</em>")],
        )
        .into_string();
        assert!(card_group.contains("<strong>X</strong>"));
        assert!(card_group.contains("<em>Y</em>"));

        let plain_card_group = RadioGroupCards(
            &RadioGroupProps::default().with_name("cards"),
            "Cards",
            "",
            &[RadioOption::default()
                .with_value("plain")
                .with_label("Plain")],
        )
        .into_string();
        assert!(plain_card_group.contains("id=\"cards-plain\""));
        assert!(plain_card_group.contains(">Plain</p>"));
    }
}
