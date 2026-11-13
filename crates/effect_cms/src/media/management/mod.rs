//! Media management: route names, list/form pages, command bindings and delete.
//!
//! The media management owner supplies store-backed list pages and media form pages
//! through the canonical rendering and forms owners, the declared management route
//! names/paths/permissions and their ordered registration, create/update resource
//! commands over the runtime, and the source's direct consumer-store delete. No CMS
//! database, SQL, schema, migration, pool or HTTP host exists here; the consumer
//! supplies the router, stores, validators, executors and template renderers.

mod bindings;
mod form_page;
mod list_page;
mod media_form;
mod routes;

pub use self::bindings::{
    MediaAssetCommandBindingOptions, MediaAssetValidationBindingOptions, MediaInvalidRequest,
    MediaURLRequiredMessage, MediaValidationInvalidRequest, bind_media_asset_resource_commands,
    bind_media_asset_resource_validation, media_asset_create_executor,
    media_asset_create_validator, media_asset_delete_executor, media_asset_update_executor,
    media_asset_update_validator,
};
pub use self::form_page::{
    MediaAssetFormPageBuilderOptions, MediaAssetFormPageOptions, MediaFormPageBuilder,
    MediaFormRenderPage, build_media_asset_form_page, media_asset_form_mode,
    new_media_asset_form_page_builder,
};
pub use self::media_form::{
    MediaFormSource, ResourceMediaAttachmentLoader, ResourceMediaFormFrame,
    ResourceMediaFormFrameOptions, ResourceMediaFormPageOptions, build_resource_media_form_frame,
    build_resource_media_form_page, default_media_asset_form_input, ensure_resource_content_id,
    media_asset_form_input_from_form, media_asset_form_input_from_record,
    media_asset_form_input_from_request, media_asset_resource_form_state,
    requested_resource_content_id,
};
pub use self::list_page::{
    BuildMediaAssetListPageOptions, MediaAssetDetailHref, MediaAssetListPageBuilder, MediaAssetListRenderPage,
    MediaAssetListPageBuilderOptions, MediaAssetListPageOptions, build_media_asset_list_page,
    load_media_asset_list, media_asset_list_item_states, new_media_asset_list_page_builder,
};
pub use self::routes::{
    MEDIA_ATTACHMENT_MUTATE_ROUTE_NAME, MEDIA_DASHBOARD_ROUTE_NAME, MEDIA_DELETE_ROUTE_NAME,
    MEDIA_EDIT_POST_ROUTE_NAME, MEDIA_EDIT_ROUTE_NAME, MEDIA_INLINE_CREATE_ROUTE_NAME,
    MEDIA_INLINE_PREVIEW_ROUTE_NAME, MEDIA_LIST_ROUTE_NAME, MEDIA_NEW_POST_ROUTE_NAME,
    MEDIA_NEW_ROUTE_NAME, MEDIA_PICKER_ATTACH_ROUTE_NAME, MEDIA_PICKER_ROUTE_NAME,
    MEDIA_SHORTCODE_ROUTE_NAME, MediaManagementHandlers, MediaManagementRouteNames,
    MediaManagementRoutePermissions, default_media_management_route_names,
    default_media_management_route_paths, default_media_resource_route_set,
    media_asset_store_runtime_routes_from_management_routes,
    media_asset_store_runtime_routes_from_registry, media_management_route_names_from_definition,
    media_management_route_names_from_registry, media_management_route_service,
    register_media_management_routes,
};

mod controller;
pub use controller::*;

mod experience;
pub use experience::*;
