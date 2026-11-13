//! Document contracts shared by more than one builtin selector.

use effect::{ParsePolicy, SchemaIssue, SchemaShape, codec};

use super::codec::{
    ContractCodec, any, array, boolean, declared_map, default_null, integer_min, loose_map,
    nested_map, nullable, object, one_of, optional, optional_string_map, required, string_map,
    string_min,
};

const CONTENT_VIEW_MODES: &[&str] = &[
    "list",
    "detail",
    "dashboard",
    "new",
    "edit",
    "public",
    "manage",
];
const CONTENT_ACTIONS: &[&str] = &[
    "create",
    "update",
    "delete",
    "publish",
    "draft",
    "attach",
    "translate",
];
const VALIDATION_CODES: &[&str] = &[
    "valid",
    "invalid-request",
    "invalid-query",
    "unsupported-action",
];
const CONTENT_FILTER_KEYS: &[&str] = &[
    "search",
    "status",
    "author",
    "tag",
    "series",
    "location",
    "organizer",
    "kind",
    "sort",
];
const RESOURCE_SECTION_KEYS: &[&str] = &[
    "identity",
    "localized-description",
    "localized-content",
    "media",
    "metadata",
    "featured",
    "secondary",
    "grid",
    "results",
    "comments",
    "metrics",
    "tools",
];
const RESOURCE_FIELD_KINDS: &[&str] = &[
    "text",
    "textarea",
    "select",
    "hidden",
    "tags",
    "localized",
    "media",
    "datetime",
    "duration",
    "url",
    "collection",
];
const RESOURCE_ITEM_ACTION_KINDS: &[&str] = &["edit", "delete", "external-link"];
const SITEMAP_CHANGE_FREQUENCIES: &[&str] = &["always", "hourly", "daily", "weekly", "monthly"];

pub(super) fn content_resource_kind() -> Result<ContractCodec, SchemaIssue> {
    string_min(1)
}

pub(super) fn content_view_mode() -> Result<ContractCodec, SchemaIssue> {
    one_of(CONTENT_VIEW_MODES)
}

pub(super) fn content_action() -> Result<ContractCodec, SchemaIssue> {
    one_of(CONTENT_ACTIONS)
}

pub(super) fn validation_code() -> Result<ContractCodec, SchemaIssue> {
    one_of(VALIDATION_CODES)
}

pub(super) fn content_filter_key() -> Result<ContractCodec, SchemaIssue> {
    one_of(CONTENT_FILTER_KEYS)
}

pub(super) fn resource_section_key() -> Result<ContractCodec, SchemaIssue> {
    one_of(RESOURCE_SECTION_KEYS)
}

pub(super) fn resource_field_key() -> Result<ContractCodec, SchemaIssue> {
    string_min(1)
}

pub(super) fn resource_field_kind() -> Result<ContractCodec, SchemaIssue> {
    one_of(RESOURCE_FIELD_KINDS)
}

pub(super) fn resource_item_action_kind() -> Result<ContractCodec, SchemaIssue> {
    one_of(RESOURCE_ITEM_ACTION_KINDS)
}

pub(super) fn seo_page_type() -> Result<ContractCodec, SchemaIssue> {
    string_min(1)
}

pub(super) fn sitemap_change_frequency() -> Result<ContractCodec, SchemaIssue> {
    one_of(SITEMAP_CHANGE_FREQUENCIES)
}

pub(super) fn validation_issue_vm(policy: ParsePolicy) -> Result<ContractCodec, SchemaIssue> {
    object(
        vec![
            required("Code", string_min(0)?),
            required("Path", string_min(0)?),
            required("Field", string_min(0)?),
            required("FieldID", string_min(0)?),
            required("FieldKey", string_min(0)?),
            required("FieldName", string_min(0)?),
            required("Message", string_min(1)?),
        ],
        policy,
    )
}

pub(super) fn validation_issue_vm_list(policy: ParsePolicy) -> Result<ContractCodec, SchemaIssue> {
    Ok(nullable(array(policy, validation_issue_vm(policy)?)))
}

pub(super) fn resource_route_set(policy: ParsePolicy) -> Result<ContractCodec, SchemaIssue> {
    object(
        vec![
            optional("Dashboard", string_min(0)?),
            optional("List", string_min(0)?),
            optional("Detail", string_min(0)?),
            optional("New", string_min(0)?),
            optional("NewPost", string_min(0)?),
            optional("Edit", string_min(0)?),
            optional("EditPost", string_min(0)?),
            optional("Delete", string_min(0)?),
            optional("Aux", optional_string_map(policy)?),
        ],
        policy,
    )
}

pub(super) fn management_breadcrumb(policy: ParsePolicy) -> Result<ContractCodec, SchemaIssue> {
    object(
        vec![
            required("DashboardRoute", string_min(1)?),
            required("DashboardLabel", string_min(1)?),
            required("CurrentLabel", string_min(1)?),
            required("CurrentSubtitle", string_min(0)?),
        ],
        policy,
    )
}

pub(super) fn submit_action(policy: ParsePolicy) -> Result<ContractCodec, SchemaIssue> {
    object(
        vec![
            required("Action", content_action()?),
            required("Label", string_min(1)?),
        ],
        policy,
    )
}

pub(super) fn localized_widget(policy: ParsePolicy) -> Result<ContractCodec, SchemaIssue> {
    object(
        vec![
            required("WidgetKey", string_min(1)?),
            required("Scope", string_min(1)?),
        ],
        policy,
    )
}

pub(super) fn resource_filter_option(policy: ParsePolicy) -> Result<ContractCodec, SchemaIssue> {
    object(
        vec![
            required("Key", content_filter_key()?),
            required("Value", string_min(0)?),
            required("Active", boolean()),
            required("Href", string_min(1)?),
            required("Label", string_min(1)?),
        ],
        policy,
    )
}

pub(super) fn resource_filter_chip(policy: ParsePolicy) -> Result<ContractCodec, SchemaIssue> {
    object(
        vec![
            required("Key", content_filter_key()?),
            required("Value", string_min(1)?),
            required("Label", string_min(1)?),
            required("AriaLabel", string_min(0)?),
            required("Href", string_min(1)?),
            required("Active", boolean()),
            required("RemoveHref", string_min(0)?),
        ],
        policy,
    )
}

pub(super) fn list_page_link(policy: ParsePolicy) -> Result<ContractCodec, SchemaIssue> {
    object(
        vec![
            required("Page", integer_min(1)?),
            required("Href", string_min(0)?),
            required("Current", boolean()),
        ],
        policy,
    )
}

pub(super) fn list_pagination(policy: ParsePolicy) -> Result<ContractCodec, SchemaIssue> {
    object(
        vec![
            required("Page", integer_min(1)?),
            required("PageSize", integer_min(1)?),
            required("TotalItems", integer_min(0)?),
            required("TotalPages", integer_min(1)?),
            required("HasPrev", boolean()),
            required("HasNext", boolean()),
            required("PrevHref", string_min(0)?),
            required("NextHref", string_min(0)?),
            required("Links", array(policy, list_page_link(policy)?)),
        ],
        policy,
    )
}

pub(super) fn seo(policy: ParsePolicy) -> Result<ContractCodec, SchemaIssue> {
    object(
        vec![
            required("Kind", content_resource_kind()?),
            required("Mode", content_view_mode()?),
            required("Type", seo_page_type()?),
            required("Title", string_min(1)?),
            required("Description", string_min(1)?),
            required("CanonicalRoute", string_min(1)?),
            required("Indexable", boolean()),
            required("IncludedInSitemap", boolean()),
        ],
        policy,
    )
}

pub(super) fn resource_field_option(policy: ParsePolicy) -> Result<ContractCodec, SchemaIssue> {
    object(
        vec![
            required("Value", string_min(0)?),
            required("Label", string_min(1)?),
            required("Selected", boolean()),
            optional("Attrs", loose_map(policy)?),
        ],
        policy,
    )
}

pub(super) fn resource_field(policy: ParsePolicy) -> Result<ContractCodec, SchemaIssue> {
    object(
        vec![
            required("Key", resource_field_key()?),
            required("Kind", resource_field_kind()?),
            required("Name", string_min(0)?),
            required("InputID", string_min(0)?),
            required("Label", string_min(1)?),
            required("Value", string_min(0)?),
            required("DisplayValue", string_min(0)?),
            required("Placeholder", string_min(0)?),
            required("HelpText", string_min(0)?),
            required("Description", string_min(0)?),
            required("Required", boolean()),
            required("Disabled", boolean()),
            required("Rows", integer_min(0)?),
            required("SecondaryName", string_min(0)?),
            required("SecondaryID", string_min(0)?),
            required("SecondaryValue", string_min(0)?),
            optional("Attrs", loose_map(policy)?),
            required("Options", array(policy, resource_field_option(policy)?)),
            optional("LocalizedTabs", any()),
            optional("MediaManager", any()),
            required("Error", boolean()),
            required("ErrorMessage", string_min(0)?),
            default_null("Errors", validation_issue_vm_list(policy)?),
        ],
        policy,
    )
}

pub(super) fn resource_form_section(policy: ParsePolicy) -> Result<ContractCodec, SchemaIssue> {
    object(
        vec![
            required("Key", resource_section_key()?),
            required("Title", string_min(0)?),
            required("Description", string_min(0)?),
            required("Fields", array(policy, resource_field(policy)?)),
        ],
        policy,
    )
}

pub(super) fn form_status(policy: ParsePolicy) -> Result<ContractCodec, SchemaIssue> {
    object(
        vec![
            required("Code", validation_code()?),
            required("Message", string_min(0)?),
            default_null("Issues", validation_issue_vm_list(policy)?),
            required("SubmitDisabled", boolean()),
            required("RequiresRehydrate", boolean()),
        ],
        policy,
    )
}

pub(super) fn list_toolbar(policy: ParsePolicy) -> Result<ContractCodec, SchemaIssue> {
    object(
        vec![
            required("Action", string_min(0)?),
            required("SearchKey", resource_field_key()?),
            required("SearchValue", string_min(0)?),
            required("SortKey", resource_field_key()?),
            required("SortValue", string_min(0)?),
        ],
        policy,
    )
}

pub(super) fn list_filters(policy: ParsePolicy) -> Result<ContractCodec, SchemaIssue> {
    object(
        vec![required(
            "Options",
            array(policy, resource_filter_option(policy)?),
        )],
        policy,
    )
}

pub(super) fn resource_meta(policy: ParsePolicy) -> Result<ContractCodec, SchemaIssue> {
    object(
        vec![
            required("Label", string_min(0)?),
            required("Value", string_min(0)?),
            required("Href", string_min(0)?),
        ],
        policy,
    )
}

pub(super) fn resource_item_action(policy: ParsePolicy) -> Result<ContractCodec, SchemaIssue> {
    object(
        vec![
            required("Kind", resource_item_action_kind()?),
            required("Label", string_min(1)?),
            required("Href", string_min(1)?),
            required("ConfirmMessage", string_min(0)?),
        ],
        policy,
    )
}

pub(super) fn resource_list_item(policy: ParsePolicy) -> Result<ContractCodec, SchemaIssue> {
    codec! {
        recursive_pure;
        name: "list_item",
        shape: |_reference| SchemaShape::Opaque,
        build: |reference| {
            object(
                vec![
                    required("Key", string_min(1)?),
                    required("Title", string_min(1)?),
                    required("Summary", string_min(0)?),
                    required("DetailHref", string_min(0)?),
                    required("ExternalHref", string_min(0)?),
                    required("Status", string_min(0)?),
                    required("ImageURL", string_min(0)?),
                    required("Meta", array(policy, resource_meta(policy)?)),
                    required("Actions", array(policy, resource_item_action(policy)?)),
                    required("ActionLabel", string_min(0)?),
                    required("ActionHref", string_min(0)?),
                    required("Children", array(policy, reference)),
                ],
                policy,
            )
        },
    }
}

pub(super) fn resource_list_section(policy: ParsePolicy) -> Result<ContractCodec, SchemaIssue> {
    object(
        vec![
            required("Key", resource_section_key()?),
            required("Title", string_min(0)?),
            required("Description", string_min(0)?),
            required("Items", array(policy, resource_list_item(policy)?)),
        ],
        policy,
    )
}

pub(super) fn resource_detail_section(policy: ParsePolicy) -> Result<ContractCodec, SchemaIssue> {
    object(
        vec![
            required("Key", resource_section_key()?),
            required("Title", string_min(0)?),
            required("Description", string_min(0)?),
            required("Body", string_min(0)?),
            required("BodyIsHTML", boolean()),
            required("Fields", array(policy, resource_field(policy)?)),
            required("Items", array(policy, resource_list_item(policy)?)),
        ],
        policy,
    )
}

pub(super) fn resource_detail_extras(policy: ParsePolicy) -> Result<ContractCodec, SchemaIssue> {
    object(
        vec![optional(
            "Links",
            nested_map(policy, resource_link(policy)?)?,
        )],
        policy,
    )
}

pub(super) fn resource_link(policy: ParsePolicy) -> Result<ContractCodec, SchemaIssue> {
    object(
        vec![
            required("Label", string_min(0)?),
            required("Href", string_min(0)?),
            required("AriaLabel", string_min(0)?),
            optional("Rel", string_min(0)?),
            optional("ImageURL", string_min(0)?),
            optional("ImageAlt", string_min(0)?),
            optional("Summary", string_min(0)?),
            optional("Meta", string_map(policy)?),
        ],
        policy,
    )
}

pub(super) fn resource_dashboard_metric(policy: ParsePolicy) -> Result<ContractCodec, SchemaIssue> {
    object(
        vec![
            required("Key", string_min(1)?),
            required("Label", string_min(1)?),
            required("Value", string_min(1)?),
            required("Description", string_min(0)?),
        ],
        policy,
    )
}

pub(super) fn resource_detail_query(policy: ParsePolicy) -> Result<ContractCodec, SchemaIssue> {
    object(
        vec![
            required("ID", string_min(0)?),
            required("Slug", string_min(0)?),
            required("Parent", string_min(0)?),
            required("Locale", string_min(0)?),
        ],
        policy,
    )
}

pub(super) fn resource_list_query(policy: ParsePolicy) -> Result<ContractCodec, SchemaIssue> {
    object(
        vec![
            required("Page", integer_min(1)?),
            required("PageSize", integer_min(1)?),
            required("Search", string_min(0)?),
            required("Sort", string_min(0)?),
            required(
                "Filters",
                declared_map(policy, CONTENT_FILTER_KEYS, string_min(0)?)?,
            ),
            required(
                "FilterValues",
                declared_map(policy, CONTENT_FILTER_KEYS, array(policy, string_min(0)?))?,
            ),
        ],
        policy,
    )
}
