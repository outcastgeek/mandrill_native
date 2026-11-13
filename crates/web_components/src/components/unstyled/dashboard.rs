#![allow(non_snake_case)]

use crate::components::{ComponentInfo, shared};
use maud::{Markup, html};
use std::collections::BTreeMap;

type Attrs = BTreeMap<String, String>;

pub const COMPONENTS: &[ComponentInfo] = &[
    ComponentInfo {
        family: "unstyled",
        module: "dashboard",
        name: "DashboardHeader",
    },
    ComponentInfo {
        family: "unstyled",
        module: "dashboard",
        name: "AIAnalysisPanel",
    },
    ComponentInfo {
        family: "unstyled",
        module: "dashboard",
        name: "DataTable",
    },
    ComponentInfo {
        family: "unstyled",
        module: "dashboard",
        name: "HistoryItem",
    },
    ComponentInfo {
        family: "unstyled",
        module: "dashboard",
        name: "historyItemContent",
    },
    ComponentInfo {
        family: "unstyled",
        module: "dashboard",
        name: "HistoryPanel",
    },
    ComponentInfo {
        family: "unstyled",
        module: "dashboard",
        name: "SimpleDashboardHeader",
    },
    ComponentInfo {
        family: "unstyled",
        module: "dashboard",
        name: "SimpleAIAnalysisPanel",
    },
    ComponentInfo {
        family: "unstyled",
        module: "dashboard",
        name: "SimpleHistoryPanel",
    },
    ComponentInfo {
        family: "unstyled",
        module: "dashboard",
        name: "QuickActions",
    },
    ComponentInfo {
        family: "unstyled",
        module: "dashboard",
        name: "QuickActionCard",
    },
    ComponentInfo {
        family: "unstyled",
        module: "dashboard",
        name: "quickActionInner",
    },
    ComponentInfo {
        family: "unstyled",
        module: "dashboard",
        name: "PlanSummaryCard",
    },
    ComponentInfo {
        family: "unstyled",
        module: "dashboard",
        name: "ChecklistCard",
    },
    ComponentInfo {
        family: "unstyled",
        module: "dashboard",
        name: "checklistStep",
    },
    ComponentInfo {
        family: "unstyled",
        module: "dashboard",
        name: "dashboardActionButton",
    },
    ComponentInfo {
        family: "unstyled",
        module: "dashboard",
        name: "PreviewListCard",
    },
    ComponentInfo {
        family: "unstyled",
        module: "dashboard",
        name: "previewListItem",
    },
    ComponentInfo {
        family: "unstyled",
        module: "dashboard",
        name: "previewListItemCard",
    },
    ComponentInfo {
        family: "unstyled",
        module: "dashboard",
        name: "analysisStatusIcon",
    },
];

macro_rules! attrs_impl {
    ($name:ident) => {
        impl $name {
            #[must_use]
            pub fn with_class(mut self, value: impl Into<String>) -> Self {
                self.class = value.into();
                self
            }

            #[must_use]
            pub fn with_attribute(
                mut self,
                name: impl Into<String>,
                value: impl Into<String>,
            ) -> Self {
                self.attributes.insert(name.into(), value.into());
                self
            }

            #[must_use]
            pub fn build_attributes(&self) -> Attrs {
                self.attributes.clone()
            }
        }
    };
}

fn attr<'a>(attrs: &'a Attrs, name: &str) -> Option<&'a str> {
    shared::attr_value(attrs, name)
}

fn optional(value: &str) -> Option<&str> {
    if value.is_empty() { None } else { Some(value) }
}

#[derive(Clone, Debug, Eq, PartialEq, Default)]
pub struct DashboardHeaderProps {
    pub title: String,
    pub description: String,
    pub show_search: bool,
    pub show_actions: bool,
    pub class: String,
    pub attributes: Attrs,
}

attrs_impl!(DashboardHeaderProps);

impl DashboardHeaderProps {
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
    pub const fn with_search(mut self, value: bool) -> Self {
        self.show_search = value;
        self
    }
    #[must_use]
    pub const fn with_actions(mut self, value: bool) -> Self {
        self.show_actions = value;
        self
    }
    #[must_use]
    pub fn build_classes(&self) -> String {
        shared::build_classes(&[
            "flex flex-col gap-4 pb-6 border-b border-border md:flex-row md:items-center md:justify-between",
            self.class.as_str(),
        ])
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AIAnalysisPanelProps {
    pub title: String,
    pub status: String,
    pub result_text: String,
    pub confidence: i32,
    pub show_actions: bool,
    pub class: String,
    pub attributes: Attrs,
}

impl Default for AIAnalysisPanelProps {
    fn default() -> Self {
        Self {
            title: String::new(),
            status: "idle".to_string(),
            result_text: String::new(),
            confidence: 0,
            show_actions: false,
            class: String::new(),
            attributes: Attrs::new(),
        }
    }
}

attrs_impl!(AIAnalysisPanelProps);

impl AIAnalysisPanelProps {
    #[must_use]
    pub fn with_title(mut self, value: impl Into<String>) -> Self {
        self.title = value.into();
        self
    }
    #[must_use]
    pub fn with_status(mut self, value: impl Into<String>) -> Self {
        self.status = value.into();
        self
    }
    #[must_use]
    pub fn with_result(mut self, text: impl Into<String>, confidence: i32) -> Self {
        self.result_text = text.into();
        self.confidence = confidence;
        self
    }
    #[must_use]
    pub const fn is_analyzing(&self) -> bool {
        matches!(self.status.as_bytes(), b"analyzing")
    }
    #[must_use]
    pub const fn is_complete(&self) -> bool {
        matches!(self.status.as_bytes(), b"complete")
    }
    #[must_use]
    pub const fn is_error(&self) -> bool {
        matches!(self.status.as_bytes(), b"error")
    }
    #[must_use]
    pub fn build_classes(&self) -> String {
        shared::build_classes(&["rounded-xl border border-border p-6", self.class.as_str()])
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Default)]
pub struct DataTableColumn {
    pub key: String,
    pub label: String,
    pub sortable: bool,
    pub width: String,
    pub align: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DataTableProps {
    pub columns: Vec<DataTableColumn>,
    pub rows: Vec<BTreeMap<String, String>>,
    pub sortable: bool,
    pub striped: bool,
    pub bordered: bool,
    pub hoverable: bool,
    pub class: String,
    pub attributes: Attrs,
}

impl Default for DataTableProps {
    fn default() -> Self {
        Self {
            columns: Vec::new(),
            rows: Vec::new(),
            sortable: false,
            striped: false,
            bordered: false,
            hoverable: true,
            class: String::new(),
            attributes: Attrs::new(),
        }
    }
}

attrs_impl!(DataTableProps);

#[derive(Clone, Debug, Eq, PartialEq, Default)]
pub struct HistoryItemProps {
    pub id: String,
    pub title: String,
    pub timestamp: String,
    pub icon: String,
    pub href: String,
    pub is_selected: bool,
    pub class: String,
    pub attributes: Attrs,
}

attrs_impl!(HistoryItemProps);

impl HistoryItemProps {
    #[must_use]
    pub fn build_classes(&self) -> String {
        shared::build_classes(&[
            "flex items-center gap-3 px-4 py-3 text-sm transition-colors duration-150",
            if self.is_selected {
                "bg-accent"
            } else {
                "hover:bg-muted"
            },
            self.class.as_str(),
        ])
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Default)]
pub struct HistoryPanelProps {
    pub title: String,
    pub items: Vec<HistoryItemProps>,
    pub show_clear: bool,
    pub empty_text: String,
    pub class: String,
    pub attributes: Attrs,
}

attrs_impl!(HistoryPanelProps);

impl HistoryPanelProps {
    #[must_use]
    pub fn build_classes(&self) -> String {
        shared::build_classes(&[
            "flex flex-col rounded-xl border border-border",
            self.class.as_str(),
        ])
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Default)]
pub struct QuickActionItem {
    pub title: String,
    pub description: String,
    pub href: String,
    pub icon: String,
    pub badge: String,
    pub datastar_click: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Default)]
pub struct QuickActionsProps {
    pub title: String,
    pub actions: Vec<QuickActionItem>,
    pub class: String,
    pub attributes: Attrs,
}

attrs_impl!(QuickActionsProps);

#[derive(Clone, Debug, Eq, PartialEq, Default)]
pub struct PlanSummaryCardProps {
    pub title: String,
    pub plan_name: String,
    pub subtitle: String,
    pub usage_hint: String,
    pub primary_label: String,
    pub primary_href: String,
    pub secondary_label: String,
    pub secondary_href: String,
    pub class: String,
    pub attributes: Attrs,
}

attrs_impl!(PlanSummaryCardProps);

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DashboardAction {
    pub label: String,
    pub href: String,
    pub datastar_click: String,
    pub variant: &'static str,
}

impl Default for DashboardAction {
    fn default() -> Self {
        Self {
            label: String::new(),
            href: String::new(),
            datastar_click: String::new(),
            variant: "",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChecklistCardProps {
    pub title: String,
    pub description: String,
    pub icon: String,
    pub steps: Vec<String>,
    pub primary_action: DashboardAction,
    pub secondary_action: DashboardAction,
    pub class: String,
    pub attributes: Attrs,
}

impl Default for ChecklistCardProps {
    fn default() -> Self {
        Self {
            title: String::new(),
            description: String::new(),
            icon: "check".to_string(),
            steps: Vec::new(),
            primary_action: DashboardAction::default(),
            secondary_action: DashboardAction::default(),
            class: String::new(),
            attributes: Attrs::new(),
        }
    }
}

attrs_impl!(ChecklistCardProps);

#[derive(Clone, Debug, Eq, PartialEq, Default)]
pub struct PreviewListItem {
    pub title: String,
    pub href: String,
    pub icon: String,
    pub timestamp: String,
    pub timestamp_iso: String,
    pub status: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PreviewListCardProps {
    pub title: String,
    pub icon: String,
    pub items: Vec<PreviewListItem>,
    pub empty_title: String,
    pub empty_description: String,
    pub current_page: i32,
    pub total_pages: i32,
    pub prev_href: String,
    pub next_href: String,
    pub class: String,
    pub attributes: Attrs,
}

impl Default for PreviewListCardProps {
    fn default() -> Self {
        Self {
            title: String::new(),
            icon: "list".to_string(),
            items: Vec::new(),
            empty_title: String::new(),
            empty_description: String::new(),
            current_page: 1,
            total_pages: 1,
            prev_href: String::new(),
            next_href: String::new(),
            class: String::new(),
            attributes: Attrs::new(),
        }
    }
}

attrs_impl!(PreviewListCardProps);

#[must_use]
pub fn getStatusClasses(status: &str) -> &'static str {
    match status {
        "analyzing" => "bg-primary/10 text-primary",
        "complete" => "bg-success/10 text-success",
        "error" => "bg-destructive/10 text-destructive",
        _ => "bg-muted text-muted-foreground",
    }
}

#[must_use]
pub fn aiAnalysisTitle(props: &AIAnalysisPanelProps) -> &str {
    if props.title.is_empty() {
        "AI analysis"
    } else {
        props.title.as_str()
    }
}

#[must_use]
pub fn historyPanelTitle(props: &HistoryPanelProps) -> &str {
    if props.title.is_empty() {
        "History"
    } else {
        props.title.as_str()
    }
}

#[must_use]
pub fn historyPanelEmptyText(props: &HistoryPanelProps) -> &str {
    if props.empty_text.is_empty() {
        "No history"
    } else {
        props.empty_text.as_str()
    }
}

#[must_use]
pub fn getHeaderCellClasses(col: &DataTableColumn) -> String {
    let align = match col.align.as_str() {
        "center" => "text-center",
        "right" => "text-right",
        _ => "",
    };
    shared::build_classes(&[
        "px-4 py-3 text-left text-xs font-medium text-muted-foreground uppercase tracking-wider",
        align,
    ])
}

#[must_use]
pub fn getRowClasses(props: &DataTableProps, index: usize) -> String {
    shared::build_classes(&[
        if props.striped && index % 2 == 1 {
            "bg-muted/30"
        } else {
            ""
        },
        if props.hoverable {
            "hover:bg-muted/50 transition-colors duration-150"
        } else {
            ""
        },
    ])
}

#[must_use]
pub fn getCellClasses(col: &DataTableColumn, bordered: bool) -> String {
    let align = match col.align.as_str() {
        "center" => "text-center",
        "right" => "text-right",
        _ => "",
    };
    shared::build_classes(&[
        "px-4 py-3",
        if bordered {
            "border-t border-border"
        } else {
            ""
        },
        align,
    ])
}

#[must_use]
pub fn previewListItemClasses(status: &str) -> &'static str {
    if status == "active" {
        "border border-primary/30 bg-card"
    } else {
        "border border-border bg-card hover:border-primary/20 hover:bg-muted/30"
    }
}

#[must_use]
pub fn dashboardActionVariant(action: &DashboardAction, fallback: &'static str) -> &'static str {
    if action.variant.is_empty() {
        fallback
    } else {
        action.variant
    }
}

#[must_use]
pub fn analysisStatusIcon(status: &str) -> Markup {
    let (name, glyph, class) = match status {
        "analyzing" => ("refresh", "↻", "animate-spin"),
        "complete" => ("check-circle", "✓", ""),
        "error" => ("alert-triangle", "⚠", ""),
        _ => ("activity", "•", ""),
    };
    html! { span data-icon=(name) class=[optional(class)] { (glyph) } }
}

#[must_use]
pub fn DashboardHeader(props: &DashboardHeaderProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        header class=(props.build_classes()) data-testid=[attr(&attrs, "data-testid")] {
            div {
                @if !props.title.is_empty() { h1 class="text-2xl font-bold tracking-tight" { (&props.title) } }
                @if !props.description.is_empty() { p class="text-muted-foreground mt-1" { (&props.description) } }
            }
            div class="flex items-center gap-3" {
                @if props.show_search {
                    div class="relative" {
                        span class="absolute left-3 top-1/2 -translate-y-1/2 text-muted-foreground" data-icon="search" { "search" }
                        input type="search" placeholder="Search" class="h-9 w-[200px] rounded-md border border-input bg-background px-9 text-sm focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring";
                    }
                }
                @if props.show_actions { div data-slot="actions" {} }
            }
        }
    }
}

#[must_use]
pub fn AIAnalysisPanel(props: &AIAnalysisPanelProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        div class=(props.build_classes()) data-testid=[attr(&attrs, "data-testid")] {
            div class="flex items-center justify-between mb-4" {
                div class="flex items-center gap-2" {
                    span class="text-primary" data-icon="robot" { "robot" }
                    h3 class="font-semibold" { (aiAnalysisTitle(props)) }
                }
                span class=(shared::build_classes(&["inline-flex items-center gap-1.5 px-2.5 py-1 rounded-full text-xs font-medium", getStatusClasses(&props.status)])) {
                    (analysisStatusIcon(&props.status))
                    span class="capitalize" { (&props.status) }
                }
            }
            @if props.is_analyzing() {
                div class="flex items-center justify-center py-8" {
                    div class="flex flex-col items-center gap-3" {
                        span class="animate-spin w-8 h-8 border-4 border-primary border-t-transparent rounded-full" {}
                        span class="text-sm text-muted-foreground" { "Analyzing" }
                    }
                }
            } @else if props.is_complete() {
                div class="space-y-4" {
                    @if !props.result_text.is_empty() { p class="text-sm leading-relaxed" { (&props.result_text) } }
                    @if props.confidence > 0 {
                        div class="flex items-center gap-2" {
                            span class="text-xs text-muted-foreground" { "Confidence" }
                            div class="flex-1 h-2 bg-muted rounded-full overflow-hidden" { div class="h-full bg-success rounded-full" style=(format!("width: {}%", props.confidence.clamp(0, 100))) {} }
                            span class="text-xs font-medium" { (props.confidence.clamp(0, 100)) "%" }
                        }
                    }
                }
            } @else if props.is_error() {
                div class="text-center py-6" {
                    span class="inline-flex justify-center text-destructive" data-icon="alert-triangle" { "⚠" }
                    p class="text-sm text-destructive mt-2" { "Analysis failed" }
                }
            } @else {
                div class="text-center py-6 text-muted-foreground" {
                    span class="inline-flex justify-center" data-icon="search" { "search" }
                    p class="text-sm mt-2" { "Ready to analyze" }
                }
            }
            @if props.show_actions {
                div class="mt-4 pt-4 border-t border-border" data-slot="actions" {}
            }
        }
    }
}

#[must_use]
pub fn DataTable(props: &DataTableProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        div class="overflow-x-auto" {
            table class=(shared::build_classes(&["w-full text-sm", if props.bordered { "border border-border rounded-lg overflow-hidden" } else { "" }, props.class.as_str()])) data-testid=[attr(&attrs, "data-testid")] {
                thead class="bg-muted/50" {
                    tr {
                        @for col in &props.columns {
                            th class=(getHeaderCellClasses(col)) style=[optional(&col.width).map(|width| format!("width: {width}"))] {
                                (&col.label)
                                @if col.sortable || props.sortable {
                                    svg xmlns="http://www.w3.org/2000/svg" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" class="inline-block ml-1 opacity-50" { path d="m7 15 5 5 5-5" {} path d="m7 9 5-5 5 5" {} }
                                }
                            }
                        }
                    }
                }
                tbody {
                    @for (idx, row) in props.rows.iter().enumerate() {
                        tr class=(getRowClasses(props, idx)) {
                            @for col in &props.columns {
                                td class=(getCellClasses(col, props.bordered)) { (row.get(&col.key).map_or("", String::as_str)) }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[must_use]
pub fn historyItemContent(props: &HistoryItemProps) -> Markup {
    html! {
        span class="flex-shrink-0 text-muted-foreground" { @if props.icon.is_empty() { "file" } @else { (&props.icon) } }
        div class="flex-1 min-w-0" {
            p class="font-medium truncate" { (&props.title) }
            @if !props.timestamp.is_empty() { p class="text-xs text-muted-foreground" { (&props.timestamp) } }
        }
    }
}

#[must_use]
pub fn HistoryItem(props: &HistoryItemProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        @if props.href.is_empty() {
            div class=(props.build_classes()) data-testid=[attr(&attrs, "data-testid")] { (historyItemContent(props)) }
        } @else {
            a href=(&props.href) class=(props.build_classes()) data-testid=[attr(&attrs, "data-testid")] { (historyItemContent(props)) }
        }
    }
}

#[must_use]
pub fn HistoryPanel(props: &HistoryPanelProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        div class=(props.build_classes()) data-testid=[attr(&attrs, "data-testid")] {
            div class="flex items-center justify-between px-4 py-3 border-b border-border" {
                h3 class="font-semibold" { (historyPanelTitle(props)) }
                @if props.show_clear && !props.items.is_empty() { button type="button" class="text-xs text-muted-foreground hover:text-foreground transition-colors duration-150" { "Clear all" } }
            }
            div class="flex-1 overflow-y-auto" {
                @if props.items.is_empty() {
                    div class="flex flex-col items-center justify-center py-8 text-muted-foreground" {
                        span class="mb-2" data-icon="clipboard" { "clipboard" }
                        p class="text-sm" { (historyPanelEmptyText(props)) }
                    }
                } @else {
                    @for item in &props.items { (HistoryItem(item)) }
                }
            }
        }
    }
}

#[must_use]
pub fn SimpleDashboardHeader(title: &str, description: &str, class: &str) -> Markup {
    DashboardHeader(
        &DashboardHeaderProps::default()
            .with_title(title)
            .with_description(description)
            .with_class(class),
    )
}

#[must_use]
pub fn SimpleAIAnalysisPanel(title: &str, status: &str, class: &str) -> Markup {
    AIAnalysisPanel(
        &AIAnalysisPanelProps::default()
            .with_title(title)
            .with_status(status)
            .with_class(class),
    )
}

#[must_use]
pub fn SimpleHistoryPanel(title: &str, items: &[&str], class: &str) -> Markup {
    let items = items
        .iter()
        .map(|title| HistoryItemProps {
            title: (*title).to_string(),
            ..Default::default()
        })
        .collect();
    HistoryPanel(&HistoryPanelProps {
        title: title.to_string(),
        items,
        class: class.to_string(),
        ..Default::default()
    })
}

#[must_use]
pub fn QuickActions(props: &QuickActionsProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        div class=(shared::build_classes(&["space-y-3", props.class.as_str()])) data-testid=[attr(&attrs, "data-testid")] {
            @if !props.title.is_empty() { h2 class="text-sm font-semibold text-muted-foreground" { (&props.title) } }
            div class="grid grid-cols-1 gap-4 sm:grid-cols-2 xl:grid-cols-4" {
                @for action in &props.actions { (QuickActionCard(action)) }
            }
        }
    }
}

#[must_use]
pub fn QuickActionCard(action: &QuickActionItem) -> Markup {
    let class = "interactive-card h-auto items-start justify-start rounded-xl border border-border bg-card px-4 py-4 text-left hover:bg-muted/50 hover:border-primary/20";
    html! {
        @if action.href.is_empty() {
            button type="button" class=(class) data-on:click=[optional(&action.datastar_click)] { (quickActionInner(action)) }
        } @else {
            a href=(&action.href) class=(class) { (quickActionInner(action)) }
        }
    }
}

#[must_use]
pub fn quickActionInner(action: &QuickActionItem) -> Markup {
    html! {
        div class="flex w-full items-start gap-3" {
            div class="flex h-10 w-10 shrink-0 items-center justify-center rounded-xl bg-muted text-foreground" { @if !action.icon.is_empty() { (&action.icon) } }
            div class="min-w-0 flex-1" {
                div class="flex items-center gap-2" {
                    span class="text-sm font-semibold text-foreground" { (&action.title) }
                    @if !action.badge.is_empty() { span class="rounded-full bg-info px-2 py-0.5 text-xs text-info-foreground" { (&action.badge) } }
                }
                @if !action.description.is_empty() { p class="mt-1 text-xs leading-relaxed text-muted-foreground" { (&action.description) } }
            }
        }
    }
}

#[must_use]
pub fn PlanSummaryCard(props: &PlanSummaryCardProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        div class=(shared::build_classes(&["rounded-2xl border border-border bg-card", props.class.as_str()])) data-testid=[attr(&attrs, "data-testid")] {
            div class="space-y-4 p-4 sm:p-5" {
                div class="space-y-1" {
                    @if !props.title.is_empty() { h3 class="text-base font-semibold text-foreground" { (&props.title) } }
                    p class="text-sm text-muted-foreground" { (&props.plan_name) }
                    @if !props.subtitle.is_empty() { p class="text-xs text-muted-foreground" { (&props.subtitle) } }
                }
                @if !props.usage_hint.is_empty() { div class="rounded-2xl bg-muted px-3 py-2" { p class="text-xs text-muted-foreground" { (&props.usage_hint) } } }
                div class="flex flex-wrap items-center gap-2" {
                    @if !props.primary_label.is_empty() && !props.primary_href.is_empty() { a href=(&props.primary_href) class="inline-flex h-9 items-center rounded-md bg-primary px-3 text-sm font-medium text-primary-foreground" { (&props.primary_label) } }
                    @if !props.secondary_label.is_empty() && !props.secondary_href.is_empty() { a href=(&props.secondary_href) class="inline-flex h-9 items-center rounded-md border border-border px-3 text-sm font-medium" { (&props.secondary_label) } }
                }
            }
        }
    }
}

#[must_use]
pub fn ChecklistCard(props: &ChecklistCardProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        div class=(shared::build_classes(&["rounded-xl border border-border bg-card", props.class.as_str()])) data-testid=[attr(&attrs, "data-testid")] {
            div class="space-y-4 p-4 sm:p-5" {
                div class="flex items-center gap-2" {
                    span class="text-accent flex-shrink-0" data-icon=(&props.icon) { (&props.icon) }
                    h3 class="text-base font-semibold text-foreground" { (&props.title) }
                }
                @if !props.description.is_empty() { p class="text-sm text-muted-foreground leading-relaxed" { (&props.description) } }
                @if !props.steps.is_empty() {
                    div class="grid grid-cols-1 gap-2 sm:grid-cols-2" { @for step in &props.steps { (checklistStep(step)) } }
                }
                div class="flex flex-wrap items-center gap-2" {
                    (dashboardActionButton(&props.primary_action, shared::VARIANT_PRIMARY))
                    (dashboardActionButton(&props.secondary_action, shared::VARIANT_OUTLINE))
                }
            }
        }
    }
}

#[must_use]
pub fn checklistStep(label: &str) -> Markup {
    html! {
        div class="flex items-start gap-3 rounded-2xl bg-muted p-3" {
            div class="mt-0.5 flex h-6 w-6 shrink-0 items-center justify-center rounded-full bg-primary/10 text-primary" { "✓" }
            p class="text-sm text-foreground leading-relaxed" { (label) }
        }
    }
}

#[must_use]
pub fn dashboardActionButton(action: &DashboardAction, fallback: &'static str) -> Markup {
    if action.label.is_empty() {
        html! {}
    } else {
        let variant = dashboardActionVariant(action, fallback);
        let class = shared::build_classes(&[
            "inline-flex h-9 items-center rounded-md px-3 text-sm font-medium",
            if variant == shared::VARIANT_PRIMARY {
                "bg-primary text-primary-foreground"
            } else {
                "border border-border text-foreground"
            },
        ]);
        html! {
            @if action.href.is_empty() {
                button type="button" class=(class) data-on:click=[optional(&action.datastar_click)] { (&action.label) }
            } @else {
                a href=(&action.href) class=(class) { (&action.label) }
            }
        }
    }
}

#[must_use]
pub fn PreviewListCard(props: &PreviewListCardProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        div class=(shared::build_classes(&["rounded-xl border border-border bg-card", props.class.as_str()])) data-testid=[attr(&attrs, "data-testid")] {
            div class="space-y-4 p-4 sm:p-5" {
                div class="flex items-center gap-2" { span data-icon=(&props.icon) { (&props.icon) } h3 class="text-base font-semibold text-foreground" { (&props.title) } }
                @if props.items.is_empty() {
                    div class="py-6 text-center" {
                        h4 class="font-medium" { (&props.empty_title) }
                        p class="text-sm text-muted-foreground" { (&props.empty_description) }
                    }
                } @else {
                    div class="space-y-2" { @for item in &props.items { (previewListItem(item)) } }
                }
                @if props.total_pages > 1 {
                    nav class="flex items-center justify-between text-sm" {
                        a href=(&props.prev_href) { "Previous" }
                        span { (props.current_page) " / " (props.total_pages) }
                        a href=(&props.next_href) { "Next" }
                    }
                }
            }
        }
    }
}

#[must_use]
pub fn previewListItem(item: &PreviewListItem) -> Markup {
    if item.href.is_empty() {
        previewListItemCard(item)
    } else {
        html! { a href=(&item.href) class="block" { (previewListItemCard(item)) } }
    }
}

#[must_use]
pub fn previewListItemCard(item: &PreviewListItem) -> Markup {
    html! {
        div class=(shared::build_classes(&[previewListItemClasses(&item.status), "p-2.5"])) {
            div class="flex items-start gap-2.5" {
                div class="flex h-9 w-9 shrink-0 items-center justify-center rounded-2xl bg-muted text-foreground" { (&item.icon) }
                div class="min-w-0 flex-1" {
                    p class="truncate text-sm font-medium text-foreground leading-snug" { (&item.title) }
                    @if !item.timestamp.is_empty() || !item.timestamp_iso.is_empty() {
                        div class="mt-1 flex items-center gap-1.5" { span data-icon="clock" { "clock" } time datetime=[optional(&item.timestamp_iso)] class="text-muted-foreground" { (&item.timestamp) } }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_contains(html: &str, expected: &str) {
        assert!(html.contains(expected), "missing `{expected}` in `{html}`");
    }

    #[test]
    fn helper_functions_cover_variants() {
        assert_eq!(getStatusClasses("analyzing"), "bg-primary/10 text-primary");
        assert_eq!(getStatusClasses("complete"), "bg-success/10 text-success");
        assert_eq!(
            getStatusClasses("error"),
            "bg-destructive/10 text-destructive"
        );
        assert_eq!(getStatusClasses("idle"), "bg-muted text-muted-foreground");
        assert_eq!(
            aiAnalysisTitle(&AIAnalysisPanelProps::default()),
            "AI analysis"
        );
        assert_eq!(historyPanelTitle(&HistoryPanelProps::default()), "History");
        assert_eq!(
            historyPanelEmptyText(&HistoryPanelProps::default()),
            "No history"
        );
        assert_eq!(
            historyPanelEmptyText(&HistoryPanelProps {
                empty_text: "Nothing yet".to_string(),
                attributes: BTreeMap::from([("data-testid".to_string(), "history".to_string())]),
                ..Default::default()
            }),
            "Nothing yet"
        );
        assert_eq!(
            HistoryPanelProps {
                class: "panel-extra".to_string(),
                attributes: BTreeMap::from([("data-testid".to_string(), "history".to_string())]),
                ..Default::default()
            }
            .with_class("panel-new")
            .with_attribute("id", "history")
            .build_attributes()
            .get("id")
            .map(String::as_str),
            Some("history")
        );
        assert_contains(
            &getHeaderCellClasses(&DataTableColumn {
                align: "right".to_string(),
                ..Default::default()
            }),
            "text-right",
        );
        assert_contains(
            &getHeaderCellClasses(&DataTableColumn {
                align: "center".to_string(),
                ..Default::default()
            }),
            "text-center",
        );
        assert_contains(
            &getRowClasses(
                &DataTableProps {
                    striped: true,
                    ..Default::default()
                },
                1,
            ),
            "bg-muted/30",
        );
        assert_eq!(
            getRowClasses(
                &DataTableProps {
                    hoverable: false,
                    striped: false,
                    ..Default::default()
                },
                0,
            ),
            ""
        );
        assert_contains(
            &getCellClasses(
                &DataTableColumn {
                    align: "center".to_string(),
                    ..Default::default()
                },
                true,
            ),
            "border-t",
        );
        assert_contains(
            &getCellClasses(
                &DataTableColumn {
                    align: "right".to_string(),
                    ..Default::default()
                },
                false,
            ),
            "text-right",
        );
        assert_eq!(
            previewListItemClasses("active"),
            "border border-primary/30 bg-card"
        );
        assert_eq!(
            dashboardActionVariant(&DashboardAction::default(), shared::VARIANT_PRIMARY),
            shared::VARIANT_PRIMARY
        );
        assert_eq!(
            dashboardActionButton(&DashboardAction::default(), shared::VARIANT_PRIMARY)
                .into_string(),
            ""
        );
        assert_contains(
            &previewListItem(&PreviewListItem {
                title: "Inline".to_string(),
                ..Default::default()
            })
            .into_string(),
            "Inline",
        );
        assert_contains(
            &analysisStatusIcon("analyzing").into_string(),
            "animate-spin",
        );
    }

    #[test]
    fn header_analysis_table_history_render_html() {
        let header = DashboardHeader(
            &DashboardHeaderProps::default()
                .with_title("Dashboard")
                .with_description("Overview")
                .with_search(true)
                .with_actions(true),
        )
        .into_string();
        assert_contains(&header, "<header");
        assert_contains(&header, "Dashboard");
        assert_contains(&header, r#"type="search""#);
        assert_contains(&header, r#"data-slot="actions""#);

        let complete = AIAnalysisPanel(
            &AIAnalysisPanelProps::default()
                .with_title("AI")
                .with_status("complete")
                .with_result("Looks good", 75),
        )
        .into_string();
        assert_contains(&complete, "Looks good");
        assert_contains(&complete, "width: 75%");
        assert_contains(
            &AIAnalysisPanel(&AIAnalysisPanelProps::default().with_status("analyzing"))
                .into_string(),
            "animate-spin",
        );
        assert_contains(
            &AIAnalysisPanel(&AIAnalysisPanelProps::default().with_status("error")).into_string(),
            "Analysis failed",
        );

        let columns = vec![DataTableColumn {
            key: "name".to_string(),
            label: "Name".to_string(),
            sortable: true,
            ..Default::default()
        }];
        let mut row = BTreeMap::new();
        row.insert("name".to_string(), "Ada".to_string());
        let table = DataTable(&DataTableProps {
            columns,
            rows: vec![row],
            bordered: true,
            striped: true,
            ..Default::default()
        })
        .into_string();
        assert_contains(&table, "<table");
        assert_contains(&table, "Ada");
        assert_contains(&table, "border-t border-border");

        let item = HistoryItem(&HistoryItemProps {
            title: "Created".to_string(),
            timestamp: "now".to_string(),
            href: "/history".to_string(),
            is_selected: true,
            ..Default::default()
        })
        .into_string();
        assert_contains(&item, r#"href="/history""#);
        assert_contains(&item, "bg-accent");
        assert_contains(
            &HistoryPanel(&HistoryPanelProps {
                show_clear: true,
                items: vec![HistoryItemProps {
                    title: "One".to_string(),
                    ..Default::default()
                }],
                ..Default::default()
            })
            .into_string(),
            "Clear all",
        );
        assert_contains(
            &HistoryPanel(&HistoryPanelProps::default()).into_string(),
            "No history",
        );
    }

    #[test]
    fn data_table_global_sortable_width_and_unbordered_variation_render_html() {
        let columns = vec![DataTableColumn {
            key: "email".to_string(),
            label: "Email".to_string(),
            width: "12rem".to_string(),
            ..Default::default()
        }];
        let mut row = BTreeMap::new();
        row.insert("email".to_string(), "ada@example.test".to_string());

        let table = DataTable(&DataTableProps {
            columns,
            rows: vec![row],
            sortable: true,
            bordered: false,
            class: "compact".to_string(),
            attributes: BTreeMap::from([("data-testid".to_string(), "users-table".to_string())]),
            ..Default::default()
        })
        .into_string();

        assert_contains(&table, r#"data-testid="users-table""#);
        assert_contains(&table, r#"style="width: 12rem""#);
        assert_contains(&table, "ada@example.test");
        assert_contains(&table, "compact");
        assert_contains(&table, r#"<svg xmlns="http://www.w3.org/2000/svg""#);
        assert!(!table.contains("border border-border rounded-lg overflow-hidden"));
    }

    #[test]
    fn simple_wrappers_quick_actions_and_plan_render_html() {
        assert_contains(
            &SimpleDashboardHeader("Title", "Desc", "extra").into_string(),
            "extra",
        );
        assert_contains(
            &SimpleAIAnalysisPanel("AI", "idle", "extra").into_string(),
            "Ready to analyze",
        );
        assert_contains(
            &SimpleHistoryPanel("Recent", &["A", "B"], "panel").into_string(),
            "Recent",
        );

        let action = QuickActionItem {
            title: "New".to_string(),
            description: "Create".to_string(),
            icon: "+".to_string(),
            badge: "Hot".to_string(),
            datastar_click: "$open = true".to_string(),
            ..Default::default()
        };
        let card = QuickActionCard(&action).into_string();
        assert_contains(&card, r#"data-on:click="$open = true""#);
        assert_contains(&card, "Hot");
        assert_contains(
            &QuickActionCard(&QuickActionItem {
                href: "/new".to_string(),
                title: "New".to_string(),
                ..Default::default()
            })
            .into_string(),
            r#"href="/new""#,
        );
        assert_contains(
            &QuickActions(&QuickActionsProps {
                title: "Actions".to_string(),
                actions: vec![action],
                ..Default::default()
            })
            .into_string(),
            "Actions",
        );

        let plan = PlanSummaryCard(&PlanSummaryCardProps {
            title: "Plan".to_string(),
            plan_name: "Pro".to_string(),
            subtitle: "Monthly".to_string(),
            usage_hint: "10 seats".to_string(),
            primary_label: "Upgrade".to_string(),
            primary_href: "/upgrade".to_string(),
            secondary_label: "Manage".to_string(),
            secondary_href: "/manage".to_string(),
            ..Default::default()
        })
        .into_string();
        assert_contains(&plan, "Pro");
        assert_contains(&plan, r#"href="/upgrade""#);
        assert_contains(&plan, "10 seats");
    }

    #[test]
    fn checklist_actions_preview_and_status_icons_render_html() {
        assert_contains(&checklistStep("Add profile").into_string(), "Add profile");
        let primary = DashboardAction {
            label: "Start".to_string(),
            href: String::new(),
            datastar_click: "$start = true".to_string(),
            variant: shared::VARIANT_PRIMARY,
        };
        let secondary = DashboardAction {
            label: "Later".to_string(),
            href: "/later".to_string(),
            variant: shared::VARIANT_OUTLINE,
            datastar_click: String::new(),
        };
        let checklist = ChecklistCard(&ChecklistCardProps {
            title: "Setup".to_string(),
            description: "Finish setup".to_string(),
            steps: vec!["One".to_string(), "Two".to_string()],
            primary_action: primary,
            secondary_action: secondary,
            ..Default::default()
        })
        .into_string();
        assert_contains(&checklist, "Finish setup");
        assert_contains(&checklist, r#"data-on:click="$start = true""#);
        assert_contains(&checklist, r#"href="/later""#);

        let empty = PreviewListCard(&PreviewListCardProps {
            title: "Docs".to_string(),
            empty_title: "No docs".to_string(),
            empty_description: "Upload one".to_string(),
            ..Default::default()
        })
        .into_string();
        assert_contains(&empty, "No docs");
        let item = PreviewListItem {
            title: "Doc".to_string(),
            href: "/doc".to_string(),
            icon: "file".to_string(),
            timestamp: "now".to_string(),
            status: "active".to_string(),
            ..Default::default()
        };
        let preview = PreviewListCard(&PreviewListCardProps {
            title: "Docs".to_string(),
            items: vec![item],
            total_pages: 2,
            prev_href: "/p1".to_string(),
            next_href: "/p2".to_string(),
            ..Default::default()
        })
        .into_string();
        assert_contains(&preview, r#"href="/doc""#);
        assert_contains(&preview, "border-primary/30");
        assert_contains(&preview, "1 / 2");
        assert_contains(
            &previewListItemCard(&PreviewListItem {
                title: "X".to_string(),
                ..Default::default()
            })
            .into_string(),
            "flex items-start gap-2.5",
        );
        assert_contains(
            &analysisStatusIcon("complete").into_string(),
            r#"data-icon="check-circle""#,
        );
    }
}
