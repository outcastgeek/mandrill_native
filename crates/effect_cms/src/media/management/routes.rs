//! Media management routes and names.
//!
//! The declared names, paths, resource route set, runtime route projection and the
//! ordered registration of the twelve management surfaces mirror the source. A route
//! is admitted only when its name, pattern and handler are present; the canonical
//! routing owner performs protection, metadata synchronization and mounting.

use effect::{Effect, Fiber, HasClock, HasConsole, HasTaskSpawner, Scope};
use effect_server::Router;

use crate::definitions::{
    RESOURCE_AUX_ROUTE_ATTACHMENT_MUTATE, RESOURCE_AUX_ROUTE_INLINE_CREATE,
    RESOURCE_AUX_ROUTE_INLINE_PREVIEW, RESOURCE_AUX_ROUTE_PICKER, RESOURCE_AUX_ROUTE_PICKER_ATTACH,
    RESOURCE_AUX_ROUTE_SHORTCODE, RESOURCE_KIND_MEDIA, Registry, ResourceAuxRoute, ResourceDefinition,
    ResourceKind, ResourceRouteSet, find_resource_definition,
};
use crate::routing::{
    AccessControl, ManagedRoute, ManagedRouteError, ManagedStartError, PermissionMode,
    RegisteredRoutes, RouteHandler, Service,
};

use crate::media::runtime::MediaAssetStoreRuntimeRoutes;

/// The media dashboard route name.
pub const MEDIA_DASHBOARD_ROUTE_NAME: &str = "admin-media-dashboard";
/// The media list route name.
pub const MEDIA_LIST_ROUTE_NAME: &str = "media-list";
/// The media new-form route name.
pub const MEDIA_NEW_ROUTE_NAME: &str = "media-new";
/// The media new-form submission route name.
pub const MEDIA_NEW_POST_ROUTE_NAME: &str = "media-new-post";
/// The media edit-form route name.
pub const MEDIA_EDIT_ROUTE_NAME: &str = "media-edit";
/// The media edit-form submission route name.
pub const MEDIA_EDIT_POST_ROUTE_NAME: &str = "media-edit-post";
/// The media delete route name.
pub const MEDIA_DELETE_ROUTE_NAME: &str = "media-delete";
/// The media picker route name.
pub const MEDIA_PICKER_ROUTE_NAME: &str = "media-picker";
/// The media picker-attach route name.
pub const MEDIA_PICKER_ATTACH_ROUTE_NAME: &str = "media-picker-attach";
/// The media inline-create route name.
pub const MEDIA_INLINE_CREATE_ROUTE_NAME: &str = "media-inline-create";
/// The media inline-preview route name.
pub const MEDIA_INLINE_PREVIEW_ROUTE_NAME: &str = "media-inline-preview";
/// The media shortcode route name.
pub const MEDIA_SHORTCODE_ROUTE_NAME: &str = "media-shortcode";
/// The media attachment-mutate route name.
pub const MEDIA_ATTACHMENT_MUTATE_ROUTE_NAME: &str = "media-attachment-mutate";

/// The declared route names for each management surface.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct MediaManagementRouteNames {
    /// List route.
    pub list: String,
    /// New-form route.
    pub new: String,
    /// New-form submission route.
    pub new_post: String,
    /// Edit-form route.
    pub edit: String,
    /// Edit-form submission route.
    pub edit_post: String,
    /// Delete route.
    pub delete: String,
    /// Picker route.
    pub picker: String,
    /// Picker-attach route.
    pub picker_attach: String,
    /// Inline-preview route.
    pub inline_preview: String,
    /// Inline-create route.
    pub inline_create: String,
    /// Shortcode route.
    pub shortcode: String,
    /// Attachment-mutate route.
    pub attachment_mutate: String,
}

/// The declared permissions for each management surface.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct MediaManagementRoutePermissions {
    /// List permissions.
    pub list: Vec<String>,
    /// New-form permissions.
    pub new: Vec<String>,
    /// New-form submission permissions.
    pub new_post: Vec<String>,
    /// Edit-form permissions.
    pub edit: Vec<String>,
    /// Edit-form submission permissions.
    pub edit_post: Vec<String>,
    /// Delete permissions.
    pub delete: Vec<String>,
    /// Picker permissions.
    pub picker: Vec<String>,
    /// Picker-attach permissions.
    pub picker_attach: Vec<String>,
    /// Inline-preview permissions.
    pub inline_preview: Vec<String>,
    /// Inline-create permissions.
    pub inline_create: Vec<String>,
    /// Shortcode permissions.
    pub shortcode: Vec<String>,
    /// Attachment-mutate permissions.
    pub attachment_mutate: Vec<String>,
}

/// The declared handlers for the management surfaces.
#[derive(Clone, Default)]
pub struct MediaManagementHandlers<E, R>
where
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    /// List handler.
    pub list: Option<RouteHandler<E, R>>,
    /// New-form handler.
    pub new: Option<RouteHandler<E, R>>,
    /// New-form submission handler.
    pub new_post: Option<RouteHandler<E, R>>,
    /// Edit-form handler.
    pub edit: Option<RouteHandler<E, R>>,
    /// Edit-form submission handler.
    pub edit_post: Option<RouteHandler<E, R>>,
    /// Delete handler.
    pub delete: Option<RouteHandler<E, R>>,
    /// Picker handler.
    pub picker: Option<RouteHandler<E, R>>,
    /// Picker-attach handler.
    pub picker_attach: Option<RouteHandler<E, R>>,
    /// Inline-preview handler.
    pub inline_preview: Option<RouteHandler<E, R>>,
    /// Inline-create handler.
    pub inline_create: Option<RouteHandler<E, R>>,
    /// Shortcode handler.
    pub shortcode: Option<RouteHandler<E, R>>,
    /// Attachment-mutate handler.
    pub attachment_mutate: Option<RouteHandler<E, R>>,
}

/// The default media management route names.
#[must_use]
pub fn default_media_management_route_names() -> MediaManagementRouteNames {
    MediaManagementRouteNames {
        list: MEDIA_LIST_ROUTE_NAME.to_owned(),
        new: MEDIA_NEW_ROUTE_NAME.to_owned(),
        new_post: MEDIA_NEW_POST_ROUTE_NAME.to_owned(),
        edit: MEDIA_EDIT_ROUTE_NAME.to_owned(),
        edit_post: MEDIA_EDIT_POST_ROUTE_NAME.to_owned(),
        delete: MEDIA_DELETE_ROUTE_NAME.to_owned(),
        picker: MEDIA_PICKER_ROUTE_NAME.to_owned(),
        picker_attach: MEDIA_PICKER_ATTACH_ROUTE_NAME.to_owned(),
        inline_preview: MEDIA_INLINE_PREVIEW_ROUTE_NAME.to_owned(),
        inline_create: MEDIA_INLINE_CREATE_ROUTE_NAME.to_owned(),
        shortcode: MEDIA_SHORTCODE_ROUTE_NAME.to_owned(),
        attachment_mutate: MEDIA_ATTACHMENT_MUTATE_ROUTE_NAME.to_owned(),
    }
}

/// The default media route set.
#[must_use]
pub fn default_media_resource_route_set() -> ResourceRouteSet {
    let names = default_media_management_route_names();
    let aux = crate::definitions::SharedMap::default();
    aux.insert(
        ResourceAuxRoute::new(RESOURCE_AUX_ROUTE_PICKER),
        names.picker,
    );
    aux.insert(
        ResourceAuxRoute::new(RESOURCE_AUX_ROUTE_PICKER_ATTACH),
        names.picker_attach,
    );
    aux.insert(
        ResourceAuxRoute::new(RESOURCE_AUX_ROUTE_INLINE_CREATE),
        names.inline_create,
    );
    aux.insert(
        ResourceAuxRoute::new(RESOURCE_AUX_ROUTE_INLINE_PREVIEW),
        names.inline_preview,
    );
    aux.insert(
        ResourceAuxRoute::new(RESOURCE_AUX_ROUTE_SHORTCODE),
        names.shortcode,
    );
    aux.insert(
        ResourceAuxRoute::new(RESOURCE_AUX_ROUTE_ATTACHMENT_MUTATE),
        names.attachment_mutate,
    );
    ResourceRouteSet {
        dashboard: MEDIA_DASHBOARD_ROUTE_NAME.to_owned(),
        list: names.list,
        detail: String::new(),
        new: names.new,
        new_post: names.new_post,
        edit: names.edit,
        edit_post: names.edit_post,
        delete: names.delete,
        aux,
    }
}

/// Resolve the management route names from a registry.
#[must_use]
pub fn media_management_route_names_from_registry(
    resources: Option<&(dyn Registry + Send + Sync)>,
    kind: &ResourceKind,
) -> MediaManagementRouteNames {
    let Some(definition) = resources.and_then(|resources| find_resource_definition(Some(resources), kind))
    else {
        return MediaManagementRouteNames::default();
    };
    media_management_route_names_from_definition(&definition)
}

/// Resolve the management route names from a definition.
#[must_use]
pub fn media_management_route_names_from_definition(
    definition: &ResourceDefinition,
) -> MediaManagementRouteNames {
    let routes = &definition.management_routes;
    let aux = |name: &str| {
        routes
            .aux
            .get(&ResourceAuxRoute::new(name))
            .unwrap_or_default()
    };
    MediaManagementRouteNames {
        list: routes.list.clone(),
        new: routes.new.clone(),
        new_post: routes.new_post.clone(),
        edit: routes.edit.clone(),
        edit_post: routes.edit_post.clone(),
        delete: routes.delete.clone(),
        picker: aux(RESOURCE_AUX_ROUTE_PICKER),
        picker_attach: aux(RESOURCE_AUX_ROUTE_PICKER_ATTACH),
        inline_preview: aux(RESOURCE_AUX_ROUTE_INLINE_PREVIEW),
        inline_create: aux(RESOURCE_AUX_ROUTE_INLINE_CREATE),
        shortcode: aux(RESOURCE_AUX_ROUTE_SHORTCODE),
        attachment_mutate: aux(RESOURCE_AUX_ROUTE_ATTACHMENT_MUTATE),
    }
}

/// Resolve runtime routes from a registry.
#[must_use]
pub fn media_asset_store_runtime_routes_from_registry(
    resources: Option<&(dyn Registry + Send + Sync)>,
    kind: &ResourceKind,
) -> MediaAssetStoreRuntimeRoutes {
    media_asset_store_runtime_routes_from_management_routes(&media_management_route_names_from_registry(
        resources, kind,
    ))
}

/// Project management route names into runtime route names.
#[must_use]
pub fn media_asset_store_runtime_routes_from_management_routes(
    names: &MediaManagementRouteNames,
) -> MediaAssetStoreRuntimeRoutes {
    MediaAssetStoreRuntimeRoutes {
        new: names.new.clone(),
        edit: names.edit.clone(),
        picker: names.picker.clone(),
        attachment_mutate: names.attachment_mutate.clone(),
        library: names.list.clone(),
        inline_create: names.inline_create.clone(),
        inline_preview: names.inline_preview.clone(),
        shortcode: names.shortcode.clone(),
        picker_attach: names.picker_attach.clone(),
    }
}

/// The default management route paths.
#[must_use]
pub fn default_media_management_route_paths() -> MediaManagementRouteNames {
    MediaManagementRouteNames {
        list: "/content/media/library".to_owned(),
        new: "/content/media/new".to_owned(),
        new_post: "/content/media/new".to_owned(),
        edit: "/content/media/{id}/edit".to_owned(),
        edit_post: "/content/media/{id}/edit".to_owned(),
        delete: "/content/media/{id}/delete".to_owned(),
        picker: "/content/media/picker".to_owned(),
        picker_attach: "/content/media/picker/attach".to_owned(),
        inline_preview: "/content/media/inline-create/preview".to_owned(),
        inline_create: "/content/media/inline-create".to_owned(),
        shortcode: "/content/media/shortcode".to_owned(),
        attachment_mutate: "/content/media/attachment-mutate".to_owned(),
    }
}

/// Register the management routes in the source order.
///
/// # Errors
///
/// Returns the canonical routing owner's protection or mounting failure.
pub fn media_management_route_service<E, R>(
    names: &MediaManagementRouteNames,
    handlers: &MediaManagementHandlers<E, R>,
    access: &AccessControl<E, R>,
    permissions: &MediaManagementRoutePermissions,
) -> Result<Service<E, R>, ManagedRouteError>
where
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    let paths = default_media_management_route_paths();
    let mut routes = Vec::new();
    let mut admit = |name: &str,
                     path: &str,
                     method: &str,
                     handler: Option<RouteHandler<E, R>>,
                     permissions: &[String]|
     -> Result<(), ManagedRouteError> {
        let Some(handler) = handler else {
            return Ok(());
        };
        if name.is_empty() || path.is_empty() {
            return Ok(());
        }
        let route = ManagedRoute {
            name: name.to_owned(),
            method: method.to_owned(),
            pattern: path.to_owned(),
            handler: Some(handler),
            permissions: permissions.to_vec(),
            permission_mode: PermissionMode::default(),
            display_name: String::new(),
            description: String::new(),
            category: RESOURCE_KIND_MEDIA.to_owned(),
        };
        routes.push(route);
        Ok(())
    };
    admit(
        &names.list,
        &paths.list,
        "GET",
        handlers.list.clone(),
        &permissions.list,
    )?;
    admit(
        &names.new,
        &paths.new,
        "GET",
        handlers.new.clone(),
        &permissions.new,
    )?;
    admit(
        &names.new_post,
        &paths.new_post,
        "POST",
        handlers.new_post.clone(),
        &permissions.new_post,
    )?;
    admit(
        &names.edit,
        &paths.edit,
        "GET",
        handlers.edit.clone(),
        &permissions.edit,
    )?;
    admit(
        &names.edit_post,
        &paths.edit_post,
        "POST",
        handlers.edit_post.clone(),
        &permissions.edit_post,
    )?;
    admit(
        &names.delete,
        &paths.delete,
        "POST",
        handlers.delete.clone(),
        &permissions.delete,
    )?;
    admit(
        &names.picker,
        &paths.picker,
        "GET",
        handlers.picker.clone(),
        &permissions.picker,
    )?;
    admit(
        &names.picker_attach,
        &paths.picker_attach,
        "POST",
        handlers.picker_attach.clone(),
        &permissions.picker_attach,
    )?;
    admit(
        &names.inline_preview,
        &paths.inline_preview,
        "GET",
        handlers.inline_preview.clone(),
        &permissions.inline_preview,
    )?;
    admit(
        &names.inline_create,
        &paths.inline_create,
        "POST",
        handlers.inline_create.clone(),
        &permissions.inline_create,
    )?;
    admit(
        &names.shortcode,
        &paths.shortcode,
        "GET",
        handlers.shortcode.clone(),
        &permissions.shortcode,
    )?;
    admit(
        &names.attachment_mutate,
        &paths.attachment_mutate,
        "POST",
        handlers.attachment_mutate.clone(),
        &permissions.attachment_mutate,
    )?;
    Ok(Service::new(crate::routing::Options { access: access.clone(), routes }))
}

/// Mount routes and start synchronization in the consumer's application scope.
pub fn register_media_management_routes<E, R, X>(
    service: &Service<E, R>,
    owner: &Scope<X>,
    router: Option<Router<E, R>>,
    mounter: Option<&crate::routing::ManagedRouteMounter<E, R>>,
) -> Effect<(RegisteredRoutes<E, R>, Option<Fiber<(), X>>), X, R>
where
    E: std::fmt::Display + Send + 'static,
    R: HasClock + HasConsole + HasTaskSpawner + Send + Sync + 'static,
    X: From<ManagedStartError<E>> + Send + Sync + 'static,
{
    crate::cms! { register_routes; service: service, scope: owner, router: router, mounter: mounter, }
}
