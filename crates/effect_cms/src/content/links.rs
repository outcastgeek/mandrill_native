//! Link shortcode catalog, resolvers and localized row selection (W18).
//!
//! The owner supplies the link shortcode request and target values, the resolver and
//! catalog options, the link catalog, the two consumer-lookup-based target resolvers,
//! and the resolved/unresolved/failed observations.
//!
//! # Details
//!
//! - a catalog contributes a link resolver only when its name is not blank and it has
//!   a resolve callback; when it includes presentation it appends the `callout` and
//!   `cta` resolvers after the link resolvers;
//! - a resolver reads its value from its value keys and its label from its label keys,
//!   defaulting the label keys to `label`, `title`, `text` when none were configured;
//!   a blank value resolves nothing and never calls the lookup;
//! - a lookup that fails fails the resolution; a target with a blank href is
//!   *unresolved* (a successful empty fragment, no error); a resolved target renders
//!   the link markup with the first non-blank of the request label, the target label
//!   and the target default label, and renders nothing when all three are blank;
//! - the cache key is the target's trimmed key, or `name|value|lang` when the target
//!   supplies none;
//! - a target resolver builds its target from the looked-up row: the default label,
//!   href and cache key are trimmed, the block flag is the caller's predicate, and a
//!   row that is absent yields an empty target, which is unresolved — the
//!   reflection-based zero-row test;
//! - a localized resolver selects the row whose language matches the requested
//!   render language with the caller's [`LocalizedRowOptions`], and falls back to the
//!   first row when no row matches; empty rows yield an empty target. A selected row
//!   slot that is absent is a zero localized row: it stays
//!   unresolved, no accessor is called on it, and it still occupies its place in the
//!   first-row fallback.
//!
//! # Consumer-supplied lookups are effectful capabilities
//!
//! The `Resolve`, `Lookup` and `Rows` callback fields receive the caller's context
//! value. They are consumer-owned data/services: the CMS neither implements them nor
//! reaches any store. Here each one is a required constructor input of type
//! `Arc<dyn Fn(..) -> Effect<.., E, R>>`, so the consumer's query runs inside the
//! caller's effect environment and error family, and the CMS only invokes the supplied
//! contract. No `Option` supplies a *required* capability: these fields stay genuinely
//! optional data, and a missing one performs no lookup.
//!
//! A row-returning lookup returns `Option<T>`: a `(zeroValue, nil)`
//! result is the zero row, which has no Rust equivalent for an unbounded row type, so
//! the consumer states absence explicitly with `None`. A localized row collection
//! states absence per slot the same way, because one slot is kept per row
//! and rejects the selected slot after reflection reports the zero value; selection
//! skips an absent slot's accessors without removing the slot, so the fallback is
//! still the first row. No `Default`, `Clone`, `PartialEq` or reflection machinery is
//! required of the row type; the only bound is `Send + 'static`, which the effect
//! kernel requires of any value carried out of a lookup effect.
//!
//! # Cross-owner notes
//!
//! The label precedence is the queries owner's `FirstNonBlank` rule. That owner is not
//! ported yet, so a private helper holds the rule for now; it is recorded in the crate
//! README and the progress ledger so it is replaced by the published queries contract
//! rather than duplicated permanently.

use std::sync::Arc;

use effect::{Effect, effect, effect_sync};

use super::presentation::{new_presentation_shortcode_catalog, render_shortcode_link_html};
use super::shortcode::{
    ContentRenderOptions, RenderedContentFragment, ShortcodeCatalog, ShortcodeResolver,
    ShortcodeToken, shortcode_arg,
};
use crate::localization::{LocalizedRowOptions, normalize_authoring_language};

/// The label keys a link resolver reads when it configures none.
const DEFAULT_LABEL_KEYS: [&str; 3] = ["label", "title", "text"];

/// One consumer-supplied link resolver.
pub type ContentLinkShortcodeResolveFn<E, R> = Arc<
    dyn Fn(
            ContentLinkShortcodeRequest,
            ContentRenderOptions,
        ) -> Effect<ContentLinkShortcodeTarget, E, R>
        + Send
        + Sync
        + 'static,
>;

/// One consumer-supplied row lookup.
pub type ContentLinkLookupFn<T, E, R> = Arc<
    dyn Fn(ContentLinkShortcodeRequest, ContentRenderOptions) -> Effect<Option<T>, E, R>
        + Send
        + Sync
        + 'static,
>;

/// One consumer-supplied localized row collection.
///
/// The collection keeps one slot per row: `None` is a slot holding a zero localized
/// row, so it is never removed from the
/// collection and no accessor is called on it.
pub type LocalizedContentLinkRowsFn<T, E, R> = Arc<
    dyn Fn(ContentLinkShortcodeRequest, ContentRenderOptions) -> Effect<Vec<Option<T>>, E, R>
        + Send
        + Sync
        + 'static,
>;

/// One consumer-supplied default label accessor.
pub type ContentLinkDefaultLabelFn<T> = Arc<dyn Fn(&T) -> String + Send + Sync + 'static>;

/// One consumer-supplied href accessor.
pub type ContentLinkHrefFn<T> =
    Arc<dyn Fn(&T, &ContentRenderOptions) -> String + Send + Sync + 'static>;

/// One consumer-supplied cache-key accessor.
pub type ContentLinkCacheKeyFn<T> =
    Arc<dyn Fn(&T, &ContentRenderOptions) -> String + Send + Sync + 'static>;

/// One consumer-supplied block-flag accessor.
pub type ContentLinkBlockFn<T> = Arc<dyn Fn(&T) -> bool + Send + Sync + 'static>;

/// A borrowed consumer-supplied render accessor (href or cache key).
type ContentLinkRenderRef<'a, T> = &'a (dyn Fn(&T, &ContentRenderOptions) -> String + Send + Sync);

/// One link shortcode request.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ContentLinkShortcodeRequest {
    /// The parsed token.
    pub token: ShortcodeToken,
    /// The resolved lookup value, empty when the token carried no value.
    pub value: String,
    /// The resolved explicit label, empty when the token carried none.
    pub label: String,
}

impl ContentLinkShortcodeRequest {
    /// The first non-blank trimmed token argument among `keys`.
    #[must_use]
    pub fn arg(&self, keys: &[&str]) -> String {
        shortcode_arg(&self.token, keys)
    }
}

/// One link shortcode target.
///
/// Every member is a zero value when the lookup supplied nothing; an
/// empty [`Self::href`] is the unresolved outcome.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ContentLinkShortcodeTarget {
    /// An explicit label supplied by the consumer.
    pub label: String,
    /// The row's default label.
    pub default_label: String,
    /// The resolved href, empty when unresolved.
    pub href: String,
    /// The explicit cache key, empty when the generic key applies.
    pub cache_key: String,
    /// Whether the rendered fragment is block level.
    pub block: bool,
}

/// The options of one link shortcode resolver.
pub struct ContentLinkShortcodeResolverOptions<E, R>
where
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    /// The shortcode name.
    pub name: String,
    /// The argument keys the lookup value is read from.
    pub value_keys: Vec<String>,
    /// The argument keys the explicit label is read from.
    pub label_keys: Vec<String>,
    /// The consumer's resolution, absent when this resolver contributes nothing.
    pub resolve: Option<ContentLinkShortcodeResolveFn<E, R>>,
}

impl<E, R> Clone for ContentLinkShortcodeResolverOptions<E, R>
where
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    fn clone(&self) -> Self {
        Self {
            name: self.name.clone(),
            value_keys: self.value_keys.clone(),
            label_keys: self.label_keys.clone(),
            resolve: self.resolve.clone(),
        }
    }
}

/// The options of a link shortcode catalog.
pub struct ContentLinkShortcodeCatalogOptions<E, R>
where
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    /// The link resolvers, in their contribution order.
    pub links: Vec<ContentLinkShortcodeResolverOptions<E, R>>,
    /// Whether the presentation resolvers are appended.
    pub include_presentation: bool,
}

impl<E, R> Clone for ContentLinkShortcodeCatalogOptions<E, R>
where
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    fn clone(&self) -> Self {
        Self {
            links: self.links.clone(),
            include_presentation: self.include_presentation,
        }
    }
}

/// The link shortcode catalog.
pub struct ContentLinkShortcodeCatalog<E, R>
where
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    options: ContentLinkShortcodeCatalogOptions<E, R>,
}

impl<E, R> Clone for ContentLinkShortcodeCatalog<E, R>
where
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    fn clone(&self) -> Self {
        Self {
            options: self.options.clone(),
        }
    }
}

impl<E, R> ContentLinkShortcodeCatalog<E, R>
where
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    /// The catalog's options.
    #[must_use]
    pub fn options(&self) -> &ContentLinkShortcodeCatalogOptions<E, R> {
        &self.options
    }
}

/// Build the link shortcode catalog.
#[must_use]
pub fn new_content_link_shortcode_catalog<E, R>(
    options: ContentLinkShortcodeCatalogOptions<E, R>,
) -> ContentLinkShortcodeCatalog<E, R>
where
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    ContentLinkShortcodeCatalog { options }
}

impl<E, R> ShortcodeCatalog<E, R> for ContentLinkShortcodeCatalog<E, R>
where
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    fn resolvers(&self) -> Vec<Arc<dyn ShortcodeResolver<E, R>>> {
        let mut resolvers: Vec<Arc<dyn ShortcodeResolver<E, R>>> =
            Vec::with_capacity(self.options.links.len() + 2);
        for link in &self.options.links {
            if link.name.trim().is_empty() || link.resolve.is_none() {
                continue;
            }
            resolvers.push(Arc::new(ContentLinkShortcodeResolver {
                options: link.clone(),
            }));
        }
        if self.options.include_presentation {
            let presentation: Vec<Arc<dyn ShortcodeResolver<E, R>>> =
                ShortcodeCatalog::<E, R>::resolvers(&new_presentation_shortcode_catalog());
            resolvers.extend(presentation);
        }
        resolvers
    }
}

/// The resolver of one configured link shortcode.
struct ContentLinkShortcodeResolver<E, R>
where
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    options: ContentLinkShortcodeResolverOptions<E, R>,
}

impl<E, R> ShortcodeResolver<E, R> for ContentLinkShortcodeResolver<E, R>
where
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    fn name(&self) -> String {
        self.options.name.trim().to_owned()
    }

    fn resolve(
        &self,
        token: ShortcodeToken,
        options: ContentRenderOptions,
    ) -> Effect<RenderedContentFragment, E, R> {
        let Some(resolve) = self.options.resolve.clone() else {
            return effect_sync! { body: RenderedContentFragment::default, };
        };
        let name = self.options.name.trim().to_owned();
        let value = shortcode_arg(&token, &borrowed_keys(&self.options.value_keys));
        let label_keys = default_content_link_label_keys(&self.options.label_keys);
        let label = shortcode_arg(&token, &borrowed_keys(&label_keys));
        let request = ContentLinkShortcodeRequest {
            token,
            value,
            label,
        };
        if request.value.is_empty() {
            return effect_sync! { body: RenderedContentFragment::default, };
        }
        effect! {
            let resolve = Arc::clone(&resolve);
            let name = name.clone();
            let request = request.clone();
            let options = options.clone();
            let target = resolve(request.clone(), options.clone())?;
            project_content_link(&request, &target, &name, &options)
        }
    }
}

/// The options of a consumer-lookup-based target resolver.
pub struct ContentLinkTargetResolverOptions<T, E, R>
where
    T: Send + 'static,
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    /// The shortcode name.
    pub name: String,
    /// The argument keys the lookup value is read from.
    pub value_keys: Vec<String>,
    /// The argument keys the explicit label is read from.
    pub label_keys: Vec<String>,
    /// The consumer's lookup; `None` means the row is absent.
    pub lookup: Option<ContentLinkLookupFn<T, E, R>>,
    /// The row's default label.
    pub default_label: Option<ContentLinkDefaultLabelFn<T>>,
    /// The row's href.
    pub href: Option<ContentLinkHrefFn<T>>,
    /// The row's cache key.
    pub cache_key: Option<ContentLinkCacheKeyFn<T>>,
    /// The row's block flag.
    pub block: Option<ContentLinkBlockFn<T>>,
}

/// The options of a localized link shortcode resolver.
pub struct LocalizedContentLinkShortcodeResolverOptions<T, E, R>
where
    T: Send + 'static,
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    /// The shortcode name.
    pub name: String,
    /// The argument keys the lookup value is read from.
    pub value_keys: Vec<String>,
    /// The argument keys the explicit label is read from.
    pub label_keys: Vec<String>,
    /// The consumer's row collection; a `None` slot is a zero row.
    pub rows: Option<LocalizedContentLinkRowsFn<T, E, R>>,
    /// The localized row accessors used to select the row.
    pub row_options: LocalizedRowOptions<T>,
    /// The row's default label.
    pub default_label: Option<ContentLinkDefaultLabelFn<T>>,
    /// The row's href.
    pub href: Option<ContentLinkHrefFn<T>>,
    /// The row's cache key.
    pub cache_key: Option<ContentLinkCacheKeyFn<T>>,
    /// The row's block flag.
    pub block: Option<ContentLinkBlockFn<T>>,
}

/// Build a resolver whose target comes from a consumer-supplied row lookup.
#[must_use]
pub fn new_content_link_target_resolver<T, E, R>(
    options: ContentLinkTargetResolverOptions<T, E, R>,
) -> ContentLinkShortcodeResolverOptions<E, R>
where
    T: Send + 'static,
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    let name = options.name.clone();
    let value_keys = options.value_keys.clone();
    let label_keys = options.label_keys.clone();
    let lookup = options.lookup.clone();
    let default_label = options.default_label.clone();
    let href = options.href.clone();
    let cache_key = options.cache_key.clone();
    let block = options.block.clone();
    let resolve: ContentLinkShortcodeResolveFn<E, R> = Arc::new(move |request, render_options| {
        let Some(lookup) = lookup.clone() else {
            return effect_sync! { body: ContentLinkShortcodeTarget::default, };
        };
        let default_label = default_label.clone();
        let href = href.clone();
        let cache_key = cache_key.clone();
        let block = block.clone();
        effect! {
            let lookup = Arc::clone(&lookup);
            let default_label = default_label.clone();
            let href = href.clone();
            let cache_key = cache_key.clone();
            let block = block.clone();
            let request = request.clone();
            let render_options = render_options.clone();
            let row = lookup(request.clone(), render_options.clone())?;
            content_link_target_from_row(
                row.as_ref(),
                &render_options,
                default_label.as_deref(),
                href.as_deref(),
                cache_key.as_deref(),
                block.as_deref(),
            )
        }
    });
    ContentLinkShortcodeResolverOptions {
        name,
        value_keys,
        label_keys,
        resolve: Some(resolve),
    }
}

/// Build a resolver whose target comes from consumer-supplied localized rows.
///
/// The row is the one whose language matches the requested render language; when no
/// row matches, the first row is used, and no rows yield an empty target. A selected
/// slot that is absent is a zero localized row: it yields an empty
/// (unresolved) target and no accessor is called.
#[must_use]
pub fn new_localized_content_link_shortcode_resolver<T, E, R>(
    options: LocalizedContentLinkShortcodeResolverOptions<T, E, R>,
) -> ContentLinkShortcodeResolverOptions<E, R>
where
    T: Send + 'static,
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    let name = options.name.clone();
    let value_keys = options.value_keys.clone();
    let label_keys = options.label_keys.clone();
    let rows = options.rows.clone();
    let row_options = options.row_options.clone();
    let default_label = options.default_label.clone();
    let href = options.href.clone();
    let cache_key = options.cache_key.clone();
    let block = options.block.clone();
    let resolve: ContentLinkShortcodeResolveFn<E, R> = Arc::new(move |request, render_options| {
        let Some(rows) = rows.clone() else {
            return effect_sync! { body: ContentLinkShortcodeTarget::default, };
        };
        let row_options = row_options.clone();
        let default_label = default_label.clone();
        let href = href.clone();
        let cache_key = cache_key.clone();
        let block = block.clone();
        effect! {
            let rows = Arc::clone(&rows);
            let row_options = row_options.clone();
            let default_label = default_label.clone();
            let href = href.clone();
            let cache_key = cache_key.clone();
            let block = block.clone();
            let request = request.clone();
            let render_options = render_options.clone();
            let collected = rows(request.clone(), render_options.clone())?;
            let row = select_localized_row(&collected, &render_options.lang, &row_options);
            content_link_target_from_row(
                row,
                &render_options,
                default_label.as_deref(),
                href.as_deref(),
                cache_key.as_deref(),
                block.as_deref(),
            )
        }
    });
    ContentLinkShortcodeResolverOptions {
        name,
        value_keys,
        label_keys,
        resolve: Some(resolve),
    }
}

/// Render the resolved link fragment.
///
/// The three observations: an unresolved target (blank
/// href) and a target whose labels are all blank render nothing successfully, and a
/// resolved target renders the link markup, the row's block flag and the explicit or
/// generic cache key.
fn project_content_link(
    request: &ContentLinkShortcodeRequest,
    target: &ContentLinkShortcodeTarget,
    name: &str,
    options: &ContentRenderOptions,
) -> RenderedContentFragment {
    if target.href.trim().is_empty() {
        return RenderedContentFragment::default();
    }
    let label = first_non_blank(&[&request.label, &target.label, &target.default_label]);
    if label.is_empty() {
        return RenderedContentFragment::default();
    }
    let cache_key = {
        let supplied = target.cache_key.trim();
        if supplied.is_empty() {
            generic_cache_key(name, &request.value, options)
        } else {
            supplied.to_owned()
        }
    };
    RenderedContentFragment {
        html: render_shortcode_link_html(&label, &target.href),
        block: target.block,
        cache_key,
        no_cache: false,
    }
}

/// Build a target from an optional row.
///
/// `None` is a zero row, which yields an empty (unresolved) target.
fn content_link_target_from_row<T>(
    row: Option<&T>,
    render_options: &ContentRenderOptions,
    default_label: Option<&(dyn Fn(&T) -> String + Send + Sync)>,
    href: Option<ContentLinkRenderRef<'_, T>>,
    cache_key: Option<ContentLinkRenderRef<'_, T>>,
    block: Option<&(dyn Fn(&T) -> bool + Send + Sync)>,
) -> ContentLinkShortcodeTarget {
    let Some(row) = row else {
        return ContentLinkShortcodeTarget::default();
    };
    let mut target = ContentLinkShortcodeTarget::default();
    if let Some(default_label) = default_label {
        target.default_label = default_label(row).trim().to_owned();
    }
    if let Some(href) = href {
        target.href = href(row, render_options).trim().to_owned();
    }
    if let Some(cache_key) = cache_key {
        target.cache_key = cache_key(row, render_options).trim().to_owned();
    }
    if let Some(block) = block {
        target.block = block(row);
    }
    target
}

/// Select the rendered row: the requested-language row, else the first row.
///
/// Selection has two steps. `FindLocalizedRowForRouteLang` scans
/// the rows in order and returns the first row whose normalized language equals the
/// normalized route language; this port scans the present slots in the same order and
/// never calls a language accessor for an absent slot. When no row matches, the
/// falls back to `rows[0]`, so the first *slot* is flattened here — an
/// absent first slot stays unresolved instead of falling through to a later row, and
/// the target builder then invokes no accessor on it.
fn select_localized_row<'rows, T>(
    rows: &'rows [Option<T>],
    route_lang: &str,
    options: &LocalizedRowOptions<T>,
) -> Option<&'rows T> {
    if rows.is_empty() {
        return None;
    }
    let first_row = rows.first().and_then(Option::as_ref);
    let Some(row_language) = options.row_language.as_deref() else {
        return first_row;
    };
    let normalizer = options.normalize_language.as_deref();
    let target = normalize_authoring_language(route_lang, normalizer);
    if let Some(row) = rows
        .iter()
        .filter_map(Option::as_ref)
        .find(|row| normalize_authoring_language(&row_language(*row), normalizer) == target)
    {
        return Some(row);
    }
    first_row
}

/// The configured label keys, or the default keys.
fn default_content_link_label_keys(keys: &[String]) -> Vec<String> {
    if keys.is_empty() {
        return DEFAULT_LABEL_KEYS
            .iter()
            .map(|key| (*key).to_owned())
            .collect();
    }
    keys.to_vec()
}

/// Borrow string keys as the argument-key slices the shortcode helper takes.
fn borrowed_keys(keys: &[String]) -> Vec<&str> {
    keys.iter().map(String::as_str).collect()
}

/// The generic cache key of a resolved link.
fn generic_cache_key(name: &str, value: &str, options: &ContentRenderOptions) -> String {
    let mut key = String::with_capacity(name.len() + value.len() + options.lang.len() + 2);
    key.push_str(name);
    key.push('|');
    key.push_str(value);
    key.push('|');
    key.push_str(options.lang.trim());
    key
}

/// The first non-blank trimmed value.
///
/// This is the queries owner's `FirstNonBlank` rule; see the module documentation for
/// why it is held privately here until that owner publishes it.
fn first_non_blank(values: &[&String]) -> String {
    for value in values {
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            return trimmed.to_owned();
        }
    }
    String::new()
}
