#![allow(non_snake_case)]

use crate::components::{ComponentInfo, shared};
use maud::{Markup, html};
use std::collections::BTreeMap;

pub const COMPONENTS: &[ComponentInfo] = &[
    ComponentInfo {
        family: "unstyled",
        module: "megamenufooter",
        name: "MegaMenuFooter",
    },
    ComponentInfo {
        family: "unstyled",
        module: "megamenufooter",
        name: "SimpleMegaMenuFooter",
    },
];

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct MegaMenuFooterLink {
    pub text: String,
    pub href: String,
}

impl MegaMenuFooterLink {
    #[must_use]
    pub fn new(text: impl Into<String>, href: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            href: href.into(),
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct MegaMenuFooterProps {
    pub links: Vec<MegaMenuFooterLink>,
    pub class: String,
    pub link_class: String,
    pub attributes: BTreeMap<String, String>,
}

impl MegaMenuFooterProps {
    #[must_use]
    pub fn with_links(mut self, value: Vec<MegaMenuFooterLink>) -> Self {
        self.links = value;
        self
    }
    #[must_use]
    pub fn add_link(mut self, text: impl Into<String>, href: impl Into<String>) -> Self {
        self.links.push(MegaMenuFooterLink::new(text, href));
        self
    }
    #[must_use]
    pub fn with_class(mut self, value: impl Into<String>) -> Self {
        self.class = value.into();
        self
    }
    #[must_use]
    pub fn with_link_class(mut self, value: impl Into<String>) -> Self {
        self.link_class = value.into();
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
            "w-full px-4 py-3 flex items-center gap-6",
            self.class.as_str(),
        ])
    }
    #[must_use]
    pub fn build_link_classes(&self) -> String {
        shared::build_classes(&[
            "text-sm font-medium transition-colors duration-150",
            self.link_class.as_str(),
        ])
    }
    #[must_use]
    pub fn build_attributes(&self) -> BTreeMap<String, String> {
        self.attributes.clone()
    }
}

#[must_use]
pub fn MegaMenuFooter(props: &MegaMenuFooterProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        div
            class=(props.build_classes())
            id=[shared::attr_value(&attrs, "id")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
        {
            @for link in &props.links {
                a href=(&link.href) class=(props.build_link_classes()) {
                    (&link.text)
                }
            }
        }
    }
}

#[must_use]
pub fn SimpleMegaMenuFooter(links: Vec<MegaMenuFooterLink>) -> Markup {
    MegaMenuFooter(&MegaMenuFooterProps::default().with_links(links))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn helpers_cover_links_builders_classes_and_attributes() {
        let link = MegaMenuFooterLink::new("Contact", "/contact");
        assert_eq!(link.text, "Contact");
        assert_eq!(link.href, "/contact");
        let props = MegaMenuFooterProps::default()
            .add_link("A", "/a")
            .with_links(vec![link.clone()])
            .with_class("footer")
            .with_link_class("link")
            .with_attribute("data-testid", "mega-footer");
        assert_eq!(props.links, vec![link]);
        assert_eq!(
            props.build_classes(),
            "w-full px-4 py-3 flex items-center gap-6 footer"
        );
        assert_eq!(
            props.build_link_classes(),
            "text-sm font-medium transition-colors duration-150 link"
        );
        assert_eq!(
            props
                .build_attributes()
                .get("data-testid")
                .map(String::as_str),
            Some("mega-footer")
        );
    }

    #[test]
    fn mega_menu_footer_renders_links_and_escapes_content() {
        let rendered = MegaMenuFooter(
            &MegaMenuFooterProps::default()
                .with_class("border-t")
                .with_link_class("hover:text-primary")
                .with_attribute("id", "footer")
                .with_links(vec![
                    MegaMenuFooterLink::new("<Contact>", "/contact?x=<y>"),
                    MegaMenuFooterLink::new("Pricing", "/pricing"),
                ]),
        )
        .into_string();
        assert!(rendered.starts_with(
            "<div class=\"w-full px-4 py-3 flex items-center gap-6 border-t\" id=\"footer\">"
        ));
        assert!(rendered.contains("<a href=\"/contact?x=&lt;y&gt;\" class=\"text-sm font-medium transition-colors duration-150 hover:text-primary\">&lt;Contact&gt;</a>"));
        assert!(rendered.contains("<a href=\"/pricing\" class=\"text-sm font-medium transition-colors duration-150 hover:text-primary\">Pricing</a>"));
    }

    #[test]
    fn simple_footer_uses_defaults_and_empty_links_render_empty_wrapper() {
        assert_eq!(
            SimpleMegaMenuFooter(vec![MegaMenuFooterLink::new("Docs", "/docs")]).into_string(),
            "<div class=\"w-full px-4 py-3 flex items-center gap-6\"><a href=\"/docs\" class=\"text-sm font-medium transition-colors duration-150\">Docs</a></div>",
        );
        assert_eq!(
            MegaMenuFooter(&MegaMenuFooterProps::default()).into_string(),
            "<div class=\"w-full px-4 py-3 flex items-center gap-6\"></div>",
        );
    }
}
