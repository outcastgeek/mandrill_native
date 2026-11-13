#![allow(non_snake_case)]

use crate::components::{ComponentInfo, shared};
use maud::{Markup, html};
use std::collections::BTreeMap;

pub const COMPONENTS: &[ComponentInfo] = &[
    ComponentInfo {
        family: "unstyled",
        module: "grid",
        name: "Grid",
    },
    ComponentInfo {
        family: "unstyled",
        module: "grid",
        name: "ResponsiveGrid",
    },
    ComponentInfo {
        family: "unstyled",
        module: "grid",
        name: "TwoColumnGrid",
    },
    ComponentInfo {
        family: "unstyled",
        module: "grid",
        name: "ThreeColumnGrid",
    },
    ComponentInfo {
        family: "unstyled",
        module: "grid",
        name: "FourColumnGrid",
    },
    ComponentInfo {
        family: "unstyled",
        module: "grid",
        name: "GridItem",
    },
];

pub const GRID_AUTO_NONE: &str = "";
pub const GRID_AUTO_FILL: &str = "auto-fill";
pub const GRID_AUTO_FIT: &str = "auto-fit";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GridProps {
    pub cols: i32,
    pub cols_sm: i32,
    pub cols_md: i32,
    pub cols_lg: i32,
    pub cols_xl: i32,
    pub auto_mode: &'static str,
    pub min_item_width: String,
    pub gap: &'static str,
    pub class: String,
    pub attributes: BTreeMap<String, String>,
}

impl Default for GridProps {
    fn default() -> Self {
        Self {
            cols: 1,
            cols_sm: 0,
            cols_md: 0,
            cols_lg: 0,
            cols_xl: 0,
            auto_mode: GRID_AUTO_NONE,
            min_item_width: String::new(),
            gap: shared::SIZE_MD,
            class: String::new(),
            attributes: BTreeMap::new(),
        }
    }
}

impl GridProps {
    #[must_use]
    pub const fn with_cols(mut self, cols: i32) -> Self {
        self.cols = cols;
        self
    }

    #[must_use]
    pub const fn with_cols_sm(mut self, cols: i32) -> Self {
        self.cols_sm = cols;
        self
    }

    #[must_use]
    pub const fn with_cols_md(mut self, cols: i32) -> Self {
        self.cols_md = cols;
        self
    }

    #[must_use]
    pub const fn with_cols_lg(mut self, cols: i32) -> Self {
        self.cols_lg = cols;
        self
    }

    #[must_use]
    pub const fn with_cols_xl(mut self, cols: i32) -> Self {
        self.cols_xl = cols;
        self
    }

    #[must_use]
    pub fn with_auto_mode(mut self, mode: &'static str) -> Self {
        self.auto_mode = mode;
        self
    }

    #[must_use]
    pub fn with_min_item_width(mut self, min_item_width: impl Into<String>) -> Self {
        self.min_item_width = min_item_width.into();
        self
    }

    #[must_use]
    pub fn with_gap(mut self, gap: &'static str) -> Self {
        self.gap = gap;
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
    pub fn is_auto_mode(&self) -> bool {
        !self.auto_mode.is_empty() && self.auto_mode != GRID_AUTO_NONE
    }

    #[must_use]
    pub fn auto_grid_style(&self) -> String {
        if !self.is_auto_mode() {
            return String::new();
        }
        let min_width = if self.min_item_width.is_empty() {
            "250px"
        } else {
            self.min_item_width.as_str()
        };
        format!(
            "grid-template-columns: repeat({}, minmax({}, 1fr))",
            self.auto_mode, min_width
        )
    }

    #[must_use]
    pub fn build_attributes(&self) -> BTreeMap<String, String> {
        self.attributes.clone()
    }
}

#[must_use]
pub fn gridColsClass(cols: i32) -> &'static str {
    match cols {
        1 => "grid-cols-1",
        2 => "grid-cols-2",
        3 => "grid-cols-3",
        4 => "grid-cols-4",
        5 => "grid-cols-5",
        6 => "grid-cols-6",
        7 => "grid-cols-7",
        8 => "grid-cols-8",
        9 => "grid-cols-9",
        10 => "grid-cols-10",
        11 => "grid-cols-11",
        12 => "grid-cols-12",
        _ => "grid-cols-1",
    }
}

fn responsive_grid_cols_class(prefix: &str, cols: i32) -> String {
    if (1..=12).contains(&cols) {
        format!("{prefix}:grid-cols-{cols}")
    } else {
        String::new()
    }
}

#[must_use]
pub fn gridColsSMClass(cols: i32) -> String {
    responsive_grid_cols_class("sm", cols)
}

#[must_use]
pub fn gridColsMDClass(cols: i32) -> String {
    responsive_grid_cols_class("md", cols)
}

#[must_use]
pub fn gridColsLGClass(cols: i32) -> String {
    responsive_grid_cols_class("lg", cols)
}

#[must_use]
pub fn gridColsXLClass(cols: i32) -> String {
    responsive_grid_cols_class("xl", cols)
}

#[must_use]
pub fn gapClass(gap: &str) -> &'static str {
    match gap {
        "none" => "gap-0",
        shared::SIZE_XS => "gap-1",
        shared::SIZE_SM => "gap-2",
        shared::SIZE_MD => "gap-4",
        shared::SIZE_LG => "gap-6",
        shared::SIZE_XL => "gap-8",
        _ => "gap-4",
    }
}

#[must_use]
pub fn colSpanClass(span: i32) -> &'static str {
    match span {
        1 => "col-span-1",
        2 => "col-span-2",
        3 => "col-span-3",
        4 => "col-span-4",
        5 => "col-span-5",
        6 => "col-span-6",
        7 => "col-span-7",
        8 => "col-span-8",
        9 => "col-span-9",
        10 => "col-span-10",
        11 => "col-span-11",
        12 => "col-span-12",
        _ => "",
    }
}

#[must_use]
pub fn Grid(props: &GridProps, children: Markup) -> Markup {
    let attrs = props.build_attributes();
    if props.is_auto_mode() {
        html! {
            div
                class=(shared::build_classes(&["grid", gapClass(props.gap), props.class.as_str()]))
                style=(props.auto_grid_style())
                id=[shared::attr_value(&attrs, "id")]
                title=[shared::attr_value(&attrs, "title")]
                role=[shared::attr_value(&attrs, "role")]
                data-on:click=[shared::attr_value(&attrs, "data-on:click")]
                data-show=[shared::attr_value(&attrs, "data-show")]
            {
                (children)
            }
        }
    } else {
        let sm = gridColsSMClass(props.cols_sm);
        let md = gridColsMDClass(props.cols_md);
        let lg = gridColsLGClass(props.cols_lg);
        let xl = gridColsXLClass(props.cols_xl);
        html! {
            div
                class=(shared::build_classes(&[
                    "grid",
                    gridColsClass(props.cols),
                    sm.as_str(),
                    md.as_str(),
                    lg.as_str(),
                    xl.as_str(),
                    gapClass(props.gap),
                    props.class.as_str(),
                ]))
                id=[shared::attr_value(&attrs, "id")]
                title=[shared::attr_value(&attrs, "title")]
                role=[shared::attr_value(&attrs, "role")]
                data-on:click=[shared::attr_value(&attrs, "data-on:click")]
                data-show=[shared::attr_value(&attrs, "data-show")]
            {
                (children)
            }
        }
    }
}

#[must_use]
pub fn ResponsiveGrid(gap: &'static str, class: impl Into<String>, children: Markup) -> Markup {
    Grid(
        &GridProps::default()
            .with_cols(1)
            .with_cols_sm(2)
            .with_cols_md(3)
            .with_cols_lg(4)
            .with_gap(gap)
            .with_class(class),
        children,
    )
}

#[must_use]
pub fn TwoColumnGrid(gap: &'static str, class: impl Into<String>, children: Markup) -> Markup {
    Grid(
        &GridProps::default()
            .with_cols(1)
            .with_cols_md(2)
            .with_gap(gap)
            .with_class(class),
        children,
    )
}

#[must_use]
pub fn ThreeColumnGrid(gap: &'static str, class: impl Into<String>, children: Markup) -> Markup {
    Grid(
        &GridProps::default()
            .with_cols(1)
            .with_cols_sm(2)
            .with_cols_md(3)
            .with_gap(gap)
            .with_class(class),
        children,
    )
}

#[must_use]
pub fn FourColumnGrid(gap: &'static str, class: impl Into<String>, children: Markup) -> Markup {
    Grid(
        &GridProps::default()
            .with_cols(1)
            .with_cols_sm(2)
            .with_cols_md(3)
            .with_cols_lg(4)
            .with_gap(gap)
            .with_class(class),
        children,
    )
}

#[must_use]
pub fn GridItem(col_span: i32, class: impl Into<String>, children: Markup) -> Markup {
    let class = class.into();
    html! {
        div class=(shared::build_classes(&[colSpanClass(col_span), class.as_str()])) {
            (children)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const COLS: &[(i32, &str)] = &[
        (0, "grid-cols-1"),
        (1, "grid-cols-1"),
        (2, "grid-cols-2"),
        (3, "grid-cols-3"),
        (4, "grid-cols-4"),
        (5, "grid-cols-5"),
        (6, "grid-cols-6"),
        (7, "grid-cols-7"),
        (8, "grid-cols-8"),
        (9, "grid-cols-9"),
        (10, "grid-cols-10"),
        (11, "grid-cols-11"),
        (12, "grid-cols-12"),
        (13, "grid-cols-1"),
    ];
    const GAPS: &[(&str, &str)] = &[
        ("none", "gap-0"),
        (shared::SIZE_XS, "gap-1"),
        (shared::SIZE_SM, "gap-2"),
        (shared::SIZE_MD, "gap-4"),
        (shared::SIZE_LG, "gap-6"),
        (shared::SIZE_XL, "gap-8"),
        ("unknown", "gap-4"),
    ];

    #[test]
    fn default_props_match_go_defaults() {
        let props = GridProps::default();
        assert_eq!(props.cols, 1);
        assert_eq!(props.gap, shared::SIZE_MD);
        assert!(!props.is_auto_mode());
        assert!(props.auto_grid_style().is_empty());
    }

    #[test]
    fn helper_functions_cover_all_columns_breakpoints_gaps_and_spans() {
        for &(cols, expected) in COLS {
            assert_eq!(gridColsClass(cols), expected);
        }
        for cols in 1..=12 {
            assert_eq!(gridColsSMClass(cols), format!("sm:grid-cols-{cols}"));
            assert_eq!(gridColsMDClass(cols), format!("md:grid-cols-{cols}"));
            assert_eq!(gridColsLGClass(cols), format!("lg:grid-cols-{cols}"));
            assert_eq!(gridColsXLClass(cols), format!("xl:grid-cols-{cols}"));
            assert_eq!(colSpanClass(cols), format!("col-span-{cols}"));
        }
        for cols in [0, 13] {
            assert!(gridColsSMClass(cols).is_empty());
            assert!(gridColsMDClass(cols).is_empty());
            assert!(gridColsLGClass(cols).is_empty());
            assert!(gridColsXLClass(cols).is_empty());
            assert!(colSpanClass(cols).is_empty());
        }
        for &(gap, expected) in GAPS {
            assert_eq!(gapClass(gap), expected);
        }
    }

    #[test]
    fn auto_mode_helpers_cover_default_and_custom_min_widths() {
        let fill = GridProps::default().with_auto_mode(GRID_AUTO_FILL);
        assert!(fill.is_auto_mode());
        assert_eq!(
            fill.auto_grid_style(),
            "grid-template-columns: repeat(auto-fill, minmax(250px, 1fr))"
        );

        let fit = GridProps::default()
            .with_auto_mode(GRID_AUTO_FIT)
            .with_min_item_width("16rem");
        assert_eq!(
            fit.auto_grid_style(),
            "grid-template-columns: repeat(auto-fit, minmax(16rem, 1fr))"
        );
    }

    #[test]
    fn build_attributes_returns_custom_attributes() {
        let attrs = GridProps::default()
            .with_attribute("title", "Grid")
            .with_attribute("data-show", "$ready")
            .build_attributes();
        assert_eq!(attrs.get("title"), Some(&"Grid".to_string()));
        assert_eq!(attrs.get("data-show"), Some(&"$ready".to_string()));
    }

    #[test]
    fn grid_renders_static_and_auto_modes_with_escaped_attributes() {
        let static_grid = Grid(
            &GridProps::default()
                .with_cols(2)
                .with_cols_sm(3)
                .with_cols_md(4)
                .with_cols_lg(5)
                .with_cols_xl(6)
                .with_gap(shared::SIZE_LG)
                .with_class("custom")
                .with_attribute("title", "\"Grid\" & more")
                .with_attribute("data-on:click", "$pick = true"),
            html! { "<Item>" },
        )
        .into_string();

        assert!(static_grid.contains("grid grid-cols-2 sm:grid-cols-3 md:grid-cols-4 lg:grid-cols-5 xl:grid-cols-6 gap-6 custom"));
        assert!(static_grid.contains(r#"title="&quot;Grid&quot; &amp; more""#));
        assert!(static_grid.contains(r#"data-on:click="$pick = true""#));
        assert!(static_grid.contains("&lt;Item&gt;"));

        let auto_grid = Grid(
            &GridProps::default()
                .with_auto_mode(GRID_AUTO_FIT)
                .with_min_item_width("20rem")
                .with_gap(shared::SIZE_SM),
            html! { "Auto" },
        )
        .into_string();
        assert!(auto_grid.contains(r#"class="grid gap-2""#));
        assert!(
            auto_grid
                .contains(r#"style="grid-template-columns: repeat(auto-fit, minmax(20rem, 1fr))""#)
        );
        assert!(!auto_grid.contains("grid-cols-"));
    }

    #[test]
    fn convenience_grids_render_expected_breakpoint_classes() {
        let responsive = ResponsiveGrid(shared::SIZE_MD, "r", html! { "R" }).into_string();
        assert!(
            responsive.contains("grid-cols-1 sm:grid-cols-2 md:grid-cols-3 lg:grid-cols-4 gap-4 r")
        );

        let two = TwoColumnGrid(shared::SIZE_SM, "two", html! { "Two" }).into_string();
        assert!(two.contains("grid-cols-1 md:grid-cols-2 gap-2 two"));

        let three = ThreeColumnGrid(shared::SIZE_LG, "three", html! { "Three" }).into_string();
        assert!(three.contains("grid-cols-1 sm:grid-cols-2 md:grid-cols-3 gap-6 three"));

        let four = FourColumnGrid(shared::SIZE_XL, "four", html! { "Four" }).into_string();
        assert!(
            four.contains("grid-cols-1 sm:grid-cols-2 md:grid-cols-3 lg:grid-cols-4 gap-8 four")
        );
    }

    #[test]
    fn grid_item_renders_span_class_custom_class_and_children() {
        for span in 0..=13 {
            let html = GridItem(span, "item", html! { "Body" }).into_string();
            if (1..=12).contains(&span) {
                assert!(html.contains(&format!("col-span-{span}")));
            } else {
                assert!(!html.contains("col-span-"));
            }
            assert!(html.contains("item"));
            assert!(html.ends_with(">Body</div>"));
        }
    }
}
