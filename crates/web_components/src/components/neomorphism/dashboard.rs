#![allow(non_snake_case)]
#![allow(clippy::needless_pass_by_value)]

use crate::components::ComponentInfo;
use crate::components::shared;
use crate::components::unstyled::dashboard as base;
use maud::Markup;

pub use base::{
    AIAnalysisPanelProps, ChecklistCardProps, DashboardAction, DashboardHeaderProps,
    DataTableColumn, DataTableProps, HistoryItemProps, HistoryPanelProps, PlanSummaryCardProps,
    PreviewListCardProps, PreviewListItem, QuickActionItem, QuickActionsProps,
    SimpleAIAnalysisPanel, SimpleDashboardHeader, SimpleHistoryPanel,
};

pub const COMPONENTS: &[ComponentInfo] = &[
    ComponentInfo {
        family: "neomorphism",
        module: "dashboard",
        name: "DashboardHeader",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "dashboard",
        name: "AIAnalysisPanel",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "dashboard",
        name: "DataTable",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "dashboard",
        name: "HistoryItem",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "dashboard",
        name: "HistoryPanel",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "dashboard",
        name: "SimpleDashboardHeader",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "dashboard",
        name: "SimpleAIAnalysisPanel",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "dashboard",
        name: "SimpleHistoryPanel",
    },
];

#[must_use]
fn addHeaderNeomorphClasses(props: &DashboardHeaderProps) -> DashboardHeaderProps {
    let mut props = props.clone();
    props.class = shared::build_classes(&[props.class.as_str(), "neomorph-flat"]);
    props
}

#[must_use]
pub fn DashboardHeader(props: &DashboardHeaderProps) -> Markup {
    let neo_props = addHeaderNeomorphClasses(props);
    base::DashboardHeader(&neo_props)
}

#[must_use]
fn addAnalysisNeomorphClasses(props: &AIAnalysisPanelProps) -> AIAnalysisPanelProps {
    let mut props = props.clone();
    props.class = shared::build_classes(&[props.class.as_str(), "neomorph-flat"]);
    props
}

#[must_use]
pub fn AIAnalysisPanel(props: &AIAnalysisPanelProps) -> Markup {
    let neo_props = addAnalysisNeomorphClasses(props);
    base::AIAnalysisPanel(&neo_props)
}

#[must_use]
fn addTableNeomorphClasses(props: &DataTableProps) -> DataTableProps {
    let mut props = props.clone();
    props.class = shared::build_classes(&[props.class.as_str(), "neomorph-flat"]);
    props
}

#[must_use]
pub fn DataTable(props: &DataTableProps) -> Markup {
    let neo_props = addTableNeomorphClasses(props);
    base::DataTable(&neo_props)
}

#[must_use]
fn addHistoryItemNeomorphClasses(props: &HistoryItemProps) -> HistoryItemProps {
    let mut props = props.clone();
    props.class = shared::build_classes(&[props.class.as_str(), "neomorph-flat"]);
    props
}

#[must_use]
pub fn HistoryItem(props: &HistoryItemProps) -> Markup {
    let neo_props = addHistoryItemNeomorphClasses(props);
    base::HistoryItem(&neo_props)
}

#[must_use]
fn addHistoryPanelNeomorphClasses(props: &HistoryPanelProps) -> HistoryPanelProps {
    let mut props = props.clone();
    props.class = shared::build_classes(&[props.class.as_str(), "neomorph-flat"]);
    props
}

#[must_use]
pub fn HistoryPanel(props: &HistoryPanelProps) -> Markup {
    let neo_props = addHistoryPanelNeomorphClasses(props);
    base::HistoryPanel(&neo_props)
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
        assert_eq!(COMPONENTS.len(), 8);
        assert!(
            COMPONENTS
                .iter()
                .all(|component| component.family == "neomorphism"
                    && component.module == "dashboard")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "DashboardHeader")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "AIAnalysisPanel")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "DataTable")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "HistoryItem")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "HistoryPanel")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "SimpleDashboardHeader")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "SimpleAIAnalysisPanel")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "SimpleHistoryPanel")
        );
    }

    #[test]
    fn wrapped_components_render_unstyled_html_with_neomorphism_classes() {
        let html = DashboardHeader(&DashboardHeaderProps::default()).into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.starts_with("<"), "expected rendered element: {html}");
        assert!(
            html.contains("neomorph-flat"),
            "missing neomorph-flat: {html}"
        );
        let html = AIAnalysisPanel(&AIAnalysisPanelProps::default()).into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.starts_with("<"), "expected rendered element: {html}");
        assert!(
            html.contains("neomorph-flat"),
            "missing neomorph-flat: {html}"
        );
        let html = DataTable(&DataTableProps::default()).into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.starts_with("<"), "expected rendered element: {html}");
        assert!(
            html.contains("neomorph-flat"),
            "missing neomorph-flat: {html}"
        );
        let html = HistoryItem(&HistoryItemProps::default()).into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.starts_with("<"), "expected rendered element: {html}");
        assert!(
            html.contains("neomorph-flat"),
            "missing neomorph-flat: {html}"
        );
        let html = HistoryPanel(&HistoryPanelProps::default()).into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.starts_with("<"), "expected rendered element: {html}");
        assert!(
            html.contains("neomorph-flat"),
            "missing neomorph-flat: {html}"
        );
    }
}
