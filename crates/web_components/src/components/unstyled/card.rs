#![allow(non_snake_case)]

use crate::components::{ComponentInfo, shared};
use maud::{Markup, html};
use std::collections::BTreeMap;

pub const COMPONENTS: &[ComponentInfo] = &[
    ComponentInfo {
        family: "unstyled",
        module: "card",
        name: "Card",
    },
    ComponentInfo {
        family: "unstyled",
        module: "card",
        name: "CardHeader",
    },
    ComponentInfo {
        family: "unstyled",
        module: "card",
        name: "CardTitle",
    },
    ComponentInfo {
        family: "unstyled",
        module: "card",
        name: "CardDescription",
    },
    ComponentInfo {
        family: "unstyled",
        module: "card",
        name: "CardContent",
    },
    ComponentInfo {
        family: "unstyled",
        module: "card",
        name: "CardFooter",
    },
    ComponentInfo {
        family: "unstyled",
        module: "card",
        name: "SimpleCard",
    },
    ComponentInfo {
        family: "unstyled",
        module: "card",
        name: "CardWithFooter",
    },
    ComponentInfo {
        family: "unstyled",
        module: "card",
        name: "InteractiveCard",
    },
    ComponentInfo {
        family: "unstyled",
        module: "card",
        name: "LinkCard",
    },
    ComponentInfo {
        family: "unstyled",
        module: "card",
        name: "StatsCard",
    },
    ComponentInfo {
        family: "unstyled",
        module: "card",
        name: "FeatureCard",
    },
    ComponentInfo {
        family: "unstyled",
        module: "card",
        name: "ProductCard",
    },
];

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CardProps {
    pub class: String,
    pub attributes: BTreeMap<String, String>,
    pub interactive: bool,
    pub hoverable: bool,
    pub aria_label: String,
    pub aria_described_by: String,
    pub role: String,
}

impl Default for CardProps {
    fn default() -> Self {
        Self {
            class: String::new(),
            attributes: BTreeMap::new(),
            interactive: false,
            hoverable: false,
            aria_label: String::new(),
            aria_described_by: String::new(),
            role: String::new(),
        }
    }
}

impl CardProps {
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
    pub const fn with_interactive(mut self, interactive: bool) -> Self {
        self.interactive = interactive;
        self
    }

    #[must_use]
    pub const fn with_hoverable(mut self, hoverable: bool) -> Self {
        self.hoverable = hoverable;
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
    pub fn with_role(mut self, role: impl Into<String>) -> Self {
        self.role = role.into();
        self
    }

    #[must_use]
    pub fn build_classes(&self) -> String {
        shared::build_classes(&[
            "rounded-lg border bg-card text-card-foreground shadow-sm transition-all duration-200 ease-out",
            if self.interactive {
                "cursor-pointer focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 hover:border-primary/20 hover:scale-[1.005] hover:shadow-md active:scale-[0.995]"
            } else {
                ""
            },
            if self.hoverable && !self.interactive {
                "hover:shadow-md hover:border-primary/20"
            } else {
                ""
            },
            self.class.as_str(),
        ])
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
        if !self.role.is_empty() {
            attrs.insert("role".to_string(), self.role.clone());
        }
        if self.interactive && !attrs.contains_key("tabindex") {
            attrs.insert("tabindex".to_string(), "0".to_string());
        }
        attrs
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Default)]
pub struct CardHeaderProps {
    pub class: String,
    pub attributes: BTreeMap<String, String>,
}

impl CardHeaderProps {
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
        shared::build_classes(&["flex flex-col space-y-1.5 p-6", self.class.as_str()])
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CardTitleProps {
    pub level: &'static str,
    pub class: String,
    pub attributes: BTreeMap<String, String>,
}

impl Default for CardTitleProps {
    fn default() -> Self {
        Self {
            level: shared::HEADING_H3,
            class: String::new(),
            attributes: BTreeMap::new(),
        }
    }
}

impl CardTitleProps {
    #[must_use]
    pub fn with_level(mut self, level: &'static str) -> Self {
        self.level = level;
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
        shared::build_classes(&[
            "text-2xl font-semibold leading-none tracking-tight",
            self.class.as_str(),
        ])
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Default)]
pub struct CardDescriptionProps {
    pub class: String,
    pub attributes: BTreeMap<String, String>,
}

impl CardDescriptionProps {
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
        shared::build_classes(&["text-sm text-muted-foreground", self.class.as_str()])
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Default)]
pub struct CardContentProps {
    pub class: String,
    pub attributes: BTreeMap<String, String>,
    pub no_padding: bool,
}

impl CardContentProps {
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
    pub const fn with_no_padding(mut self, no_padding: bool) -> Self {
        self.no_padding = no_padding;
        self
    }

    #[must_use]
    pub fn build_classes(&self) -> String {
        shared::build_classes(&[
            if self.no_padding { "" } else { "p-6 pt-0" },
            self.class.as_str(),
        ])
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CardFooterProps {
    pub class: String,
    pub attributes: BTreeMap<String, String>,
    pub align: &'static str,
}

impl Default for CardFooterProps {
    fn default() -> Self {
        Self {
            class: String::new(),
            attributes: BTreeMap::new(),
            align: "left",
        }
    }
}

impl CardFooterProps {
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
    pub fn with_align(mut self, align: &'static str) -> Self {
        self.align = align;
        self
    }

    #[must_use]
    pub fn build_classes(&self) -> String {
        let align_classes = match self.align {
            "center" => "justify-center",
            "right" => "justify-end",
            "between" => "justify-between",
            _ => "justify-start",
        };
        shared::build_classes(&[
            "flex items-center p-6 pt-0",
            align_classes,
            self.class.as_str(),
        ])
    }
}

#[must_use]
pub fn Card(props: &CardProps, children: Markup) -> Markup {
    let attrs = props.build_attributes();
    html! {
        div
            class=(props.build_classes())
            id=[shared::attr_value(&attrs, "id")]
            title=[shared::attr_value(&attrs, "title")]
            role=[shared::attr_value(&attrs, "role")]
            tabindex=[shared::attr_value(&attrs, "tabindex")]
            aria-label=[shared::attr_value(&attrs, "aria-label")]
            aria-describedby=[shared::attr_value(&attrs, "aria-describedby")]
            onclick=[shared::attr_value(&attrs, "onclick")]
            data-on:click=[shared::attr_value(&attrs, "data-on:click")]
            data-show=[shared::attr_value(&attrs, "data-show")]
            data-bind=[shared::attr_value(&attrs, "data-bind")]
            data-signals=[shared::attr_value(&attrs, "data-signals")]
        {
            (children)
        }
    }
}

#[must_use]
pub fn CardHeader(props: &CardHeaderProps, children: Markup) -> Markup {
    html! {
        div
            class=(props.build_classes())
            id=[shared::attr_value(&props.attributes, "id")]
            title=[shared::attr_value(&props.attributes, "title")]
            data-show=[shared::attr_value(&props.attributes, "data-show")]
        {
            (children)
        }
    }
}

fn render_title_tag(tag: &str, props: &CardTitleProps, children: Markup) -> Markup {
    let classes = props.build_classes();
    match tag {
        shared::HEADING_H1 => html! {
            h1 class=(classes) id=[shared::attr_value(&props.attributes, "id")] title=[shared::attr_value(&props.attributes, "title")] { (children) }
        },
        shared::HEADING_H2 => html! {
            h2 class=(classes) id=[shared::attr_value(&props.attributes, "id")] title=[shared::attr_value(&props.attributes, "title")] { (children) }
        },
        shared::HEADING_H4 => html! {
            h4 class=(classes) id=[shared::attr_value(&props.attributes, "id")] title=[shared::attr_value(&props.attributes, "title")] { (children) }
        },
        shared::HEADING_H5 => html! {
            h5 class=(classes) id=[shared::attr_value(&props.attributes, "id")] title=[shared::attr_value(&props.attributes, "title")] { (children) }
        },
        shared::HEADING_H6 => html! {
            h6 class=(classes) id=[shared::attr_value(&props.attributes, "id")] title=[shared::attr_value(&props.attributes, "title")] { (children) }
        },
        _ => html! {
            h3 class=(classes) id=[shared::attr_value(&props.attributes, "id")] title=[shared::attr_value(&props.attributes, "title")] { (children) }
        },
    }
}

#[must_use]
pub fn CardTitle(props: &CardTitleProps, children: Markup) -> Markup {
    render_title_tag(props.level, props, children)
}

#[must_use]
pub fn CardDescription(props: &CardDescriptionProps, children: Markup) -> Markup {
    html! {
        p
            class=(props.build_classes())
            id=[shared::attr_value(&props.attributes, "id")]
            title=[shared::attr_value(&props.attributes, "title")]
        {
            (children)
        }
    }
}

#[must_use]
pub fn CardContent(props: &CardContentProps, children: Markup) -> Markup {
    html! {
        div
            class=(props.build_classes())
            id=[shared::attr_value(&props.attributes, "id")]
            title=[shared::attr_value(&props.attributes, "title")]
            data-show=[shared::attr_value(&props.attributes, "data-show")]
        {
            (children)
        }
    }
}

#[must_use]
pub fn CardFooter(props: &CardFooterProps, children: Markup) -> Markup {
    html! {
        div
            class=(props.build_classes())
            id=[shared::attr_value(&props.attributes, "id")]
            title=[shared::attr_value(&props.attributes, "title")]
            data-show=[shared::attr_value(&props.attributes, "data-show")]
        {
            (children)
        }
    }
}

#[must_use]
pub fn SimpleCard(
    title: &str,
    description: &str,
    card_props: &CardProps,
    children: Markup,
) -> Markup {
    Card(
        card_props,
        html! {
            (CardHeader(&CardHeaderProps::default(), html! {
                (CardTitle(&CardTitleProps::default(), html! { (title) }))
                @if !description.is_empty() {
                    (CardDescription(&CardDescriptionProps::default(), html! { (description) }))
                }
            }))
            (CardContent(&CardContentProps::default(), children))
        },
    )
}

#[must_use]
pub fn CardWithFooter(
    title: &str,
    description: &str,
    footer_align: &'static str,
    children: Markup,
) -> Markup {
    Card(
        &CardProps::default(),
        html! {
            (CardHeader(&CardHeaderProps::default(), html! {
                (CardTitle(&CardTitleProps::default(), html! { (title) }))
                @if !description.is_empty() {
                    (CardDescription(&CardDescriptionProps::default(), html! { (description) }))
                }
            }))
            (CardContent(&CardContentProps::default(), children.clone()))
            (CardFooter(&CardFooterProps::default().with_align(footer_align), children))
        },
    )
}

#[must_use]
pub fn InteractiveCard(title: &str, description: &str, on_click: &str, children: Markup) -> Markup {
    Card(
        &CardProps::default()
            .with_interactive(true)
            .with_hoverable(true)
            .with_attribute("onclick", on_click),
        html! {
            (CardHeader(&CardHeaderProps::default(), html! {
                (CardTitle(&CardTitleProps::default(), html! { (title) }))
                @if !description.is_empty() {
                    (CardDescription(&CardDescriptionProps::default(), html! { (description) }))
                }
            }))
            (CardContent(&CardContentProps::default(), children))
        },
    )
}

#[must_use]
pub fn LinkCard(title: &str, description: &str, href: &str, children: Markup) -> Markup {
    html! {
        a href=(href) class="block focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 rounded-lg" {
            (Card(
                &CardProps::default().with_hoverable(true),
                html! {
                    (CardHeader(&CardHeaderProps::default(), html! {
                        (CardTitle(&CardTitleProps::default(), html! { (title) }))
                        @if !description.is_empty() {
                            (CardDescription(&CardDescriptionProps::default(), html! { (description) }))
                        }
                    }))
                    (CardContent(&CardContentProps::default(), children))
                },
            ))
        }
    }
}

#[must_use]
pub fn StatsCard(title: &str, value: &str, description: &str, trend: &str) -> Markup {
    Card(
        &CardProps::default(),
        CardContent(
            &CardContentProps::default().with_class("p-6"),
            html! {
                div class="space-y-2" {
                    p class="text-sm font-medium text-muted-foreground" { (title) }
                    div class="flex items-baseline gap-2" {
                        h3 class="text-3xl font-bold tracking-tight" { (value) }
                        @if !trend.is_empty() {
                            span class="text-xs font-medium text-muted-foreground" { (trend) }
                        }
                    }
                    @if !description.is_empty() {
                        p class="text-xs text-muted-foreground" { (description) }
                    }
                }
            },
        ),
    )
}

#[must_use]
pub fn FeatureCard(title: &str, description: &str, icon_class: &str, children: Markup) -> Markup {
    Card(
        &CardProps::default().with_hoverable(true),
        html! {
            (CardHeader(&CardHeaderProps::default(), html! {
                div class="w-12 h-12 rounded-lg bg-primary/10 flex items-center justify-center mb-4" {
                    div class=(icon_class) {}
                }
                (CardTitle(&CardTitleProps::default(), html! { (title) }))
                @if !description.is_empty() {
                    (CardDescription(&CardDescriptionProps::default(), html! { (description) }))
                }
            }))
            (CardContent(&CardContentProps::default(), children))
        },
    )
}

#[must_use]
pub fn ProductCard(
    title: &str,
    price: &str,
    description: &str,
    image_url: &str,
    children: Markup,
) -> Markup {
    Card(
        &CardProps::default().with_hoverable(true),
        html! {
            @if !image_url.is_empty() {
                div class="aspect-square overflow-hidden rounded-t-lg" {
                    img src=(image_url) alt=(title) class="w-full h-full object-cover transition-transform hover:scale-105";
                }
            }
            (CardHeader(&CardHeaderProps::default(), html! {
                (CardTitle(&CardTitleProps::default(), html! { (title) }))
                @if !price.is_empty() {
                    p class="text-lg font-bold text-primary" { (price) }
                }
                @if !description.is_empty() {
                    (CardDescription(&CardDescriptionProps::default(), html! { (description) }))
                }
            }))
            (CardFooter(&CardFooterProps::default().with_align("between"), children))
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const TITLE_LEVELS: &[(&str, &str)] = &[
        (shared::HEADING_H1, "h1"),
        (shared::HEADING_H2, "h2"),
        (shared::HEADING_H3, "h3"),
        (shared::HEADING_H4, "h4"),
        (shared::HEADING_H5, "h5"),
        (shared::HEADING_H6, "h6"),
        ("unknown", "h3"),
    ];
    const FOOTER_ALIGNS: &[(&str, &str)] = &[
        ("left", "justify-start"),
        ("center", "justify-center"),
        ("right", "justify-end"),
        ("between", "justify-between"),
        ("unknown", "justify-start"),
    ];

    #[test]
    fn default_props_match_go_defaults() {
        assert_eq!(
            CardProps::default(),
            CardProps::default().with_hoverable(false)
        );
        assert_eq!(CardTitleProps::default().level, shared::HEADING_H3);
        assert!(!CardContentProps::default().no_padding);
        assert_eq!(CardFooterProps::default().align, "left");
    }

    #[test]
    fn helper_classes_cover_card_state_combinations() {
        for interactive in [false, true] {
            for hoverable in [false, true] {
                let classes = CardProps::default()
                    .with_interactive(interactive)
                    .with_hoverable(hoverable)
                    .with_class("custom")
                    .build_classes();

                assert!(classes.contains("rounded-lg border bg-card"));
                assert!(classes.contains("custom"));
                assert_eq!(classes.contains("cursor-pointer"), interactive);
                assert_eq!(
                    classes.contains("hover:shadow-md hover:border-primary/20"),
                    hoverable && !interactive
                );
            }
        }
    }

    #[test]
    fn helper_attributes_cover_aria_role_tabindex_and_preserved_custom_tabindex() {
        let attrs = CardProps::default()
            .with_interactive(true)
            .with_aria_label("Open")
            .with_aria_described_by("hint")
            .with_role("button")
            .with_attribute("onclick", "doIt()")
            .build_attributes();

        assert_eq!(attrs.get("aria-label"), Some(&"Open".to_string()));
        assert_eq!(attrs.get("aria-describedby"), Some(&"hint".to_string()));
        assert_eq!(attrs.get("role"), Some(&"button".to_string()));
        assert_eq!(attrs.get("onclick"), Some(&"doIt()".to_string()));
        assert_eq!(attrs.get("tabindex"), Some(&"0".to_string()));

        let attrs = CardProps::default()
            .with_interactive(true)
            .with_attribute("tabindex", "-1")
            .build_attributes();
        assert_eq!(attrs.get("tabindex"), Some(&"-1".to_string()));
    }

    #[test]
    fn section_helpers_render_actual_html_and_escape_attributes() {
        let header = CardHeader(
            &CardHeaderProps::default()
                .with_class("extra")
                .with_attribute("title", "\"Header\" & more"),
            html! { "Head" },
        )
        .into_string();
        assert!(header.contains(r#"class="flex flex-col space-y-1.5 p-6 extra""#));
        assert!(header.contains(r#"title="&quot;Header&quot; &amp; more""#));
        assert!(header.ends_with(">Head</div>"));

        let description = CardDescription(
            &CardDescriptionProps::default()
                .with_class("desc")
                .with_attribute("id", "desc"),
            html! { "Description" },
        )
        .into_string();
        assert_eq!(
            description,
            r#"<p class="text-sm text-muted-foreground desc" id="desc">Description</p>"#
        );

        let content = CardContent(
            &CardContentProps::default()
                .with_no_padding(true)
                .with_class("tight")
                .with_attribute("id", "content"),
            html! { "Body" },
        )
        .into_string();
        assert_eq!(content, r#"<div class="tight" id="content">Body</div>"#);
    }

    #[test]
    fn title_renders_every_heading_level_and_default_branch() {
        for &(level, tag) in TITLE_LEVELS {
            let html = CardTitle(
                &CardTitleProps::default()
                    .with_level(level)
                    .with_class("headline")
                    .with_attribute("id", "title"),
                html! { "Title" },
            )
            .into_string();

            assert!(html.starts_with(&format!(r#"<{tag} class="text-2xl"#)));
            assert!(html.contains("headline"));
            assert!(html.contains(r#"id="title""#));
            assert!(html.ends_with(&format!(">Title</{tag}>")));
        }
    }

    #[test]
    fn footer_classes_and_html_cover_all_alignments() {
        for &(align, expected) in FOOTER_ALIGNS {
            let props = CardFooterProps::default()
                .with_align(align)
                .with_class("foot")
                .with_attribute("data-testid", "footer");
            assert!(props.build_classes().contains(expected));
            assert_eq!(
                props.attributes.get("data-testid").map(String::as_str),
                Some("footer")
            );

            let html = CardFooter(&props, html! { "Footer" }).into_string();
            assert!(html.contains(expected));
            assert!(html.contains("foot"));
            assert!(html.ends_with(">Footer</div>"));
        }
    }

    #[test]
    fn card_renders_interactive_attributes_datastar_and_children() {
        let html = Card(
            &CardProps::default()
                .with_interactive(true)
                .with_aria_label("Open")
                .with_attribute("data-on:click", "$open = true")
                .with_attribute("title", "\"Card\" & go"),
            html! { "<Body>" },
        )
        .into_string();

        assert!(html.starts_with(r#"<div class="rounded-lg border bg-card"#));
        assert!(html.contains("cursor-pointer"));
        assert!(html.contains(r#"tabindex="0""#));
        assert!(html.contains(r#"aria-label="Open""#));
        assert!(html.contains(r#"data-on:click="$open = true""#));
        assert!(html.contains(r#"title="&quot;Card&quot; &amp; go""#));
        assert!(html.contains("&lt;Body&gt;"));
    }

    #[test]
    fn convenience_cards_render_expected_conditional_content() {
        let simple = SimpleCard(
            "Hello",
            "World",
            &CardProps::default().with_class("outer"),
            html! { "Inner" },
        )
        .into_string();
        assert!(simple.contains("outer"));
        assert!(simple.contains(">Hello</h3>"));
        assert!(simple.contains(">World</p>"));
        assert!(simple.contains(">Inner</div>"));

        let no_description =
            SimpleCard("Hello", "", &CardProps::default(), html! { "Inner" }).into_string();
        assert!(!no_description.contains("text-sm text-muted-foreground"));

        let with_footer =
            CardWithFooter("Title", "Desc", "between", html! { "Repeated" }).into_string();
        assert_eq!(with_footer.matches("Repeated").count(), 2);
        assert!(with_footer.contains("justify-between"));

        let interactive = InteractiveCard("Click", "", "doIt()", html! { "Body" }).into_string();
        assert!(interactive.contains(r#"onclick="doIt()""#));
        assert!(interactive.contains("cursor-pointer"));
        let interactive_with_description =
            InteractiveCard("Click", "Details", "doIt()", html! {}).into_string();
        assert!(interactive_with_description.contains(">Details</p>"));

        let link = LinkCard("Go", "There", "/path", html! { "Body" }).into_string();
        assert!(link.starts_with(r#"<a href="/path""#));
        assert!(link.contains(">Go</h3>"));
        assert!(link.contains(">There</p>"));
    }

    #[test]
    fn stats_feature_and_product_cards_render_all_conditional_sections() {
        let stats = StatsCard("Users", "42", "Active", "+3%").into_string();
        assert!(stats.contains(">Users</p>"));
        assert!(stats.contains(">42</h3>"));
        assert!(stats.contains(">+3%</span>"));
        assert!(stats.contains(">Active</p>"));

        let stats_min = StatsCard("Users", "42", "", "").into_string();
        assert!(!stats_min.contains("text-xs text-muted-foreground"));

        let feature =
            FeatureCard("Fast", "Loads quickly", "i-speed", html! { "Body" }).into_string();
        assert!(feature.contains(r#"<div class="i-speed"></div>"#));
        assert!(feature.contains(">Fast</h3>"));
        assert!(feature.contains(">Loads quickly</p>"));

        let product =
            ProductCard("Plan", "$9", "Monthly", "/plan.png", html! { "Buy" }).into_string();
        assert!(product.contains(r#"<img src="/plan.png" alt="Plan" class="w-full h-full object-cover transition-transform hover:scale-105">"#));
        assert!(product.contains(r#"<p class="text-lg font-bold text-primary">$9</p>"#));
        assert!(product.contains("justify-between"));
        assert!(product.contains(">Buy</div>"));

        let product_min = ProductCard("Plan", "", "", "", html! { "Buy" }).into_string();
        assert!(!product_min.contains("<img "));
        assert!(!product_min.contains("text-lg font-bold text-primary"));
    }
}
