//! The SEO document-head markup.
//!
//! The head fragment emits, in order: the description, keyword and robots meta tags;
//! the canonical link; the hreflang alternates; the Open Graph type/title/description/
//! URL/image/locale/site-name tags; the supplied Open Graph tags; the Twitter card and
//! title/description/image tags; the supplied Twitter tags; the extra meta tags; and
//! the JSON-LD script blocks.
//!
//! # Details
//!
//! - Simple tags are emitted exactly when their value is not empty, without trimming.
//! - The keyword list is emitted when it has at least one entry, joined with `, `.
//! - An empty Twitter card falls back to `summary_large_image`.
//! - A supplied Open Graph or Twitter tag is emitted only when its content and its
//!   name/property are both non-empty. An extra tag uses its name first and its
//!   property only when the name is empty.
//! - A JSON-LD block is emitted only when its trimmed text is not empty. The block
//!   content is serialized JSON data, inserted inside the script element without HTML
//!   re-escaping; the script element itself is authored here.
//!
//! # HTML
//!
//! The fragment is authored with Maud, and every dynamic value, including attribute
//! values, uses Maud's escaping. No authored markup is assembled by concatenation.
//! The fragment contains head children only; the enclosing `head` element belongs to
//! the page shell.

use maud::{Markup, PreEscaped, html};

use super::props::SEOProps;

/// Render the SEO head children.
#[must_use]
pub fn seo_head(seo: &SEOProps) -> Markup {
    html! {
        @if !seo.description.is_empty() {
            meta name="description" content=(seo.description);
        }
        @if !seo.keywords.is_empty() {
            meta name="keywords" content=(seo.keywords.join(", "));
        }
        @if !seo.robots.is_empty() {
            meta name="robots" content=(seo.robots);
        }
        @if !seo.url.is_empty() {
            link rel="canonical" href=(seo.url);
        }
        @for hreflang in &seo.hreflangs {
            link rel="alternate" hreflang=(hreflang.lang) href=(hreflang.url);
        }
        @if !seo.page_type.is_empty() {
            meta property="og:type" content=(seo.page_type);
        }
        @if !seo.title.is_empty() {
            meta property="og:title" content=(seo.title);
        }
        @if !seo.description.is_empty() {
            meta property="og:description" content=(seo.description);
        }
        @if !seo.url.is_empty() {
            meta property="og:url" content=(seo.url);
        }
        @if !seo.image.is_empty() {
            meta property="og:image" content=(seo.image);
        }
        @if !seo.locale.is_empty() {
            meta property="og:locale" content=(seo.locale);
        }
        @if !seo.site_name.is_empty() {
            meta property="og:site_name" content=(seo.site_name);
        }
        @for meta in &seo.open_graph_meta {
            @if !meta.property.is_empty() && !meta.content.is_empty() {
                meta property=(meta.property) content=(meta.content);
            }
        }
        @if !seo.twitter_card.is_empty() {
            meta name="twitter:card" content=(seo.twitter_card);
        } @else {
            meta name="twitter:card" content="summary_large_image";
        }
        @if !seo.title.is_empty() {
            meta name="twitter:title" content=(seo.title);
        }
        @if !seo.description.is_empty() {
            meta name="twitter:description" content=(seo.description);
        }
        @if !seo.image.is_empty() {
            meta name="twitter:image" content=(seo.image);
        }
        @for meta in &seo.twitter_meta {
            @if !meta.name.is_empty() && !meta.content.is_empty() {
                meta name=(meta.name) content=(meta.content);
            }
        }
        @for meta in &seo.extra_meta {
            @if !meta.name.is_empty() && !meta.content.is_empty() {
                meta name=(meta.name) content=(meta.content);
            } @else if !meta.property.is_empty() && !meta.content.is_empty() {
                meta property=(meta.property) content=(meta.content);
            }
        }
        @for block in &seo.jsonld_blocks {
            @if !block.trim().is_empty() {
                script type="application/ld+json" { (PreEscaped(block)) }
            }
        }
    }
}
