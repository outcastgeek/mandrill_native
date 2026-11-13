//! Callout and call-to-action shortcode presentation (W18).
//!
//! The owner supplies the callout and call-to-action decision rules, the presentation
//! catalog and the three markup builders.
//!
//! # Details
//!
//! - a callout with no title and no body renders nothing (an empty fragment, not an
//!   empty block);
//! - a call-to-action with no title, body and href renders nothing; a call-to-action
//!   with content and a blank label renders with the label `Learn more`;
//! - the tone is trimmed and lowercased, and only `success`, `warning`,
//!   `danger`/`destructive` and `info` change the class list; every other tone keeps
//!   the default classes;
//! - a rendered callout or call-to-action fragment is block level with no cache key;
//! - the blank-title, blank-body and blank-href paragraphs and links are omitted
//!   exactly where they are omitted, and a link label falls back to the href
//!   when it is blank;
//! - every element name, class list, `href` attribute and text position is the
//!   behavior.
//!
//! # Rendering boundary
//!
//! Every markup string is authored with Maud and returned as [`Markup`].
//! Interpolated values use Maud's escaping, which is the mandated rendering boundary
//! for authored HTML. Title, body, label and href values
//! are ordinary interpolations, so no value reaches the markup unescaped and no
//! markup is assembled by concatenation.
//!
//! The owner injects its two resolve closures into
//! `NewPresentationShortcodeCatalog` so the catalog can call owner-private
//! functions. Rust keeps those functions private to this module, so the catalog is a
//! unit value and the closures are unnecessary; no policy moved.

use std::sync::Arc;

use effect::{Effect, effect_sync};
use maud::{Markup, html};

use super::shortcode::{
    ContentRenderOptions, RenderedContentFragment, ShortcodeCatalog, ShortcodeResolver,
    ShortcodeToken, shortcode_arg,
};

/// The presentation shortcode catalog: `callout` and `cta`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PresentationShortcodeCatalog;

/// Build the presentation shortcode catalog.
#[must_use]
pub fn new_presentation_shortcode_catalog() -> PresentationShortcodeCatalog {
    PresentationShortcodeCatalog
}

impl<E, R> ShortcodeCatalog<E, R> for PresentationShortcodeCatalog
where
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    fn resolvers(&self) -> Vec<Arc<dyn ShortcodeResolver<E, R>>> {
        vec![Arc::new(CalloutResolver), Arc::new(CtaResolver)]
    }
}

/// The `callout` resolver.
struct CalloutResolver;

impl<E, R> ShortcodeResolver<E, R> for CalloutResolver
where
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    fn name(&self) -> String {
        "callout".to_owned()
    }

    fn resolve(
        &self,
        token: ShortcodeToken,
        _options: ContentRenderOptions,
    ) -> Effect<RenderedContentFragment, E, R> {
        effect_sync! { body: move || resolve_callout(&token), }
    }
}

/// The `cta` resolver.
struct CtaResolver;

impl<E, R> ShortcodeResolver<E, R> for CtaResolver
where
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    fn name(&self) -> String {
        "cta".to_owned()
    }

    fn resolve(
        &self,
        token: ShortcodeToken,
        _options: ContentRenderOptions,
    ) -> Effect<RenderedContentFragment, E, R> {
        effect_sync! { body: move || resolve_cta(&token), }
    }
}

/// The callout arguments the decision reads.
struct CalloutInput {
    /// The requested tone.
    tone: String,
    /// The callout title.
    title: String,
    /// The callout body.
    body: String,
}

/// The call-to-action arguments the decision reads.
struct CtaInput {
    /// The call-to-action title.
    title: String,
    /// The call-to-action body.
    body: String,
    /// The call-to-action target.
    href: String,
    /// The call-to-action link label.
    label: String,
}

/// Resolve one `callout` token.
fn resolve_callout(token: &ShortcodeToken) -> RenderedContentFragment {
    let input = CalloutInput {
        body: shortcode_arg(token, &["body", "text", "value"]),
        title: shortcode_arg(token, &["title", "heading"]),
        tone: shortcode_arg(token, &["tone", "variant"]),
    };
    if input.title.is_empty() && input.body.is_empty() {
        return RenderedContentFragment::default();
    }
    RenderedContentFragment {
        html: render_callout_shortcode_html(&input.tone, &input.title, &input.body),
        block: true,
        ..RenderedContentFragment::default()
    }
}

/// Resolve one `cta` token.
fn resolve_cta(token: &ShortcodeToken) -> RenderedContentFragment {
    let mut input = CtaInput {
        title: shortcode_arg(token, &["title", "heading"]),
        body: shortcode_arg(token, &["body", "text", "description"]),
        href: shortcode_arg(token, &["href", "url", "link"]),
        label: shortcode_arg(token, &["label", "cta", "button"]),
    };
    if input.title.is_empty() && input.body.is_empty() && input.href.is_empty() {
        return RenderedContentFragment::default();
    }
    if input.label.is_empty() {
        input.label = "Learn more".to_owned();
    }
    RenderedContentFragment {
        html: render_cta_shortcode_html(&input.title, &input.body, &input.href, &input.label),
        block: true,
        ..RenderedContentFragment::default()
    }
}

/// Render one shortcode link.
///
/// A blank label falls back to the href.
#[must_use]
pub fn render_shortcode_link_html(label: &str, href: &str) -> Markup {
    let trimmed = label.trim();
    let resolved_label = if trimmed.is_empty() { href } else { trimmed };
    html! {
        a class="inline-flex items-center gap-2 rounded-lg border border-border bg-muted/20 px-3 py-2 text-sm font-medium text-foreground no-underline transition-colors hover:bg-muted" href=(href) {
            (resolved_label)
        }
    }
}

/// Render one callout.
#[must_use]
pub fn render_callout_shortcode_html(tone: &str, title: &str, body: &str) -> Markup {
    let resolved_tone = tone.trim().to_lowercase();
    let classes = match resolved_tone.as_str() {
        "success" => "border-border bg-success-muted text-success-foreground",
        "warning" => "border-border bg-warning-muted text-warning-foreground",
        "danger" | "destructive" => {
            "border-border bg-destructive-muted text-destructive-foreground"
        }
        "info" => "border-border bg-info-muted text-info-foreground",
        _ => "border-border bg-muted/30 text-foreground",
    };
    let title = title.trim();
    let body = body.trim();
    html! {
        div class={ "my-5 rounded-xl border px-4 py-3 " (classes) } {
            @if !title.is_empty() {
                p class="text-sm font-semibold" { (title) }
            }
            @if !body.is_empty() {
                p class="mt-2 text-sm leading-relaxed" { (body) }
            }
        }
    }
}

/// Render one call to action.
///
/// The call-to-action link is emitted only when the href is not blank; the label and
/// href are trimmed before rendering.
#[must_use]
pub fn render_cta_shortcode_html(title: &str, body: &str, href: &str, label: &str) -> Markup {
    let title = title.trim();
    let body = body.trim();
    let href = href.trim();
    let label = label.trim();
    html! {
        div class="my-6 rounded-2xl border border-border bg-card p-5 shadow-sm" {
            @if !title.is_empty() {
                p class="text-lg font-semibold text-foreground" { (title) }
            }
            @if !body.is_empty() {
                p class="mt-2 text-sm leading-relaxed text-muted-foreground" { (body) }
            }
            @if !href.is_empty() {
                div class="mt-4" {
                    a class="inline-flex items-center rounded-lg bg-primary px-4 py-2 text-sm font-medium text-primary-foreground no-underline transition-opacity hover:opacity-90" href=(href) {
                        (label)
                    }
                }
            }
        }
    }
}
