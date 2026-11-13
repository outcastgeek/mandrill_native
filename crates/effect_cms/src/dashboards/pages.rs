use crate::definitions::{DashboardActionLocation, ResourceKind, ResourceRouteSet, ResourceSectionKey, ResourceViewMode, SharedList};
use crate::rendering::{ListPaginationVM, ResourceItemActionKind, ResourceListItemVM};
use crate::routing::{ManagementBreadcrumbVM, ManagementPageMeta};
use super::DashboardSearchVM;

#[derive(Clone, Debug, Default)]
pub struct ActionVM { pub label: String, pub href: String, pub icon: String, pub location: DashboardActionLocation }
pub type DashboardActionVM = ActionVM;
#[derive(Clone, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct DashboardStatIcon(pub String);
#[derive(Clone, Debug, Default)]
pub struct DashboardStatVM { pub key: String, pub label: String, pub value: String, pub icon: DashboardStatIcon }
pub const DASHBOARD_STAT_ICON_CALENDAR: &str = "calendar";
pub const DASHBOARD_STAT_ICON_CHECK: &str = "check";
pub const DASHBOARD_STAT_ICON_COMMENT: &str = "comment";
pub const DASHBOARD_STAT_ICON_DRAFT: &str = "draft";
pub const DASHBOARD_STAT_ICON_MEDIA: &str = "media";
pub const DASHBOARD_STAT_ICON_SETTINGS: &str = "settings";
#[derive(Clone, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct DashboardLayout(pub String);
pub const DASHBOARD_LAYOUT_DEFAULT: &str = "";
pub const DASHBOARD_LAYOUT_MEDIA_WORKBENCH: &str = "media-workbench";
#[derive(Clone, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct DashboardRowBadgeVariant(pub String);
pub const DASHBOARD_ROW_BADGE_VARIANT_STATUS_POSITIVE: &str = "status-positive";
pub const DASHBOARD_ROW_BADGE_VARIANT_STATUS_MUTED: &str = "status-muted";
pub const DASHBOARD_ROW_BADGE_VARIANT_OUTLINE: &str = "outline";
pub const DASHBOARD_ROW_BADGE_VARIANT_SECONDARY: &str = "secondary";
#[derive(Clone, Debug, Default)]
pub struct DashboardPaginationVM { pub base_href: String, pub page_param: String, pub current_page: i64, pub total_pages: i64 }
#[derive(Clone, Debug, Default)]
pub struct DashboardListSectionVM { pub title: String, pub action: ActionVM, pub empty_title: String, pub empty_description: String, pub pagination: DashboardPaginationVM, pub rows: Option<SharedList<DashboardRowVM>> }
#[derive(Clone, Debug, Default)]
pub struct DashboardSummaryRowVM { pub label: String, pub value: String }
#[derive(Clone, Debug, Default)]
pub struct DashboardSummarySectionVM { pub title: String, pub rows: Option<SharedList<DashboardSummaryRowVM>> }
#[derive(Clone, Debug, Default)]
pub struct DashboardMetricItemVM { pub primary: String, pub secondary: String, pub badge: String }
#[derive(Clone, Debug, Default)]
pub struct DashboardMetricSectionVM { pub title: String, pub items: Option<SharedList<DashboardMetricItemVM>>, pub empty_text: String, pub pagination: DashboardPaginationVM }
#[derive(Clone, Debug, Default)]
pub struct DashboardToolSectionVM { pub title: String, pub actions: Option<SharedList<ActionVM>> }
#[derive(Clone, Debug, Default)]
pub struct DashboardFilterChipVM { pub label: String, pub href: String, pub active: bool, pub count: String }
#[derive(Clone, Debug, Default)]
pub struct DashboardRowBadgeVM { pub label: String, pub variant: DashboardRowBadgeVariant }
#[derive(Clone, Debug, Default)]
pub struct DashboardRowActionVM { pub kind: ResourceItemActionKind, pub label: String, pub href: String, pub confirm_message: String }
#[derive(Clone, Debug, Default)]
pub struct DashboardRowVM { pub title: String, pub title_href: String, pub secondary_text: String, pub meta_text: String, pub trailing_text: String, pub preview_url: String, pub preview_alt: String, pub preview_kind: String, pub badges: Option<SharedList<DashboardRowBadgeVM>>, pub actions: Option<SharedList<DashboardRowActionVM>> }
#[derive(Clone, Debug, Default)]
pub struct ContentDashboardShellVM { pub page_meta: ManagementPageMeta, pub header_actions: Option<SharedList<ActionVM>>, pub tool_actions: Option<SharedList<ActionVM>> }
#[derive(Clone, Debug, Default)]
pub struct ContentDashboardPageVM { pub layout: DashboardLayout, pub page_meta: ManagementPageMeta, pub header_actions: Option<SharedList<ActionVM>>, pub search: DashboardSearchVM, pub filters: Option<SharedList<DashboardFilterChipVM>>, pub stats_grid_class: String, pub stats: Option<SharedList<DashboardStatVM>>, pub primary_section: DashboardListSectionVM, pub metric_section: Option<DashboardMetricSectionVM>, pub summary_section: DashboardSummarySectionVM, pub tool_section: DashboardToolSectionVM }
#[derive(Clone, Debug, Default)]
pub struct DashboardCardVM { pub kind: ResourceKind, pub title: String, pub description: String, pub href: String }
#[derive(Clone, Debug, Default)]
pub struct ResourceDashboardMetricVM { pub key: String, pub label: String, pub value: String, pub description: String }
#[derive(Clone, Debug, Default)]
pub struct ResourceDashboardSectionVM { pub key: ResourceSectionKey, pub title: String, pub description: String, pub metrics: Option<SharedList<ResourceDashboardMetricVM>>, pub pagination: ListPaginationVM, pub items: Option<SharedList<ResourceListItemVM>>, pub actions: Option<SharedList<ActionVM>> }
#[derive(Clone, Debug, Default)]
pub struct ResourceDashboardPageVM { pub kind: ResourceKind, pub mode: ResourceViewMode, pub title: String, pub subtitle: String, pub breadcrumb: ManagementBreadcrumbVM, pub routes: ResourceRouteSet, pub actions: Option<SharedList<ActionVM>>, pub search: DashboardSearchVM, pub sections: Option<SharedList<ResourceDashboardSectionVM>> }

pub type BreadcrumbVM = ManagementBreadcrumbVM;
