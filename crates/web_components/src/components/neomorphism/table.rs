#![allow(non_snake_case)]
#![allow(clippy::needless_pass_by_value)]

use crate::components::ComponentInfo;
use crate::components::unstyled::table as base;
use maud::Markup;

pub use base::{
    SimpleTable, SortableTable, StripedTable, TableBodyProps, TableCaption, TableCaptionProps,
    TableCell, TableCellProps, TableFooterProps, TableHeadProps, TableHeaderProps, TableProps,
    TableRowProps,
};

pub const COMPONENTS: &[ComponentInfo] = &[
    ComponentInfo {
        family: "neomorphism",
        module: "table",
        name: "Table",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "table",
        name: "TableHeader",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "table",
        name: "TableBody",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "table",
        name: "TableFooter",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "table",
        name: "TableRow",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "table",
        name: "TableHead",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "table",
        name: "TableCell",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "table",
        name: "TableCaption",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "table",
        name: "SimpleTable",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "table",
        name: "StripedTable",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "table",
        name: "SortableTable",
    },
];

#[must_use]
fn addTableNeomorphClasses(props: &TableProps) -> TableProps {
    let mut props = props.clone();
    props.class = format!("neomorph-card {}", props.class);
    props
}

#[must_use]
pub fn withStriped(props: &TableProps) -> TableProps {
    let mut props = props.clone();
    props.striped = true;
    props
}

#[must_use]
fn addTableHeaderNeomorphClasses(props: &TableHeaderProps) -> TableHeaderProps {
    let mut props = props.clone();
    props.class = format!(
        "bg-gradient-to-b from-card/50 to-transparent {}",
        props.class
    );
    props
}

#[must_use]
fn addTableFooterNeomorphClasses(props: &TableFooterProps) -> TableFooterProps {
    let mut props = props.clone();
    props.class = format!("neomorph-flat-sm {}", props.class);
    props
}

#[must_use]
fn addTableRowNeomorphClasses(props: &TableRowProps) -> TableRowProps {
    let mut props = props.clone();
    if props.clickable {
        props.class = format!("neomorph-hover-lift {}", props.class);
    }
    props
}

#[must_use]
fn addTableHeadNeomorphClasses(props: &TableHeadProps) -> TableHeadProps {
    let mut props = props.clone();
    props.class = format!("neomorph-flat-sm font-semibold {}", props.class);
    props
}

#[must_use]
pub fn Table(props: &TableProps, children: Markup) -> Markup {
    let neo_props = addTableNeomorphClasses(props);
    base::Table(&neo_props, children)
}

#[must_use]
pub fn TableHeader(props: &TableHeaderProps, children: Markup) -> Markup {
    let neo_props = addTableHeaderNeomorphClasses(props);
    base::TableHeader(&neo_props, children)
}

#[must_use]
pub fn TableBody(props: &TableBodyProps, children: Markup) -> Markup {
    base::TableBody(props, children)
}

#[must_use]
pub fn TableFooter(props: &TableFooterProps, children: Markup) -> Markup {
    let neo_props = addTableFooterNeomorphClasses(props);
    base::TableFooter(&neo_props, children)
}

#[must_use]
pub fn TableRow(props: &TableRowProps, children: Markup) -> Markup {
    let neo_props = addTableRowNeomorphClasses(props);
    base::TableRow(&neo_props, children)
}

#[must_use]
pub fn TableHead(props: &TableHeadProps, children: Markup) -> Markup {
    let neo_props = addTableHeadNeomorphClasses(props);
    base::TableHead(&neo_props, children)
}

#[cfg(test)]
mod tests {
    use super::*;
    use maud::html;
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
        assert_eq!(COMPONENTS.len(), 11);
        assert!(
            COMPONENTS
                .iter()
                .all(|component| component.family == "neomorphism" && component.module == "table")
        );
        assert!(COMPONENTS.iter().any(|component| component.name == "Table"));
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "TableHeader")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "TableBody")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "TableFooter")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "TableRow")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "TableHead")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "TableCell")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "TableCaption")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "SimpleTable")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "StripedTable")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "SortableTable")
        );
    }

    #[test]
    fn neomorphism_only_components_render_html_without_generated_stub_markers() {
        let table_props = addTableNeomorphClasses(&TableProps::default().with_class("custom"));
        assert_eq!(table_props.class, "neomorph-card custom");
        assert!(withStriped(&TableProps::default()).striped);

        let html = Table(
            &TableProps::default(),
            html! { tbody { tr { td { "Body" } } } },
        )
        .into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.contains("neomorph-card"));
        assert!(html.contains("<td>Body</td>"));

        let html =
            TableHeader(&TableHeaderProps::default(), html! { tr { th { "Head" } } }).into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.contains("<thead"));
        assert!(html.contains("bg-gradient-to-b"));
        assert!(html.contains("<th>Head</th>"));
        let html =
            TableBody(&TableBodyProps::default(), html! { tr { td { "Body" } } }).into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.contains("<tbody"));
        assert!(html.contains("<td>Body</td>"));
        let html =
            TableFooter(&TableFooterProps::default(), html! { tr { td { "Foot" } } }).into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.contains("<tfoot"));
        assert!(html.contains("neomorph-flat-sm"));
        assert!(html.contains("<td>Foot</td>"));

        let row_props = addTableRowNeomorphClasses(&TableRowProps::default().with_clickable(true));
        assert!(row_props.class.contains("neomorph-hover-lift"));
        let static_row_props = addTableRowNeomorphClasses(&TableRowProps::default());
        assert!(!static_row_props.class.contains("neomorph-hover-lift"));
        let row = TableRow(
            &TableRowProps::default().with_clickable(true),
            html! { td { "Row" } },
        )
        .into_string();
        assert!(row.contains("neomorph-hover-lift"));
        assert!(row.contains("<td>Row</td>"));

        let head_props = addTableHeadNeomorphClasses(&TableHeadProps::default());
        assert!(head_props.class.contains("neomorph-flat-sm font-semibold"));
        let head = TableHead(&TableHeadProps::default(), html! { "Name" }).into_string();
        assert!(head.contains("neomorph-flat-sm"));
        assert!(head.contains(">Name</th>"));
    }
}
