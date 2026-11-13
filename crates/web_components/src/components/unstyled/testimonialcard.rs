#![allow(non_snake_case)]

use crate::components::unstyled::starrating::{StarRating, StarRatingProps};
use crate::components::{ComponentInfo, shared};
use maud::{Markup, html};
use std::collections::BTreeMap;

pub const COMPONENTS: &[ComponentInfo] = &[
    ComponentInfo {
        family: "unstyled",
        module: "testimonialcard",
        name: "TestimonialCard",
    },
    ComponentInfo {
        family: "unstyled",
        module: "testimonialcard",
        name: "SimpleTestimonialCard",
    },
];

#[derive(Clone, Debug, PartialEq)]
pub struct TestimonialCardProps {
    pub quote: String,
    pub name: String,
    pub role: String,
    pub company: String,
    pub company_logo: String,
    pub avatar_url: String,
    pub avatar_alt: String,
    pub rating: f64,
    pub max_rating: i32,
    pub class: String,
    pub attributes: BTreeMap<String, String>,
}

impl Default for TestimonialCardProps {
    fn default() -> Self {
        Self {
            quote: String::new(),
            name: String::new(),
            role: String::new(),
            company: String::new(),
            company_logo: String::new(),
            avatar_url: String::new(),
            avatar_alt: String::new(),
            rating: 5.0,
            max_rating: 5,
            class: String::new(),
            attributes: BTreeMap::new(),
        }
    }
}

impl TestimonialCardProps {
    #[must_use]
    pub fn with_quote(mut self, value: impl Into<String>) -> Self {
        self.quote = value.into();
        self
    }
    #[must_use]
    pub fn with_author(
        mut self,
        name: impl Into<String>,
        role: impl Into<String>,
        company: impl Into<String>,
    ) -> Self {
        self.name = name.into();
        self.role = role.into();
        self.company = company.into();
        self
    }
    #[must_use]
    pub fn with_avatar(mut self, url: impl Into<String>, alt: impl Into<String>) -> Self {
        self.avatar_url = url.into();
        self.avatar_alt = alt.into();
        self
    }
    #[must_use]
    pub fn with_company_logo(mut self, value: impl Into<String>) -> Self {
        self.company_logo = value.into();
        self
    }
    #[must_use]
    pub const fn with_rating(mut self, value: f64) -> Self {
        self.rating = value;
        self
    }
    #[must_use]
    pub const fn with_max_rating(mut self, value: i32) -> Self {
        self.max_rating = value;
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
        shared::build_classes(&["p-6 flex flex-col gap-4", self.class.as_str()])
    }
    #[must_use]
    pub fn build_attributes(&self) -> BTreeMap<String, String> {
        self.attributes.clone()
    }
}

#[must_use]
pub fn author_initial(name: &str) -> String {
    name.chars()
        .next()
        .map_or_else(String::new, |ch| ch.to_string())
}

#[must_use]
pub fn author_line(role: &str, company: &str) -> String {
    if company.is_empty() {
        role.to_string()
    } else {
        format!("{role} at {company}")
    }
}

#[must_use]
pub fn TestimonialCard(props: &TestimonialCardProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        div
            class=(props.build_classes())
            id=[shared::attr_value(&attrs, "id")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
        {
            @if props.rating > 0.0 {
                (StarRating(&StarRatingProps::default()
                    .with_rating(props.rating)
                    .with_max_rating(props.max_rating)
                    .with_size(shared::SIZE_SM)
                    .with_color("yellow")))
            }
            blockquote class="text-lg leading-relaxed" {
                (format!("\"{}\"", props.quote))
            }
            div class="flex items-center gap-3 mt-auto" {
                @if !props.avatar_url.is_empty() {
                    img src=(&props.avatar_url) alt=(&props.avatar_alt) class="w-12 h-12 rounded-full object-cover";
                } @else if !props.name.is_empty() {
                    div class="w-12 h-12 rounded-full bg-muted flex items-center justify-center" {
                        span class="text-xl font-bold" {
                            (author_initial(&props.name))
                        }
                    }
                }
                div class="flex-1" {
                    div class="font-semibold" { (&props.name) }
                    div class="text-sm" {
                        (author_line(&props.role, &props.company))
                    }
                }
                @if !props.company_logo.is_empty() {
                    img
                        src=(&props.company_logo)
                        alt=(format!("{} logo", props.company))
                        class="h-8 w-auto opacity-60";
                }
            }
        }
    }
}

#[must_use]
pub fn SimpleTestimonialCard(quote: &str, name: &str, role: &str, rating: f64) -> Markup {
    TestimonialCard(
        &TestimonialCardProps::default()
            .with_quote(quote)
            .with_author(name, role, "")
            .with_rating(rating),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn helpers_cover_defaults_builders_classes_attributes_initials_and_author_line() {
        let props = TestimonialCardProps::default();
        assert_eq!(props.rating, 5.0);
        assert_eq!(props.max_rating, 5);
        assert_eq!(props.build_classes(), "p-6 flex flex-col gap-4");
        assert_eq!(author_initial("Alice"), "A");
        assert_eq!(author_initial(""), "");
        assert_eq!(author_initial("Élodie"), "É");
        assert_eq!(author_line("Founder", ""), "Founder");
        assert_eq!(author_line("Founder", "Acme"), "Founder at Acme");

        let attrs = TestimonialCardProps::default()
            .with_quote("Quote")
            .with_author("Ada", "CTO", "Acme")
            .with_avatar("/ada.png", "Ada avatar")
            .with_company_logo("/acme.svg")
            .with_rating(4.5)
            .with_max_rating(10)
            .with_class("custom")
            .with_attribute("data-testid", "testimonial")
            .build_attributes();
        assert_eq!(
            attrs.get("data-testid").map(String::as_str),
            Some("testimonial")
        );
        let built = TestimonialCardProps::default()
            .with_quote("Quote")
            .with_author("Ada", "CTO", "Acme")
            .with_avatar("/ada.png", "Ada avatar")
            .with_company_logo("/acme.svg")
            .with_rating(4.5)
            .with_max_rating(10)
            .with_class("custom");
        assert_eq!(built.quote, "Quote");
        assert_eq!(built.name, "Ada");
        assert_eq!(built.role, "CTO");
        assert_eq!(built.company, "Acme");
        assert_eq!(built.avatar_url, "/ada.png");
        assert_eq!(built.avatar_alt, "Ada avatar");
        assert_eq!(built.company_logo, "/acme.svg");
        assert_eq!(built.rating, 4.5);
        assert_eq!(built.max_rating, 10);
        assert_eq!(built.build_classes(), "p-6 flex flex-col gap-4 custom");
    }

    #[test]
    fn renders_rating_quote_avatar_author_company_and_logo() {
        let rendered = TestimonialCard(
            &TestimonialCardProps::default()
                .with_quote("<Reliable>")
                .with_author("Ada", "CTO", "Acme")
                .with_avatar("/avatar.png", "Ada Lovelace")
                .with_company_logo("/logo.svg")
                .with_rating(4.5)
                .with_attribute("id", "card")
                .with_attribute("data-testid", "testimonial"),
        )
        .into_string();
        assert!(rendered.starts_with(
            "<div class=\"p-6 flex flex-col gap-4\" id=\"card\" data-testid=\"testimonial\">"
        ));
        assert!(rendered.contains("aria-label=\"Rating: 4.5 out of 5 stars\""));
        assert!(rendered.contains("<blockquote class=\"text-lg leading-relaxed\">&quot;&lt;Reliable&gt;&quot;</blockquote>"));
        assert!(rendered.contains("<img src=\"/avatar.png\" alt=\"Ada Lovelace\" class=\"w-12 h-12 rounded-full object-cover\">"));
        assert!(rendered.contains("<div class=\"font-semibold\">Ada</div>"));
        assert!(rendered.contains("<div class=\"text-sm\">CTO at Acme</div>"));
        assert!(
            rendered.contains(
                "<img src=\"/logo.svg\" alt=\"Acme logo\" class=\"h-8 w-auto opacity-60\">"
            )
        );
    }

    #[test]
    fn renders_initial_fallback_and_skips_optional_sections() {
        let rendered = TestimonialCard(
            &TestimonialCardProps::default()
                .with_quote("Plain")
                .with_author("Grace", "Engineer", "")
                .with_rating(0.0),
        )
        .into_string();
        assert!(!rendered.contains("role=\"img\""));
        assert!(rendered.contains("<span class=\"text-xl font-bold\">G</span>"));
        assert!(rendered.contains("<div class=\"text-sm\">Engineer</div>"));
        assert!(!rendered.contains("opacity-60"));
        assert!(!rendered.contains("object-cover"));

        let anonymous =
            TestimonialCard(&TestimonialCardProps::default().with_quote("No author")).into_string();
        assert!(!anonymous.contains("text-xl font-bold"));
        assert!(anonymous.contains("<div class=\"font-semibold\"></div>"));
    }

    #[test]
    fn simple_testimonial_card_renders_expected_defaults() {
        let rendered = SimpleTestimonialCard("Great", "Lin", "Founder", 3.0).into_string();
        assert!(rendered.contains(
            "<blockquote class=\"text-lg leading-relaxed\">&quot;Great&quot;</blockquote>"
        ));
        assert!(rendered.contains("<span class=\"text-xl font-bold\">L</span>"));
        assert!(rendered.contains("<div class=\"font-semibold\">Lin</div>"));
        assert!(rendered.contains("<div class=\"text-sm\">Founder</div>"));
        assert!(rendered.contains("aria-label=\"Rating: 3.0 out of 5 stars\""));
    }
}
