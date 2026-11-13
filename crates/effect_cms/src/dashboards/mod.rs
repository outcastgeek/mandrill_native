//! Dashboard data and search presentation helpers.

mod cards;
mod builders;
mod page_builders;
mod service;
pub use builders::*;
pub use page_builders::*;
pub use service::*;
mod helpers;
mod pages;
pub use cards::*;
pub use helpers::*;
pub use pages::*;

/// Default search input identifier.
pub const DEFAULT_DASHBOARD_SEARCH_ID: &str = "contentmgmt-dashboard-search";
/// Default search form identifier.
pub const DEFAULT_DASHBOARD_SEARCH_FORM_ID: &str = "contentmgmt-dashboard-search-form";

/// Search state displayed on a dashboard.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DashboardSearchVM {
    pub enabled: bool,
    pub id: String,
    pub form_id: String,
    pub name: String,
    pub value: crate::commands::InputBytes,
    pub placeholder: String,
    pub aria_label: String,
    pub action: String,
    pub shortcut_label: String,
}

/// Inputs for constructing dashboard search state.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DashboardSearchOptions {
    pub enabled: bool,
    pub id: String,
    pub form_id: String,
    pub name: String,
    pub value: crate::commands::InputBytes,
    pub placeholder: String,
    pub aria_label: String,
    pub action: String,
    pub shortcut_label: String,
}

pub type VM = DashboardSearchVM;
pub type SearchOptions = DashboardSearchOptions;

/// Construct search state, applying its defaults and whitespace normalization.
#[must_use]
pub fn new_dashboard_search_vm(options: DashboardSearchOptions) -> DashboardSearchVM {
    let id = first_non_blank(&options.id, DEFAULT_DASHBOARD_SEARCH_ID);
    let form_id = first_non_blank(&options.form_id, &format!("{id}-form"));
    DashboardSearchVM {
        enabled: options.enabled,
        id,
        form_id,
        name: first_non_blank(&options.name, crate::queries::SEARCH_QUERY_PARAM),
        value: options.value.trim().to_owned(),
        placeholder: options.placeholder.trim().to_owned(),
        aria_label: options.aria_label.trim().to_owned(),
        action: options.action.trim().to_owned(),
        shortcut_label: options.shortcut_label.trim().to_owned(),
    }
}

fn first_non_blank(value: &str, fallback: &str) -> String {
    if value.trim().is_empty() { fallback.trim().to_owned() } else { value.trim().to_owned() }
}

/// Whether search has an active destination.
#[must_use]
pub fn dashboard_search_enabled(search: &DashboardSearchVM) -> bool {
    search.enabled && !search.action.trim().is_empty()
}

/// Resolve the input name.
#[must_use]
pub fn dashboard_search_name(search: &DashboardSearchVM) -> String {
    first_non_blank(&search.name, crate::queries::SEARCH_QUERY_PARAM)
}

/// Resolve the placeholder.
#[must_use]
pub fn dashboard_search_placeholder(search: &DashboardSearchVM, fallback: &str) -> String {
    first_non_blank(&search.placeholder, fallback)
}

/// Resolve the input label.
#[must_use]
pub fn dashboard_search_aria_label(search: &DashboardSearchVM, fallback: &str) -> String {
    if search.aria_label.trim().is_empty() {
        dashboard_search_placeholder(search, fallback)
    } else {
        search.aria_label.trim().to_owned()
    }
}

/// Resolve the shortcut label.
#[must_use]
pub fn dashboard_search_shortcut_label(search: &DashboardSearchVM, fallback: &str) -> String {
    first_non_blank(&search.shortcut_label, fallback)
}
