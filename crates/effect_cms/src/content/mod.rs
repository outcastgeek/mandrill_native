//! Content: formats, contract payloads, text helpers, assets and shortcodes.
//!
//! Content owns:
//!
//! - the storage formats (`markdown`, `org`, `edn`), the EDN storage prefix, stored
//!   format precedence, source content, the persisted storage-column selection and
//!   the per-language contract body (W17);
//! - the contract payload mapper, the seeded language maps, the draft maps, the
//!   serializable contract value and completeness (W17);
//! - the tag, slug, delimited-value and keyed/paired JSON text helpers, the
//!   HTML sanitation policy and its sanitizer;
//! - the explicit CDN/base configuration and the asset URL rules;
//! - the shortcode vocabulary this owner consumes, the callout and call-to-action
//!   presentation catalog and its markup, and the link shortcode catalog, target
//!   resolvers and localized row selection with consumer-supplied effectful lookups
//!   (W18).
//!
//! # API
//!
//! The public surface keeps the owner's names and responsibilities. Rust needs a
//! name for callback types, so the consumer-supplied callbacks are
//! declared as type aliases ([`ContentLinkShortcodeResolveFn`],
//! [`ContentLinkLookupFn`], [`LocalizedContentLinkRowsFn`] and the row accessors).
//! Every HTML-producing function returns [`maud::Markup`] rather than a `String`,
//! because authored markup must cross the port's rendering boundary as trusted
//! markup.
//!
//! # Consumers, data and optionality
//!
//! The owner holds no store and performs no database read or write. The only
//! consumer-supplied behavior is the link shortcode lookup and row collection, which
//! are required constructor inputs of type
//! `Arc<dyn Fn(..) -> Effect<.., E, R>>`; the CMS invokes those contracts and owns
//! nothing behind them. Every optional callback stays genuinely
//! optional data (`Option<..>`), not an optional required capability: a link
//! resolver with no callback contributes nothing and performs no lookup.
//!
//! The CDN base URL is an explicit [`ContentAssetConfig`] value built by the
//! consumer, which reads [`CONTENT_ASSET_BASE_URL_ENV_KEY`] at composition time. The
//! domain never reads the process environment.
//!
//! # Effects and macros
//!
//! Pure calculations stay pure and are lifted by their callers. The effectful
//! boundaries use the workspace `Effect<A, E, R>` model built with `effect!`: a link
//! resolution runs the consumer's effectful lookup inside the caller's environment
//! and error family, and the presentation resolvers lift their pure decisions with
//! `Effect::sync`. `E` and `R` are the caller's own types; the owner adds no error,
//! environment or cancellation policy, no timeout and no retry.
//!
//! # HTML
//!
//! Every authored HTML path is Maud: the callout, call-to-action and link markup,
//! including their class lists, attributes and omission rules, are Maud templates
//! whose interpolated values are escaped by Maud. Author-supplied HTML is not
//! CMS-authored markup: after the sanitation policy has filtered its tokens, the
//! surviving stream is written by `html5ever`'s HTML serializer, so even that
//! processed-content string comes from the serializer rather than from tag, attribute
//! or text assembly in this owner.
//!
//! # Dependencies
//!
//! `effect` supplies the `Effect` kernel and the `effect` macro, `maud` the required
//! HTML templating, `serde`/`serde_json` the JSON syntax the helpers consume and
//! produce, `regex` the policy's value patterns, and `html5ever` the WHATWG tokenizer
//! and processed-content serializer the sanitizer uses. The localization owner supplies the
//! localized draft and row types it publishes. No database, SQL, schema, migration,
//! pool, backend or credential handling exists anywhere in this owner.
//!
//! # Private implementation
//!
//! `format`, `payload`, `text`, `sanitize`, `assets`, `shortcode`, `presentation` and
//! `links` are private implementation modules of this owner: they hold the
//! declarations and the mechanics, while this module is the owner's deliberate public
//! surface.

mod assets;
mod format;
mod links;
mod payload;
mod presentation;
mod sanitize;
mod shortcode;
mod text;

pub use self::assets::{
    CONTENT_ASSET_BASE_URL_ENV_KEY, ContentAssetConfig, absolute_content_asset_url,
    content_asset_url, is_absolute_asset_url, resolve_content_asset_url,
};
pub use self::format::{
    CONTENT_EDN_STORAGE_PREFIX, CONTENT_FORMAT_EDN, CONTENT_FORMAT_MARKDOWN, CONTENT_FORMAT_ORG,
    ContentStorageColumns, content_format_from_stored_fields, content_storage_columns_for_format,
    normalize_content_format, source_content_from_stored_fields,
};
pub use self::links::{
    ContentLinkBlockFn, ContentLinkCacheKeyFn, ContentLinkDefaultLabelFn, ContentLinkHrefFn,
    ContentLinkLookupFn, ContentLinkShortcodeCatalog, ContentLinkShortcodeCatalogOptions,
    ContentLinkShortcodeRequest, ContentLinkShortcodeResolveFn,
    ContentLinkShortcodeResolverOptions, ContentLinkShortcodeTarget,
    ContentLinkTargetResolverOptions, LocalizedContentLinkRowsFn,
    LocalizedContentLinkShortcodeResolverOptions, new_content_link_shortcode_catalog,
    new_content_link_target_resolver, new_localized_content_link_shortcode_resolver,
};
pub use self::payload::{
    ContractContentBody, ContractPayloadMapper, ContractPayloadMapperOptions,
    clone_contract_language_keys, contract_content_body_from_localized_draft,
    contract_content_complete, contract_content_map, contract_content_map_from_localized_drafts,
    contract_content_value, contract_string_map, contract_string_map_from_localized_content_drafts,
    contract_string_map_from_localized_text_drafts, contract_strings_complete,
    new_contract_payload_mapper, split_comma_separated_unique,
};
pub use self::presentation::{
    PresentationShortcodeCatalog, new_presentation_shortcode_catalog,
    render_callout_shortcode_html, render_cta_shortcode_html, render_shortcode_link_html,
};
pub use self::shortcode::{
    CONTENT_SURFACE_CHAT_HTML, CONTENT_SURFACE_SLIDE_HTML, CONTENT_SURFACE_TRANSCRIPT_HTML,
    ContentRenderOptions, RenderedContentFragment, ShortcodeCatalog, ShortcodeResolver,
    ShortcodeToken, shortcode_arg,
};
pub use self::text::{
    delimited_json_text_pairs, json_text_object_pointer, json_text_object_value,
    json_text_pair_values, optional_string, ptr_or_empty, sanitize_html, slug_filename, slugify,
    split_delimited_values, string_value, tags_from_json, tags_to_json, tags_to_string,
};

/// Compile-time witness that the link resolver ports keep their `Effect` shape.
///
/// The witness spells the complete producer → consumer path: a consumer-supplied
/// lookup is required by the target-resolver options and the catalog constructor is
/// generic in the caller's error and environment, so a resolver can only be built for
/// an `E`/`R` that satisfies the effect kernel's bounds.
const _: fn() = || {
    fn witness<E, R>()
    where
        E: Send + 'static,
        R: Send + Sync + 'static,
    {
        let _: fn(
            ContentLinkTargetResolverOptions<u8, E, R>,
        ) -> ContentLinkShortcodeResolverOptions<E, R> =
            new_content_link_target_resolver::<u8, E, R>;
        let _: fn(
            LocalizedContentLinkShortcodeResolverOptions<u8, E, R>,
        ) -> ContentLinkShortcodeResolverOptions<E, R> =
            new_localized_content_link_shortcode_resolver::<u8, E, R>;
        let _: fn(ContentLinkShortcodeCatalogOptions<E, R>) -> ContentLinkShortcodeCatalog<E, R> =
            new_content_link_shortcode_catalog::<E, R>;
    }
    let _ = witness::<(), ()>;
};

pub(crate) use sanitize::encode_reference_url;
