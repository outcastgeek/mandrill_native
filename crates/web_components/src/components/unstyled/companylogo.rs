#![allow(non_snake_case)]

use crate::components::{ComponentInfo, shared};
use maud::{Markup, html};
use std::collections::BTreeMap;

pub const COMPONENTS: &[ComponentInfo] = &[
    ComponentInfo {
        family: "unstyled",
        module: "companylogo",
        name: "CompanyLogo",
    },
    ComponentInfo {
        family: "unstyled",
        module: "companylogo",
        name: "SimpleCompanyLogo",
    },
];

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CompanyLogoProps {
    pub logo_url: String,
    pub logo_alt: String,
    pub name: String,
    pub size: &'static str,
    pub class: String,
    pub attributes: BTreeMap<String, String>,
}

impl Default for CompanyLogoProps {
    fn default() -> Self {
        Self {
            logo_url: String::new(),
            logo_alt: String::new(),
            name: String::new(),
            size: shared::SIZE_MD,
            class: String::new(),
            attributes: BTreeMap::new(),
        }
    }
}

impl CompanyLogoProps {
    #[must_use]
    pub fn with_logo(mut self, url: impl Into<String>, alt: impl Into<String>) -> Self {
        self.logo_url = url.into();
        self.logo_alt = alt.into();
        self
    }

    #[must_use]
    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
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
    pub fn with_attribute(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.attributes.insert(name.into(), value.into());
        self
    }

    #[must_use]
    pub fn build_classes(&self) -> String {
        let size_class = match self.size {
            shared::SIZE_SM => "h-6",
            shared::SIZE_MD => "h-8",
            shared::SIZE_LG => "h-10",
            shared::SIZE_XL => "h-12",
            _ => "",
        };
        shared::build_classes(&["w-auto transition-opacity", size_class, self.class.as_str()])
    }

    #[must_use]
    pub fn build_attributes(&self) -> BTreeMap<String, String> {
        self.attributes.clone()
    }
}

#[must_use]
pub fn CompanyLogo(props: &CompanyLogoProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        @if !props.logo_url.is_empty() {
            img
                src=(&props.logo_url)
                alt=(&props.logo_alt)
                class=(props.build_classes())
                id=[shared::attr_value(&attrs, "id")]
                title=[shared::attr_value(&attrs, "title")]
                data-testid=[shared::attr_value(&attrs, "data-testid")]
            {}
        } @else if !props.name.is_empty() {
            div class=(format!("flex items-center justify-center font-semibold {}", props.class)) {
                (&props.name)
            }
        }
    }
}

#[must_use]
pub fn SimpleCompanyLogo(logo_url: impl Into<String>, alt: impl Into<String>) -> Markup {
    CompanyLogo(&CompanyLogoProps::default().with_logo(logo_url, alt))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_props_match_go_defaults() {
        assert_eq!(CompanyLogoProps::default().size, shared::SIZE_MD);
    }

    #[test]
    fn helpers_cover_size_classes_attrs_and_builders() {
        for (size, expected) in [
            (shared::SIZE_SM, "h-6"),
            (shared::SIZE_MD, "h-8"),
            (shared::SIZE_LG, "h-10"),
            (shared::SIZE_XL, "h-12"),
            (shared::SIZE_XS, ""),
        ] {
            assert!(
                CompanyLogoProps::default()
                    .with_size(size)
                    .build_classes()
                    .contains(expected)
            );
        }
        let props = CompanyLogoProps::default()
            .with_logo("/logo.png", "Logo")
            .with_name("Name")
            .with_class("extra")
            .with_attribute("title", "Company");
        assert!(props.build_classes().contains("extra"));
        assert_eq!(
            props.build_attributes().get("title").map(String::as_str),
            Some("Company")
        );
    }

    #[test]
    fn company_logo_renders_image_name_fallback_and_empty_state() {
        let image = CompanyLogo(
            &CompanyLogoProps::default()
                .with_logo("/logo?<x>", "<Alt>")
                .with_attribute("data-testid", "logo"),
        )
        .into_string();
        assert!(image.starts_with("<img "));
        assert!(image.contains("src=\"/logo?&lt;x&gt;\""));
        assert!(image.contains("alt=\"&lt;Alt&gt;\""));
        assert!(image.contains("data-testid=\"logo\""));

        let name = CompanyLogo(
            &CompanyLogoProps::default()
                .with_name("<Name>")
                .with_class("extra"),
        )
        .into_string();
        assert_eq!(
            name,
            "<div class=\"flex items-center justify-center font-semibold extra\">&lt;Name&gt;</div>"
        );

        assert_eq!(CompanyLogo(&CompanyLogoProps::default()).into_string(), "");
        assert!(
            SimpleCompanyLogo("/a.png", "A")
                .into_string()
                .contains("src=\"/a.png\"")
        );
    }
}
