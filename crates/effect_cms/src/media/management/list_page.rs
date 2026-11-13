//! Store-backed media list pages.
//!
//! The list query is admitted by the caller; this owner asks the consumer store for
//! one page, projects each asset into the canonical list item state and delegates to
//! the rendering owner's definition list builder. The store owns every read; an
//! absent store is the source's empty result.

use std::sync::Arc;

use effect::{Effect, effect, effect_sync};
use effect_server::Request;
use maud::Markup;
use crate::definitions::{Registry, ResourceKind};
use crate::localization::TranslateKeyFunc;
use crate::media::assets::{
    DEFAULT_MEDIA_LIBRARY_PAGE_SIZE, MEDIA_KIND_IMAGE, MediaAssetListResult, MediaAssetRecord,
    MediaAssetStore, MediaAssetURLResolver, media_asset_display_name, media_asset_preview_url, normalize_media_kind, normalize_media_status,
};
use crate::media::surface::MediaQuery;
use crate::rendering::{
    DefinitionResourcePageBuilderOptions, ResourceListItemMetaState, ResourceListItemState,
    ResourceItemActionVM, ResourceItemActionKind, RESOURCE_ITEM_ACTION_KIND_EDIT,
    RESOURCE_ITEM_ACTION_KIND_DELETE,
    ResourceListPageState, ResourceListPageVM, build_definition_list_page_with_items,
    resource_list_current_page,
};

/// Resolves one asset's detail target.
pub type MediaAssetDetailHref =
    Arc<dyn Fn(&Request, &MediaAssetRecord) -> String + Send + Sync + 'static>;

/// Renders the assembled media list page with the caller's Maud renderer.
pub type MediaAssetListRenderPage =
    Arc<dyn Fn(&Request, &ResourceListPageVM) -> Markup + Send + Sync + 'static>;

/// The supplied result and state for one list page.
pub struct MediaAssetListPageOptions<E, R>
where
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    /// The consumer store.
    pub store: Option<Arc<dyn MediaAssetStore<E, R>>>,
    /// The resource registry.
    pub resources: Option<Arc<dyn Registry + Send + Sync>>,
    /// The resource kind.
    pub kind: ResourceKind,
    /// Whether the management surface is requested.
    pub manage: bool,
    /// The supplied query.
    pub query: MediaQuery,
    /// The store result, when already loaded.
    pub result: Option<MediaAssetListResult>,
    /// The default page size.
    pub default_page_size: i64,
    /// The detail target resolver.
    pub detail_href: Option<MediaAssetDetailHref>,
    /// The delete target resolver.
    pub delete_href: Option<MediaAssetDetailHref>,
    /// The preview image kind.
    pub image_kind: String,
    /// The asset URL resolver.
    pub resolve_url: Option<MediaAssetURLResolver>,
    /// The list page state.
    pub state: ResourceListPageState,
    /// The translation port.
    pub translate_key: Option<TranslateKeyFunc<Request>>,
    /// The rendering page options.
    pub page_options: DefinitionResourcePageBuilderOptions<E, R>,
}

impl<E, R> Default for MediaAssetListPageOptions<E, R>
where
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    fn default() -> Self {
        Self {
            store: None,
            resources: None,
            kind: ResourceKind::default(),
            manage: false,
            query: MediaQuery::default(),
            result: None,
            default_page_size: 0,
            detail_href: None,
            delete_href: None,
            image_kind: String::new(),
            resolve_url: None,
            state: ResourceListPageState::default(),
            translate_key: None,
            page_options: DefinitionResourcePageBuilderOptions::default(),
        }
    }
}

/// Configures a store-backed list page builder.
pub struct MediaAssetListPageBuilderOptions<E, R>
where
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    /// The consumer store.
    pub store: Option<Arc<dyn MediaAssetStore<E, R>>>,
    /// The resource registry.
    pub resources: Arc<dyn Registry + Send + Sync>,
    /// The resource kind.
    pub kind: ResourceKind,
    /// Whether the management surface is requested.
    pub manage: bool,
    /// The default page size.
    pub default_page_size: i64,
    /// The detail target resolver.
    pub detail_href: Option<MediaAssetDetailHref>,
    /// The delete target resolver.
    pub delete_href: Option<MediaAssetDetailHref>,
    /// The preview image kind.
    pub image_kind: String,
    /// The asset URL resolver.
    pub resolve_url: Option<MediaAssetURLResolver>,
    /// The state resolver.
    pub state: Option<Arc<dyn Fn(&Request) -> ResourceListPageState + Send + Sync>>,
    /// The translation port.
    pub translate_key: Option<TranslateKeyFunc<Request>>,
    /// The rendering page options.
    pub page_options: DefinitionResourcePageBuilderOptions<E, R>,
    /// Optional renderer for the completed page.
    pub render_page: Option<MediaAssetListRenderPage>,
}

impl<E, R> Clone for MediaAssetListPageBuilderOptions<E, R>
where
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    fn clone(&self) -> Self {
        Self {
            store: self.store.clone(),
            resources: Arc::clone(&self.resources),
            kind: self.kind.clone(),
            manage: self.manage,
            default_page_size: self.default_page_size,
            detail_href: self.detail_href.clone(),
            delete_href: self.delete_href.clone(),
            image_kind: self.image_kind.clone(),
            resolve_url: self.resolve_url.clone(),
            state: self.state.clone(),
            translate_key: self.translate_key.clone(),
            page_options: self.page_options.clone(),
            render_page: self.render_page.clone(),
        }
    }
}

/// The options for one definition-driven media list page.
pub struct BuildMediaAssetListPageOptions<E, R>
where
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    /// The request.
    pub request: Request,
    /// The resource registry.
    pub resources: Arc<dyn Registry + Send + Sync>,
    /// The resource kind.
    pub kind: ResourceKind,
    /// Whether the management surface is requested.
    pub manage: bool,
    /// The supplied query.
    pub query: MediaQuery,
    /// The supplied store result.
    pub result: MediaAssetListResult,
    /// The default page size.
    pub default_page_size: i64,
    /// The detail target resolver.
    pub detail_href: Option<MediaAssetDetailHref>,
    /// The delete target resolver.
    pub delete_href: Option<MediaAssetDetailHref>,
    /// The preview image kind.
    pub image_kind: String,
    /// The asset URL resolver.
    pub resolve_url: Option<MediaAssetURLResolver>,
    /// The list page state.
    pub state: ResourceListPageState,
    /// The translation port.
    pub translate_key: Option<TranslateKeyFunc<Request>>,
    /// The rendering page options.
    pub page_options: DefinitionResourcePageBuilderOptions<E, R>,
}

/// Builds one media list page for a request and query.
pub type MediaAssetListPageBuilder<E, R> = Arc<
    dyn Fn(&Request, MediaQuery) -> Effect<Option<Markup>, E, R> + Send + Sync + 'static,
>;

impl<E, R> Clone for BuildMediaAssetListPageOptions<E, R>
where
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    fn clone(&self) -> Self {
        Self {
            request: self.request.clone(),
            resources: Arc::clone(&self.resources),
            kind: self.kind.clone(),
            manage: self.manage,
            query: self.query.clone(),
            result: self.result.clone(),
            default_page_size: self.default_page_size,
            detail_href: self.detail_href.clone(),
            delete_href: self.delete_href.clone(),
            image_kind: self.image_kind.clone(),
            resolve_url: self.resolve_url.clone(),
            state: self.state.clone(),
            translate_key: self.translate_key.clone(),
            page_options: self.page_options.clone(),
        }
    }
}

/// Construct the store-backed list page builder.
#[must_use]
pub fn new_media_asset_list_page_builder<E, R>(
    options: MediaAssetListPageBuilderOptions<E, R>,
) -> MediaAssetListPageBuilder<E, R>
where
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    Arc::new(move |request, query| {
        let request = request.clone();
        let options = options.clone();
        effect! {
            let request = request.clone();
            let options = options.clone();
            let query = query.clone();
            let result = load_media_asset_list::<E, R>(options.store.clone(), query.clone())?;
            let state = match options.state.as_ref() {
                Some(resolve) => resolve(&request),
                None => ResourceListPageState::default(),
            };
            let page = build_media_asset_list_page(BuildMediaAssetListPageOptions {
                request: request.clone(),
                resources: Arc::clone(&options.resources),
                kind: options.kind.clone(),
                manage: options.manage,
                query: query.clone(),
                result,
                default_page_size: options.default_page_size,
                detail_href: options.detail_href.clone(),
                delete_href: options.delete_href.clone(),
                image_kind: options.image_kind.clone(),
                resolve_url: options.resolve_url.clone(),
                state,
                translate_key: options.translate_key.clone(),
                page_options: options.page_options.clone(),
            })?;
            match options.render_page.as_ref() {
                Some(render) => Some(render(&request, &page)),
                None => None,
            }
        }
    })
}

/// Build one media list page from a supplied result.
#[must_use]
pub fn build_media_asset_list_page<E, R>(
    options: BuildMediaAssetListPageOptions<E, R>,
) -> Effect<ResourceListPageVM, E, R>
where
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    effect! {
        let request = options.request.clone();
        let options = options.clone();
        let fallback_size = if options.default_page_size > 0 {
            options.default_page_size
        } else {
            DEFAULT_MEDIA_LIBRARY_PAGE_SIZE
        };
        let page_size = if options.query.limit > 0 {
            options.query.limit
        } else {
            fallback_size
        };
        let page = resource_list_current_page(options.query.offset, page_size, fallback_size);
        let items = media_asset_list_item_states(
            &request,
            &options.result,
            options.detail_href.as_ref(),
            options.delete_href.as_ref(),
            if options.image_kind.trim().is_empty() {
                MEDIA_KIND_IMAGE
            } else {
                options.image_kind.trim()
            },
            options.resolve_url.as_ref(),
            options.translate_key.as_ref(),
        );
        build_definition_list_page_with_items(
            &request,
            Arc::clone(&options.resources),
            &options.kind,
            options.manage,
            page,
            page_size,
            options.result.total,
            items,
            false,
            options.state,
            options.translate_key,
            options.page_options,
        )?
    }
}

/// Project store records into canonical list item states.
#[must_use]
pub fn media_asset_list_item_states(
    request: &Request,
    result: &MediaAssetListResult,
    detail_href: Option<&MediaAssetDetailHref>,
    delete_href: Option<&MediaAssetDetailHref>,
    image_kind: &str,
    resolve_url: Option<&MediaAssetURLResolver>,
    translate_key: Option<&TranslateKeyFunc<Request>>,
) -> Vec<ResourceListItemState> {
    let translate = |key: &str| match translate_key {
        Some(translate) => translate(request, key),
        None => key.to_owned(),
    };
    let mut states = Vec::with_capacity(result.items.len());
    for asset in &result.items {
        let id = asset.id.trim();
        if id.is_empty() { continue; }
        let detail_href = detail_href.map(|resolve| resolve(request, asset).trim().to_owned()).unwrap_or_default();
        let delete_href = delete_href.map(|resolve| resolve(request, asset).trim().to_owned()).unwrap_or_default();
        let image_url = media_asset_preview_url(asset, image_kind, resolve_url);
        let meta = vec![
            ResourceListItemMetaState {
                value: translate(&format!("media.kind.{}", normalize_media_kind(&asset.kind))),
                ..ResourceListItemMetaState::default()
            },
            ResourceListItemMetaState {
                value: translate(&format!("media.status.{}", normalize_media_status(&asset.status))),
                ..ResourceListItemMetaState::default()
            },
        ];
        let mut actions = Vec::with_capacity(2);
        if !detail_href.is_empty() {
            actions.push(ResourceItemActionVM {
                kind: ResourceItemActionKind::new(RESOURCE_ITEM_ACTION_KIND_EDIT),
                label: translate("common.edit"),
                href: detail_href.clone(),
                ..ResourceItemActionVM::default()
            });
        }
        if !delete_href.is_empty() {
            actions.push(ResourceItemActionVM {
                kind: ResourceItemActionKind::new(RESOURCE_ITEM_ACTION_KIND_DELETE),
                label: translate("common.delete"),
                href: delete_href,
                confirm_message: translate("media.list.delete_confirm"),
            });
        }
        states.push(ResourceListItemState {
            key: id.to_owned(),
            title: media_asset_display_name(asset),
            summary: asset.url.trim().to_owned(),
            detail_href,
            status: asset.status.trim().to_owned(),
            image_url,
            meta,
            actions,
            ..ResourceListItemState::default()
        });
    }
    states
}

/// Load the store page result.
#[must_use]
pub fn load_media_asset_list<E, R>(
    store: Option<Arc<dyn MediaAssetStore<E, R>>>,
    query: MediaQuery,
) -> Effect<MediaAssetListResult, E, R>
where
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    let Some(store) = store else {
        return effect_sync! { body { MediaAssetListResult::default() } };
    };
    effect! {
        let store = store.clone();
        let query = query.clone();
        let result = store.list(query)?;
        result.unwrap_or_default()
    }
}
