//! Resource-definition vocabulary: kinds, modes, actions, route slots, filter
//! keys, section and field keys.
//!
//! Every named string is an *open* value: consumers declare their own kinds,
//! actions, field keys and section keys, and the owner must not reject them as if
//! the named constants were a closed enum. The constants below carry the canonical
//! spellings, including the empty split management-route mode, and exist so consumers
//! can name the same value without copying the string.

/// One of the owner's named string types.
///
/// The value is an open string: the owner performs no vocabulary validation, so a
/// consumer-defined value is as valid as a named constant. The constants are the
/// only closed part, and they are spellings, not an admission rule.
macro_rules! open_vocabulary_value {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Clone, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
        pub struct $name(String);

        impl $name {
            /// Admit the caller-supplied value exactly as written.
            #[must_use]
            pub fn new(value: impl Into<String>) -> Self {
                Self(value.into())
            }

            /// The value exactly as written.
            #[must_use]
            pub fn as_str(&self) -> &str {
                self.0.as_str()
            }
        }

        impl From<&str> for $name {
            fn from(value: &str) -> Self {
                Self::new(value)
            }
        }

        impl From<String> for $name {
            fn from(value: String) -> Self {
                Self::new(value)
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                self.0.as_str()
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str(self.0.as_str())
            }
        }
    };
}

open_vocabulary_value! {
    /// Identifies one registered resource type.
    ResourceKind
}

/// The media resource kind.
pub const RESOURCE_KIND_MEDIA: &str = "media";

open_vocabulary_value! {
    /// Identifies the surface a page is produced for.
    ResourceViewMode
}

/// The list surface.
pub const RESOURCE_VIEW_MODE_LIST: &str = "list";
/// The detail surface.
pub const RESOURCE_VIEW_MODE_DETAIL: &str = "detail";
/// The dashboard surface.
pub const RESOURCE_VIEW_MODE_DASHBOARD: &str = "dashboard";
/// The new-resource form surface.
pub const RESOURCE_VIEW_MODE_NEW: &str = "new";
/// The edit-resource form surface.
pub const RESOURCE_VIEW_MODE_EDIT: &str = "edit";
/// The public surface.
pub const RESOURCE_VIEW_MODE_PUBLIC: &str = "public";
/// The management surface.
pub const RESOURCE_VIEW_MODE_MANAGE: &str = "manage";

open_vocabulary_value! {
    /// Selects split or unified management routing.
    ResourceManagementRouteMode
}

/// Split management routing, which is also the default mode.
pub const RESOURCE_MANAGEMENT_ROUTE_MODE_SPLIT: &str = "";
/// Unified management routing.
pub const RESOURCE_MANAGEMENT_ROUTE_MODE_UNIFIED: &str = "unified";

open_vocabulary_value! {
    /// Identifies one mutation the platform can perform.
    ResourceAction
}

/// The create action.
pub const RESOURCE_ACTION_CREATE: &str = "create";
/// The update action.
pub const RESOURCE_ACTION_UPDATE: &str = "update";
/// The delete action.
pub const RESOURCE_ACTION_DELETE: &str = "delete";
/// The publish action.
pub const RESOURCE_ACTION_PUBLISH: &str = "publish";
/// The draft action.
pub const RESOURCE_ACTION_DRAFT: &str = "draft";
/// The attach action.
pub const RESOURCE_ACTION_ATTACH: &str = "attach";
/// The translate action.
pub const RESOURCE_ACTION_TRANSLATE: &str = "translate";

open_vocabulary_value! {
    /// Names one route slot in a route set.
    ResourceRouteSlot
}

/// The dashboard route slot.
pub const RESOURCE_ROUTE_SLOT_DASHBOARD: &str = "dashboard";
/// The list route slot.
pub const RESOURCE_ROUTE_SLOT_LIST: &str = "list";
/// The detail route slot.
pub const RESOURCE_ROUTE_SLOT_DETAIL: &str = "detail";
/// The new-form route slot.
pub const RESOURCE_ROUTE_SLOT_NEW: &str = "new";
/// The new-form submission route slot.
pub const RESOURCE_ROUTE_SLOT_NEW_POST: &str = "new-post";
/// The edit-form route slot.
pub const RESOURCE_ROUTE_SLOT_EDIT: &str = "edit";
/// The edit-form submission route slot.
pub const RESOURCE_ROUTE_SLOT_EDIT_POST: &str = "edit-post";
/// The delete route slot.
pub const RESOURCE_ROUTE_SLOT_DELETE: &str = "delete";

open_vocabulary_value! {
    /// Identifies a list filter.
    ResourceFilterKey
}

/// The search filter.
pub const RESOURCE_FILTER_SEARCH: &str = "search";
/// The status filter.
pub const RESOURCE_FILTER_STATUS: &str = "status";
/// The author filter.
pub const RESOURCE_FILTER_AUTHOR: &str = "author";
/// The tag filter.
pub const RESOURCE_FILTER_TAG: &str = "tag";
/// The series filter.
pub const RESOURCE_FILTER_SERIES: &str = "series";
/// The location filter.
pub const RESOURCE_FILTER_LOCATION: &str = "location";
/// The organizer filter.
pub const RESOURCE_FILTER_ORGANIZER: &str = "organizer";
/// The kind filter.
pub const RESOURCE_FILTER_KIND: &str = "kind";
/// The sort filter.
pub const RESOURCE_FILTER_SORT: &str = "sort";

open_vocabulary_value! {
    /// One filter value.
    ResourceFilterValue
}

open_vocabulary_value! {
    /// Identifies single or multi-value filter behaviour.
    ResourceFilterMode
}

/// A single-value filter.
pub const RESOURCE_FILTER_MODE_SINGLE: &str = "single";
/// A multi-value filter where every selected value must match.
pub const RESOURCE_FILTER_MODE_MULTI_ALL: &str = "multi_all";

open_vocabulary_value! {
    /// Places a dashboard action.
    DashboardActionLocation
}

/// The dashboard header location.
pub const DASHBOARD_ACTION_LOCATION_HEADER: &str = "header";
/// The dashboard tools location.
pub const DASHBOARD_ACTION_LOCATION_TOOLS: &str = "tools";

open_vocabulary_value! {
    /// Names an auxiliary route slot.
    ResourceAuxRoute
}

/// The server-sent-events aux route.
pub const RESOURCE_AUX_ROUTE_SSE: &str = "sse";
/// The filter-alias aux route.
pub const RESOURCE_AUX_ROUTE_FILTER_ALIAS: &str = "filter-alias";
/// The upcoming aux route.
pub const RESOURCE_AUX_ROUTE_UPCOMING: &str = "upcoming";
/// The by-tag aux route.
pub const RESOURCE_AUX_ROUTE_BY_TAG: &str = "by-tag";
/// The by-location aux route.
pub const RESOURCE_AUX_ROUTE_BY_LOCATION: &str = "by-location";
/// The by-organizer aux route.
pub const RESOURCE_AUX_ROUTE_BY_ORGANIZER: &str = "by-organizer";
/// The media picker aux route.
pub const RESOURCE_AUX_ROUTE_PICKER: &str = "picker";
/// The media picker attach aux route.
pub const RESOURCE_AUX_ROUTE_PICKER_ATTACH: &str = "picker-attach";
/// The inline-create aux route.
pub const RESOURCE_AUX_ROUTE_INLINE_CREATE: &str = "inline-create";
/// The inline-preview aux route.
pub const RESOURCE_AUX_ROUTE_INLINE_PREVIEW: &str = "inline-preview";
/// The shortcode aux route.
pub const RESOURCE_AUX_ROUTE_SHORTCODE: &str = "shortcode";
/// The attachment-mutate aux route.
pub const RESOURCE_AUX_ROUTE_ATTACHMENT_MUTATE: &str = "attachment-mutate";

open_vocabulary_value! {
    /// Identifies the schema.org page type.
    SEOPageType
}

/// The website page type.
pub const SEO_PAGE_TYPE_WEBSITE: &str = "website";

open_vocabulary_value! {
    /// Identifies a sitemap change frequency.
    SitemapChangeFrequency
}

/// The always change frequency.
pub const SITEMAP_CHANGE_FREQUENCY_ALWAYS: &str = "always";
/// The hourly change frequency.
pub const SITEMAP_CHANGE_FREQUENCY_HOURLY: &str = "hourly";
/// The daily change frequency.
pub const SITEMAP_CHANGE_FREQUENCY_DAILY: &str = "daily";
/// The weekly change frequency.
pub const SITEMAP_CHANGE_FREQUENCY_WEEKLY: &str = "weekly";
/// The monthly change frequency.
pub const SITEMAP_CHANGE_FREQUENCY_MONTHLY: &str = "monthly";

open_vocabulary_value! {
    /// Identifies the list layout.
    ResourceListLayout
}

/// The results list layout.
pub const RESOURCE_LIST_LAYOUT_RESULTS: &str = "results";
/// The featured list layout.
pub const RESOURCE_LIST_LAYOUT_FEATURED: &str = "featured";

open_vocabulary_value! {
    /// Identifies a section within a page definition.
    ResourceSectionKey
}

/// The identity section.
pub const RESOURCE_SECTION_KEY_IDENTITY: &str = "identity";
/// The localized-description section.
pub const RESOURCE_SECTION_KEY_LOCALIZED_DESCRIPTION: &str = "localized-description";
/// The localized-content section.
pub const RESOURCE_SECTION_KEY_LOCALIZED_CONTENT: &str = "localized-content";
/// The media section.
pub const RESOURCE_SECTION_KEY_MEDIA: &str = "media";
/// The metadata section.
pub const RESOURCE_SECTION_KEY_METADATA: &str = "metadata";
/// The featured section.
pub const RESOURCE_SECTION_KEY_FEATURED: &str = "featured";
/// The secondary section.
pub const RESOURCE_SECTION_KEY_SECONDARY: &str = "secondary";
/// The grid section.
pub const RESOURCE_SECTION_KEY_GRID: &str = "grid";
/// The results section.
pub const RESOURCE_SECTION_KEY_RESULTS: &str = "results";
/// The comments section.
pub const RESOURCE_SECTION_KEY_COMMENTS: &str = "comments";
/// The metrics section.
pub const RESOURCE_SECTION_KEY_METRICS: &str = "metrics";
/// The tools section.
pub const RESOURCE_SECTION_KEY_TOOLS: &str = "tools";

open_vocabulary_value! {
    /// Identifies a form or detail field.
    ResourceFieldKey
}

/// The title field.
pub const RESOURCE_FIELD_KEY_TITLE: &str = "title";
/// The slug field.
pub const RESOURCE_FIELD_KEY_SLUG: &str = "slug";
/// The author field.
pub const RESOURCE_FIELD_KEY_AUTHOR: &str = "author";
/// The series field.
pub const RESOURCE_FIELD_KEY_SERIES: &str = "series";
/// The tags field.
pub const RESOURCE_FIELD_KEY_TAGS: &str = "tags";
/// The description field.
pub const RESOURCE_FIELD_KEY_DESCRIPTION: &str = "description";
/// The content field.
pub const RESOURCE_FIELD_KEY_CONTENT: &str = "content";
/// The status field.
pub const RESOURCE_FIELD_KEY_STATUS: &str = "status";
/// The content-id field.
pub const RESOURCE_FIELD_KEY_CONTENT_ID: &str = "content-id";
/// The city field.
pub const RESOURCE_FIELD_KEY_CITY: &str = "city";
/// The country field.
pub const RESOURCE_FIELD_KEY_COUNTRY: &str = "country";
/// The url field.
pub const RESOURCE_FIELD_KEY_URL: &str = "url";
/// The date-time field.
pub const RESOURCE_FIELD_KEY_DATE_TIME: &str = "date-time";
/// The time-zone field.
pub const RESOURCE_FIELD_KEY_TIME_ZONE: &str = "time-zone";
/// The duration field.
pub const RESOURCE_FIELD_KEY_DURATION: &str = "duration";
/// The location field.
pub const RESOURCE_FIELD_KEY_LOCATION: &str = "location";
/// The search field.
pub const RESOURCE_FIELD_KEY_SEARCH: &str = "search";
/// The sort field.
pub const RESOURCE_FIELD_KEY_SORT: &str = "sort";
/// The kind field.
pub const RESOURCE_FIELD_KEY_KIND: &str = "kind";

open_vocabulary_value! {
    /// Identifies the rendered input kind.
    ResourceFieldKind
}

/// The typed-value input kind.
pub const RESOURCE_FIELD_KIND_TYPED_VALUE: &str = "typed_value";
/// The single-line text input kind.
pub const RESOURCE_FIELD_KIND_TEXT: &str = "text";
/// The text-area input kind.
pub const RESOURCE_FIELD_KIND_TEXT_AREA: &str = "textarea";
/// The select input kind.
pub const RESOURCE_FIELD_KIND_SELECT: &str = "select";
/// The hidden input kind.
pub const RESOURCE_FIELD_KIND_HIDDEN: &str = "hidden";
/// The tags input kind.
pub const RESOURCE_FIELD_KIND_TAGS: &str = "tags";
/// The localized input kind.
pub const RESOURCE_FIELD_KIND_LOCALIZED: &str = "localized";
/// The media input kind.
pub const RESOURCE_FIELD_KIND_MEDIA: &str = "media";
/// The date-time input kind.
pub const RESOURCE_FIELD_KIND_DATE_TIME: &str = "datetime";
/// The duration input kind.
pub const RESOURCE_FIELD_KIND_DURATION: &str = "duration";
/// The url input kind.
pub const RESOURCE_FIELD_KIND_URL: &str = "url";
/// The collection input kind.
pub const RESOURCE_FIELD_KIND_COLLECTION: &str = "collection";

/// Canonical route parameter name for a resource id.
pub const RESOURCE_ID_PATH_PARAM: &str = "id";
/// Canonical route parameter name for a resource slug.
pub const RESOURCE_SLUG_PATH_PARAM: &str = "slug";
/// Canonical route parameter name for a parent resource.
pub const RESOURCE_PARENT_PATH_PARAM: &str = "parent";
/// Canonical route parameter name for a parent resource name.
pub const RESOURCE_PARENT_NAME_PATH_PARAM: &str = "parentName";
