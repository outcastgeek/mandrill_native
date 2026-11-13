//! The resource definition model: route sets, display, filters, list, form,
//! detail, dashboard, SEO, sitemap and the aggregate definition.
//!
//! The model is plain declaration data. Two representation rules preserve its
//! observable behaviour:
//!
//! - Every collection shared by reference (collections and maps inside a definition
//!   value) is a shared handle from `sharing`, so an **in-place element** mutation (a
//!   filter field, an option label, an aux route value) stays visible through the
//!   registry and vice versa. Copying a definition copies each list header — shared
//!   backing storage plus that copy's own start, length and capacity — and shares each
//!   map. Appends extend only the handle they are issued through, writing into shared
//!   storage within its capacity or detaching its visible elements when exhausted.
//!   Detached storage uses its native allocation capacity.
//! - Relation declarations are the deliberate exception: they are detached, so a
//!   definition copy owns its own relation data (see [`super::relations`]).
//!
//! The model is a zero-valued default: empty kind and strings, empty collections,
//! `false` flags and zero numbers. No invariant is invented here.

use super::relations::ResourceRelationDefinition;
use super::sharing::{SharedList, SharedMap};
use super::vocabulary::{
    DashboardActionLocation, ResourceAction, ResourceAuxRoute, ResourceFieldKey, ResourceFieldKind,
    ResourceFilterKey, ResourceFilterMode, ResourceFilterValue, ResourceKind, ResourceListLayout,
    ResourceManagementRouteMode, ResourceRouteSlot, ResourceSectionKey, SEOPageType,
    SitemapChangeFrequency,
};

/// The route names for every resource slot.
#[derive(Clone, Debug, Default)]
pub struct ResourceRouteSet {
    /// Dashboard route name.
    pub dashboard: String,
    /// List route name.
    pub list: String,
    /// Detail route name.
    pub detail: String,
    /// New-form route name.
    pub new: String,
    /// New-form submission route name.
    pub new_post: String,
    /// Edit-form route name.
    pub edit: String,
    /// Edit-form submission route name.
    pub edit_post: String,
    /// Delete route name.
    pub delete: String,
    /// Auxiliary route names, shared by reference.
    pub aux: SharedMap<ResourceAuxRoute, String>,
}

/// The translated title keys for each surface.
#[derive(Clone, Debug, Default)]
pub struct ResourceDisplayDefinition {
    /// Dashboard title key.
    pub dashboard_title_key: String,
    /// Dashboard subtitle key.
    pub dashboard_subtitle_key: String,
    /// List title key.
    pub list_title_key: String,
    /// List subtitle key.
    pub list_subtitle_key: String,
    /// New-form title key.
    pub new_title_key: String,
    /// Edit-form title key.
    pub edit_title_key: String,
    /// Detail title key.
    pub detail_title_key: String,
}

/// One list filter declaration.
#[derive(Clone, Debug, Default)]
pub struct ResourceFilterDefinition {
    /// Filter key.
    pub key: ResourceFilterKey,
    /// Query parameter name.
    pub param: String,
    /// Filter mode.
    pub mode: ResourceFilterMode,
    /// Label key.
    pub label_key: String,
    /// Add-label key.
    pub add_label_key: String,
    /// Remove-label key.
    pub remove_label_key: String,
    /// Default value.
    pub default_value: ResourceFilterValue,
    /// Allowed values, shared by reference.
    pub allowed: SharedList<ResourceFilterValue>,
    /// Selectable options, shared by reference.
    pub options: SharedList<ResourceFilterOptionDefinition>,
}

/// One filter option declaration.
#[derive(Clone, Debug, Default)]
pub struct ResourceFilterOptionDefinition {
    /// Option value.
    pub value: ResourceFilterValue,
    /// Option label key.
    pub label_key: String,
}

/// A dashboard action coupled to one filter value.
#[derive(Clone, Debug, Default)]
pub struct ResourceDashboardActionFilterDefinition {
    /// Filter key.
    pub key: ResourceFilterKey,
    /// Filter value.
    pub value: ResourceFilterValue,
}

/// One dashboard action declaration.
#[derive(Clone, Debug, Default)]
pub struct ResourceDashboardActionDefinition {
    /// Where the action is placed.
    pub location: DashboardActionLocation,
    /// Label key.
    pub label_key: String,
    /// Icon name.
    pub icon: String,
    /// Route slot the action targets.
    pub route_slot: ResourceRouteSlot,
    /// Route name the action targets.
    pub route_name: String,
    /// Filter values the action carries, shared by reference.
    pub filters: SharedList<ResourceDashboardActionFilterDefinition>,
}

/// SEO behaviour for a resource.
#[derive(Clone, Debug, Default)]
pub struct ResourceSEODefinition {
    /// schema.org page type.
    pub page_type: SEOPageType,
    /// List title key.
    pub list_title_key: String,
    /// List description key.
    pub list_description_key: String,
    /// Whether the unfiltered list is indexable.
    pub indexable_list: bool,
    /// Whether a filtered list is indexable.
    pub indexable_filtered_list: bool,
    /// Whether the detail page is indexable.
    pub indexable_detail: bool,
}

/// Sitemap behaviour for a resource.
#[derive(Clone, Debug, Default)]
pub struct ResourceSitemapDefinition {
    /// Whether the resource contributes to the sitemap.
    pub enabled: bool,
    /// Sitemap route name.
    pub route_name: String,
    /// Declared change frequency.
    pub change_frequency: SitemapChangeFrequency,
    /// Declared priority.
    pub priority: f64,
}

/// One list section declaration.
#[derive(Clone, Debug, Default)]
pub struct ResourceListSectionDefinition {
    /// Section key.
    pub key: ResourceSectionKey,
    /// Title key.
    pub title_key: String,
    /// Description key.
    pub description_key: String,
}

/// One list item meta field declaration.
#[derive(Clone, Debug, Default)]
pub struct ResourceListItemMetaDefinition {
    /// Meta label.
    pub label: String,
    /// Meta label key.
    pub label_key: String,
}

/// The list item shape.
#[derive(Clone, Debug, Default)]
pub struct ResourceListItemDefinition {
    /// Meta fields, shared by reference.
    pub meta: SharedList<ResourceListItemMetaDefinition>,
}

/// The list page shape.
#[derive(Clone, Debug, Default)]
pub struct ResourceListDefinition {
    /// List layout.
    pub layout: ResourceListLayout,
    /// List sections, shared by reference.
    pub sections: SharedList<ResourceListSectionDefinition>,
    /// List item shape.
    pub item: ResourceListItemDefinition,
}

/// One form field declaration.
#[derive(Clone, Debug, Default)]
pub struct ResourceFormFieldDefinition {
    /// Field key.
    pub key: ResourceFieldKey,
    /// Rendered input kind.
    pub kind: ResourceFieldKind,
    /// Submitted field name.
    pub name: String,
    /// Input element id.
    pub input_id: String,
    /// Label key.
    pub label_key: String,
    /// Placeholder key.
    pub placeholder_key: String,
    /// Help-text key.
    pub help_text_key: String,
    /// Description key.
    pub description_key: String,
    /// Whether the field is required.
    pub required: bool,
    /// Rows for a text-area input.
    pub rows: i64,
    /// Secondary submitted field name.
    pub secondary_name: String,
    /// Secondary input element id.
    pub secondary_id: String,
}

/// One form section declaration.
#[derive(Clone, Debug, Default)]
pub struct ResourceFormSectionDefinition {
    /// Section key.
    pub key: ResourceSectionKey,
    /// Title key.
    pub title_key: String,
    /// Description key.
    pub description_key: String,
    /// Section fields, shared by reference.
    pub fields: SharedList<ResourceFormFieldDefinition>,
}

/// The form page shape.
#[derive(Clone, Debug, Default)]
pub struct ResourceFormDefinition {
    /// Form sections, shared by reference.
    pub sections: SharedList<ResourceFormSectionDefinition>,
}

/// One detail field declaration.
#[derive(Clone, Debug, Default)]
pub struct ResourceDetailFieldDefinition {
    /// Field key.
    pub key: ResourceFieldKey,
    /// Field name.
    pub name: String,
    /// Label key.
    pub label_key: String,
}

/// One detail section declaration.
#[derive(Clone, Debug, Default)]
pub struct ResourceDetailSectionDefinition {
    /// Section key.
    pub key: ResourceSectionKey,
    /// Title key.
    pub title_key: String,
    /// Description key.
    pub description_key: String,
    /// Section fields, shared by reference.
    pub fields: SharedList<ResourceDetailFieldDefinition>,
}

/// The detail page shape.
#[derive(Clone, Debug, Default)]
pub struct ResourceDetailDefinition {
    /// Detail sections, shared by reference.
    pub sections: SharedList<ResourceDetailSectionDefinition>,
}

/// One dashboard section declaration.
#[derive(Clone, Debug, Default)]
pub struct ResourceDashboardSectionDefinition {
    /// Section key.
    pub key: ResourceSectionKey,
    /// Title key.
    pub title_key: String,
    /// Description key.
    pub description_key: String,
}

/// The dashboard page shape.
#[derive(Clone, Debug, Default)]
pub struct ResourceDashboardDefinition {
    /// Dashboard sections, shared by reference.
    pub sections: SharedList<ResourceDashboardSectionDefinition>,
}

/// Identifies one localized authoring widget declaration.
#[derive(Clone, Debug, Default)]
pub struct LocalizedWidgetVM {
    /// Widget key.
    pub widget_key: String,
    /// Widget scope.
    pub scope: String,
}

/// The complete declaration of one resource type.
#[derive(Clone, Debug, Default)]
pub struct ResourceDefinition {
    /// Relation declarations, deliberately detached from the caller.
    pub relations: Vec<ResourceRelationDefinition>,
    /// Resource kind; an empty kind is not registered.
    pub kind: ResourceKind,
    /// Display title keys.
    pub display: ResourceDisplayDefinition,
    /// Management route names.
    pub management_routes: ResourceRouteSet,
    /// Management route mode.
    pub management_route_mode: ResourceManagementRouteMode,
    /// Public route names.
    pub public_routes: ResourceRouteSet,
    /// List filters, shared by reference.
    pub filters: SharedList<ResourceFilterDefinition>,
    /// Declared actions, shared by reference.
    pub actions: SharedList<ResourceAction>,
    /// Dashboard actions, shared by reference.
    pub dashboard_actions: SharedList<ResourceDashboardActionDefinition>,
    /// Localized authoring widgets, shared by reference.
    pub widgets: SharedList<LocalizedWidgetVM>,
    /// SEO behaviour.
    pub seo: ResourceSEODefinition,
    /// Sitemap behaviour.
    pub sitemap: ResourceSitemapDefinition,
    /// List page shape.
    pub list: ResourceListDefinition,
    /// Form page shape.
    pub form: ResourceFormDefinition,
    /// Detail page shape.
    pub detail: ResourceDetailDefinition,
    /// Dashboard page shape.
    pub dashboard: ResourceDashboardDefinition,
}

/// Resolve the canonical state key for a form field.
///
/// The trimmed field name wins when it is not blank; otherwise the field key is
/// used exactly as declared.
#[must_use]
pub fn resource_form_field_state_key(field: &ResourceFormFieldDefinition) -> String {
    let name = field.name.trim();
    if name.is_empty() {
        field.key.as_str().to_owned()
    } else {
        name.to_owned()
    }
}
