//! The shortcode vocabulary this owner consumes.
//!
//! Content consumes five declarations from the shared text-processing surface —
//! `ShortcodeToken`, `ContentRenderSurface` and its constants,
//! `ContentRenderOptions`, `RenderedContentFragment`, and the
//! `ShortcodeCatalog`/`ShortcodeResolver` contracts. That surface is outside this
//! owner's scope; this module declares only the consumed declarations, with their source
//! shapes and no behaviour of the platform silently dropped as excluded from the port.
//!
//! # Differences that are forced by the port's contracts
//!
//! - `RenderedContentFragment.HTML` is a `String`, produced by the
//!   textprocessing renderer and embedded as trusted markup. It is a [`Markup`] here:
//!   the port's rendering boundary must distinguish trusted rendered markup from a
//!   text value a caller might escape again, and every HTML-producing owner in this
//!   port emits [`Markup`]. A sanitized rich-content string is wrapped with
//!   `maud::PreEscaped` at that boundary, never concatenated.
//! - `ContentRenderOptions.Surface` is an open string value, so
//!   its zero value is the empty string and any value is representable. It stays a
//!   [`String`] with the three constants; no closed Rust enum narrows the open value.
//! - Resolving is effectful: a resolver returns a
//!   fragment and an error, which is an [`Effect`] here, and the catalog/resolver
//!   contracts are generic in the caller's error (`E`) and environment (`R`) types.
//!   The effect run's context, not an extra
//!   parameter.
//!
//! These declarations are not a new content policy. When a shared textprocessing
//! port exists, they move there and the content owner imports them.

use std::collections::BTreeMap;
use std::sync::Arc;

use effect::Effect;
use maud::Markup;

/// The render surface for chat views.
pub const CONTENT_SURFACE_CHAT_HTML: &str = "chat_html";

/// The render surface for slide and presentation views.
pub const CONTENT_SURFACE_SLIDE_HTML: &str = "slide_html";

/// The render surface for transcript views.
pub const CONTENT_SURFACE_TRANSCRIPT_HTML: &str = "transcript_html";

/// One parsed shortcode token.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ShortcodeToken {
    /// The raw token text.
    pub raw: String,
    /// The shortcode name.
    pub name: String,
    /// The parsed arguments.
    pub args: BTreeMap<String, String>,
}

/// The viewer and surface metadata a resolution may use.
///
/// Every member is a zero value: an empty surface and empty viewer
/// fields are meaningful defaults, not absent capabilities.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ContentRenderOptions {
    /// The render surface.
    pub surface: String,
    /// The requested content language.
    pub lang: String,
    /// The viewer's role.
    pub viewer_role: String,
    /// The viewer's identity.
    pub viewer_user_id: String,
    /// The target identity of the rendered content.
    pub target_user_id: String,
}

/// The resolved output of one shortcode token.
#[derive(Clone, Debug, Default)]
pub struct RenderedContentFragment {
    /// The rendered markup, already trusted.
    pub html: Markup,
    /// Whether the fragment is block level.
    pub block: bool,
    /// The resolution cache key, empty when the fragment must not be cached by key.
    pub cache_key: String,
    /// Whether the fragment must not be cached at all.
    pub no_cache: bool,
}

/// A resolver of one shortcode name.
///
/// `E` and `R` are the caller's effect error and environment; a resolver holds no
/// capability of its own beyond the consumer-supplied callbacks it was built with.
pub trait ShortcodeResolver<E, R>: Send + Sync
where
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    /// The shortcode name this resolver handles.
    fn name(&self) -> String;

    /// Resolve one token into a rendered fragment.
    fn resolve(
        &self,
        token: ShortcodeToken,
        options: ContentRenderOptions,
    ) -> Effect<RenderedContentFragment, E, R>;
}

/// A contribution of one or more shortcode resolvers.
pub trait ShortcodeCatalog<E, R>: Send + Sync
where
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    /// The resolvers this catalog contributes, in their resolution order.
    fn resolvers(&self) -> Vec<Arc<dyn ShortcodeResolver<E, R>>>;
}

/// The first non-blank trimmed argument among `keys`.
///
/// A shortcode argument: keys are tried in the caller's order and
/// the first non-blank value wins.
#[must_use]
pub fn shortcode_arg(token: &ShortcodeToken, keys: &[&str]) -> String {
    for key in keys {
        if let Some(value) = token.args.get(*key) {
            let value = value.trim();
            if !value.is_empty() {
                return value.to_owned();
            }
        }
    }
    String::new()
}
