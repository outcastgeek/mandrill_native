//! Resource definitions: the vocabulary, model, builders, registry and relation
//! declarations that every other CMS owner consumes.
//!
//! Definitions owns the resource-definition vocabulary (kinds, view modes,
//! management-route modes, actions, route slots, filter keys and modes, dashboard
//! action locations, aux routes, SEO page types, sitemap frequencies, list
//! layouts, section keys, field keys, field kinds and the canonical route
//! parameters), the complete `ResourceDefinition` model, the definition builders
//! and management-route normalization, the definition registry with its unknown-kind
//! error, and relation declarations with their checked descriptor.
//!
//! # API
//!
//! - [`Registry`] is the lookup contract; [`new_definition_registry`] constructs the
//!   concrete registry over caller-supplied definitions.
//! - [`find_resource_definition`] reports whether a kind is registered.
//! - The model types, builders and [`normalize_resource_definition`] describe a
//!   resource without performing any I/O.
//! - [`new_resource_relation_descriptor`] and [`resolve_resource_relation`] admit a
//!   checked [`ResourceRelationDescriptor`]; refused declarations return
//!   [`RelationError`] with the refusal message.
//!
//! # Data-sharing contract
//!
//! Copying a definition copies each collection and map header over shared backing
//! storage, so an in-place element mutation (a
//! filter field or aux route value) made by the caller after registration is
//! visible through later lookups, and one made through a lookup result is visible to
//! the caller and to later lookups. Each copied slice also keeps its own length, so
//! an append extends only the handle it is issued through; lengths are the only
//! stable part, because an append can overwrite a backing element that a longer
//! handle's view reaches. The model keeps both properties with explicit
//! [`SharedList`]/[`SharedMap`] handles — shared element storage with a per-handle
//! length and capacity for lists, a shared map for maps — rather than silently
//! detaching the collections with Rust value semantics. An append inside the handle's
//! capacity writes into the shared store and an append that exhausts it allocates a
//! fresh store for that handle alone (reallocation independence). Supplied capacity
//! and overlapping initialized views are retained; detached storage uses its native
//! allocation capacity. Relation declarations are the deliberate exception: they
//! are detached on normalization and on every lookup.
//!
//! # Dependencies and effects
//!
//! This owner depends on no other CMS owner, holds no capability, opens no store
//! and performs no I/O: definitions are caller-supplied data. Its operations are
//! pure and deterministic, so they are not wrapped in `Effect<A, E, R>` here; an
//! effectful workflow (queries, validation, commands, forms, rendering) lifts these
//! functions at its own boundary. There is no database, SQL, schema, migration or
//! pool anywhere in this owner.
//!
//! # Private implementation
//!
//! `vocabulary`, `model`, `builders`, `registry`, `relations`,
//! `relation_validation` and `sharing` are private implementation modules of this
//! owner: they hold the declarations and the mechanics, while this module is the
//! owner's deliberate public surface.

mod builders;
mod model;
mod registry;
mod relation_validation;
mod relations;
mod sharing;
mod vocabulary;

pub use self::model::{
    LocalizedWidgetVM, ResourceDashboardActionDefinition, ResourceDashboardActionFilterDefinition,
    ResourceDashboardDefinition, ResourceDashboardSectionDefinition, ResourceDefinition,
    ResourceDetailDefinition, ResourceDetailFieldDefinition, ResourceDetailSectionDefinition,
    ResourceDisplayDefinition, ResourceFilterDefinition, ResourceFilterOptionDefinition,
    ResourceFormDefinition, ResourceFormFieldDefinition, ResourceFormSectionDefinition,
    ResourceListDefinition, ResourceListItemDefinition, ResourceListItemMetaDefinition,
    ResourceListSectionDefinition, ResourceRouteSet, ResourceSEODefinition,
    ResourceSitemapDefinition,
};
pub use self::registry::{DefinitionRegistry, UnknownResourceKind};
pub use self::relations::{
    RelationError, ResourceRelationDefinition, ResourceRelationDescriptor,
    ResourceRelationEditorDefinition, ResourceRelationKeyDefinition,
    ResourceRelationTargetDefinition,
};
pub use self::sharing::{SharedList, SharedMap};
pub use self::vocabulary::{
    DASHBOARD_ACTION_LOCATION_HEADER, DASHBOARD_ACTION_LOCATION_TOOLS, DashboardActionLocation,
    RESOURCE_ACTION_ATTACH, RESOURCE_ACTION_CREATE, RESOURCE_ACTION_DELETE, RESOURCE_ACTION_DRAFT,
    RESOURCE_ACTION_PUBLISH, RESOURCE_ACTION_TRANSLATE, RESOURCE_ACTION_UPDATE,
    RESOURCE_AUX_ROUTE_ATTACHMENT_MUTATE, RESOURCE_AUX_ROUTE_BY_LOCATION,
    RESOURCE_AUX_ROUTE_BY_ORGANIZER, RESOURCE_AUX_ROUTE_BY_TAG, RESOURCE_AUX_ROUTE_FILTER_ALIAS,
    RESOURCE_AUX_ROUTE_INLINE_CREATE, RESOURCE_AUX_ROUTE_INLINE_PREVIEW, RESOURCE_AUX_ROUTE_PICKER,
    RESOURCE_AUX_ROUTE_PICKER_ATTACH, RESOURCE_AUX_ROUTE_SHORTCODE, RESOURCE_AUX_ROUTE_SSE,
    RESOURCE_AUX_ROUTE_UPCOMING, RESOURCE_FIELD_KEY_AUTHOR, RESOURCE_FIELD_KEY_CITY,
    RESOURCE_FIELD_KEY_CONTENT, RESOURCE_FIELD_KEY_CONTENT_ID, RESOURCE_FIELD_KEY_COUNTRY,
    RESOURCE_FIELD_KEY_DATE_TIME, RESOURCE_FIELD_KEY_DESCRIPTION, RESOURCE_FIELD_KEY_DURATION,
    RESOURCE_FIELD_KEY_KIND, RESOURCE_FIELD_KEY_LOCATION, RESOURCE_FIELD_KEY_SEARCH,
    RESOURCE_FIELD_KEY_SERIES, RESOURCE_FIELD_KEY_SLUG, RESOURCE_FIELD_KEY_SORT,
    RESOURCE_FIELD_KEY_STATUS, RESOURCE_FIELD_KEY_TAGS, RESOURCE_FIELD_KEY_TIME_ZONE,
    RESOURCE_FIELD_KEY_TITLE, RESOURCE_FIELD_KEY_URL, RESOURCE_FIELD_KIND_COLLECTION,
    RESOURCE_FIELD_KIND_DATE_TIME, RESOURCE_FIELD_KIND_DURATION, RESOURCE_FIELD_KIND_HIDDEN,
    RESOURCE_FIELD_KIND_LOCALIZED, RESOURCE_FIELD_KIND_MEDIA, RESOURCE_FIELD_KIND_SELECT,
    RESOURCE_FIELD_KIND_TAGS, RESOURCE_FIELD_KIND_TEXT, RESOURCE_FIELD_KIND_TEXT_AREA,
    RESOURCE_FIELD_KIND_TYPED_VALUE, RESOURCE_FIELD_KIND_URL, RESOURCE_FILTER_AUTHOR,
    RESOURCE_FILTER_KIND, RESOURCE_FILTER_LOCATION, RESOURCE_FILTER_MODE_MULTI_ALL,
    RESOURCE_FILTER_MODE_SINGLE, RESOURCE_FILTER_ORGANIZER, RESOURCE_FILTER_SEARCH,
    RESOURCE_FILTER_SERIES, RESOURCE_FILTER_SORT, RESOURCE_FILTER_STATUS, RESOURCE_FILTER_TAG,
    RESOURCE_ID_PATH_PARAM, RESOURCE_KIND_MEDIA, RESOURCE_LIST_LAYOUT_FEATURED,
    RESOURCE_LIST_LAYOUT_RESULTS, RESOURCE_MANAGEMENT_ROUTE_MODE_SPLIT,
    RESOURCE_MANAGEMENT_ROUTE_MODE_UNIFIED, RESOURCE_PARENT_NAME_PATH_PARAM,
    RESOURCE_PARENT_PATH_PARAM, RESOURCE_ROUTE_SLOT_DASHBOARD, RESOURCE_ROUTE_SLOT_DELETE,
    RESOURCE_ROUTE_SLOT_DETAIL, RESOURCE_ROUTE_SLOT_EDIT, RESOURCE_ROUTE_SLOT_EDIT_POST,
    RESOURCE_ROUTE_SLOT_LIST, RESOURCE_ROUTE_SLOT_NEW, RESOURCE_ROUTE_SLOT_NEW_POST,
    RESOURCE_SECTION_KEY_COMMENTS, RESOURCE_SECTION_KEY_FEATURED, RESOURCE_SECTION_KEY_GRID,
    RESOURCE_SECTION_KEY_IDENTITY, RESOURCE_SECTION_KEY_LOCALIZED_CONTENT,
    RESOURCE_SECTION_KEY_LOCALIZED_DESCRIPTION, RESOURCE_SECTION_KEY_MEDIA,
    RESOURCE_SECTION_KEY_METADATA, RESOURCE_SECTION_KEY_METRICS, RESOURCE_SECTION_KEY_RESULTS,
    RESOURCE_SECTION_KEY_SECONDARY, RESOURCE_SECTION_KEY_TOOLS, RESOURCE_SLUG_PATH_PARAM,
    RESOURCE_VIEW_MODE_DASHBOARD, RESOURCE_VIEW_MODE_DETAIL, RESOURCE_VIEW_MODE_EDIT,
    RESOURCE_VIEW_MODE_LIST, RESOURCE_VIEW_MODE_MANAGE, RESOURCE_VIEW_MODE_NEW,
    RESOURCE_VIEW_MODE_PUBLIC, ResourceAction, ResourceAuxRoute, ResourceFieldKey,
    ResourceFieldKind, ResourceFilterKey, ResourceFilterMode, ResourceFilterValue, ResourceKind,
    ResourceListLayout, ResourceManagementRouteMode, ResourceRouteSlot, ResourceSectionKey,
    ResourceViewMode, SEO_PAGE_TYPE_WEBSITE, SEOPageType, SITEMAP_CHANGE_FREQUENCY_ALWAYS,
    SITEMAP_CHANGE_FREQUENCY_DAILY, SITEMAP_CHANGE_FREQUENCY_HOURLY,
    SITEMAP_CHANGE_FREQUENCY_MONTHLY, SITEMAP_CHANGE_FREQUENCY_WEEKLY, SitemapChangeFrequency,
};

/// Lookup contract for registered resource definitions.
///
/// Both operations return definitions whose relation declarations are detached and
/// whose model collections keep their shared semantics.
pub trait Registry {
    /// Resolve one registered definition.
    ///
    /// # Errors
    ///
    /// Returns [`UnknownResourceKind`] when the kind is not registered.
    fn definition(&self, kind: &ResourceKind) -> Result<ResourceDefinition, UnknownResourceKind>;

    /// Every registered definition, in registration order.
    ///
    /// A kind registered more than once appears once per insertion and resolves to
    /// the last stored definition. The returned outer list is independent of the
    /// registry and of every other call.
    fn definitions(&self) -> Vec<ResourceDefinition>;
}

/// Compile-time witness that the concrete registry supplies the owner's lookup
/// contract.
const _: fn() = || {
    fn assert_registry<T: Registry>() {}
    assert_registry::<DefinitionRegistry>();
};

/// Construct the resource definition registry.
///
/// This is the owner's only construction path: it registers the complete supplied
/// definition set, so an incomplete registry cannot be published as complete.
#[must_use]
pub fn new_definition_registry(
    values: impl IntoIterator<Item = ResourceDefinition>,
) -> DefinitionRegistry {
    DefinitionRegistry::new(values)
}

/// Resolve a definition and report whether it exists.
///
/// An absent optional registry yields no definition. Absence is a property of this
/// lookup helper's input, not of the owner's required production registry, which stays a required
/// typed input everywhere it is needed (`new_definition_registry` builds it, and
/// every present lookup borrows it). An absent call names the registry type, e.g.
/// `find_resource_definition::<DefinitionRegistry>(None, &kind)`; a present call
/// passes `Some(&registry)` for any concrete or `dyn` [`Registry`].
#[must_use]
pub fn find_resource_definition<R: Registry + ?Sized>(
    registry: Option<&R>,
    kind: &ResourceKind,
) -> Option<ResourceDefinition> {
    registry?.definition(kind).ok()
}

/// Admit one detached relation declaration for its source resource.
///
/// # Errors
///
/// Returns [`RelationError`] with the refusal message when the declaration is
/// refused.
pub fn new_resource_relation_descriptor(
    source: ResourceKind,
    definition: ResourceRelationDefinition,
) -> Result<ResourceRelationDescriptor, RelationError> {
    relations::new_resource_relation_descriptor(source, definition)
}

/// Resolve a relation from its declared resource owner.
///
/// # Errors
///
/// Returns [`RelationError`] when the relation is unknown, duplicated or refused.
pub fn resolve_resource_relation(
    resource: &ResourceDefinition,
    name: &str,
) -> Result<ResourceRelationDescriptor, RelationError> {
    relations::resolve_resource_relation(resource, name)
}

/// Build a form definition from sections.
#[must_use]
pub fn new_resource_form_definition(
    sections: Vec<ResourceFormSectionDefinition>,
) -> ResourceFormDefinition {
    builders::new_resource_form_definition(sections)
}

/// Build an untitled form section.
#[must_use]
pub fn new_resource_form_section(
    key: ResourceSectionKey,
    fields: Vec<ResourceFormFieldDefinition>,
) -> ResourceFormSectionDefinition {
    builders::new_resource_form_section(key, fields)
}

/// Build a titled form section.
#[must_use]
pub fn new_titled_resource_form_section(
    key: ResourceSectionKey,
    title_key: String,
    description_key: String,
    fields: Vec<ResourceFormFieldDefinition>,
) -> ResourceFormSectionDefinition {
    builders::new_titled_resource_form_section(key, title_key, description_key, fields)
}

/// Build a list definition with section keys.
#[must_use]
pub fn new_resource_list_definition(
    layout: ResourceListLayout,
    sections: Vec<ResourceSectionKey>,
    meta: Vec<ResourceListItemMetaDefinition>,
) -> ResourceListDefinition {
    builders::new_resource_list_definition(layout, sections, meta)
}

/// Build the featured list layout.
#[must_use]
pub fn new_featured_resource_list_definition(
    meta: Vec<ResourceListItemMetaDefinition>,
) -> ResourceListDefinition {
    builders::new_featured_resource_list_definition(meta)
}

/// Build the results list layout.
#[must_use]
pub fn new_results_resource_list_definition(
    meta: Vec<ResourceListItemMetaDefinition>,
) -> ResourceListDefinition {
    builders::new_results_resource_list_definition(meta)
}

/// Build a detail definition from sections.
#[must_use]
pub fn new_resource_detail_definition(
    sections: Vec<ResourceDetailSectionDefinition>,
) -> ResourceDetailDefinition {
    builders::new_resource_detail_definition(sections)
}

/// Build a dashboard definition from sections.
#[must_use]
pub fn new_resource_dashboard_definition(
    sections: Vec<ResourceDashboardSectionDefinition>,
) -> ResourceDashboardDefinition {
    builders::new_resource_dashboard_definition(sections)
}

/// Resolve the canonical state key for a form field.
#[must_use]
pub fn resource_form_field_state_key(field: &ResourceFormFieldDefinition) -> String {
    model::resource_form_field_state_key(field)
}

/// Apply the owner's management-route normalization.
#[must_use]
pub fn normalize_resource_definition(definition: ResourceDefinition) -> ResourceDefinition {
    builders::normalize_resource_definition(definition)
}

/// Map the dashboard route onto the list slot.
#[must_use]
pub fn unified_management_route_set(route_set: ResourceRouteSet) -> ResourceRouteSet {
    builders::unified_management_route_set(route_set)
}

/// Whether the definition uses unified management routes.
#[must_use]
pub fn resource_uses_unified_management_routes(definition: &ResourceDefinition) -> bool {
    builders::resource_uses_unified_management_routes(definition)
}
