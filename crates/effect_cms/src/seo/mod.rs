//! SEO: page metadata, structured data, sitemap documents and the document head.
//!
//! This owner supplies:
//!
//! - the page SEO value model ([`SEOVM`]) and the render props ([`SEOProps`]) used by
//!   page shells, including canonical URLs, hreflang alternates and locale/site
//!   metadata;
//! - the filtered-list title and description patterns, the filtered canonical URL,
//!   filtered hreflang alternates and the encoded query values they use;
//! - structured data: schema.org object nodes, collection pages, item lists, video
//!   objects and their Open Graph/Twitter metadata, all serialized as JSON text;
//! - sitemap documents, the sitemap index and the sitemap lastmod date/time forms;
//! - the SEO service over the registered resource definitions, with consumer-supplied
//!   validators applied exactly once to the value that is returned;
//! - the document-head markup.
//!
//! # API
//!
//! - [`Service`] (constructed by [`new_service`]) produces the page, sitemap-document
//!   and sitemap-index values as lazy effects. Its [`Options`] carry the translation
//!   port, the canonical-route path builder and the three validators.
//! - Pure projections keep the owner's names and behavior: [`structured_seo_props_in_place`],
//!   [`collection_seo_props_in_place`], [`structured_seo_props`],
//!   [`collection_seo_props`], [`resource_filtered_list_seo_title`],
//!   [`resource_filtered_list_seo_description`], [`resource_filtered_canonical_url`],
//!   [`resource_filtered_hreflangs`], [`resource_filtered_query_values`],
//!   [`seo_props_from_view_model`] and [`resource_seo_props`].
//! - Native composition uses [`resource_seo_props_effect`], [`resource_list_seo_props`],
//!   [`resource_detail_seo_props`] and [`new_resource_seo_props_resolvers`].
//! - [`SEORequestOrigin`] supplies the direct transport fact through request extensions.
//! - Structured data helpers: [`schema_object_json_ld`],
//!   [`schema_object_json_ld_from_map`], [`schema_typed_node`],
//!   [`collection_page_json_ld`], [`item_list_json_ld`],
//!   [`collection_list_json_ld_blocks`], [`video_object_json_ld`],
//!   [`video_open_graph_meta`], [`video_twitter_card`], [`video_twitter_meta`],
//!   [`build_item_list_entries`] and [`json_ld`].
//! - Sitemap helpers: [`definition_sitemap_document`],
//!   [`definition_sitemap_index`], [`normalize_sitemap_lastmod`],
//!   [`format_sitemap_lastmod_date`] and [`format_sitemap_lastmod_date_time`].
//! - Markup: [`seo_head`].
//!
//! # Data and sharing
//!
//! The value shapes keep the observable collection behavior. Props assembly into a
//! reusable base mutates and returns that same value: [`structured_seo_props_in_place`]
//! and [`collection_seo_props_in_place`] take `&mut SEOProps` and return the same
//! reference, while the owned [`structured_seo_props`]/[`collection_seo_props`] are the
//! absent-base allocation path. The structured schema map mutation keeps the caller's
//! map as the published map, so a caller observes the added context/video members.
//! Variable-length props fields are owned collections: moving a caller's collection
//! into the builder keeps it detached from the builder's later changes, while a caller
//! that wants to retain its own copy takes one explicitly.
//!
//! # Capabilities and effects
//!
//! The definition registry is a required constructor capability. Translation,
//! canonical-route construction, locale/site-name resolution and the three validators
//! are consumer-supplied callbacks stored in the owner's option values; absent
//! callbacks are a genuine optional behavior, not a missing required dependency.
//! Consumer render/data providers remain outside this owner.
//!
//! The service operations are lazy [`Effect`](effect::Effect) values under the
//! caller's error family and environment. A lookup failure and a validator failure
//! keep their own [`SEOError`] variants, and a validator error is returned unchanged
//! inside its variant. The validator receives the exact value that is returned, so its
//! in-place mutation is observable and no second validation happens.
//!
//! # HTML
//!
//! [`seo_head`] authors the head elements with Maud. Its JSON-LD block contents are
//! serialized JSON, not authored HTML, and are emitted as such inside the script
//! element.
//!
//! # No store
//!
//! This owner reads no database, owns no table, schema, migration, query, pool or
//! backend, and serves no listener.
//!
//! # Private implementation
//!
//! `props`, `structured`, `documents`, `service`, `publication` and `head` are this
//! owner's implementation modules. `publication` is the private stage pipeline that
//! consumes each prepared value exactly once.

mod documents;
mod head;
mod props;
mod publication;
mod projection;
pub use projection::*;
mod service;
mod structured;

pub use self::documents::{
    SitemapDocumentVM, SitemapEntryVM, SitemapIndexVM, definition_sitemap_document,
    definition_sitemap_index, format_sitemap_lastmod_date, format_sitemap_lastmod_date_time,
    normalize_sitemap_lastmod,
};
pub use self::head::seo_head;
pub use self::props::{
    CollectionSEOPropsInput, Hreflang, ResourceFilteredSEOInput, ResourceSEOPropsOptions,
    SEOCanonicalURLForPathFunc, SEOLocaleFunc, SEOProps, SEOPropsOptions, SEORequestOrigin, SEOSiteNameFunc, SEOVM,
    StructuredSEOPropsInput, collection_seo_props, collection_seo_props_in_place,
    resource_filtered_canonical_url, resource_filtered_hreflangs,
    resource_filtered_list_seo_description, resource_filtered_list_seo_title,
    resource_filtered_query_values, resource_seo_props, seo_props_from_view_model,
    structured_seo_props, structured_seo_props_in_place,
};
pub use self::service::{
    Options, SEOError, SEOValidator, Service, SitemapDocumentValidator, SitemapIndexValidator,
    new_service, seo_indexable,
};
pub use self::structured::{
    ItemListEntry, MetaTag, SchemaObjectJSONLDInput, StructuredVideo, build_item_list_entries,
    collection_list_json_ld_blocks, collection_page_json_ld, item_list_json_ld, json_ld,
    schema_object_json_ld, schema_object_json_ld_from_map, schema_typed_node, video_object_json_ld,
    video_open_graph_meta, video_twitter_card, video_twitter_meta,
};

/// Compile-time witness that the SEO service keeps the owner's effect shape.
///
/// Every operation is usable for any caller error family and environment, and its
/// result stays a lazy [`Effect`](effect::Effect) value carrying that error family.
const _: fn() = || {
    fn witness<E, R>()
    where
        E: Send + 'static,
        R: Send + Sync + 'static,
    {
        #[allow(clippy::type_complexity)]
        let _: fn(
            &Service<E>,
            &effect_server::Request,
            &crate::definitions::ResourceKind,
            &crate::definitions::ResourceViewMode,
        ) -> effect::Effect<SEOVM, SEOError<E>, R> = Service::<E>::page::<R>;
        #[allow(clippy::type_complexity)]
        let _: fn(
            &Service<E>,
            &crate::definitions::ResourceKind,
        ) -> effect::Effect<SitemapDocumentVM, SEOError<E>, R> = Service::<E>::sitemap::<R>;
        #[allow(clippy::type_complexity)]
        let _: fn(&Service<E>) -> effect::Effect<SitemapIndexVM, SEOError<E>, R> =
            Service::<E>::sitemap_index::<R>;
    }
    witness::<std::fmt::Error, ()>();
};
