//! Rendering: W3 list page and W4 detail page production.
//!
//! This owner supplies:
//!
//! - the list page value model: filter options/chips, toolbar, filters, pagination,
//!   metadata, item actions, nested items, sections, breadcrumb/query and the
//!   aggregate list page with its page SEO value and optional render props;
//! - the detail page value model: fields (the forms owner's field value), sections,
//!   links, extras and the aggregate detail page;
//! - the list item and section builders, including the source trimming and
//!   blank-key admission, declared-metadata pairing, recursive children, the
//!   featured/secondary/grid split and the results layout;
//! - the detail section builders, identity section construction, supplied-body
//!   section update and declared-field projection;
//! - the frame and page builders, their query/render/SEO callback order, current-title
//!   and state overrides, route targets, alternate-language links and pagination;
//!   render-props assembly shares the caller's [`SEOView`] handle so in-place
//!   alternate-language updates stay observable to a caller retaining that handle;
//! - the W3/W4 services over the required definition registry and SEO capability,
//!   with private prepared stages, list filtered-indexability adjustment and exact
//!   page validator invocation;
//! - the resource freshness classification and the aggregate rendering composition.
//!
//! # Capabilities and effects
//!
//! The definition registry and the SEO service are required constructor inputs. Every
//! other consumer input is a genuinely optional callback from the source workflow:
//! request-language/management detection, admitted-query sources, rendering
//! callbacks, route/path builders, breadcrumb and translation ports, SEO/hreflang
//! resolvers and page validators. The CMS owns no database backend, SQL adapter,
//! schema, migration, pool, listener, scanner or host; consumer CRUD and data
//! providers stay consumer-owned.
//!
//! Service operations, frame resolution and page assembly return lazy
//! [`Effect`](effect::Effect) values under the caller's error family and environment.
//! No definition lookup, query admission, rendering callback, SEO operation,
//! translation port or validator runs when a description is constructed. The
//! prepared stages are private values consumed by the next stage, so the SEO stage
//! and page assembly run at most once per page call; no stage value is published.
//! Errors, defects, interruption and finalization failures keep their own effect
//! channels. Page validators receive the exact page value that is returned and may
//! mutate it in place.
//!
//! The pure builders (items, sections, freshness, pagination arithmetic) are ordinary
//! functions over caller-supplied data and are lifted by their effectful callers.
//!
//! # Data sharing
//!
//! Detail extras and link metadata are shared map handles, so copying a page/state
//! value preserves the source's shallow-map behavior. Sections/items built from
//! supplied state use the source's detached copies. Lists keep their declared order.
//!
//! # Private implementation
//!
//! `contracts`, `error`, `list_vms`, `list_items`, `list_pages`, `list_service`,
//! `list_freshness`, `detail_vms`, `detail_builders`, `detail_pages`,
//! `detail_service` and `composition` hold the owner's contracts and mechanics;
//! this module is the deliberate public surface.

mod composition;
pub(crate) mod datastar;
mod full_service;
pub use full_service::*;
mod contracts;
mod detail_builders;
mod detail_pages;
mod detail_service;
mod detail_vms;
mod error;
mod list_freshness;
mod list_items;
mod list_pages;
mod list_service;
mod list_vms;

pub use self::composition::{Service, new_service};
pub use self::contracts::{
    DefinitionResourcePageBuilderOptions, DetailPageOptions, DetailPageSource,
    DetailPageValidator, FrameBuilderOptions, HreflangPathResolver, HreflangsForPathResolver,
    ListPageOptions, ListPageSource, ListPageValidator, PageBuilderOptions, RenderingCallbacks,
    RenderingService, ResourceRouteSetResolver, ResourceSEOPropsResolver, SEOView,
};
pub use self::detail_builders::{
    ResourceDetailSectionState, ResourceDetailState, build_resource_detail_sections,
    build_resource_detail_sections_for_kind, resource_detail_field_state_key,
    resource_detail_identity_state, resource_detail_set_body_section,
    resource_has_detail_content,
};
pub use self::detail_pages::{
    ResourceDetailFrame, ResourceDetailPageState, build_definition_detail_page,
    build_resource_detail_frame, build_resource_detail_page,
};
pub use self::detail_service::{
    DetailService, new_detail_service,
};
pub use self::detail_vms::{ExtrasVM, FieldVM, LinkVM, PageVM, SectionVM};
pub use self::error::RenderingError;
pub use self::list_freshness::{resource_freshness_kind, resource_freshness_kind_at};
pub use self::list_items::{
    ResourceListItemMetaState, ResourceListItemState, ResourceListSectionState, ResourceListState,
    build_resource_list_items, build_resource_list_items_for_kind, build_resource_list_sections,
    build_resource_list_sections_for_kind, resource_list_meta_states, resource_meta_state_link,
    resource_meta_state_value, resource_split_list_sections,
};
pub use self::list_pages::{
    ResourceListFrame, ResourceListPageState, build_definition_list_page,
    build_definition_list_page_with_items, build_resource_list_frame, build_resource_list_page,
    build_resource_list_pagination, resource_list_current_page, resource_list_page_href,
};
pub use self::list_service::{ListService, new_list_service};
pub use self::list_vms::{
    BreadcrumbVM, ListFiltersVM, ListPageLinkVM, ListPaginationVM, ListToolbarVM, Query,
    RESOURCE_ITEM_ACTION_KIND_DELETE, RESOURCE_ITEM_ACTION_KIND_EDIT,
    RESOURCE_ITEM_ACTION_KIND_EXTERNAL_LINK, ResourceFilterOptionVM, ResourceItemActionKind,
    ResourceItemActionVM, ResourceListItemVM, ResourceListPageVM, ResourceListSectionVM,
    ResourceMetaVM,
};

/// The promoted detail page name.
pub type ResourceDetailPageVM = PageVM;
/// The promoted detail section name.
pub type ResourceDetailSectionVM = SectionVM;
/// The promoted list filter chip name.
pub type ResourceFilterChipVM = crate::queries::ResourceFilterChipVM;
/// The promoted breadcrumb name.
pub type ManagementBreadcrumbVM = crate::routing::ManagementBreadcrumbVM;

pub use crate::definitions::{
    ResourceDetailDefinition, ResourceDetailFieldDefinition, ResourceKind, ResourceListDefinition,
    ResourceListItemDefinition, ResourceRouteSet, ResourceSectionKey,
};
pub use crate::localization::TranslateKeyFunc;
pub use crate::routing::RoutePathBuilder;

/// Compile-time witness that the services keep their declared lazy effect shapes.
const _: fn() = || {
    fn witness<E, R>()
    where
        E: Send + 'static,
        R: Send + Sync + 'static,
    {
        let _: fn(
            &Service<E>,
            &effect_server::Request,
            &ResourceKind,
            crate::queries::ResourceListQuery,
        ) -> effect::Effect<ResourceListPageVM, RenderingError<E>, R> =
            Service::list_page::<R>;
        let _: fn(
            &Service<E>,
            &effect_server::Request,
            &ResourceKind,
            crate::queries::ResourceDetailQuery,
        ) -> effect::Effect<PageVM, RenderingError<E>, R> = Service::detail_page::<R>;
    }
    witness::<(), ()>();
};

