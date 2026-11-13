#![allow(non_snake_case)]

use crate::components::{ComponentInfo, shared};
use maud::{Markup, html};
use std::collections::BTreeMap;

pub const COMPONENTS: &[ComponentInfo] = &[
    ComponentInfo {
        family: "unstyled",
        module: "table",
        name: "Table",
    },
    ComponentInfo {
        family: "unstyled",
        module: "table",
        name: "TableHeader",
    },
    ComponentInfo {
        family: "unstyled",
        module: "table",
        name: "TableBody",
    },
    ComponentInfo {
        family: "unstyled",
        module: "table",
        name: "TableFooter",
    },
    ComponentInfo {
        family: "unstyled",
        module: "table",
        name: "TableRow",
    },
    ComponentInfo {
        family: "unstyled",
        module: "table",
        name: "TableHead",
    },
    ComponentInfo {
        family: "unstyled",
        module: "table",
        name: "TableCell",
    },
    ComponentInfo {
        family: "unstyled",
        module: "table",
        name: "TableCaption",
    },
    ComponentInfo {
        family: "unstyled",
        module: "table",
        name: "SimpleTable",
    },
    ComponentInfo {
        family: "unstyled",
        module: "table",
        name: "StripedTable",
    },
    ComponentInfo {
        family: "unstyled",
        module: "table",
        name: "SortableTable",
    },
];

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TableProps {
    pub class: String,
    pub attributes: BTreeMap<String, String>,
    pub striped: bool,
    pub hoverable: bool,
    pub size: &'static str,
    pub sticky_header: bool,
    pub aria_label: String,
    pub aria_described_by: String,
}

impl Default for TableProps {
    fn default() -> Self {
        Self {
            class: String::new(),
            attributes: BTreeMap::new(),
            striped: false,
            hoverable: false,
            size: shared::SIZE_MD,
            sticky_header: false,
            aria_label: String::new(),
            aria_described_by: String::new(),
        }
    }
}

macro_rules! simple_props {
    ($name:ident, $base:expr) => {
        #[derive(Clone, Debug, Default, Eq, PartialEq)]
        pub struct $name {
            pub class: String,
            pub attributes: BTreeMap<String, String>,
        }

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
            pub fn build_classes(&self) -> String {
                shared::build_classes(&[$base, self.class.as_str()])
            }
        }
    };
}

simple_props!(TableHeaderProps, "[&_tr]:border-b");
simple_props!(TableBodyProps, "[&_tr:last-child]:border-0");
simple_props!(
    TableFooterProps,
    "border-t bg-muted/50 font-medium [&>tr]:last:border-b-0"
);

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct TableRowProps {
    pub class: String,
    pub attributes: BTreeMap<String, String>,
    pub selected: bool,
    pub clickable: bool,
    pub on_click: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TableHeadProps {
    pub class: String,
    pub attributes: BTreeMap<String, String>,
    pub sortable: bool,
    pub sort_dir: String,
    pub align: String,
}

impl Default for TableHeadProps {
    fn default() -> Self {
        Self {
            class: String::new(),
            attributes: BTreeMap::new(),
            sortable: false,
            sort_dir: String::new(),
            align: "left".to_string(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TableCellProps {
    pub class: String,
    pub attributes: BTreeMap<String, String>,
    pub align: String,
}

impl Default for TableCellProps {
    fn default() -> Self {
        Self {
            class: String::new(),
            attributes: BTreeMap::new(),
            align: "left".to_string(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TableCaptionProps {
    pub class: String,
    pub attributes: BTreeMap<String, String>,
    pub position: String,
}

impl Default for TableCaptionProps {
    fn default() -> Self {
        Self {
            class: String::new(),
            attributes: BTreeMap::new(),
            position: "top".to_string(),
        }
    }
}

impl TableProps {
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
    pub const fn with_striped(mut self, value: bool) -> Self {
        self.striped = value;
        self
    }

    #[must_use]
    pub const fn with_hoverable(mut self, value: bool) -> Self {
        self.hoverable = value;
        self
    }

    #[must_use]
    pub const fn with_size(mut self, value: &'static str) -> Self {
        self.size = value;
        self
    }

    #[must_use]
    pub const fn with_sticky_header(mut self, value: bool) -> Self {
        self.sticky_header = value;
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
        shared::build_classes(&["w-full caption-bottom text-sm", self.class.as_str()])
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
        attrs.insert("role".to_string(), "table".to_string());
        attrs
    }
}

impl TableRowProps {
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
    pub const fn with_selected(mut self, value: bool) -> Self {
        self.selected = value;
        self
    }

    #[must_use]
    pub const fn with_clickable(mut self, value: bool) -> Self {
        self.clickable = value;
        self
    }

    #[must_use]
    pub fn with_on_click(mut self, value: impl Into<String>) -> Self {
        self.on_click = value.into();
        self
    }

    #[must_use]
    pub fn build_classes(&self) -> String {
        shared::build_classes(&[
            "border-b transition-colors duration-150",
            if self.clickable {
                "hover:bg-muted/50 cursor-pointer"
            } else {
                ""
            },
            if self.selected { "bg-muted" } else { "" },
            self.class.as_str(),
        ])
    }

    #[must_use]
    pub fn build_attributes(&self) -> BTreeMap<String, String> {
        let mut attrs = self.attributes.clone();
        if !self.on_click.is_empty() {
            attrs.insert("onclick".to_string(), self.on_click.clone());
        }
        if self.clickable {
            attrs
                .entry("tabindex".to_string())
                .or_insert_with(|| "0".to_string());
        }
        attrs
    }
}

impl TableHeadProps {
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
    pub const fn with_sortable(mut self, value: bool) -> Self {
        self.sortable = value;
        self
    }

    #[must_use]
    pub fn with_sort_dir(mut self, value: impl Into<String>) -> Self {
        self.sort_dir = value.into();
        self
    }

    #[must_use]
    pub fn with_align(mut self, value: impl Into<String>) -> Self {
        self.align = value.into();
        self
    }

    #[must_use]
    pub fn build_classes(&self) -> String {
        let align = match self.align.as_str() {
            "center" => "text-center",
            "right" => "text-right",
            _ => "text-left",
        };
        let sortable = if self.sortable {
            "cursor-pointer select-none hover:text-foreground"
        } else {
            ""
        };
        shared::build_classes(&[
            "h-12 px-4 font-medium text-muted-foreground",
            align,
            sortable,
            self.class.as_str(),
        ])
    }

    #[must_use]
    pub fn build_attributes(&self) -> BTreeMap<String, String> {
        let mut attrs = self.attributes.clone();
        if self.sortable {
            let aria_sort = match self.sort_dir.as_str() {
                "asc" => "ascending",
                "desc" => "descending",
                _ => "none",
            };
            attrs.insert("aria-sort".to_string(), aria_sort.to_string());
            attrs.insert("data-sortable".to_string(), "true".to_string());
            if !self.sort_dir.is_empty() {
                attrs.insert("data-sort-dir".to_string(), self.sort_dir.clone());
            }
        }
        attrs
    }
}

impl TableCellProps {
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
    pub fn with_align(mut self, value: impl Into<String>) -> Self {
        self.align = value.into();
        self
    }

    #[must_use]
    pub fn build_classes(&self) -> String {
        let align = match self.align.as_str() {
            "center" => "text-center",
            "right" => "text-right",
            _ => "text-left",
        };
        shared::build_classes(&["p-4", align, self.class.as_str()])
    }
}

impl TableCaptionProps {
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
    pub fn with_position(mut self, value: impl Into<String>) -> Self {
        self.position = value.into();
        self
    }

    #[must_use]
    pub fn build_classes(&self) -> String {
        let position = if self.position == "top" {
            "caption-top"
        } else {
            "caption-bottom"
        };
        shared::build_classes(&[
            "mt-4 text-sm text-muted-foreground",
            position,
            self.class.as_str(),
        ])
    }
}

#[must_use]
pub fn Table(props: &TableProps, children: Markup) -> Markup {
    let attrs = props.build_attributes();
    html! {
        div class="relative w-full overflow-auto" {
            table
                class=(props.build_classes())
                role=[shared::attr_value(&attrs, "role")]
                aria-label=[shared::attr_value(&attrs, "aria-label")]
                aria-describedby=[shared::attr_value(&attrs, "aria-describedby")]
                id=[shared::attr_value(&attrs, "id")]
                data-testid=[shared::attr_value(&attrs, "data-testid")]
            {
                (children)
            }
        }
    }
}

macro_rules! section_component {
    ($fn_name:ident, $tag:ident, $props_ty:ty) => {
        #[must_use]
        pub fn $fn_name(props: &$props_ty, children: Markup) -> Markup {
            let attrs = props.attributes.clone();
            html! {
                $tag
                    class=(props.build_classes())
                    id=[shared::attr_value(&attrs, "id")]
                    data-testid=[shared::attr_value(&attrs, "data-testid")]
                {
                    (children)
                }
            }
        }
    };
}

section_component!(TableHeader, thead, TableHeaderProps);
section_component!(TableBody, tbody, TableBodyProps);
section_component!(TableFooter, tfoot, TableFooterProps);

#[must_use]
pub fn TableRow(props: &TableRowProps, children: Markup) -> Markup {
    let attrs = props.build_attributes();
    html! {
        tr
            class=(props.build_classes())
            onclick=[shared::attr_value(&attrs, "onclick")]
            tabindex=[shared::attr_value(&attrs, "tabindex")]
            id=[shared::attr_value(&attrs, "id")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
        {
            (children)
        }
    }
}

#[must_use]
pub fn TableHead(props: &TableHeadProps, children: Markup) -> Markup {
    let attrs = props.build_attributes();
    html! {
        th
            class=(props.build_classes())
            aria-sort=[shared::attr_value(&attrs, "aria-sort")]
            data-sortable=[shared::attr_value(&attrs, "data-sortable")]
            data-sort-dir=[shared::attr_value(&attrs, "data-sort-dir")]
            id=[shared::attr_value(&attrs, "id")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
        {
            @if props.sortable {
                div class="flex items-center gap-2" {
                    (children)
                    span class="sort-indicator" aria-hidden="true" {
                        @if props.sort_dir == "asc" {
                            "↑"
                        } @else if props.sort_dir == "desc" {
                            "↓"
                        } @else {
                            "⇅"
                        }
                    }
                }
            } @else {
                (children)
            }
        }
    }
}

#[must_use]
pub fn TableCell(props: &TableCellProps, children: Markup) -> Markup {
    let attrs = props.attributes.clone();
    html! {
        td
            class=(props.build_classes())
            id=[shared::attr_value(&attrs, "id")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
        {
            (children)
        }
    }
}

#[must_use]
pub fn TableCaption(props: &TableCaptionProps, children: Markup) -> Markup {
    let attrs = props.attributes.clone();
    html! {
        caption
            class=(props.build_classes())
            id=[shared::attr_value(&attrs, "id")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
        {
            (children)
        }
    }
}

#[must_use]
pub fn SimpleTable(props: &TableProps, children: Markup) -> Markup {
    Table(props, children)
}

#[must_use]
pub fn withStripedProps(mut props: TableProps) -> TableProps {
    props.striped = true;
    props
}

#[must_use]
pub fn StripedTable(props: &TableProps, children: Markup) -> Markup {
    Table(&withStripedProps(props.clone()), children)
}

#[must_use]
pub fn SortableTable(props: &TableProps, children: Markup) -> Markup {
    Table(props, children)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_helpers_cover_defaults_classes_and_attributes() {
        let props = TableProps::default();
        assert_eq!(props.size, shared::SIZE_MD);
        assert!(!props.striped);
        assert!(!props.hoverable);
        assert!(!props.sticky_header);
        assert_eq!(props.build_classes(), "w-full caption-bottom text-sm");
        let attrs = props
            .with_class("min-w-96")
            .with_aria_label("Invoices")
            .with_aria_described_by("invoice-help")
            .with_attribute("id", "invoices")
            .build_attributes();
        assert_eq!(attrs.get("role").map(String::as_str), Some("table"));
        assert_eq!(
            attrs.get("aria-label").map(String::as_str),
            Some("Invoices")
        );
        assert_eq!(
            attrs.get("aria-describedby").map(String::as_str),
            Some("invoice-help")
        );
        assert_eq!(attrs.get("id").map(String::as_str), Some("invoices"));
        assert_eq!(
            TableProps::default().with_class("min-w-96").build_classes(),
            "w-full caption-bottom text-sm min-w-96"
        );
        let striped = withStripedProps(TableProps::default());
        assert!(striped.striped);
    }

    #[test]
    fn section_row_head_cell_caption_helpers_cover_variations() {
        assert_eq!(
            TableHeaderProps::default()
                .with_class("sticky")
                .build_classes(),
            "[&_tr]:border-b sticky"
        );
        assert_eq!(
            TableBodyProps::default().build_classes(),
            "[&_tr:last-child]:border-0"
        );
        assert_eq!(
            TableFooterProps::default().build_classes(),
            "border-t bg-muted/50 font-medium [&>tr]:last:border-b-0"
        );
        assert_eq!(
            TableRowProps::default()
                .with_clickable(true)
                .with_selected(true)
                .with_class("row")
                .build_classes(),
            "border-b transition-colors duration-150 hover:bg-muted/50 cursor-pointer bg-muted row"
        );
        let row_attrs = TableRowProps::default()
            .with_clickable(true)
            .with_on_click("selectRow()")
            .with_attribute("tabindex", "-1")
            .build_attributes();
        assert_eq!(
            row_attrs.get("onclick").map(String::as_str),
            Some("selectRow()")
        );
        assert_eq!(row_attrs.get("tabindex").map(String::as_str), Some("-1"));
        assert_eq!(
            TableHeadProps::default()
                .with_align("right")
                .with_sortable(true)
                .with_sort_dir("desc")
                .build_classes(),
            "h-12 px-4 font-medium text-muted-foreground text-right cursor-pointer select-none hover:text-foreground"
        );
        let head_attrs = TableHeadProps::default()
            .with_sortable(true)
            .with_sort_dir("asc")
            .build_attributes();
        assert_eq!(
            head_attrs.get("aria-sort").map(String::as_str),
            Some("ascending")
        );
        assert_eq!(
            head_attrs.get("data-sortable").map(String::as_str),
            Some("true")
        );
        assert_eq!(
            head_attrs.get("data-sort-dir").map(String::as_str),
            Some("asc")
        );
        assert_eq!(
            TableHeadProps::default()
                .with_sortable(true)
                .build_attributes()
                .get("aria-sort")
                .map(String::as_str),
            Some("none")
        );
        assert_eq!(
            TableCellProps::default()
                .with_align("center")
                .with_class("cell")
                .build_classes(),
            "p-4 text-center cell"
        );
        assert_eq!(
            TableCaptionProps::default().build_classes(),
            "mt-4 text-sm text-muted-foreground caption-top"
        );
        assert_eq!(
            TableCaptionProps::default()
                .with_position("bottom")
                .build_classes(),
            "mt-4 text-sm text-muted-foreground caption-bottom"
        );
    }

    #[test]
    fn table_parts_render_real_html_and_escape_children() {
        let table = Table(
            &TableProps::default()
                .with_aria_label("Users")
                .with_attribute("data-testid", "users-table"),
            html! {
                (TableCaption(&TableCaptionProps::default(), html! { "<Caption>" }))
                (TableHeader(
                    &TableHeaderProps::default().with_attribute("id", "head"),
                    html! {
                        (TableRow(&TableRowProps::default(), html! {
                            (TableHead(&TableHeadProps::default().with_sortable(true).with_sort_dir("asc"), html! { "Name" }))
                            (TableHead(&TableHeadProps::default().with_sortable(true).with_sort_dir("desc"), html! { "Email" }))
                            (TableHead(&TableHeadProps::default().with_sortable(true), html! { "Role" }))
                        }))
                    },
                ))
                (TableBody(&TableBodyProps::default(), html! {
                    (TableRow(&TableRowProps::default().with_clickable(true).with_on_click("openUser()"), html! {
                        (TableCell(&TableCellProps::default(), html! { "<Ada>" }))
                    }))
                }))
                (TableFooter(&TableFooterProps::default(), html! { (TableRow(&TableRowProps::default(), html! {}) ) }))
            },
        )
        .into_string();
        assert!(table.starts_with("<div class=\"relative w-full overflow-auto\"><table class=\"w-full caption-bottom text-sm\""));
        assert!(table.contains("role=\"table\""));
        assert!(table.contains("aria-label=\"Users\""));
        assert!(table.contains("data-testid=\"users-table\""));
        assert!(table.contains("<caption class=\"mt-4 text-sm text-muted-foreground caption-top\">&lt;Caption&gt;</caption>"));
        assert!(table.contains("<thead class=\"[&amp;_tr]:border-b\" id=\"head\">"));
        assert!(
            table.contains("aria-sort=\"ascending\" data-sortable=\"true\" data-sort-dir=\"asc\"")
        );
        assert!(table.contains("<span class=\"sort-indicator\" aria-hidden=\"true\">↑</span>"));
        assert!(table.contains("<span class=\"sort-indicator\" aria-hidden=\"true\">↓</span>"));
        assert!(table.contains("<span class=\"sort-indicator\" aria-hidden=\"true\">⇅</span>"));
        assert!(table.contains("<tr class=\"border-b transition-colors duration-150 hover:bg-muted/50 cursor-pointer\" onclick=\"openUser()\" tabindex=\"0\">"));
        assert!(table.contains("<td class=\"p-4 text-left\">&lt;Ada&gt;</td>"));
        assert!(table.contains(
            "<tfoot class=\"border-t bg-muted/50 font-medium [&amp;&gt;tr]:last:border-b-0\">"
        ));
    }

    #[test]
    fn convenience_tables_render_expected_wrappers() {
        let props = TableProps::default().with_attribute("id", "simple");
        let simple = SimpleTable(&props, html! { tbody {} }).into_string();
        assert!(simple.contains(
            "<table class=\"w-full caption-bottom text-sm\" role=\"table\" id=\"simple\">"
        ));
        let striped = StripedTable(&TableProps::default(), html! {}).into_string();
        assert!(striped.contains("<table class=\"w-full caption-bottom text-sm\" role=\"table\">"));
        let sortable =
            SortableTable(&TableProps::default().with_class("sortable"), html! {}).into_string();
        assert!(sortable.contains("class=\"w-full caption-bottom text-sm sortable\""));
    }

    #[test]
    fn builder_methods_cover_every_table_prop_variation_and_rendered_attrs() {
        let configured = TableProps::default()
            .with_striped(true)
            .with_hoverable(true)
            .with_size(shared::SIZE_LG)
            .with_sticky_header(true)
            .with_class("table-extra")
            .with_attribute("data-testid", "table");
        assert!(configured.striped);
        assert!(configured.hoverable);
        assert!(configured.sticky_header);
        assert_eq!(configured.size, shared::SIZE_LG);
        assert_eq!(
            configured.build_classes(),
            "w-full caption-bottom text-sm table-extra"
        );
        assert_eq!(
            configured
                .build_attributes()
                .get("data-testid")
                .map(String::as_str),
            Some("table")
        );

        let default_row_attrs = TableRowProps::default()
            .with_clickable(true)
            .build_attributes();
        assert_eq!(
            default_row_attrs.get("tabindex").map(String::as_str),
            Some("0")
        );
        assert_eq!(
            TableRowProps::default().build_classes(),
            "border-b transition-colors duration-150"
        );

        let center_head = TableHeadProps::default()
            .with_align("center")
            .with_class("head-extra")
            .with_attribute("id", "h")
            .build_classes();
        assert_eq!(
            center_head,
            "h-12 px-4 font-medium text-muted-foreground text-center head-extra"
        );
        assert_eq!(
            TableHeadProps::default()
                .with_sortable(true)
                .with_sort_dir("desc")
                .build_attributes()
                .get("aria-sort")
                .map(String::as_str),
            Some("descending")
        );
        assert!(TableHead(
            &TableHeadProps::default()
                .with_align("center")
                .with_attribute("id", "name"),
            html! { "Name" },
        )
        .into_string()
        .contains(r#"<th class="h-12 px-4 font-medium text-muted-foreground text-center" id="name">Name</th>"#));

        let cell = TableCell(
            &TableCellProps::default()
                .with_align("right")
                .with_class("cell-extra")
                .with_attribute("id", "amount"),
            html! { "$10" },
        )
        .into_string();
        assert!(cell.contains(r#"<td class="p-4 text-right cell-extra" id="amount">$10</td>"#));

        let caption = TableCaption(
            &TableCaptionProps::default()
                .with_class("caption-extra")
                .with_attribute("data-testid", "cap")
                .with_position("bottom"),
            html! { "Caption" },
        )
        .into_string();
        assert!(caption.contains("caption-bottom caption-extra"));
        assert!(caption.contains(r#"data-testid="cap""#));
    }
}
