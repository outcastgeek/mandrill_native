//! Command-input and result document contracts.

use effect::{ParsePolicy, SchemaIssue};

use super::codec::{
    ContractCodec, any, array, boolean, nullable, object, optional, optional_string_map, required,
    string_map, string_min,
};
use super::shared::{
    content_action, content_resource_kind, resource_detail_query, resource_list_query,
    validation_code,
};

pub(super) fn command_input(policy: ParsePolicy) -> Result<ContractCodec, SchemaIssue> {
    object(
        vec![
            required("Kind", content_resource_kind()?),
            required("Action", content_action()?),
            required("ListQuery", resource_list_query(policy)?),
            required("DetailQuery", resource_detail_query(policy)?),
            required("FormValues", string_map(policy)?),
        ],
        policy,
    )
}

pub(super) fn validation_result(policy: ParsePolicy) -> Result<ContractCodec, SchemaIssue> {
    object(
        vec![
            required("Action", content_action()?),
            required("Code", validation_code()?),
            required("Message", string_min(0)?),
            optional("Issues", nullable(array(policy, any()))),
            required("SubmitDisabled", boolean()),
            required("RequiresRehydrate", boolean()),
        ],
        policy,
    )
}

pub(super) fn mutation_result(policy: ParsePolicy) -> Result<ContractCodec, SchemaIssue> {
    object(
        vec![
            required("Kind", content_resource_kind()?),
            required("Action", content_action()?),
            required("RedirectRoute", string_min(1)?),
            optional("RedirectLanguage", string_min(0)?),
            optional("RedirectParams", optional_string_map(policy)?),
        ],
        policy,
    )
}
