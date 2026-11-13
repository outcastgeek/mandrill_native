#![allow(non_snake_case)]

use crate::components::{ComponentInfo, shared};
use crate::components::unstyled::{card, heading};
use maud::{Markup, html};
use std::collections::BTreeMap;

pub const COMPONENTS: &[ComponentInfo] = &[
    ComponentInfo { family: "unstyled", module: "resourceui", name: "Toolbar" },
    ComponentInfo { family: "unstyled", module: "resourceui", name: "Split" },
    ComponentInfo { family: "unstyled", module: "resourceui", name: "StatGroup" },
    ComponentInfo { family: "unstyled", module: "resourceui", name: "StatChip" },
    ComponentInfo { family: "unstyled", module: "resourceui", name: "FilterGroup" },
    ComponentInfo { family: "unstyled", module: "resourceui", name: "FilterChip" },
    ComponentInfo { family: "unstyled", module: "resourceui", name: "Panel" },
    ComponentInfo { family: "unstyled", module: "resourceui", name: "ResultList" },
    ComponentInfo { family: "unstyled", module: "resourceui", name: "ResultRow" },
    ComponentInfo { family: "unstyled", module: "resourceui", name: "PreviewFrame" },
    ComponentInfo { family: "unstyled", module: "resourceui", name: "ActionGroup" },
    ComponentInfo { family: "unstyled", module: "resourceui", name: "PaginationBar" },
    ComponentInfo { family: "unstyled", module: "resourceui", name: "FormLayout" },
    ComponentInfo { family: "unstyled", module: "resourceui", name: "FormSection" },
];

#[derive(Clone, Debug, Default)]
pub struct SurfaceProps {
    pub class: String,
    pub attributes: BTreeMap<String, String>,
}

impl SurfaceProps {
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
    pub fn build_classes(&self, base: &str) -> String {
        shared::build_classes(&[base, &self.class])
    }

    #[must_use]
    pub fn build_attributes(&self) -> BTreeMap<String, String> {
        self.attributes.clone()
    }
}

#[derive(Clone, Debug, Default)]
pub struct ToolbarProps {
    pub class: String,
    pub attributes: BTreeMap<String, String>,
}

impl ToolbarProps {
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
    pub fn build_classes(&self) -> String {
        shared::build_classes(&["rounded-xl border border-border bg-card p-4 shadow-sm", &self.class])
    }

    #[must_use]
    pub fn build_attributes(&self) -> BTreeMap<String, String> {
        self.attributes.clone()
    }
}

#[derive(Clone, Debug, Default)]
pub struct SplitProps {
    pub class: String,
    pub attributes: BTreeMap<String, String>,
}

impl SplitProps {
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
    pub fn build_classes(&self) -> String {
        shared::build_classes(&["grid gap-4 lg:grid-cols-[minmax(0,1fr)_auto] lg:items-center", &self.class])
    }

    #[must_use]
    pub fn build_attributes(&self) -> BTreeMap<String, String> {
        self.attributes.clone()
    }
}

#[derive(Clone, Debug, Default)]
pub struct StatGroupProps {
    pub class: String,
    pub attributes: BTreeMap<String, String>,
}

impl StatGroupProps {
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
    pub fn build_classes(&self) -> String {
        shared::build_classes(&["flex flex-wrap gap-2", &self.class])
    }

    #[must_use]
    pub fn build_attributes(&self) -> BTreeMap<String, String> {
        self.attributes.clone()
    }
}

#[derive(Clone, Debug, Default)]
pub struct StatChipProps {
    pub label: String,
    pub value: String,
    pub icon: Option<Markup>,
    pub class: String,
    pub attributes: BTreeMap<String, String>,
}

impl StatChipProps {
    #[must_use]
    pub fn with_label(mut self, value: impl Into<String>) -> Self {
        self.label = value.into();
        self
    }

    #[must_use]
    pub fn with_value(mut self, value: impl Into<String>) -> Self {
        self.value = value.into();
        self
    }

    #[must_use]
    pub fn with_icon(mut self, value: Markup) -> Self {
        self.icon = Some(value);
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
    pub fn build_classes(&self) -> String {
        shared::build_classes(&["inline-flex min-h-9 items-center gap-2 rounded-full border border-border bg-background px-3 py-1.5 text-sm", &self.class])
    }

    #[must_use]
    pub fn build_attributes(&self) -> BTreeMap<String, String> {
        self.attributes.clone()
    }
}

#[derive(Clone, Debug, Default)]
pub struct FilterGroupProps {
    pub aria_label: String,
    pub class: String,
    pub attributes: BTreeMap<String, String>,
}

impl FilterGroupProps {
    #[must_use]
    pub fn with_aria_label(mut self, value: impl Into<String>) -> Self {
        self.aria_label = value.into();
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
    pub fn build_classes(&self) -> String {
        shared::build_classes(&["flex flex-wrap gap-2", &self.class])
    }

    #[must_use]
    pub fn build_attributes(&self) -> BTreeMap<String, String> {
        let mut attrs = self.attributes.clone();
        if !self.aria_label.is_empty() {
            attrs.insert("aria-label".into(), self.aria_label.clone());
        }
        attrs.entry("role".into()).or_insert_with(|| "group".into());
        attrs
    }
}

#[derive(Clone, Debug, Default)]
pub struct FilterChipProps {
    pub label: String,
    pub count: String,
    pub href: String,
    pub active: bool,
    pub class: String,
    pub attributes: BTreeMap<String, String>,
}

impl FilterChipProps {
    #[must_use]
    pub fn with_label(mut self, value: impl Into<String>) -> Self {
        self.label = value.into();
        self
    }

    #[must_use]
    pub fn with_count(mut self, value: impl Into<String>) -> Self {
        self.count = value.into();
        self
    }

    #[must_use]
    pub fn with_href(mut self, value: impl Into<String>) -> Self {
        self.href = value.into();
        self
    }

    #[must_use]
    pub fn with_active(mut self, value: bool) -> Self {
        self.active = value;
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
    pub fn build_classes(&self) -> String {
        shared::build_classes(&[
            "inline-flex min-h-9 items-center gap-2 rounded-full border px-3 py-1.5 text-sm font-medium no-underline transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2",
            if self.active { "border-primary bg-primary text-primary-foreground" } else { "border-border bg-card text-foreground hover:bg-muted" },
            &self.class,
        ])
    }

    #[must_use]
    pub fn build_attributes(&self) -> BTreeMap<String, String> {
        let mut attrs = self.attributes.clone();
        if self.active {
            attrs.insert("aria-current".into(), "true".into());
        }
        attrs
    }
}

#[derive(Clone, Debug, Default)]
pub struct PanelProps {
    pub title: String,
    pub description: String,
    pub header_class: String,
    pub body_class: String,
    pub no_padding: bool,
    pub class: String,
    pub attributes: BTreeMap<String, String>,
}

impl PanelProps {
    #[must_use]
    pub fn with_title(mut self, value: impl Into<String>) -> Self {
        self.title = value.into();
        self
    }

    #[must_use]
    pub fn with_description(mut self, value: impl Into<String>) -> Self {
        self.description = value.into();
        self
    }

    #[must_use]
    pub fn with_header_class(mut self, value: impl Into<String>) -> Self {
        self.header_class = value.into();
        self
    }

    #[must_use]
    pub fn with_body_class(mut self, value: impl Into<String>) -> Self {
        self.body_class = value.into();
        self
    }

    #[must_use]
    pub fn with_no_padding(mut self, value: bool) -> Self {
        self.no_padding = value;
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
    pub fn build_classes(&self) -> String {
        shared::build_classes(&["overflow-hidden rounded-xl border border-border bg-card text-card-foreground shadow-sm", &self.class])
    }

    #[must_use]
    pub fn build_header_classes(&self) -> String {
        shared::build_classes(&["border-b border-border bg-muted/40 px-4 py-3", &self.header_class])
    }

    #[must_use]
    pub fn build_body_classes(&self) -> String {
        shared::build_classes(&[if self.no_padding { "" } else { "p-4" }, &self.body_class])
    }

    #[must_use]
    pub fn build_attributes(&self) -> BTreeMap<String, String> {
        self.attributes.clone()
    }
}

#[derive(Clone, Debug, Default)]
pub struct ResultListProps {
    pub class: String,
    pub attributes: BTreeMap<String, String>,
}

impl ResultListProps {
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
    pub fn build_classes(&self) -> String {
        shared::build_classes(&["divide-y divide-border", &self.class])
    }

    #[must_use]
    pub fn build_attributes(&self) -> BTreeMap<String, String> {
        let mut attrs = self.attributes.clone();
        attrs.entry("role".into()).or_insert_with(|| "list".into());
        attrs
    }
}

#[derive(Clone, Debug, Default)]
pub struct ResultRowProps {
    pub title: String,
    pub title_href: String,
    pub secondary_text: String,
    pub meta_text: String,
    pub preview: Option<Markup>,
    pub badges: Option<Markup>,
    pub actions: Option<Markup>,
    pub class: String,
    pub attributes: BTreeMap<String, String>,
}

impl ResultRowProps {
    #[must_use]
    pub fn with_title(mut self, value: impl Into<String>) -> Self {
        self.title = value.into();
        self
    }

    #[must_use]
    pub fn with_title_href(mut self, value: impl Into<String>) -> Self {
        self.title_href = value.into();
        self
    }

    #[must_use]
    pub fn with_secondary_text(mut self, value: impl Into<String>) -> Self {
        self.secondary_text = value.into();
        self
    }

    #[must_use]
    pub fn with_meta_text(mut self, value: impl Into<String>) -> Self {
        self.meta_text = value.into();
        self
    }

    #[must_use]
    pub fn with_preview(mut self, value: Markup) -> Self {
        self.preview = Some(value);
        self
    }

    #[must_use]
    pub fn with_badges(mut self, value: Markup) -> Self {
        self.badges = Some(value);
        self
    }

    #[must_use]
    pub fn with_actions(mut self, value: Markup) -> Self {
        self.actions = Some(value);
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
    pub fn build_classes(&self) -> String {
        shared::build_classes(&["grid gap-4 p-4 transition-colors hover:bg-muted/40 md:grid-cols-[4.5rem_minmax(0,1fr)_auto]", &self.class])
    }

    #[must_use]
    pub fn build_attributes(&self) -> BTreeMap<String, String> {
        let mut attrs = self.attributes.clone();
        attrs.entry("role".into()).or_insert_with(|| "listitem".into());
        attrs
    }
}

#[derive(Clone, Debug, Default)]
pub struct PreviewFrameProps {
    pub src: String,
    pub alt: String,
    pub fallback: Option<Markup>,
    pub large: bool,
    pub decorative: bool,
    pub class: String,
    pub attributes: BTreeMap<String, String>,
}

impl PreviewFrameProps {
    #[must_use]
    pub fn with_src(mut self, value: impl Into<String>) -> Self {
        self.src = value.into();
        self
    }

    #[must_use]
    pub fn with_alt(mut self, value: impl Into<String>) -> Self {
        self.alt = value.into();
        self
    }

    #[must_use]
    pub fn with_fallback(mut self, value: Markup) -> Self {
        self.fallback = Some(value);
        self
    }

    #[must_use]
    pub fn with_large(mut self, value: bool) -> Self {
        self.large = value;
        self
    }

    #[must_use]
    pub fn with_decorative(mut self, value: bool) -> Self {
        self.decorative = value;
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
    pub fn build_classes(&self) -> String {
        shared::build_classes(&[
            "flex items-center justify-center overflow-hidden rounded-lg border border-border bg-background text-muted-foreground",
            if self.large { "aspect-video w-full" } else { "h-[72px] w-[72px]" },
            &self.class,
        ])
    }

    #[must_use]
    pub fn build_attributes(&self) -> BTreeMap<String, String> {
        let mut attrs = self.attributes.clone();
        if self.decorative {
            attrs.insert("aria-hidden".into(), "true".into());
        }
        attrs
    }
}

#[derive(Clone, Debug, Default)]
pub struct ActionGroupProps {
    pub class: String,
    pub attributes: BTreeMap<String, String>,
}

impl ActionGroupProps {
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
    pub fn build_classes(&self) -> String {
        shared::build_classes(&["flex flex-wrap items-start justify-start gap-2 md:justify-end", &self.class])
    }

    #[must_use]
    pub fn build_attributes(&self) -> BTreeMap<String, String> {
        self.attributes.clone()
    }
}

#[derive(Clone, Debug, Default)]
pub struct PaginationBarProps {
    pub aria_label: String,
    pub class: String,
    pub attributes: BTreeMap<String, String>,
}

impl PaginationBarProps {
    #[must_use]
    pub fn with_aria_label(mut self, value: impl Into<String>) -> Self {
        self.aria_label = value.into();
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
    pub fn build_classes(&self) -> String {
        shared::build_classes(&["flex flex-wrap items-center justify-end gap-2 border-t border-border p-4", &self.class])
    }

    #[must_use]
    pub fn build_attributes(&self) -> BTreeMap<String, String> {
        let mut attrs = self.attributes.clone();
        if !self.aria_label.is_empty() {
            attrs.insert("aria-label".into(), self.aria_label.clone());
        }
        attrs.entry("role".into()).or_insert_with(|| "navigation".into());
        attrs
    }
}

#[derive(Clone, Debug, Default)]
pub struct FormLayoutProps {
    pub class: String,
    pub attributes: BTreeMap<String, String>,
}

impl FormLayoutProps {
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
    pub fn build_classes(&self) -> String {
        shared::build_classes(&["grid grid-cols-1 gap-6 lg:grid-cols-[minmax(0,1fr)_22rem]", &self.class])
    }

    #[must_use]
    pub fn build_attributes(&self) -> BTreeMap<String, String> {
        self.attributes.clone()
    }
}

#[derive(Clone, Debug, Default)]
pub struct FormSectionProps {
    pub title: String,
    pub description: String,
    pub body_class: String,
    pub class: String,
    pub attributes: BTreeMap<String, String>,
}

impl FormSectionProps {
    #[must_use]
    pub fn with_title(mut self, value: impl Into<String>) -> Self {
        self.title = value.into();
        self
    }

    #[must_use]
    pub fn with_description(mut self, value: impl Into<String>) -> Self {
        self.description = value.into();
        self
    }

    #[must_use]
    pub fn with_body_class(mut self, value: impl Into<String>) -> Self {
        self.body_class = value.into();
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
    pub fn build_classes(&self) -> String {
        shared::build_classes(&["space-y-4 rounded-xl border border-border bg-background/60 p-4", &self.class])
    }

    #[must_use]
    pub fn build_body_classes(&self) -> String {
        shared::build_classes(&["space-y-4", &self.body_class])
    }

    #[must_use]
    pub fn build_attributes(&self) -> BTreeMap<String, String> {
        self.attributes.clone()
    }
}

macro_rules! resource_element {
    ($tag:ident, $props:expr, $children:expr) => {{
        let props = $props;
        let attrs = props.build_attributes();
        html! {
            $tag
                class=(props.build_classes())
                ..(&attrs)
            { ($children) }
        }
    }};
}

#[must_use]
pub fn Toolbar(props: &ToolbarProps, children: Markup) -> Markup {
    resource_element!(section, props, children)
}

#[must_use]
pub fn Split(props: &SplitProps, children: Markup) -> Markup {
    resource_element!(div, props, children)
}

#[must_use]
pub fn StatGroup(props: &StatGroupProps, children: Markup) -> Markup {
    resource_element!(div, props, children)
}

#[must_use]
pub fn FilterGroup(props: &FilterGroupProps, children: Markup) -> Markup {
    resource_element!(div, props, children)
}

#[must_use]
pub fn ResultList(props: &ResultListProps, children: Markup) -> Markup {
    resource_element!(div, props, children)
}

#[must_use]
pub fn ActionGroup(props: &ActionGroupProps, children: Markup) -> Markup {
    resource_element!(div, props, children)
}

#[must_use]
pub fn PaginationBar(props: &PaginationBarProps, children: Markup) -> Markup {
    resource_element!(nav, props, children)
}

#[must_use]
pub fn FormLayout(props: &FormLayoutProps, children: Markup) -> Markup {
    resource_element!(div, props, children)
}

#[must_use]
pub fn StatChip(props: &StatChipProps) -> Markup {
    resource_element!(div, props, html! {
        @if let Some(icon) = &props.icon {
            span class="text-primary" aria-hidden="true" { (icon) }
        }
        span class="text-muted-foreground" { (&props.label) }
        span class="font-semibold text-foreground" { (&props.value) }
    })
}

#[must_use]
pub fn FilterChip(props: &FilterChipProps) -> Markup {
    let contents = html! {
        span { (&props.label) }
        @if !props.count.trim().is_empty() {
            span class="rounded-full bg-background px-2 py-0.5 text-xs font-semibold text-foreground" { (&props.count) }
        }
    };
    if props.href.trim().is_empty() {
        resource_element!(span, props, contents)
    } else {
        let attrs = props.build_attributes();
        html! {
            a href=(&props.href) class=(props.build_classes())
                ..(&attrs)
            { (contents) }
        }
    }
}

#[must_use]
pub fn Panel(props: &PanelProps, children: Markup) -> Markup {
    card::Card(
        &card::CardProps {
            class: props.build_classes(),
            attributes: props.build_attributes(),
            ..card::CardProps::default()
        },
        html! {
            @if !props.title.trim().is_empty() || !props.description.trim().is_empty() {
                div class=(props.build_header_classes()) {
                    @if !props.title.trim().is_empty() {
                        (heading::H2("lg", false, "font-semibold", html! { (&props.title) }))
                    }
                    @if !props.description.trim().is_empty() {
                        p class="mt-0.5 text-sm text-muted-foreground" { (&props.description) }
                    }
                }
            }
            (card::CardContent(
                &card::CardContentProps {
                    no_padding: props.no_padding,
                    class: props.build_body_classes(),
                    ..card::CardContentProps::default()
                },
                children,
            ))
        },
    )
}

#[must_use]
pub fn ResultRow(props: &ResultRowProps) -> Markup {
    resource_element!(div, props, html! {
        @if let Some(preview) = &props.preview { (preview) }
        div class="min-w-0" {
            @if !props.title_href.trim().is_empty() {
                a href=(&props.title_href) class="break-words font-semibold text-foreground no-underline [overflow-wrap:anywhere] hover:text-primary" {
                    (&props.title)
                }
            } @else {
                p class="break-words font-semibold text-foreground [overflow-wrap:anywhere]" { (&props.title) }
            }
            @if !props.secondary_text.trim().is_empty() {
                p class="mt-1 break-words font-mono text-xs text-muted-foreground [overflow-wrap:anywhere]" { (&props.secondary_text) }
            }
            @if !props.meta_text.trim().is_empty() {
                p class="mt-1 text-sm text-muted-foreground" { (&props.meta_text) }
            }
            @if let Some(badges) = &props.badges { (badges) }
        }
        @if let Some(actions) = &props.actions { (actions) }
    })
}

#[must_use]
pub fn PreviewFrame(props: &PreviewFrameProps) -> Markup {
    resource_element!(div, props, html! {
        @if !props.src.trim().is_empty() {
            img src=(&props.src) alt=(&props.alt) class="h-full w-full object-cover";
        } @else if let Some(fallback) = &props.fallback {
            (fallback)
        }
    })
}

#[must_use]
pub fn FormSection(props: &FormSectionProps, children: Markup) -> Markup {
    resource_element!(section, props, html! {
        @if !props.title.trim().is_empty() || !props.description.trim().is_empty() {
            div class="space-y-1" {
                @if !props.title.trim().is_empty() {
                    (heading::H2("lg", false, "font-semibold", html! { (&props.title) }))
                }
                @if !props.description.trim().is_empty() {
                    p class="text-sm text-muted-foreground" { (&props.description) }
                }
            }
        }
        div class=(props.build_body_classes()) { (children) }
    })
}
