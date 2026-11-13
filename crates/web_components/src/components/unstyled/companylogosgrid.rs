#![allow(non_snake_case)]

use crate::components::unstyled::companylogo::{CompanyLogo, CompanyLogoProps};
use crate::components::{ComponentInfo, shared};
use maud::{Markup, html};
use std::collections::BTreeMap;

pub const COMPONENTS: &[ComponentInfo] = &[
    ComponentInfo {
        family: "unstyled",
        module: "companylogosgrid",
        name: "CompanyLogosGrid",
    },
    ComponentInfo {
        family: "unstyled",
        module: "companylogosgrid",
        name: "SimpleCompanyLogosGrid",
    },
];

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CompanyLogosGridProps {
    pub logos: Vec<CompanyLogoProps>,
    pub columns: i32,
    pub columns_md: i32,
    pub columns_sm: i32,
    pub gap: String,
    pub class: String,
    pub attributes: BTreeMap<String, String>,
}

impl Default for CompanyLogosGridProps {
    fn default() -> Self {
        Self {
            logos: Vec::new(),
            columns: 6,
            columns_md: 4,
            columns_sm: 3,
            gap: "8".to_string(),
            class: String::new(),
            attributes: BTreeMap::new(),
        }
    }
}

impl CompanyLogosGridProps {
    #[must_use]
    pub fn with_logos(mut self, logos: Vec<CompanyLogoProps>) -> Self {
        self.logos = logos;
        self
    }

    #[must_use]
    pub const fn with_columns(mut self, sm: i32, md: i32, lg: i32) -> Self {
        self.columns_sm = sm;
        self.columns_md = md;
        self.columns = lg;
        self
    }

    #[must_use]
    pub fn with_gap(mut self, gap: impl Into<String>) -> Self {
        self.gap = gap.into();
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
        let grid_classes = format!(
            "grid grid-cols-{} md:grid-cols-{} lg:grid-cols-{} gap-{} items-center justify-items-center",
            int_to_string(self.columns_sm),
            int_to_string(self.columns_md),
            int_to_string(self.columns),
            self.gap
        );
        shared::build_classes(&[grid_classes.as_str(), self.class.as_str()])
    }

    #[must_use]
    pub fn build_attributes(&self) -> BTreeMap<String, String> {
        self.attributes.clone()
    }
}

#[must_use]
pub fn int_to_string(i: i32) -> &'static str {
    match i {
        1 => "1",
        2 => "2",
        3 => "3",
        4 => "4",
        5 => "5",
        6 => "6",
        _ => "4",
    }
}

#[must_use]
pub fn CompanyLogosGrid(props: &CompanyLogosGridProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        div
            class=(props.build_classes())
            id=[shared::attr_value(&attrs, "id")]
            title=[shared::attr_value(&attrs, "title")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
        {
            @for logo in &props.logos {
                (CompanyLogo(logo))
            }
        }
    }
}

#[must_use]
pub fn SimpleCompanyLogosGrid(logos: Vec<CompanyLogoProps>) -> Markup {
    CompanyLogosGrid(&CompanyLogosGridProps::default().with_logos(logos))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_and_int_to_string_match_go_defaults() {
        let props = CompanyLogosGridProps::default();
        assert_eq!(props.columns, 6);
        assert_eq!(props.columns_md, 4);
        assert_eq!(props.columns_sm, 3);
        assert_eq!(props.gap, "8");
        for (value, expected) in [
            (1, "1"),
            (2, "2"),
            (3, "3"),
            (4, "4"),
            (5, "5"),
            (6, "6"),
            (7, "4"),
        ] {
            assert_eq!(int_to_string(value), expected);
        }
    }

    #[test]
    fn helpers_build_classes_attrs_and_builders() {
        let props = CompanyLogosGridProps::default()
            .with_columns(1, 2, 3)
            .with_gap("6")
            .with_class("extra")
            .with_attribute("title", "Grid");
        assert_eq!(
            props.build_classes(),
            "grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-6 items-center justify-items-center extra"
        );
        assert_eq!(
            props.build_attributes().get("title").map(String::as_str),
            Some("Grid")
        );
    }

    #[test]
    fn company_logos_grid_renders_logos_and_attrs() {
        let logos = vec![
            CompanyLogoProps::default().with_logo("/a.png", "A"),
            CompanyLogoProps::default().with_name("Name"),
        ];
        let html = CompanyLogosGrid(
            &CompanyLogosGridProps::default()
                .with_logos(logos)
                .with_attribute("data-testid", "logos"),
        )
        .into_string();
        assert!(html.starts_with("<div class=\"grid grid-cols-3 md:grid-cols-4 lg:grid-cols-6 gap-8 items-center justify-items-center\""));
        assert!(html.contains("data-testid=\"logos\""));
        assert!(html.contains("src=\"/a.png\""));
        assert!(html.contains(">Name</div>"));

        assert!(
            SimpleCompanyLogosGrid(vec![CompanyLogoProps::default().with_logo("/b.png", "B")])
                .into_string()
                .contains("src=\"/b.png\"")
        );
    }
}
