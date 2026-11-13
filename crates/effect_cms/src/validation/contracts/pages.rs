//! Form, list, detail and dashboard page document contracts.

use effect::{ParsePolicy, SchemaIssue};

use super::codec::{
    ContractCodec, any, array, boolean, object, one_of, optional, required, string_min,
};
use super::shared::{
    content_resource_kind, list_filters, list_pagination, list_toolbar, management_breadcrumb,
    resource_dashboard_metric, resource_detail_extras, resource_detail_section,
    resource_filter_chip, resource_filter_option, resource_form_section, resource_list_item,
    resource_list_query, resource_list_section, resource_route_set, resource_section_key, seo,
};

const FORM_PAGE_MODES: &[&str] = &["new", "edit"];
const LIST_PAGE_MODES: &[&str] = &["list", "manage"];
const DETAIL_PAGE_MODES: &[&str] = &["detail", "edit"];
const DASHBOARD_PAGE_MODES: &[&str] = &["dashboard"];
const DASHBOARD_ACTION_LOCATIONS: &[&str] = &["header", "tools"];

pub(super) fn resource_form_page(policy: ParsePolicy) -> Result<ContractCodec, SchemaIssue> {
    object(
        vec![
            required("Kind", content_resource_kind()?),
            required("Mode", one_of(FORM_PAGE_MODES)?),
            required("Title", string_min(1)?),
            required("Breadcrumb", management_breadcrumb(policy)?),
            required("Routes", resource_route_set(policy)?),
            required("FormID", string_min(0)?),
            required("ActionHref", string_min(0)?),
            required("CancelHref", string_min(0)?),
            required(
                "Actions",
                array(policy, super::shared::submit_action(policy)?),
            ),
            required(
                "Widgets",
                array(policy, super::shared::localized_widget(policy)?),
            ),
            required("Status", super::shared::form_status(policy)?),
            required("Sections", array(policy, resource_form_section(policy)?)),
        ],
        policy,
    )
}

pub(super) fn resource_list_page(policy: ParsePolicy) -> Result<ContractCodec, SchemaIssue> {
    object(
        vec![
            required("Kind", content_resource_kind()?),
            required("Mode", one_of(LIST_PAGE_MODES)?),
            required("Title", string_min(1)?),
            required("Subtitle", string_min(0)?),
            required("Breadcrumb", management_breadcrumb(policy)?),
            required("Routes", resource_route_set(policy)?),
            required("NewHref", string_min(0)?),
            required("IsLoggedIn", boolean()),
            required("CanCreate", boolean()),
            required("ManageView", boolean()),
            required("Query", resource_list_query(policy)?),
            required("Filters", array(policy, resource_filter_option(policy)?)),
            required(
                "ActiveFilterChips",
                array(policy, resource_filter_chip(policy)?),
            ),
            required("Toolbar", list_toolbar(policy)?),
            required("FilterState", list_filters(policy)?),
            required("Pagination", list_pagination(policy)?),
            required("Sections", array(policy, resource_list_section(policy)?)),
            required("SEO", seo(policy)?),
            optional("SEOView", any()),
        ],
        policy,
    )
}

pub(super) fn resource_detail_page(policy: ParsePolicy) -> Result<ContractCodec, SchemaIssue> {
    object(
        vec![
            required("Kind", content_resource_kind()?),
            required("Mode", one_of(DETAIL_PAGE_MODES)?),
            required("Title", string_min(1)?),
            required("Breadcrumb", management_breadcrumb(policy)?),
            required("Routes", resource_route_set(policy)?),
            required("BackHref", string_min(0)?),
            required("EditHref", string_min(0)?),
            required("DeleteAction", string_min(0)?),
            required("DeleteConfirm", string_min(0)?),
            required("CanEdit", boolean()),
            required("IsLoggedIn", boolean()),
            required("ManageView", boolean()),
            required("Sections", array(policy, resource_detail_section(policy)?)),
            optional("Extras", resource_detail_extras(policy)?),
            required("SEO", seo(policy)?),
            optional("SEOView", any()),
        ],
        policy,
    )
}

pub(super) fn dashboard_action(policy: ParsePolicy) -> Result<ContractCodec, SchemaIssue> {
    object(
        vec![
            required("Label", string_min(1)?),
            required("Href", string_min(1)?),
            required("Icon", string_min(0)?),
            required("Location", one_of(DASHBOARD_ACTION_LOCATIONS)?),
        ],
        policy,
    )
}

pub(super) fn dashboard_search(policy: ParsePolicy) -> Result<ContractCodec, SchemaIssue> {
    object(
        vec![
            required("Enabled", boolean()),
            required("ID", string_min(0)?),
            required("FormID", string_min(0)?),
            required("Name", string_min(0)?),
            required("Value", string_min(0)?),
            required("Placeholder", string_min(0)?),
            required("AriaLabel", string_min(0)?),
            required("Action", string_min(0)?),
            required("ShortcutLabel", string_min(0)?),
        ],
        policy,
    )
}

pub(super) fn resource_dashboard_section(
    policy: ParsePolicy,
) -> Result<ContractCodec, SchemaIssue> {
    object(
        vec![
            required("Key", resource_section_key()?),
            required("Title", string_min(0)?),
            required("Description", string_min(0)?),
            required("Metrics", array(policy, resource_dashboard_metric(policy)?)),
            required("Pagination", list_pagination(policy)?),
            required("Items", array(policy, resource_list_item(policy)?)),
            required("Actions", array(policy, dashboard_action(policy)?)),
        ],
        policy,
    )
}

pub(super) fn resource_dashboard_page(policy: ParsePolicy) -> Result<ContractCodec, SchemaIssue> {
    object(
        vec![
            required("Kind", content_resource_kind()?),
            required("Mode", one_of(DASHBOARD_PAGE_MODES)?),
            required("Title", string_min(1)?),
            required("Subtitle", string_min(0)?),
            required("Breadcrumb", management_breadcrumb(policy)?),
            required("Routes", resource_route_set(policy)?),
            required("Actions", array(policy, dashboard_action(policy)?)),
            optional("Search", dashboard_search(policy)?),
            required(
                "Sections",
                array(policy, resource_dashboard_section(policy)?),
            ),
        ],
        policy,
    )
}
