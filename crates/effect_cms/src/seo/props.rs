//! SEO page values, render props and the filtered-list projection.
//!
//! The value model is plain data: [`SEOVM`] is the page SEO identity a page service
//! publishes, and [`SEOProps`] is the render-facing projection consumed by page
//! shells. The props assemblers, the filtered-list patterns and the canonical/hreflang
//! projection keep their declared order, blank handling and optional callback
//! behavior.
//!
//! # Details
//!
//! - [`structured_seo_props_in_place`] and [`collection_seo_props_in_place`] assemble
//!   into the caller's reusable base: they mutate that same value and return the same
//!   mutable reference, so a caller retaining the base observes every update. The
//!   owned [`structured_seo_props`] and [`collection_seo_props`] are the absent-base
//!   allocation path, starting from an empty value.
//! - Keyword, JSON-LD-block and hreflang collections are owned by the assembled props.
//! - The filtered title/description pattern is consulted only for a filtered query and
//!   only when a translation port is present. A blank translation or one that returns
//!   the key unchanged falls back to the trimmed base text.
//! - A canonical filtered URL is built from the admitted query's search, sort and
//!   declared filter values; a repeated multi-all filter contributes its normalized
//!   values in order.
//! - Hreflang alternates are built for each non-blank language in the supplied order;
//!   `x-default` uses the `en` path prefix, and an absent request yields no
//!   alternates.
//!
//! # Effects
//!
//! Every function here is a pure projection over caller-supplied values. No store is
//! read and no capability is resolved after construction.

use std::collections::BTreeMap;
use std::sync::Arc;

use effect_server::Request;
use http::header::HOST;
use serde::Serialize;

use crate::definitions::{
    RESOURCE_FILTER_MODE_MULTI_ALL, RESOURCE_FILTER_SEARCH, RESOURCE_FILTER_SORT,
    ResourceFilterDefinition, ResourceFilterKey, ResourceKind, ResourceViewMode, SEOPageType,
};
use crate::localization::TranslateKeyFunc;
use crate::queries::{
    DEFAULT_MAX_FILTER_VALUES, FilterValueOptions, QueryService, QueryValues, ResourceListQuery,
    SEARCH_QUERY_PARAM, SORT_QUERY_PARAM, filter_query_param, normalize_filter_values,
    resource_filter_mode, resource_url_with_query,
};

use super::structured::{
    ItemListEntry, MetaTag, StructuredVideo, collection_list_json_ld_blocks, video_open_graph_meta,
    video_twitter_card, video_twitter_meta,
};

/// One hreflang alternate link.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct Hreflang {
    /// The declared language value.
    #[serde(rename = "Lang")]
    pub lang: String,
    /// The absolute alternate URL.
    #[serde(rename = "URL")]
    pub url: String,
}

/// The render-facing SEO metadata for a page shell.
///
/// Serialization keeps the declared contract field names so the value composes with
/// the published page contracts.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct SEOProps {
    /// The page title.
    #[serde(rename = "Title")]
    pub title: String,
    /// The page description.
    #[serde(rename = "Description")]
    pub description: String,
    /// The canonical URL.
    #[serde(rename = "URL")]
    pub url: String,
    /// The share image URL.
    #[serde(rename = "Image")]
    pub image: String,
    /// The Open Graph page type.
    #[serde(rename = "Type")]
    pub page_type: String,
    /// The page locale.
    #[serde(rename = "Locale")]
    pub locale: String,
    /// The site name.
    #[serde(rename = "SiteName")]
    pub site_name: String,
    /// The robots directive.
    #[serde(rename = "Robots")]
    pub robots: String,
    /// The keyword values.
    #[serde(rename = "Keywords")]
    pub keywords: Vec<String>,
    /// The Twitter card type.
    #[serde(rename = "TwitterCard")]
    pub twitter_card: String,
    /// The serialized JSON-LD blocks.
    #[serde(rename = "JSONLDBlocks")]
    pub jsonld_blocks: Vec<String>,
    /// The hreflang alternates.
    #[serde(rename = "Hreflangs")]
    pub hreflangs: Vec<Hreflang>,
    /// Additional meta tags.
    #[serde(rename = "ExtraMeta")]
    pub extra_meta: Vec<MetaTag>,
    /// Additional Open Graph meta tags.
    #[serde(rename = "OpenGraphMeta")]
    pub open_graph_meta: Vec<MetaTag>,
    /// Additional Twitter meta tags.
    #[serde(rename = "TwitterMeta")]
    pub twitter_meta: Vec<MetaTag>,
}

/// The page SEO view model published by the SEO service.
///
/// Serialization keeps the declared contract field names so the page value composes
/// with the published page contracts.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct SEOVM {
    /// The resource kind.
    #[serde(rename = "Kind", serialize_with = "serialize_kind")]
    pub kind: ResourceKind,
    /// The page surface.
    #[serde(rename = "Mode", serialize_with = "serialize_mode")]
    pub mode: ResourceViewMode,
    /// The schema.org page type.
    #[serde(rename = "Type", serialize_with = "serialize_page_type")]
    pub page_type: SEOPageType,
    /// The page title.
    #[serde(rename = "Title")]
    pub title: String,
    /// The page description.
    #[serde(rename = "Description")]
    pub description: String,
    /// The canonical route path.
    #[serde(rename = "CanonicalRoute")]
    pub canonical_route: String,
    /// Whether search engines may index the page.
    #[serde(rename = "Indexable")]
    pub indexable: bool,
    /// Whether the page is included in the sitemap.
    #[serde(rename = "IncludedInSitemap")]
    pub included_in_sitemap: bool,
}

/// Resolves the page locale for one request.
pub type SEOLocaleFunc = Arc<dyn Fn(&Request) -> String + Send + Sync + 'static>;

/// Resolves the site name for one request.
pub type SEOSiteNameFunc = Arc<dyn Fn(&Request) -> String + Send + Sync + 'static>;

/// Builds the absolute canonical URL for a canonical route path.
pub type SEOCanonicalURLForPathFunc = Arc<dyn Fn(&Request, &str) -> String + Send + Sync + 'static>;

/// The environment-derived SEO facts for one request.
///
/// Each field is a genuinely optional callback. The request is the context value
/// passed to the locale, site-name and canonical-URL callbacks.
#[derive(Clone, Default)]
pub struct SEOPropsOptions {
    /// Resolves the page locale.
    pub locale: Option<SEOLocaleFunc>,
    /// Resolves the site name.
    pub site_name: Option<SEOSiteNameFunc>,
    /// Builds the absolute canonical URL for a canonical route path.
    pub canonical_url_for_path: Option<SEOCanonicalURLForPathFunc>,
}

/// Describes a collection page's SEO facts.
///
/// The reusable base is supplied separately to [`collection_seo_props_in_place`];
/// this value carries only the collection's facts.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CollectionSEOPropsInput {
    /// The collection title.
    pub title: String,
    /// The collection description.
    pub description: String,
    /// The collection URL.
    pub url: String,
    /// The number of items in the collection.
    pub total: i64,
    /// The collection item-list entries.
    pub entries: Vec<ItemListEntry>,
    /// The hreflang alternates.
    pub hreflangs: Vec<Hreflang>,
}

/// Describes a single page's SEO facts.
///
/// The reusable base is supplied separately to [`structured_seo_props_in_place`];
/// this value carries only the page's facts.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct StructuredSEOPropsInput {
    /// The page title.
    pub title: String,
    /// The page description.
    pub description: String,
    /// The page URL.
    pub url: String,
    /// The share image URL.
    pub image_url: String,
    /// The keyword values.
    pub keywords: Vec<String>,
    /// The page locale.
    pub locale: String,
    /// The hreflang alternates.
    pub hreflangs: Vec<Hreflang>,
    /// The serialized JSON-LD blocks.
    pub jsonld_blocks: Vec<String>,
    /// The structured video facts, when the page has one.
    pub structured_video: Option<StructuredVideo>,
}

/// Describes a filtered list's SEO inputs.
///
/// The context type is the caller's own translation context value; the translation
/// port is stored only when the caller supplies one.
#[derive(Clone, Default)]
pub struct ResourceFilteredSEOInput<C> {
    /// The translation context value.
    pub context: C,
    /// The unfiltered list title.
    pub base_title: String,
    /// The unfiltered list description.
    pub base_description: String,
    /// The admitted list query.
    pub query: ResourceListQuery,
    /// The display label per filter key.
    pub filter_labels: BTreeMap<ResourceFilterKey, String>,
    /// The translation port.
    pub translate_key: Option<TranslateKeyFunc<C>>,
}

/// Aggregates the SEO projection inputs used by page composition.
///
/// The query service and the rendering value are supplied by the composing caller and
/// are not interpreted by this owner; the render-props options are the same options the
/// projection functions take.
#[derive(Clone, Default)]
pub struct ResourceSEOPropsOptions<Rendering> {
    /// The query admission service value, when the composing caller has one.
    pub queries: Option<QueryService>,
    /// The consumer-supplied rendering value.
    pub rendering: Rendering,
    /// The render-props options.
    pub props: SEOPropsOptions,
}

/// Assemble SEO props into the caller's reusable base.
///
/// The supplied base is mutated in place and returned by the same mutable reference,
/// so a caller that keeps the base observes the update. Video metadata replaces the
/// Open Graph/Twitter metadata and the Twitter card, while every base field the input
/// does not describe keeps its current value. The keyword/JSON-LD-block/hreflang
/// collections are moved from the input into the props, so they are detached from the
/// input value.
#[must_use]
pub fn structured_seo_props_in_place<'a>(
    props: &'a mut SEOProps,
    input: StructuredSEOPropsInput,
) -> &'a mut SEOProps {
    props.title = input.title;
    props.description = input.description;
    props.url = input.url;
    props.image = input.image_url;
    props.locale = input.locale;
    props.keywords = input.keywords;
    props.jsonld_blocks = input.jsonld_blocks;
    props.hreflangs = input.hreflangs;
    props.open_graph_meta = video_open_graph_meta(input.structured_video.as_ref());
    props.twitter_card = video_twitter_card(input.structured_video.as_ref());
    props.twitter_meta = video_twitter_meta(input.structured_video.as_ref());
    props
}

/// Assemble newly allocated SEO props for one page.
///
/// This is the absent-base allocation path; a caller with a reusable base uses
/// [`structured_seo_props_in_place`] so it observes the same value.
#[must_use]
pub fn structured_seo_props(input: StructuredSEOPropsInput) -> SEOProps {
    let mut props = SEOProps::default();
    let _ = structured_seo_props_in_place(&mut props, input);
    props
}

/// Assemble SEO props for one collection page into the caller's reusable base.
///
/// The supplied base is mutated in place and returned by the same mutable reference.
/// The JSON-LD blocks are the collection-page and item-list documents in that order,
/// the hreflang collection is moved from the input into the props, and every base field
/// the input does not describe keeps its current value.
#[must_use]
pub fn collection_seo_props_in_place<'a>(
    props: &'a mut SEOProps,
    input: CollectionSEOPropsInput,
) -> &'a mut SEOProps {
    let jsonld_blocks = collection_list_json_ld_blocks(
        &input.title,
        &input.description,
        &input.url,
        input.total,
        &input.entries,
    );
    props.title = input.title;
    props.description = input.description;
    props.url = input.url;
    props.jsonld_blocks = jsonld_blocks;
    props.hreflangs = input.hreflangs;
    props
}

/// Assemble newly allocated SEO props for one collection page.
///
/// This is the absent-base allocation path; a caller with a reusable base uses
/// [`collection_seo_props_in_place`] so it observes the same value.
#[must_use]
pub fn collection_seo_props(input: CollectionSEOPropsInput) -> SEOProps {
    let mut props = SEOProps::default();
    let _ = collection_seo_props_in_place(&mut props, input);
    props
}

/// Apply the filtered-list title pattern.
#[must_use]
pub fn resource_filtered_list_seo_title<C>(input: &ResourceFilteredSEOInput<C>) -> String {
    filtered_seo_text(input, &input.base_title, "contentmgmt.seo.filtered_title")
}

/// Apply the filtered-list description pattern.
#[must_use]
pub fn resource_filtered_list_seo_description<C>(input: &ResourceFilteredSEOInput<C>) -> String {
    filtered_seo_text(
        input,
        &input.base_description,
        "contentmgmt.seo.filtered_description",
    )
}

/// Apply one filtered-list pattern.
///
/// The base text is trimmed. A blank base is still the fallback value. The pattern is
/// consulted only for a filtered query with a translation port, and a blank pattern
/// or one that returns the key unchanged falls back to the base text.
fn filtered_seo_text<C>(input: &ResourceFilteredSEOInput<C>, base: &str, key: &str) -> String {
    let base = base.trim();
    if !input.query.filtered() {
        return base.to_owned();
    }
    let Some(translate) = input.translate_key.as_ref() else {
        return base.to_owned();
    };
    let filter_text = resource_filtered_seo_filter_text(&input.query, &input.filter_labels);
    if filter_text.is_empty() {
        return base.to_owned();
    }
    let pattern = translate(&input.context, key);
    let pattern = pattern.trim();
    if pattern.is_empty() || pattern == key {
        return base.to_owned();
    }
    format_filtered_pattern(pattern, base, &filter_text)
        .trim()
        .to_owned()
}

/// Join the selected filter values into the display text used by the patterns.
///
/// Each stored value collection is normalized (trimmed, deduplicated, sorted and
/// capped by the query owner's filter options) and joined with `, `. A declared label
/// prefixes the joined values when it is not blank, and the parts are joined with
/// `; `.
fn resource_filtered_seo_filter_text(
    query: &ResourceListQuery,
    labels: &BTreeMap<ResourceFilterKey, String>,
) -> String {
    let options = FilterValueOptions {
        sort: true,
        lowercase: false,
        max_values: DEFAULT_MAX_FILTER_VALUES,
    };
    let mut parts: Vec<String> = Vec::new();
    for (key, selected) in &query.filter_values {
        let values = normalize_filter_values(selected, &options);
        if values.is_empty() {
            continue;
        }
        let label = labels.get(key).map_or("", |label| label.trim());
        let value_text = values.iter().map(|value| crate::byte_text::encoding_text(value.as_bytes())).collect::<Vec<_>>().join(", ");
        if label.is_empty() {
            parts.push(value_text);
        } else {
            parts.push(format!("{label}: {value_text}"));
        }
    }
    parts.join("; ")
}

/// Format the two supplied strings through the shared percent-format boundary.
fn format_filtered_pattern(pattern:&str,base:&str,filter_text:&str)->String {
    crate::text_format::format_strings(pattern,&[base,filter_text])
}

/// Build the filtered-list canonical URL.
///
/// The request is accepted for the resolver contract and is not read; the admitted
/// query's encoded values are appended to the supplied base path.
#[must_use]
pub fn resource_filtered_canonical_url(
    _request: &Request,
    base_path: &str,
    query: &ResourceListQuery,
    filter_definitions: &[ResourceFilterDefinition],
) -> String {
    resource_url_with_query(
        base_path,
        &resource_filtered_query_values(query, filter_definitions),
    )
}

/// Build the filtered-list hreflang alternates.
///
/// A blank language is skipped. `x-default` uses the `en` path prefix. An absent
/// request yields no alternates. The base URL keeps the request host; the
/// direct transport TLS fact takes precedence. Otherwise a recognized forwarded
/// protocol selects the scheme, with `http` as the fallback.
#[must_use]
pub fn resource_filtered_hreflangs(
    request: Option<&Request>,
    path_without_lang: &str,
    query: &ResourceListQuery,
    filter_definitions: &[ResourceFilterDefinition],
    langs: &[String],
) -> Vec<Hreflang> {
    let Some(request) = request else {
        return Vec::new();
    };
    let path_without_lang = ensure_leading_slash(path_without_lang);
    let values = resource_filtered_query_values(query, filter_definitions);
    let base_url = request_base_url(Some(request));
    let mut hreflangs = Vec::with_capacity(langs.len());
    for lang in langs {
        let lang = lang.trim();
        if lang.is_empty() {
            continue;
        }
        let path_lang = if lang == "x-default" { "en" } else { lang };
        let path = format!(
            "/{}{}",
            path_lang.trim_matches('/'),
            path_without_lang
        );
        hreflangs.push(Hreflang {
            lang: lang.to_owned(),
            url: format!("{base_url}{}", resource_url_with_query(&path, &values)),
        });
    }
    hreflangs
}

/// Encode the admitted query's search, sort and filter values.
///
/// A non-blank search and sort are always encoded. Each declared filter then
/// contributes its query parameter: search/sort filters mirror the corresponding
/// query field, a multi-all filter contributes its normalized values when any exist,
/// and every other filter contributes its trimmed single value when it is not blank.
#[must_use]
pub fn resource_filtered_query_values(
    query: &ResourceListQuery,
    filter_definitions: &[ResourceFilterDefinition],
) -> QueryValues {
    let mut values = QueryValues::new();
    let search = query.search.trim();
    if !search.is_empty() {
        values.insert(SEARCH_QUERY_PARAM.into(), vec![search.to_owned()]);
    }
    let sort = query.sort.trim();
    if !sort.is_empty() {
        values.insert(SORT_QUERY_PARAM.into(), vec![sort.to_owned()]);
    }
    for definition in filter_definitions {
        let param = filter_query_param(definition);
        match definition.key.as_str() {
            RESOURCE_FILTER_SEARCH => {
                let search = query.search.trim();
                if !search.is_empty() {
                    values.insert(param.into(), vec![search.to_owned()]);
                }
            }
            RESOURCE_FILTER_SORT => {
                let sort = query.sort.trim();
                if !sort.is_empty() {
                    values.insert(param.into(), vec![sort.to_owned()]);
                }
            }
            _ => {
                if resource_filter_mode(definition).as_str() == RESOURCE_FILTER_MODE_MULTI_ALL {
                    let filter_values = normalize_filter_values(
                        &query.values_for_filter(&definition.key),
                        &FilterValueOptions {
                            sort: true,
                            lowercase: false,
                            max_values: DEFAULT_MAX_FILTER_VALUES,
                        },
                    );
                    if !filter_values.is_empty() {
                        values.insert(param.into(), filter_values);
                    }
                    continue;
                }
                let value = query.filter_value(&definition.key);
                let value = value.trim();
                if !value.is_empty() {
                    values.insert(param.into(), vec![value.to_owned()]);
                }
            }
        }
    }
    values
}

/// Project an SEO view model into render props.
///
/// An absent view model yields empty props. The page type is the declared schema.org
/// type text. The locale and site name callbacks are consulted only when present; a
/// non-indexable page carries the `noindex,follow` robots directive; and the canonical
/// URL is the canonical-URL callback's result for a non-blank canonical route.
#[must_use]
pub fn seo_props_from_view_model(
    request: &Request,
    vm: Option<&SEOVM>,
    options: &SEOPropsOptions,
) -> SEOProps {
    let Some(vm) = vm else {
        return SEOProps::default();
    };
    let mut props = SEOProps {
        title: vm.title.clone(),
        description: vm.description.clone(),
        page_type: vm.page_type.as_str().to_owned(),
        ..SEOProps::default()
    };
    if let Some(locale) = options.locale.as_ref() {
        props.locale = locale(request);
    }
    if let Some(site_name) = options.site_name.as_ref() {
        props.site_name = site_name(request);
    }
    if !vm.indexable {
        props.robots = "noindex,follow".to_owned();
    }
    if !vm.canonical_route.is_empty() {
        if let Some(build) = options.canonical_url_for_path.as_ref() {
            props.url = build(request, &vm.canonical_route);
        }
    }
    props
}

/// Project a page value into SEO props through the supplied query/render/view
/// callbacks.
///
/// An absent query or render callback yields empty props. A render failure or an
/// absent page yields empty props. The view callback's absent view model yields empty
/// props. The callbacks run when this function is called, in the declared
/// query-then-render-then-view order.
#[must_use]
pub fn resource_seo_props<Query, Page, RenderError>(
    request: &Request,
    kind: &ResourceKind,
    options: &SEOPropsOptions,
    query: Option<&dyn Fn(&Request, &ResourceKind) -> Query>,
    render: Option<&dyn Fn(&Request, &ResourceKind, Query) -> Result<Option<Page>, RenderError>>,
    view: &dyn Fn(&Page) -> Option<SEOVM>,
) -> SEOProps {
    let (Some(query), Some(render)) = (query, render) else {
        return SEOProps::default();
    };
    let Ok(page) = render(request, kind, query(request, kind)) else {
        return SEOProps::default();
    };
    let Some(page) = page else {
        return SEOProps::default();
    };
    let vm = view(&page);
    seo_props_from_view_model(request, vm.as_ref(), options)
}

/// Direct transport facts supplied by the native host as a request extension.
/// URI schemes and forwarded headers do not identify a direct TLS connection.
#[derive(Clone,Copy,Debug,Default,Eq,PartialEq)]
pub struct SEORequestOrigin { pub direct_tls:bool }

/// Direct TLS takes precedence; an insecure connection may use the forwarded protocol.
fn request_base_url(request:Option<&Request>)->String {
    let Some(request)=request else{return String::new()};
    let host=request.uri().authority().filter(|a|!a.as_str().is_empty()).map(|a|a.as_str().to_owned())
        .unwrap_or_else(||request.headers().get(HOST).map(|h|crate::byte_text::encoding_text(h.as_bytes())).unwrap_or_default());
    let direct_tls=request.with_extensions(|extensions|extensions.get::<SEORequestOrigin>().is_some_and(|origin|origin.direct_tls));
    let scheme=if direct_tls {"https"} else {
        match request.headers().get("x-forwarded-proto").and_then(|h|h.to_str().ok()).map(str::trim){Some("https")=>"https",_=>"http"}
    };
    format!("{scheme}://{host}")
}

/// Ensure one leading slash, mapping a blank path to `/`.
fn ensure_leading_slash(path: &str) -> String {
    let path = path.trim();
    if path.is_empty() {
        return "/".to_owned();
    }
    if path.starts_with('/') {
        return path.to_owned();
    }
    format!("/{path}")
}

/// Serialize a resource kind as its own text.
fn serialize_kind<S>(value: &ResourceKind, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    serializer.serialize_str(value.as_str())
}

/// Serialize a view mode as its own text.
fn serialize_mode<S>(value: &ResourceViewMode, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    serializer.serialize_str(value.as_str())
}

/// Serialize a schema.org page type as its own text.
fn serialize_page_type<S>(value: &SEOPageType, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    serializer.serialize_str(value.as_str())
}
