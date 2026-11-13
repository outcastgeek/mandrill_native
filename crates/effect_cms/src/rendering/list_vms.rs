//! The list page view models.
//!
//! Every value here is the source's published list contract: filter options and
//! chips, pagination/toolbar/filter state, item actions and metadata, nested list
//! items, sections, the aggregate page and its breadcrumb/query aliases. The filter
//! chip and breadcrumb contracts are re-exported from their owning modules rather
//! than redefined.

use maud::Markup;

use crate::definitions::{
    ResourceFieldKey, ResourceFilterKey, ResourceKind, ResourceRouteSet, ResourceSectionKey,
    ResourceViewMode, SharedList,
};
use crate::queries::ResourceFilterChipVM;
use crate::routing::ManagementBreadcrumbVM;
use crate::seo::SEOVM;

use super::contracts::SEOView;

pub use crate::queries::ResourceListQuery as Query;
pub use crate::routing::ManagementBreadcrumbVM as BreadcrumbVM;

/// One rendered filter option.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ResourceFilterOptionVM {
    /// Filter key.
    pub key: ResourceFilterKey,
    /// Option value.
    pub value: String,
    /// Whether the option is active.
    pub active: bool,
    /// Option target.
    pub href: String,
    /// Localized option label.
    pub label: String,
}

/// One pagination link.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ListPageLinkVM {
    /// Page number.
    pub page: i64,
    /// Page target.
    pub href: String,
    /// Whether this is the current page.
    pub current: bool,
}

/// The rendered list toolbar.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ListToolbarVM {
    /// Search target.
    pub action: String,
    /// Search field key.
    pub search_key: ResourceFieldKey,
    /// Search value.
    pub search_value: crate::commands::InputBytes,
    /// Sort field key.
    pub sort_key: ResourceFieldKey,
    /// Sort value.
    pub sort_value: crate::commands::InputBytes,
}

/// The rendered filter state.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ListFiltersVM {
    /// Filter options.
    pub options: Vec<ResourceFilterOptionVM>,
}

/// The rendered pagination state.
#[derive(Clone, Debug, Default)]
pub struct ListPaginationVM {
    /// Current page.
    pub page: i64,
    /// Effective page size.
    pub page_size: i64,
    /// Total item count.
    pub total_items: i64,
    /// Total page count.
    pub total_pages: i64,
    /// Whether a previous page exists.
    pub has_prev: bool,
    /// Whether a next page exists.
    pub has_next: bool,
    /// Previous-page target.
    pub prev_href: String,
    /// Next-page target.
    pub next_href: String,
    /// Rendered page links.
    pub links: Option<SharedList<ListPageLinkVM>>,
}

impl PartialEq for ListPaginationVM {
    fn eq(&self, other: &Self) -> bool {
        self.page == other.page
            && self.page_size == other.page_size
            && self.total_items == other.total_items
            && self.total_pages == other.total_pages
            && self.has_prev == other.has_prev
            && self.has_next == other.has_next
            && self.prev_href == other.prev_href
            && self.next_href == other.next_href
            && self.links.as_ref().map(SharedList::to_vec)
                == other.links.as_ref().map(SharedList::to_vec)
    }
}

impl Eq for ListPaginationVM {}

/// One rendered item meta value.
#[derive(Clone, Debug, Default)]
pub struct ResourceMetaVM {
    /// A prepared metadata component, when the consumer supplied one.
    pub component: Option<Markup>,
    /// Localized meta label.
    pub label: String,
    /// Meta value.
    pub value: String,
    /// Meta target.
    pub href: String,
}

/// Classifies an item action.
#[derive(Clone, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ResourceItemActionKind(String);

impl ResourceItemActionKind {
    /// Admit the declared action kind exactly as written.
    #[must_use]
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// The action kind exactly as written.
    #[must_use]
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl From<&str> for ResourceItemActionKind {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

/// The edit action.
pub const RESOURCE_ITEM_ACTION_KIND_EDIT: &str = "edit";
/// The delete action.
pub const RESOURCE_ITEM_ACTION_KIND_DELETE: &str = "delete";
/// The external-link action.
pub const RESOURCE_ITEM_ACTION_KIND_EXTERNAL_LINK: &str = "external-link";

/// One rendered item action.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ResourceItemActionVM {
    /// Action kind.
    pub kind: ResourceItemActionKind,
    /// Localized action label.
    pub label: String,
    /// Action target.
    pub href: String,
    /// Confirmation message.
    pub confirm_message: String,
}

/// One rendered list item.
#[derive(Clone, Debug, Default)]
pub struct ResourceListItemVM {
    /// Stable item key.
    pub key: String,
    /// Item title.
    pub title: crate::commands::InputBytes,
    /// Item summary.
    pub summary: crate::commands::InputBytes,
    /// Detail target.
    pub detail_href: String,
    /// External target.
    pub external_href: String,
    /// Status value.
    pub status: String,
    /// Image target.
    pub image_url: crate::commands::InputBytes,
    /// Rendered metadata.
    pub meta: Option<SharedList<ResourceMetaVM>>,
    /// Rendered actions.
    pub actions: Option<SharedList<ResourceItemActionVM>>,
    /// Primary action label.
    pub action_label: String,
    /// Primary action target.
    pub action_href: String,
    /// Nested child items.
    pub children: Option<SharedList<ResourceListItemVM>>,
}

/// One rendered list section.
#[derive(Clone, Debug, Default)]
pub struct ResourceListSectionVM {
    /// Section key.
    pub key: ResourceSectionKey,
    /// Localized section title.
    pub title: String,
    /// Localized section description.
    pub description: String,
    /// Section items.
    pub items: Vec<ResourceListItemVM>,
}

/// The rendered list page.
#[derive(Clone, Debug, Default)]
pub struct ResourceListPageVM {
    /// Resource kind.
    pub kind: ResourceKind,
    /// Page surface.
    pub mode: ResourceViewMode,
    /// Localized page title.
    pub title: String,
    /// Localized page subtitle.
    pub subtitle: String,
    /// Management breadcrumb.
    pub breadcrumb: ManagementBreadcrumbVM,
    /// Selected route names.
    pub routes: ResourceRouteSet,
    /// New-resource target.
    pub new_href: String,
    /// Whether the caller is logged in.
    pub is_logged_in: bool,
    /// Whether the caller may create.
    pub can_create: bool,
    /// Whether this is the management surface.
    pub manage_view: bool,
    /// The admitted query.
    pub query: Query,
    /// Rendered filter options.
    pub filters: Vec<ResourceFilterOptionVM>,
    /// Active filter chips.
    pub active_filter_chips: Vec<ResourceFilterChipVM>,
    /// Rendered toolbar.
    pub toolbar: ListToolbarVM,
    /// Rendered filter state.
    pub filter_state: ListFiltersVM,
    /// Rendered pagination.
    pub pagination: ListPaginationVM,
    /// Rendered sections.
    pub sections: Vec<ResourceListSectionVM>,
    /// The service-produced page SEO value.
    pub seo: SEOVM,
    /// Optional consumer-supplied render props.
    pub seo_view: SEOView,
}
