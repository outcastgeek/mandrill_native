//! Sitemap document and index contracts.

use effect::{ParsePolicy, SchemaIssue};

use super::codec::{ContractCodec, array, number_between, object, required, string_min};
use super::shared::{content_resource_kind, sitemap_change_frequency};

pub(super) fn sitemap_entry(policy: ParsePolicy) -> Result<ContractCodec, SchemaIssue> {
    object(
        vec![
            required("Kind", content_resource_kind()?),
            required("RouteName", string_min(1)?),
            required("ChangeFrequency", sitemap_change_frequency()?),
            required("Priority", number_between(0, 1)?),
        ],
        policy,
    )
}

pub(super) fn sitemap_document(policy: ParsePolicy) -> Result<ContractCodec, SchemaIssue> {
    object(
        vec![
            required("Kind", content_resource_kind()?),
            required("Entries", array(policy, sitemap_entry(policy)?)),
        ],
        policy,
    )
}

pub(super) fn sitemap_index(policy: ParsePolicy) -> Result<ContractCodec, SchemaIssue> {
    object(
        vec![required("Entries", array(policy, sitemap_entry(policy)?))],
        policy,
    )
}
