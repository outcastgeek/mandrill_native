//! Routing: route sets, route paths, managed registration/protection, permission
//! metadata synchronization and redirects (workflow W14).
//!
//! Routing owns:
//!
//! - the selectable route sets of a definition, whole-set selection and form surfaces;
//! - route targets: titles, breadcrumbs, page metadata, form submission/cancel targets,
//!   localized paths, optional paths and canonical detail paths;
//! - the managed route description, its admission, its any/all permission requirement
//!   and the additive permission-metadata synchronization its declared permissions need;
//! - redirect targets and responses: query-preserving route redirects, legacy redirect
//!   routes, server-sent-events URLs and mutation-result targets with their sorted
//!   parameter pairs.
//!
//! # API
//!
//! - Route selection: [`has_route_set`], [`route_set_for_mode`],
//!   [`route_set_for_request`], [`resource_route_set_for_definition`],
//!   [`resource_route_set_for_kind`], [`resource_form_mode`], [`resource_detail_route`].
//! - Route targets: [`resource_mode_title`], [`management_breadcrumb`],
//!   [`resource_page_meta`] and its list/detail/form projections,
//!   [`resource_form_action_href`], [`resource_form_cancel_href`],
//!   [`resource_localized_paths`], [`optional_route_path`], [`translate`].
//! - Composition: [`new_localized_route_path_builder`],
//!   [`new_path_prefix_management_request_detector`], [`new_translate_key_resolver`],
//!   [`new_variadic_translate_key_resolver`], [`default_canonical_route_params`],
//!   [`canonical_route_path`], [`default_action_label_keys`],
//!   [`new_default_action_label_resolver`].
//! - Managed routes: [`ManagedRoute`], [`AccessControl::protect`], [`register`],
//!   [`Service::register_routes`], [`Service::register_and_start_routes`],
//!   [`RegisteredRoutes`], [`ManagedRouteMounter`], [`without_names`],
//!   [`route_permissions`], [`normalize_permissions`], [`route_method`],
//!   [`route_display_name`], [`route_category`], [`AccessControl::sync_metadata`],
//!   [`AccessControl::metadata_sync`], [`MetadataSync`], [`ManagedStartError`].
//! - Redirects: [`new_query_preserving_route_redirect_handler`],
//!   [`new_legacy_management_redirect_route`], [`build_resource_sse_url`],
//!   [`build_resource_sse_url_for_kind`], [`mutation_result_param_pairs`],
//!   [`mutation_result_path`], [`redirect_mutation_result`].
//!
//! # Consumer-supplied capabilities
//!
//! The permission middleware enforces a requirement, the route/permission catalogue holds
//! or persists permission metadata, the typed mounter installs one protected route in the
//! consumer's own router, route path builders construct URL targets, and the translation
//! port resolves keys. The CMS owns no route table, permission record, database, SQL
//! statement, schema, migration or pool, serves no listener and keeps no process-global
//! state. Registration is performed by this owner through the consumer's mounter and
//! starts its synchronization in the consumer's application-lifetime scope.
//!
//! # Effects
//!
//! Metadata synchronization and its scheduling, query-preserving redirects and
//! mutation-result redirects return lazy [`Effect`] values whose supplied capabilities run
//! only when the description runs. Permissions, titles, paths and parameter flattening are
//! pure calculations over caller-supplied values.
//!
//! # HTML
//!
//! This owner authors no HTML. Redirect bodies and not-found bodies are produced by the
//! existing redirect response and the plain text status body; no markup is assembled here.

mod composition_helpers;
mod managed_routes;
mod paths;
mod redirects;
mod route_sets;

pub use self::composition_helpers::{
    DEFAULT_DELETE_ACTION_LABEL_KEY, DEFAULT_DRAFT_ACTION_LABEL_KEY,
    DEFAULT_PUBLISH_ACTION_LABEL_KEY, DEFAULT_SAVE_ACTION_LABEL_KEY,
    DEFAULT_TRANSLATE_ACTION_LABEL_KEY, LanguagePathPrefixer, LanguagePathStripper,
    ManagementRequestDetector, PathLanguageFunc, RequestValueFunc, RouteToPathFunc,
    VariadicTranslateKeyFunc, canonical_route_path, default_action_label_keys,
    default_canonical_route_params, new_default_action_label_resolver,
    new_localized_route_path_builder, new_path_prefix_management_request_detector,
    new_translate_key_resolver, new_variadic_translate_key_resolver,
};
pub use self::managed_routes::{
    AccessControl, AccessControlError, DEFAULT_ASYNC_SYNC_ATTEMPTS, DEFAULT_ASYNC_SYNC_RETRY_DELAY,
    DEFAULT_ASYNC_SYNC_SLOTS, DEFAULT_ROUTE_CATEGORY, ManagedRoute, ManagedRouteError,
    ManagedRouteMounter, ManagedRouteRegistration, ManagedStartError, MetadataSync,
    MetadataSyncError, Options, PERMISSION_MODE_ALL, PERMISSION_MODE_ANY, PermissionByName,
    PermissionGuard, PermissionMiddleware, PermissionMode, PermissionRequirementBuilder,
    PreparedManagedRouteCapacity, PreparedManagedRouteStorage,
    RegisteredRoutes, RouteByPathAndMethod, RouteCatalogue, RouteDraft, RouteHandler,
    RoutePermission, RoutePermissionLookup, RouteRecord, RouteRequirement, Service,
    normalize_permissions, register, route_category, route_display_name, route_method,
    route_permissions, without_names,
};
pub use self::paths::{
    CanonicalRoutePathBuilder, LocalizedRoutePathFunc, ManagementBreadcrumbBuilder,
    ManagementBreadcrumbVM, ManagementPageMeta, RoutePathBuilder, RouteSetForModeResolver,
    RouteSetForRequestResolver, management_breadcrumb, optional_route_path,
    resource_detail_page_meta, resource_form_action_href, resource_form_cancel_href,
    resource_form_page_meta, resource_list_page_meta, resource_localized_paths,
    resource_mode_title, resource_page_meta, translate,
};
pub use self::redirects::{
    LegacyRoute, MutationResultRoutePathBuilder, build_resource_sse_url,
    build_resource_sse_url_for_kind, build_resource_sse_url_into, mutation_result_param_pairs, mutation_result_path,
    new_legacy_management_redirect_route, new_query_preserving_route_redirect_handler,
    redirect_mutation_result,
};
pub use self::route_sets::{
    has_route_set, resource_detail_route, resource_form_mode, resource_route_set_for_definition,
    resource_route_set_for_kind, route_set_for_mode, route_set_for_request,
};

use effect::Effect;

/// Compile-time witness that the routing contracts keep their declared shapes.
const _: fn() = || {
    #[allow(clippy::type_complexity)]
    fn witness<E, R>()
    where
        E: Send + 'static,
        R: effect::HasClock + Send + Sync + 'static,
    {
        let _: fn(
            &AccessControl<E, R>,
            &ManagedRoute<E, R>,
            &[String],
        ) -> Effect<(), MetadataSyncError<E>, R> = AccessControl::sync_metadata;
        let _: fn(
            &AccessControl<E, R>,
            &ManagedRoute<E, R>,
        ) -> Result<Option<ManagedRouteRegistration<E, R>>, AccessControlError> =
            AccessControl::protect;
        let _: fn(&str, Option<RoutePathBuilder>, http::StatusCode) -> RouteHandler<E, R> =
            new_query_preserving_route_redirect_handler;
        let _: fn(
            Option<&crate::commands::MutationResult>,
            &str,
            Option<MutationResultRoutePathBuilder>,
        ) -> RouteHandler<E, R> = redirect_mutation_result;
    }
    witness::<std::fmt::Error, effect::TestClock>();
};
