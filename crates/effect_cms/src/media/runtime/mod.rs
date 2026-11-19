//! Media runtime: the store-backed runtime, request/query resolvers and picker.
//!
//! The runtime holds the consumer's asset and attachment stores plus the media
//! surface policy the source runtime exposes: image kind, attachment roles, scope
//! normalization, asset/route URL resolvers, current-user identity, context labels,
//! mounts and page-size/total-page defaults. It composes the canonical attachment
//! and asset workflows; it owns no database, SQL, schema, migration, pool or host.
//!
//! Pickering runs the source order — store list, surface decode, page publication —
//! as private prepared stages. The presentation owner supplies the page value through
//! a publisher callback, so no second picker view model is declared here. Every entry
//! point is a lazy reusable [`Effect`]; defects, interruption and cleanup failures
//! keep their own channels.

mod picker;

use crate::commands::InputBytes;
use crate::validation::trim_unicode_whitespace;
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use effect::{AtomicRef, Effect, effect, effect_sync};
use effect_server::{ByteFormData, Request, ServerError};
use uuid::Uuid;

use crate::attachments::{
    MEDIA_ATTACHMENT_ROLE_GALLERY, MEDIA_ATTACHMENT_ROLE_HERO, MEDIA_ATTACHMENT_ROLE_INLINE,
    MediaAttachmentFormItem, MediaAttachmentManagerProps, MediaAttachmentRoleOptions,
    MediaSurfaceMounts, RoleNormalizer, normalize_media_attachment_role,
};
use crate::commands::{CommandInput, ResourceCommandBindings};
use crate::content::PreparedCmsContent;
use effect::memory::MemoryError;
use crate::validation::Bindings;

use crate::definitions::{
    RESOURCE_AUX_ROUTE_ATTACHMENT_MUTATE, RESOURCE_AUX_ROUTE_INLINE_CREATE,
    RESOURCE_AUX_ROUTE_INLINE_PREVIEW, RESOURCE_AUX_ROUTE_PICKER, RESOURCE_AUX_ROUTE_PICKER_ATTACH,
    RESOURCE_AUX_ROUTE_SHORTCODE, ResourceKind, ResourceRouteSet, find_resource_definition,
};
use crate::media::assets::{
    ContentMediaAttachmentStore, DEFAULT_MEDIA_LIBRARY_PAGE_SIZE, MEDIA_KIND_IMAGE,
    MediaAssetFormInput, MediaAssetIdGenerator, MediaAssetListResult, MediaAssetNowUnix,
    MediaAssetRecord, MediaAssetStore, MediaAssetURLResolver, MediaStoreAttachmentOptions,
    apply_media_asset_form_record, list_media_attachment_form_items_from_store,
    list_prepared_media_attachment_form_items_from_store,
    media_shortcode_items_from_result, prepare_record_for_create, prepare_record_for_update,
    record_from_form, replace_media_attachments_from_store_request,
    replace_prepared_media_attachments_from_store_request,
};
use crate::attachments::MediaAttachmentInput;
use crate::media::attachmentflow::{
    CurrentMediaAttachmentInputOptions, MediaAttachmentAssetLoader, MediaAttachmentItemBuilder,
    MediaAttachmentItemLoader, MediaAttachmentPreviewOptions,
    PreparedAttachmentRows,
    current_media_attachment_inputs, preview_media_attachment_form_items_from_inputs,
    preview_media_attachment_form_items_from_request,
    preview_prepared_media_attachment_form_items_from_request,
};
use crate::media::assets::media_attachment_form_item_from_records;
use crate::media::surface::{
    DEFAULT_MEDIA_CREATE_ROOT_ID, DEFAULT_MEDIA_PICKER_ROOT_ID, MEDIA_SURFACE_INLINE,
    MediaQuery, MediaSurfaceOptions, MediaSurfaceRequest, build_media_attachment_manager_from_request,
    media_query_from_request, parse_media_surface_request,
};
use crate::queries::first_non_blank;
use crate::routing::RoutePathBuilder;

use self::picker::run_picker_pipeline;

/// Resolves a URL for one request.
pub type MediaRequestURLResolver = Arc<dyn Fn(&Request) -> String + Send + Sync + 'static>;
/// Resolves a request-derived value.
pub type MediaRequestValueResolver = MediaRequestURLResolver;
/// Resolves a request-localized message.
pub type MediaRequestMessage = MediaRequestURLResolver;
/// Resolves an asset URL for one request.
pub type MediaRequestAssetURLResolver =
    Arc<dyn Fn(&Request, &[u8]) -> InputBytes + Send + Sync + 'static>;
/// Translates one dashboard text key.
pub type TranslateDashboardTextFunc = crate::media::presentation::TranslateDashboardTextFunc;

/// The default owner-form suffix used when no manager is built.
pub const DEFAULT_MEDIA_OWNER_FORM_SUFFIX: &str = "-form";

/// Supplies the configured media runtime.
pub trait MediaAssetRuntimeProvider<E, R>
where
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    /// The store-backed runtime.
    fn runtime(&self) -> MediaAssetStoreRuntime<E, R>;
}

/// Supplies the configured media controller.
pub trait MediaAssetControllerProvider<Controller> {
    /// The management controller.
    fn controller(&self) -> Controller;
}

/// The source media service contract over the canonical command/validation binders.
pub trait MediaAssetService<E, R>: MediaAssetRuntimeProvider<E, R>
where
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    /// Bind this service's resource command executors.
    fn bind_resource_commands(&self, bindings: &ResourceCommandBindings<E, R>);

    /// Bind this service's resource validators.
    fn bind_resource_validation(&self, bindings: &Bindings<CommandInput, E, R>);
}

/// The route names the runtime resolves.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct MediaAssetStoreRuntimeRoutes {
    /// New-form route.
    pub new: String,
    /// Edit-form route.
    pub edit: String,
    /// Picker route.
    pub picker: String,
    /// Attachment-mutate route.
    pub attachment_mutate: String,
    /// Library route.
    pub library: String,
    /// Inline-create route.
    pub inline_create: String,
    /// Inline-preview route.
    pub inline_preview: String,
    /// Shortcode route.
    pub shortcode: String,
    /// Picker-attach route.
    pub picker_attach: String,
}

/// Options for the store-backed runtime.
pub struct MediaAssetStoreRuntimeOptions<E, R>
where
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    /// Consumer asset store.
    pub assets: Option<Arc<dyn MediaAssetStore<E, R>>>,
    /// Consumer attachment store.
    pub attachments: Option<Arc<dyn ContentMediaAttachmentStore<E, R>>>,
    /// Default scope name.
    pub default_scope: String,
    /// Hero scope name.
    pub hero_scope: String,
    /// Optional scope normalizer.
    pub normalize_scope: Option<Arc<dyn Fn(&[u8]) -> InputBytes + Send + Sync + 'static>>,
    /// Optional request asset URL resolver.
    pub resolve_url: Option<MediaRequestAssetURLResolver>,
    /// Optional edit URL resolver.
    pub edit_url: Option<Arc<dyn Fn(&Request, &MediaAssetRecord) -> String + Send + Sync>>,
    /// Optional current-user identity resolver.
    pub current_user_id: Option<Arc<dyn Fn(&Request) -> String + Send + Sync>>,
    /// Optional context label resolver.
    pub context_label:
        Option<Arc<dyn Fn(&Request, &MediaSurfaceRequest) -> String + Send + Sync>>,
    /// Optional route path builder.
    pub route_path: Option<RoutePathBuilder>,
    /// Runtime route names.
    pub routes: MediaAssetStoreRuntimeRoutes,
    /// Optional mounts resolver.
    pub mounts_for_scope: Option<crate::media::surface::MountsResolver>,
    /// Default page size.
    pub default_page_size: i64,
    /// Optional total-page resolver.
    pub total_pages: Option<Arc<dyn Fn(i64, i64) -> i64 + Send + Sync>>,
    /// Optional identity generator; the source's generated identity is the default.
    pub new_id: Option<MediaAssetIdGenerator>,
    /// Optional time source; the source's current Unix time is the default.
    pub now_unix: Option<MediaAssetNowUnix>,
}

impl<E, R> Default for MediaAssetStoreRuntimeOptions<E, R>
where
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    fn default() -> Self {
        Self {
            assets: None,
            attachments: None,
            default_scope: String::new(),
            hero_scope: String::new(),
            normalize_scope: None,
            resolve_url: None,
            edit_url: None,
            current_user_id: None,
            context_label: None,
            route_path: None,
            routes: MediaAssetStoreRuntimeRoutes::default(),
            mounts_for_scope: None,
            default_page_size: 0,
            total_pages: None,
            new_id: None,
            now_unix: None,
        }
    }
}

impl<E, R> Clone for MediaAssetStoreRuntimeOptions<E, R>
where
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    fn clone(&self) -> Self {
        Self {
            assets: self.assets.clone(),
            attachments: self.attachments.clone(),
            default_scope: self.default_scope.clone(),
            hero_scope: self.hero_scope.clone(),
            normalize_scope: self.normalize_scope.clone(),
            resolve_url: self.resolve_url.clone(),
            edit_url: self.edit_url.clone(),
            current_user_id: self.current_user_id.clone(),
            context_label: self.context_label.clone(),
            route_path: self.route_path.clone(),
            routes: self.routes.clone(),
            mounts_for_scope: self.mounts_for_scope.clone(),
            default_page_size: self.default_page_size,
            total_pages: self.total_pages.clone(),
            new_id: self.new_id.clone(),
            now_unix: self.now_unix.clone(),
        }
    }
}

/// Options for binding a runtime from consumer values.
pub struct MediaAssetStoreRuntimeBindingOptions<E, R>
where
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    /// Consumer asset store.
    pub assets: Option<Arc<dyn MediaAssetStore<E, R>>>,
    /// Consumer attachment store.
    pub attachments: Option<Arc<dyn ContentMediaAttachmentStore<E, R>>>,
    /// Asset URL resolver.
    pub resolve_url: Option<MediaAssetURLResolver>,
    /// Current-user identity resolver.
    pub current_user_id: Option<Arc<dyn Fn(&Request) -> String + Send + Sync>>,
    /// Dashboard text translator.
    pub translate: Option<TranslateDashboardTextFunc>,
    /// Default scope name.
    pub default_scope: String,
    /// Hero scope name.
    pub hero_scope: String,
    /// Allowed scope names.
    pub scopes: Vec<String>,
    /// Default context label key.
    pub context_default_key: String,
    /// Hero context scope.
    pub context_hero_scope: String,
    /// Hero context label key.
    pub context_hero_key: String,
    /// Scope-specific context label keys.
    pub context_scope_keys: BTreeMap<String, String>,
    /// Optional mounts resolver.
    pub mounts_for_scope: Option<crate::media::surface::MountsResolver>,
    /// Scope-to-mounts map.
    pub mounts: BTreeMap<String, MediaSurfaceMounts>,
    /// Route path builder.
    pub route_path: Option<RoutePathBuilder>,
    /// Runtime route names.
    pub routes: MediaAssetStoreRuntimeRoutes,
    /// Resource registry used to fill route names when absent.
    pub resources: Option<Arc<dyn crate::definitions::Registry + Send + Sync>>,
    /// Resource kind used to fill route names when absent.
    pub resource_kind: ResourceKind,
    /// Default page size.
    pub default_page_size: i64,
    /// Optional total-page resolver.
    pub total_pages: Option<Arc<dyn Fn(i64, i64) -> i64 + Send + Sync>>,
}

impl<E, R> Default for MediaAssetStoreRuntimeBindingOptions<E, R>
where
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    fn default() -> Self {
        Self {
            assets: None,
            attachments: None,
            resolve_url: None,
            current_user_id: None,
            translate: None,
            default_scope: String::new(),
            hero_scope: String::new(),
            scopes: Vec::new(),
            context_default_key: String::new(),
            context_hero_scope: String::new(),
            context_hero_key: String::new(),
            context_scope_keys: BTreeMap::new(),
            mounts_for_scope: None,
            mounts: BTreeMap::new(),
            route_path: None,
            routes: MediaAssetStoreRuntimeRoutes::default(),
            resources: None,
            resource_kind: ResourceKind::default(),
            default_page_size: 0,
            total_pages: None,
        }
    }
}

impl<E, R> Clone for MediaAssetStoreRuntimeBindingOptions<E, R>
where
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    fn clone(&self) -> Self {
        Self {
            assets: self.assets.clone(),
            attachments: self.attachments.clone(),
            resolve_url: self.resolve_url.clone(),
            current_user_id: self.current_user_id.clone(),
            translate: self.translate.clone(),
            default_scope: self.default_scope.clone(),
            hero_scope: self.hero_scope.clone(),
            scopes: self.scopes.clone(),
            context_default_key: self.context_default_key.clone(),
            context_hero_scope: self.context_hero_scope.clone(),
            context_hero_key: self.context_hero_key.clone(),
            context_scope_keys: self.context_scope_keys.clone(),
            mounts_for_scope: self.mounts_for_scope.clone(),
            mounts: self.mounts.clone(),
            route_path: self.route_path.clone(),
            routes: self.routes.clone(),
            resources: self.resources.clone(),
            resource_kind: self.resource_kind.clone(),
            default_page_size: self.default_page_size,
            total_pages: self.total_pages.clone(),
        }
    }
}

/// The store-backed media runtime.
pub struct MediaAssetStoreRuntime<E, R>
where
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    /// Consumer asset store.
    pub assets: Option<Arc<dyn MediaAssetStore<E, R>>>,
    /// Consumer attachment store.
    pub attachments: Option<Arc<dyn ContentMediaAttachmentStore<E, R>>>,
    /// Image kind used for previews.
    pub image_kind: String,
    /// Hero role.
    pub hero_role: String,
    /// Gallery role.
    pub gallery_role: String,
    /// Inline role.
    pub inline_role: String,
    /// Default scope.
    pub default_scope: String,
    /// Hero scope.
    pub hero_scope: String,
    /// Scope normalizer.
    pub normalize_scope: Option<Arc<dyn Fn(&[u8]) -> InputBytes + Send + Sync + 'static>>,
    /// Request asset URL resolver.
    pub resolve_url: Option<MediaRequestAssetURLResolver>,
    /// Edit URL resolver.
    pub edit_url: Option<Arc<dyn Fn(&Request, &MediaAssetRecord) -> String + Send + Sync>>,
    /// Current-user identity resolver.
    pub current_user_id: Option<Arc<dyn Fn(&Request) -> String + Send + Sync>>,
    /// Context label resolver.
    pub context_label:
        Option<Arc<dyn Fn(&Request, &MediaSurfaceRequest) -> String + Send + Sync>>,
    /// New-form URL.
    pub media_new_url: Option<MediaRequestURLResolver>,
    /// Picker URL.
    pub picker_url: Option<MediaRequestURLResolver>,
    /// Attachment-mutation URL.
    pub attachment_mutation_url: Option<MediaRequestURLResolver>,
    /// Library URL.
    pub library_url: Option<MediaRequestURLResolver>,
    /// Inline-create URL.
    pub inline_create_url: Option<MediaRequestURLResolver>,
    /// Inline-preview URL.
    pub inline_preview_url: Option<MediaRequestURLResolver>,
    /// Shortcode URL.
    pub shortcode_url: Option<MediaRequestURLResolver>,
    /// Picker-attach URL.
    pub picker_attach_url: Option<MediaRequestURLResolver>,
    /// Create base path.
    pub create_base_path: Option<MediaRequestURLResolver>,
    /// Filter-action URL.
    pub filter_action_url: Option<MediaRequestURLResolver>,
    /// Mounts resolver.
    pub mounts_for_scope: Option<crate::media::surface::MountsResolver>,
    /// Default picker root identity.
    pub default_picker_root_id: String,
    /// Default create root identity.
    pub default_create_root_id: String,
    /// Inline surface name.
    pub inline_surface: String,
    /// Default page size.
    pub default_page_size: i64,
    /// Total-page resolver.
    pub total_pages: Option<Arc<dyn Fn(i64, i64) -> i64 + Send + Sync>>,
    /// Asset identity generator.
    pub new_id: MediaAssetIdGenerator,
    /// Time source.
    pub now_unix: MediaAssetNowUnix,
}

impl<E, R> Clone for MediaAssetStoreRuntime<E, R>
where
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    fn clone(&self) -> Self {
        Self {
            assets: self.assets.clone(),
            attachments: self.attachments.clone(),
            image_kind: self.image_kind.clone(),
            hero_role: self.hero_role.clone(),
            gallery_role: self.gallery_role.clone(),
            inline_role: self.inline_role.clone(),
            default_scope: self.default_scope.clone(),
            hero_scope: self.hero_scope.clone(),
            normalize_scope: self.normalize_scope.clone(),
            resolve_url: self.resolve_url.clone(),
            edit_url: self.edit_url.clone(),
            current_user_id: self.current_user_id.clone(),
            context_label: self.context_label.clone(),
            media_new_url: self.media_new_url.clone(),
            picker_url: self.picker_url.clone(),
            attachment_mutation_url: self.attachment_mutation_url.clone(),
            library_url: self.library_url.clone(),
            inline_create_url: self.inline_create_url.clone(),
            inline_preview_url: self.inline_preview_url.clone(),
            shortcode_url: self.shortcode_url.clone(),
            picker_attach_url: self.picker_attach_url.clone(),
            create_base_path: self.create_base_path.clone(),
            filter_action_url: self.filter_action_url.clone(),
            mounts_for_scope: self.mounts_for_scope.clone(),
            default_picker_root_id: self.default_picker_root_id.clone(),
            default_create_root_id: self.default_create_root_id.clone(),
            inline_surface: self.inline_surface.clone(),
            default_page_size: self.default_page_size,
            total_pages: self.total_pages.clone(),
            new_id: self.new_id.clone(),
            now_unix: self.now_unix.clone(),
        }
    }
}

impl<E, R> Default for MediaAssetStoreRuntime<E, R>
where
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    fn default() -> Self {
        new_media_asset_store_runtime(MediaAssetStoreRuntimeOptions::default())
    }
}

/// Construct the store-backed runtime.
#[must_use]
pub fn new_media_asset_store_runtime<E, R>(
    options: MediaAssetStoreRuntimeOptions<E, R>,
) -> MediaAssetStoreRuntime<E, R>
where
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    let routes = &options.routes;
    let route_path = options.route_path.clone();
    let edit_url = options.edit_url.or_else(|| {
        let route_path = route_path.clone();
        let route_name = routes.edit.clone();
        Some(Arc::new(move |request: &Request, asset: &MediaAssetRecord| {
            match route_path.as_ref() {
                Some(path) if !route_name.trim().is_empty() && !asset.id.trim().is_empty() => {
                    path(request, &route_name, &[("id".to_owned(), asset.id.clone())])
                }
                _ => String::new(),
            }
        }) as Arc<dyn Fn(&Request, &MediaAssetRecord) -> String + Send + Sync>)
    });
    MediaAssetStoreRuntime {
        assets: options.assets,
        attachments: options.attachments,
        image_kind: MEDIA_KIND_IMAGE.to_owned(),
        hero_role: MEDIA_ATTACHMENT_ROLE_HERO.to_owned(),
        gallery_role: MEDIA_ATTACHMENT_ROLE_GALLERY.to_owned(),
        inline_role: MEDIA_ATTACHMENT_ROLE_INLINE.to_owned(),
        default_scope: options.default_scope,
        hero_scope: options.hero_scope,
        normalize_scope: options.normalize_scope,
        resolve_url: options.resolve_url,
        edit_url,
        current_user_id: options.current_user_id,
        context_label: options.context_label,
        media_new_url: media_runtime_route_path(route_path.clone(), &routes.new),
        picker_url: media_runtime_route_path(route_path.clone(), &routes.picker),
        attachment_mutation_url: media_runtime_route_path(
            route_path.clone(),
            &routes.attachment_mutate,
        ),
        library_url: media_runtime_route_path(route_path.clone(), &routes.library),
        inline_create_url: media_runtime_route_path(route_path.clone(), &routes.inline_create),
        inline_preview_url: media_runtime_route_path(route_path.clone(), &routes.inline_preview),
        shortcode_url: media_runtime_route_path(route_path.clone(), &routes.shortcode),
        picker_attach_url: media_runtime_route_path(route_path.clone(), &routes.picker_attach),
        create_base_path: media_runtime_route_path(route_path.clone(), &routes.new),
        filter_action_url: media_runtime_route_path(route_path, &routes.picker),
        mounts_for_scope: options.mounts_for_scope,
        default_picker_root_id: DEFAULT_MEDIA_PICKER_ROOT_ID.to_owned(),
        default_create_root_id: DEFAULT_MEDIA_CREATE_ROOT_ID.to_owned(),
        inline_surface: MEDIA_SURFACE_INLINE.to_owned(),
        default_page_size: first_positive(options.default_page_size, DEFAULT_MEDIA_LIBRARY_PAGE_SIZE),
        total_pages: options.total_pages,
        new_id: options
            .new_id
            .unwrap_or_else(|| Arc::new(|| Uuid::new_v4().to_string())),
        now_unix: options.now_unix.unwrap_or_else(|| {
            Arc::new(|| {
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|duration| duration.as_secs() as i64)
                    .unwrap_or(0)
            })
        }),
    }
}

/// Construct a runtime from consumer binding values.
#[must_use]
pub fn new_media_asset_store_runtime_from_bindings<E, R>(
    options: MediaAssetStoreRuntimeBindingOptions<E, R>,
) -> MediaAssetStoreRuntime<E, R>
where
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    let normalize_scope = new_media_scope_normalizer(&options.default_scope, &options.scopes);
    let routes = if options.routes == MediaAssetStoreRuntimeRoutes::default() {
        media_runtime_routes_from_registry(
            options.resources.as_deref(),
            &options.resource_kind,
        )
    } else {
        options.routes.clone()
    };
    let context = new_media_surface_context_labeler(MediaSurfaceContextLabelOptions {
        normalize_scope: Some(normalize_scope.clone()),
        default_scope: options.default_scope.clone(),
        hero_scope: first_non_blank(&[
            options.context_hero_scope.clone(),
            options.hero_scope.clone(),
            options.default_scope.clone(),
        ]),
        hero_role: MEDIA_ATTACHMENT_ROLE_HERO.to_owned(),
        default_key: options.context_default_key.clone(),
        hero_key: options.context_hero_key.clone(),
        scope_keys: options.context_scope_keys.clone(),
        translate: options.translate.clone(),
    });
    let mounts_for_scope = options.mounts_for_scope.clone().or_else(|| {
        if options.mounts.is_empty() {
            Some(new_default_media_surface_mounts_resolver(
                normalize_scope.clone(),
                &options.default_scope,
                &options.scopes,
            ))
        } else {
            Some(new_media_surface_mounts_resolver(
                normalize_scope.clone(),
                &options.default_scope,
                &options.mounts,
            ))
        }
    });
    let resolve_url = options.resolve_url.clone();
    new_media_asset_store_runtime(MediaAssetStoreRuntimeOptions {
        assets: options.assets,
        attachments: options.attachments,
        default_scope: options.default_scope.clone(),
        hero_scope: first_non_blank(&[options.hero_scope.clone(), options.default_scope.clone()]),
        normalize_scope: Some(normalize_scope),
        resolve_url: Some(Arc::new(move |_request, raw| match resolve_url.as_ref() {
            Some(resolve) => resolve(raw),
            None => InputBytes::from(raw),
        })),
        edit_url: None,
        current_user_id: options.current_user_id,
        context_label: Some(context),
        route_path: options.route_path,
        routes,
        mounts_for_scope,
        default_page_size: options.default_page_size,
        total_pages: options.total_pages,
        new_id: None,
        now_unix: None,
    })
}

/// Resolve the configured runtime, binding from values when no runtime field is set.
#[must_use]
pub fn resolve_media_asset_service_runtime<E, R>(
    runtime: MediaAssetStoreRuntime<E, R>,
    bindings: MediaAssetStoreRuntimeBindingOptions<E, R>,
) -> MediaAssetStoreRuntime<E, R>
where
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    if runtime.assets.is_some()
        || runtime.attachments.is_some()
        || runtime.mounts_for_scope.is_some()
        || runtime.resolve_url.is_some()
    {
        return runtime;
    }
    if bindings.assets.is_some()
        || bindings.attachments.is_some()
        || bindings.resources.is_some()
        || bindings.route_path.is_some()
    {
        return new_media_asset_store_runtime_from_bindings(bindings);
    }
    runtime
}

/// Resolve the media resource kind, defaulting to the media kind.
#[must_use]
pub fn resolve_media_asset_service_kind(kind: &ResourceKind) -> ResourceKind {
    if kind.as_str().is_empty() {
        ResourceKind::new(crate::definitions::RESOURCE_KIND_MEDIA)
    } else {
        kind.clone()
    }
}

/// Build a request URL resolver from a route path builder and route name.
#[must_use]
pub fn media_runtime_route_path(
    route_path: Option<RoutePathBuilder>,
    route_name: &str,
) -> Option<MediaRequestURLResolver> {
    let route_name = route_name.trim().to_owned();
    if route_name.is_empty() {
        return None;
    }
    route_path.map(|build| {
        let build = build;
        let route_name = route_name.clone();
        Arc::new(move |request: &Request| build(request, &route_name, &[])) as MediaRequestURLResolver
    })
}

/// Build a request asset URL resolver from an asset URL resolver.
#[must_use]
pub fn media_runtime_asset_url(
    resolve: Option<MediaAssetURLResolver>,
) -> MediaRequestAssetURLResolver {
    Arc::new(move |_request, raw| match resolve.as_ref() {
        Some(resolve) => resolve(raw),
        None => InputBytes::from(raw),
    })
}

/// Normalize a scope against the allowed/default scopes.
#[must_use]
pub fn normalize_media_scope(raw: &[u8], default_scope: &str, allowed: &[String]) -> InputBytes {
    let value = simple_lowercase(&crate::byte_text::encoding_text(trim_unicode_whitespace(raw)));
    for scope in allowed {
        let scope = simple_lowercase(scope.trim());
        if !scope.is_empty() && value == scope {
            return scope.into();
        }
    }
    let default_scope = simple_lowercase(default_scope.trim());
    if !default_scope.is_empty() {
        return default_scope.into();
    }
    if let Some(first) = allowed.first() {
        let first = simple_lowercase(first.trim());
        if !first.is_empty() {
            return first.into();
        }
    }
    value.into()
}

/// Build a scope normalizer.
#[must_use]
pub fn new_media_scope_normalizer(
    default_scope: &str,
    allowed: &[String],
) -> Arc<dyn Fn(&[u8]) -> InputBytes + Send + Sync + 'static> {
    let default_scope = default_scope.to_owned();
    let allowed = allowed.to_vec();
    Arc::new(move |raw| normalize_media_scope(raw, &default_scope, &allowed))
}

/// Build default mounts for one scope.
#[must_use]
pub fn media_default_surface_mounts(scope: impl AsRef<[u8]>) -> MediaSurfaceMounts {
    let scope = trim_unicode_whitespace(scope.as_ref());
    if scope.is_empty() {
        return MediaSurfaceMounts::default();
    }
    MediaSurfaceMounts {
        section_id: InputBytes::from([scope, b"-media-section"].concat()),
        owner_form_id: InputBytes::from([scope, DEFAULT_MEDIA_OWNER_FORM_SUFFIX.as_bytes()].concat()),
        picker_root_id: InputBytes::from([scope, b"-media-picker-root"].concat()),
        create_root_id: InputBytes::from([scope, b"-media-create-root"].concat()),
        shortcode_root_id: InputBytes::from([scope, b"-media-shortcode-root"].concat()),
        shortcode_dialog_id: InputBytes::from([scope, b"-media-shortcode-dialog"].concat()),
    }
}

/// Build the default mounts map.
#[must_use]
pub fn media_default_surface_mount_map(scopes: &[String]) -> BTreeMap<String, MediaSurfaceMounts> {
    let mut mounts = BTreeMap::new();
    for scope in scopes {
        let trimmed = scope.trim();
        if trimmed.is_empty() {
            continue;
        }
        mounts.insert(trimmed.to_owned(), media_default_surface_mounts(trimmed));
    }
    mounts
}

/// Build a mounts resolver over a supplied scope map.
#[must_use]
pub fn new_media_surface_mounts_resolver(
    normalize: Arc<dyn Fn(&[u8]) -> InputBytes + Send + Sync + 'static>,
    default_scope: &str,
    mounts: &BTreeMap<String, MediaSurfaceMounts>,
) -> crate::media::surface::MountsResolver {
    let normalized_default = normalize(default_scope.trim().as_bytes());
    let mut normalized = BTreeMap::new();
    for (scope, value) in mounts {
        normalized.insert(normalize(trim_unicode_whitespace(scope.as_ref())), value.clone());
    }
    Arc::new(move |scope: &[u8]| {
        let scope = normalize(trim_unicode_whitespace(scope.as_ref()));
        normalized
            .get(&scope)
            .or_else(|| normalized.get(&normalized_default))
            .cloned()
            .unwrap_or_default()
    })
}

/// Build a default mounts resolver.
#[must_use]
pub fn new_default_media_surface_mounts_resolver(
    normalize: Arc<dyn Fn(&[u8]) -> InputBytes + Send + Sync + 'static>,
    default_scope: &str,
    scopes: &[String],
) -> crate::media::surface::MountsResolver {
    new_media_surface_mounts_resolver(
        normalize,
        default_scope,
        &media_default_surface_mount_map(scopes),
    )
}

/// Build the declared attachment role options.
#[must_use]
pub fn default_media_attachment_role_options() -> MediaAttachmentRoleOptions {
    MediaAttachmentRoleOptions {
        hero_role: MEDIA_ATTACHMENT_ROLE_HERO.to_owned(),
        gallery_role: MEDIA_ATTACHMENT_ROLE_GALLERY.to_owned(),
        normalize_role: Some(Arc::new(|raw: &[u8]| normalize_media_attachment_role(raw).into()) as RoleNormalizer),
    }
}

/// Scope and role policy for the request's media context label.
#[derive(Clone, Default)]
pub struct MediaSurfaceContextLabelOptions {
    /// Optional scope normalizer; otherwise the configured default scope applies.
    pub normalize_scope: Option<Arc<dyn Fn(&[u8]) -> InputBytes + Send + Sync>>,
    /// Default scope.
    pub default_scope: String,
    /// Hero scope, falling back to the default scope.
    pub hero_scope: String,
    /// Hero role, falling back to the declared hero role.
    pub hero_role: String,
    /// Default label key.
    pub default_key: String,
    /// Hero label key.
    pub hero_key: String,
    /// Per-scope label keys.
    pub scope_keys: BTreeMap<String, String>,
    /// Request-aware translation callback.
    pub translate: Option<TranslateDashboardTextFunc>,
}

/// Bind the scope and role label policy for media surfaces.
#[must_use]
pub fn new_media_surface_context_labeler(
    options: MediaSurfaceContextLabelOptions,
) -> Arc<dyn Fn(&Request, &MediaSurfaceRequest) -> String + Send + Sync> {
    let normalize_scope = options.normalize_scope.unwrap_or_else(||
        new_media_scope_normalizer(&options.default_scope, &[]));
    let hero_scope = normalize_scope(first_non_blank(&[
        options.hero_scope, options.default_scope,
    ]).as_bytes());
    let hero_role = normalize_media_attachment_role(first_non_blank(&[
        options.hero_role, MEDIA_ATTACHMENT_ROLE_HERO.to_owned(),
    ]));
    let default_key = options.default_key.trim().to_owned();
    let hero_key = options.hero_key.trim().to_owned();
    let mut normalized = BTreeMap::new();
    for (scope, key) in options.scope_keys {
        let key = key.trim();
        if !key.is_empty() {
            normalized.insert(normalize_scope(scope.as_bytes()), key.to_owned());
        }
    }
    let translate = options.translate;
    Arc::new(move |request, surface| {
        let mut key = default_key.clone();
        let scope = normalize_scope(surface.scope.as_bytes());
        if let Some(scope_key) = normalized.get(&scope) {
            key = scope_key.clone();
        }
        if scope == hero_scope
            && normalize_media_attachment_role(&surface.role) == hero_role
            && !hero_key.is_empty()
        {
            key = hero_key.clone();
        }
        if key.is_empty() { return String::new(); }
        match translate.as_ref() {
            Some(translate) => translate(Some(request), &key, &[]),
            None => key,
        }
    })
}

fn first_positive(value: i64, fallback: i64) -> i64 {
    if value > 0 { value } else { fallback }
}

/// The runtime route names from a definition registry.
#[must_use]
pub fn media_runtime_routes_from_registry(
    resources: Option<&(dyn crate::definitions::Registry + Send + Sync)>,
    kind: &ResourceKind,
) -> MediaAssetStoreRuntimeRoutes {
    let kind = resolve_media_asset_service_kind(kind);
    let Some(definition) = resources.and_then(|resources| find_resource_definition(Some(resources), &kind))
    else {
        return MediaAssetStoreRuntimeRoutes::default();
    };
    media_runtime_routes_from_definition(&definition.management_routes)
}

/// The runtime route names from a route set.
#[must_use]
pub fn media_runtime_routes_from_definition(routes: &ResourceRouteSet) -> MediaAssetStoreRuntimeRoutes {
    MediaAssetStoreRuntimeRoutes {
        new: routes.new.clone(),
        edit: routes.edit.clone(),
        picker: routes.aux.get(&crate::definitions::ResourceAuxRoute::new(RESOURCE_AUX_ROUTE_PICKER)).unwrap_or_default(),
        attachment_mutate: routes.aux.get(&crate::definitions::ResourceAuxRoute::new(RESOURCE_AUX_ROUTE_ATTACHMENT_MUTATE)).unwrap_or_default(),
        library: routes.list.clone(),
        inline_create: routes.aux.get(&crate::definitions::ResourceAuxRoute::new(RESOURCE_AUX_ROUTE_INLINE_CREATE)).unwrap_or_default(),
        inline_preview: routes.aux.get(&crate::definitions::ResourceAuxRoute::new(RESOURCE_AUX_ROUTE_INLINE_PREVIEW)).unwrap_or_default(),
        shortcode: routes.aux.get(&crate::definitions::ResourceAuxRoute::new(RESOURCE_AUX_ROUTE_SHORTCODE)).unwrap_or_default(),
        picker_attach: routes.aux.get(&crate::definitions::ResourceAuxRoute::new(RESOURCE_AUX_ROUTE_PICKER_ATTACH)).unwrap_or_default(),
    }
}

impl<E, R> MediaAssetStoreRuntime<E, R>
where
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    /// Normalize one scope.
    #[must_use]
    pub fn normalize_scope_value(&self, raw: impl AsRef<[u8]>) -> InputBytes {
        let raw = raw.as_ref();
        if let Some(normalize) = self.normalize_scope.as_ref() {
            let value = normalize(raw).trim().to_owned();
            if !value.is_empty() {
                return value;
            }
        }
        normalize_media_scope(raw, &self.default_scope, &[self.default_scope.clone()])
    }

    /// The default attachment role for one surface.
    #[must_use]
    pub fn default_role(&self, surface: &MediaSurfaceRequest) -> String {
        if self.normalize_scope_value(&surface.scope) == self.normalize_scope_value(
            first_non_blank(&[self.hero_scope.clone(), self.default_scope.clone()]).as_bytes(),
        ) {
            self.hero_role_name()
        } else {
            self.gallery_role_name()
        }
    }

    /// The role options.
    #[must_use]
    pub fn role_options(&self) -> MediaAttachmentRoleOptions {
        MediaAttachmentRoleOptions {
            hero_role: self.hero_role_name(),
            gallery_role: self.gallery_role_name(),
            normalize_role: Some(Arc::new(|raw: &[u8]| normalize_media_attachment_role(raw).into()) as RoleNormalizer),
        }
    }

    /// The store attachment options for one owner.
    #[must_use]
    pub fn store_attachment_options(
        &self,
        request: &Request,
        owner_type: &InputBytes,
        owner_content_id: &InputBytes,
    ) -> MediaStoreAttachmentOptions<E, R> {
        MediaStoreAttachmentOptions {
            request: Some(request.clone()),
            form: None,
            assets: self.assets.clone(),
            attachments: self.attachments.clone(),
            owner_type: owner_type.to_owned(),
            owner_content_id: owner_content_id.to_owned(),
            normalize_role: Some(Arc::new(|raw: &[u8]| normalize_media_attachment_role(raw).into()) as RoleNormalizer),
            edit_url: self.edit_url.clone().map(|resolve| {
                Arc::new(move |request: Option<&Request>, record: &MediaAssetRecord| {
                    request
                        .map(|request| resolve(request, record))
                        .unwrap_or_default()
                }) as crate::media::assets::MediaAssetEditUrl
            }),
            image_kind: self.image_kind_name(),
            resolve_url: Some(self.request_asset_resolver(request)),
        }
    }

    /// List one owner's attachment items.
    #[must_use]
    pub fn list_attachment_items(
        &self,
        request: &Request,
        owner_type: InputBytes,
        owner_content_id: InputBytes,
    ) -> Effect<Vec<MediaAttachmentFormItem>, E, R> {
        let options = self.store_attachment_options(request, &owner_type, &owner_content_id);
        list_media_attachment_form_items_from_store(&options)
    }

    /// List through the same consumer stores with the selected CMS content memory.
    pub fn list_attachment_items_prepared(
        &self,
        request: &Request,
        owner_type: InputBytes,
        owner_content_id: InputBytes,
        memory: &PreparedCmsContent,
        on_memory: impl Fn(MemoryError) -> E + Send + Sync + 'static,
    ) -> Effect<PreparedAttachmentRows<MediaAttachmentFormItem>, E, R> {
        let options = self.store_attachment_options(request, &owner_type, &owner_content_id);
        list_prepared_media_attachment_form_items_from_store(&options, memory, on_memory)
    }

    /// List the current attachment items for one owner.
    #[must_use]
    pub fn current_attachment_items(
        &self,
        request: &Request,
        owner_type: InputBytes,
        owner_content_id: InputBytes,
    ) -> Effect<Vec<MediaAttachmentFormItem>, E, R> {
        self.list_attachment_items(request, owner_type, owner_content_id)
    }

    /// Resolve the current attachment inputs for a surface.
    #[must_use]
    pub fn current_attachment_inputs(
        &self,
        request: &Request,
        surface: &MediaSurfaceRequest,
        form: Option<Arc<ByteFormData>>,
    ) -> Effect<Vec<MediaAttachmentInput>, E, R> {
        let runtime = self.clone();
        let request = request.clone();
        let request_for_items = request.clone();
        let owner_type = surface.scope.clone();
        let owner_content_id = surface.owner_content_id.clone();
        let load_current_items: MediaAttachmentItemLoader<E, R> = Arc::new(move || {
            runtime.current_attachment_items(
                &request_for_items,
                owner_type.clone(),
                owner_content_id.clone(),
            )
        });
        current_media_attachment_inputs(
            CurrentMediaAttachmentInputOptions {
                request: Some(request),
                form,
                normalize_role: Some(Arc::new(|raw: &[u8]| normalize_media_attachment_role(raw).into()) as RoleNormalizer),
                load_current_items: Some(load_current_items),
            },
        )
    }

    /// The asset loader used by attachment previews.
    fn attachment_asset_loader(
        &self,
    ) -> Option<MediaAttachmentAssetLoader<MediaAssetRecord, E, R>> {
        self.assets.as_ref().map(|assets| {
            let assets = Arc::clone(assets);
            Arc::new(move |ids: Vec<InputBytes>| assets.get_by_ids(ids))
                as MediaAttachmentAssetLoader<MediaAssetRecord, E, R>
        })
    }

    /// The presented-item builder used by attachment previews.
    fn attachment_item_builder(
        &self,
        request: &Request,
    ) -> MediaAttachmentItemBuilder<MediaAssetRecord> {
        let edit_url = self.edit_url.clone();
        let image_kind = self.image_kind_name();
        let resolve = self.request_asset_resolver(request);
        let request = request.clone();
        Arc::new(move |index, asset: MediaAssetRecord, attachment| {
            let edit = edit_url
                .as_ref()
                .map(|resolve| resolve(&request, &asset))
                .unwrap_or_default();
            media_attachment_form_item_from_records(
                index,
                &asset,
                &attachment,
                &edit,
                &image_kind,
                Some(&resolve),
            )
        })
    }

    /// Preview already-collected attachment inputs.
    #[must_use]
    pub fn preview_attachment_items_from_inputs(
        &self,
        request: &Request,
        inputs: Vec<MediaAttachmentInput>,
    ) -> Effect<Vec<MediaAttachmentFormItem>, E, R> {
        preview_media_attachment_form_items_from_inputs(MediaAttachmentPreviewOptions {
            request: Some(request.clone()),
            form: None,
            inputs,
            normalize_role: Some(Arc::new(|raw: &[u8]| normalize_media_attachment_role(raw).into()) as RoleNormalizer),
            load_assets_by_id: self.attachment_asset_loader(),
            build_item: Some(self.attachment_item_builder(request)),
        })
    }

    /// Preview the request's attachment selection.
    #[must_use]
    pub fn preview_attachment_items_from_request(
        &self,
        request: &Request,
        form: Option<Arc<ByteFormData>>,
    ) -> Effect<Vec<MediaAttachmentFormItem>, E, R> {
        let load_assets_by_id = self.attachment_asset_loader();
        let build_item = self.attachment_item_builder(request);
        preview_media_attachment_form_items_from_request(
            MediaAttachmentPreviewOptions {
                request: Some(request.clone()),
                form,
                inputs: Vec::new(),
                normalize_role: Some(Arc::new(|raw: &[u8]| normalize_media_attachment_role(raw).into()) as RoleNormalizer),
                load_assets_by_id,
                build_item: Some(build_item),
            },
        )
    }

    /// Use the existing asset loader and URL/projector callbacks with prepared previews.
    pub fn preview_attachment_items_from_request_prepared(
        &self,
        request: &Request,
        form: Option<Arc<ByteFormData>>,
        memory: &PreparedCmsContent,
        on_memory: impl Fn(MemoryError) -> E + Send + Sync + 'static,
    ) -> Effect<PreparedAttachmentRows<MediaAttachmentFormItem>, E, R> {
        preview_prepared_media_attachment_form_items_from_request(
            MediaAttachmentPreviewOptions {
                request: Some(request.clone()),
                form,
                inputs: Vec::new(),
                normalize_role: Some(Arc::new(|raw: &[u8]| normalize_media_attachment_role(raw).into()) as RoleNormalizer),
                load_assets_by_id: self.attachment_asset_loader(),
                build_item: Some(self.attachment_item_builder(request)),
            },
            memory.attachments.clone(),
            memory.attachment_items.clone(),
            memory.attachment_metadata,
            on_memory,
        )
    }

    /// Replace one owner's attachments from the request.
    #[must_use]
    pub fn replace_attachments_from_request(
        &self,
        request: &Request,
        owner_type: InputBytes,
        owner_content_id: InputBytes,
        form: Option<Arc<ByteFormData>>,
    ) -> Effect<Vec<MediaAttachmentFormItem>, E, R> {
        let mut options = self.store_attachment_options(request, &owner_type, &owner_content_id);
        options.form = form;
        replace_media_attachments_from_store_request(&options)
    }

    /// Replace once through the existing consumer store after admitting local input and output work.
    pub fn replace_attachments_from_request_prepared(
        &self,
        request: &Request,
        owner_type: InputBytes,
        owner_content_id: InputBytes,
        form: Option<Arc<ByteFormData>>,
        memory: &PreparedCmsContent,
        on_memory: impl Fn(MemoryError) -> E + Send + Sync + 'static,
    ) -> Effect<PreparedAttachmentRows<MediaAttachmentFormItem>, E, R> {
        let mut options = self.store_attachment_options(request, &owner_type, &owner_content_id);
        options.form = form;
        replace_prepared_media_attachments_from_store_request(&options, memory, on_memory)
    }

    /// Build the attachment manager props for one owner surface.
    #[must_use]
    pub fn build_attachment_manager(
        &self,
        request: &Request,
        form: Option<&ByteFormData>,
        scope: InputBytes,
        owner_content_id: InputBytes,
        attachments: Vec<MediaAttachmentFormItem>,
    ) -> Option<MediaAttachmentManagerProps> {
        let mounts_for_scope = self.mounts_for_scope.as_ref()?;
        let scope = scope.trim().to_owned();
        if scope.is_empty() {
            return None;
        }
        Some(build_media_attachment_manager_from_request(
            request,
            form,
            &scope,
            &owner_content_id,
            attachments,
            Some(mounts_for_scope),
            &self.request_url(
                request,
                self.picker_url.as_ref().or(self.filter_action_url.as_ref()),
            ),
            &self.request_url(
                request,
                self.media_new_url.as_ref().or(self.create_base_path.as_ref()),
            ),
            &self.request_url(request, self.attachment_mutation_url.as_ref()),
            &self.resolved_inline_surface(),
        ))
    }

    /// Resolve the media surface request from decoded form values and the query.
    #[must_use]
    pub fn surface_request(
        &self,
        request: &Request,
        form: Option<&ByteFormData>,
    ) -> MediaSurfaceRequest {
        let runtime = self.clone();
        let options = MediaSurfaceOptions {
            normalize_scope: Some(Arc::new(move |raw| runtime.normalize_scope_value(raw))),
            normalize_role: Some(Arc::new(|raw: &[u8]| normalize_media_attachment_role(raw).into())),
        };
        parse_media_surface_request(request, form, &options)
    }

    /// Resolve a media query from the request and page size.
    #[must_use]
    pub fn media_query(&self, request: &Request, limit: i64) -> MediaQuery {
        media_query_from_request(request, None, limit)
    }

    /// Whether one asset exists.
    #[must_use]
    pub fn asset_exists(&self, id: InputBytes) -> Effect<bool, E, R> {
        let Some(assets) = self.assets.clone() else {
            return effect_sync! { body { false } };
        };
        effect! {
            let assets = assets.clone();
            let id = id.clone();
            let record = assets.get_by_id(id)?;
            matches!(record, Some(_))
        }
    }

    /// Create an asset from form input and return its resulting identity.
    #[must_use]
    pub fn create_asset_from_form(
        &self,
        request: &Request,
        input: MediaAssetFormInput,
    ) -> Effect<String, E, R> {
        let Some(assets) = self.assets.clone() else {
            return effect_sync! { body { String::new() } };
        };
        let current_user_id = self.current_user_id.clone();
        let request_owner = request.clone();
        let new_id = self.new_id.clone();
        let now = self.now_unix.clone();
        effect! {
            let assets = assets.clone();
            let current_user_id = current_user_id.clone();
            let request_owner = request_owner.clone();
            let input = input.clone();
            let new_id = new_id.clone();
            let now = now.clone();
            let created_by = current_user_id
                .as_ref()
                .map(|resolve| resolve(&request_owner))
                .unwrap_or_default();
            let record = record_from_form(&input, &created_by);
            let prepared = prepare_record_for_create(record, Some(&new_id), Some(&now))?;
            let handle = AtomicRef::new(prepared);
            assets.create(handle.clone())?;
            handle.with(|record| record.id.clone())?
        }
    }

    /// Update an asset from form input.
    #[must_use]
    pub fn update_asset_from_form(
        &self,
        id: InputBytes,
        input: MediaAssetFormInput,
    ) -> Effect<(), E, R> {
        let Some(assets) = self.assets.clone() else {
            return effect_sync! { body { () } };
        };
        let now = self.now_unix.clone();
        effect! {
            let assets = assets.clone();
            let id = id.clone();
            let input = input.clone();
            let now = now.clone();
            let record = assets.get_by_id(id.trim().to_owned())?;
            if let Some(record) = record {
                let applied = apply_media_asset_form_record(record, &input);
                let prepared = prepare_record_for_update(applied, Some(&now))?;
                let handle = AtomicRef::new(prepared);
                assets.update(handle)?;
            }
            ()
        }
    }

    /// Delete one asset directly through the consumer store.
    #[must_use]
    pub fn delete_asset(&self, id: InputBytes) -> Effect<(), E, R> {
        let Some(assets) = self.assets.clone() else {
            return effect_sync! { body { () } };
        };
        assets.delete(id)
    }

    /// List assets for the shortcode surface.
    #[must_use]
    pub fn list_assets(&self, query: MediaQuery) -> Effect<Option<MediaAssetListResult>, E, R> {
        let Some(assets) = self.assets.clone() else {
            return effect_sync! { body { None } };
        };
        assets.list(query)
    }

    /// Load the shortcode items for one query.
    #[must_use]
    pub fn load_shortcode_items(
        &self,
        request: &Request,
        query: MediaQuery,
    ) -> Effect<(Vec<MediaAttachmentFormItem>, i64), E, R> {
        if self.assets.is_none() {
            return effect_sync! { body { (Vec::new(), 0) } };
        }
        let runtime = self.clone();
        let request = request.clone();
        effect! {
            let runtime = runtime.clone();
            let request = request.clone();
            let query = query.clone();
            let result = runtime.list_assets(query)?;
            let total = result.as_ref().map(|result| result.total).unwrap_or(0);
            let edit_url = {
                let edit = runtime.edit_url.clone();
                let request = request.clone();
                move |asset: &MediaAssetRecord| {
                    edit.as_ref()
                        .map(|resolve| resolve(&request, asset))
                        .unwrap_or_default()
                }
            };
            let items = media_shortcode_items_from_result(
                result.as_ref(),
                &runtime.inline_role_name(),
                &runtime.image_kind_name(),
                Some(&runtime.request_asset_resolver(&request)),
                &edit_url,
            );
            (items, total)
        }
    }

    /// Resolve a surface and retain its initialized form for subsequent consumers.
    #[must_use]
    pub fn surface_request_from_request(
        &self,
        request: &Request,
        form: Option<Arc<ByteFormData>>,
        policy: crate::attachments::MediaFormPolicy,
        on_error: impl Fn(ServerError) -> E + Send + Sync + 'static,
    ) -> Effect<(MediaSurfaceRequest, Arc<ByteFormData>), E, R> {
        let runtime = self.clone();
        let options = MediaSurfaceOptions {
            normalize_scope: Some(Arc::new(move |raw| runtime.normalize_scope_value(raw))),
            normalize_role: Some(Arc::new(|raw: &[u8]| normalize_media_attachment_role(raw).into())),
        };
        let request = request.clone();
        let on_error: Arc<dyn Fn(ServerError) -> E + Send + Sync> = Arc::new(on_error);
        effect! {
            let request = request.clone();
            let on_error = on_error.clone();
            let options = options.clone();
            let form = form.clone();
            let form = crate::media::assets::media_request_form::<E, R>(
                &request,
                form,
                policy,
                on_error,
            )?;
            let surface = parse_media_surface_request(&request, Some(&form), &options);
            (surface, Arc::new(form))
        }
    }

    /// Build the picker page through the supplied presentation publisher.
    #[must_use]
    pub fn build_picker_page<P>(
        &self,
        request: &Request,
        query: MediaQuery,
        error_msg: String,
        form: Option<Arc<ByteFormData>>,
        policy: crate::attachments::MediaFormPolicy,
        on_error: Arc<dyn Fn(ServerError) -> E + Send + Sync>,
        publish: Arc<dyn Fn(PickerPageState) -> P + Send + Sync + 'static>,
    ) -> Effect<P, E, R>
    where
        P: Default + Send + 'static,
    {
        run_picker_pipeline(self.clone(), request, query, error_msg, form, policy, on_error, publish)
    }

    /// The configured image kind.
    #[must_use]
    pub fn image_kind_name(&self) -> String {
        if self.image_kind.trim().is_empty() {
            MEDIA_KIND_IMAGE.to_owned()
        } else {
            self.image_kind.trim().to_owned()
        }
    }

    /// The configured hero role.
    #[must_use]
    pub fn hero_role_name(&self) -> String {
        if self.hero_role.trim().is_empty() {
            MEDIA_ATTACHMENT_ROLE_HERO.to_owned()
        } else {
            normalize_media_attachment_role(&self.hero_role)
        }
    }

    /// The configured gallery role.
    #[must_use]
    pub fn gallery_role_name(&self) -> String {
        if self.gallery_role.trim().is_empty() {
            MEDIA_ATTACHMENT_ROLE_GALLERY.to_owned()
        } else {
            normalize_media_attachment_role(&self.gallery_role)
        }
    }

    /// The configured inline role.
    #[must_use]
    pub fn inline_role_name(&self) -> String {
        if self.inline_role.trim().is_empty() {
            MEDIA_ATTACHMENT_ROLE_INLINE.to_owned()
        } else {
            normalize_media_attachment_role(&self.inline_role)
        }
    }

    /// The configured inline surface.
    #[must_use]
    pub fn resolved_inline_surface(&self) -> String {
        let value = self.inline_surface.trim();
        if value.is_empty() {
            MEDIA_SURFACE_INLINE.to_owned()
        } else {
            value.to_owned()
        }
    }

    /// The context label for one surface.
    #[must_use]
    pub fn context_label_for(
        &self,
        request: &Request,
        surface: &MediaSurfaceRequest,
    ) -> String {
        self.context_label
            .as_ref()
            .map(|resolve| resolve(request, surface).trim().to_owned())
            .unwrap_or_default()
    }

    /// The request asset URL resolver.
    #[must_use]
    pub fn request_asset_resolver(&self, request: &Request) -> MediaAssetURLResolver {
        let resolve = self.resolve_url.clone();
        let request = request.clone();
        Arc::new(move |raw| match resolve.as_ref() {
            Some(resolve) => resolve(&request, raw).trim().to_owned(),
            None => InputBytes::from(trim_unicode_whitespace(raw)),
        })
    }

    /// Resolve one request URL.
    #[must_use]
    pub fn request_url(
        &self,
        request: &Request,
        resolve: Option<&MediaRequestURLResolver>,
    ) -> String {
        resolve.map(|resolve| resolve(request).trim().to_owned()).unwrap_or_default()
    }
}

/// The page state handed to the presentation publisher after the picker's list and
/// surface-decode stages have run.
#[derive(Clone)]
pub struct PickerPageState {
    /// Request.
    pub request: Request,
    /// Decoded surface.
    pub surface: MediaSurfaceRequest,
    /// Admitted query.
    pub query: MediaQuery,
    /// Error message.
    pub error_msg: String,
    /// Store-listed assets, when present.
    pub result: Option<MediaAssetListResult>,
    /// Current attachment items.
    pub current_attachments: Vec<MediaAttachmentFormItem>,
    /// Total row count.
    pub total: i64,
    /// Default picker root identity.
    pub default_picker_root_id: String,
    /// Default create root identity.
    pub default_create_root_id: String,
    /// Default role.
    pub default_role: String,
    /// Default page size.
    pub default_page_size: i64,
    /// Picker attach URL.
    pub attach_url: String,
    /// Create base path.
    pub create_base_path: String,
    /// Inline surface name.
    pub inline_surface: String,
    /// Context label.
    pub context_label: String,
    /// Filter action URL.
    pub filter_action: String,
    /// Image kind used by the presentation's asset projection.
    pub image_kind: String,
    /// Hero role used by the presentation's selected-item projection.
    pub hero_role: String,
    /// Request-bound asset URL resolver used by presentation.
    pub resolve_url: MediaAssetURLResolver,
    /// Optional source page-count callback used by presentation.
    pub total_pages: Option<Arc<dyn Fn(i64, i64) -> i64 + Send + Sync>>,
}

fn simple_lowercase(raw: &str) -> String { raw.chars().map(|c| c.to_lowercase().next().unwrap_or(c)).collect() }
