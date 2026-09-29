//! Stable encodings of workflow identities and records.

use super::WorkflowCodecError;
use crate::{JsonValue, ParseLimits, SchemaIssueKind};
use std::sync::Arc;

/// # Parameters
/// - `value` (`&JsonValue`): Value read, transformed, encoded, or stored by this operation.
/// - `limits` (`ParseLimits`): Explicit resource bounds enforced while processing this input.
///
/// # Returns
/// Type: `Result<Arc<[u8]>, WorkflowCodecError>`.
pub(super) fn encode_json(
    value: &JsonValue,
    limits: ParseLimits,
) -> Result<Arc<[u8]>, WorkflowCodecError> {
    let bytes = crate::schema::internal::parser::encode_json_bytes(value, &limits)
        .map_err(WorkflowCodecError::Schema)?;
    Ok(Arc::from(bytes))
}

/// # Parameters
/// - `bytes` (`&[u8]`): Encoded bytes to validate, decode, write, or transfer.
/// - `limits` (`ParseLimits`): Explicit resource bounds enforced while processing this input.
///
/// # Returns
/// Type: `Result<JsonValue, WorkflowCodecError>`.
pub(super) fn decode_json(
    bytes: &[u8],
    limits: ParseLimits,
) -> Result<JsonValue, WorkflowCodecError> {
    if bytes.len() > limits.bytes.get() {
        return Err(WorkflowCodecError::ByteLimitExceeded);
    }
    crate::schema::internal::parser::parse_json_bytes(
        bytes,
        crate::schema::internal::parser::strict_wire_policy(limits),
    )
    .map_err(|issue| {
        if matches!(issue.kind(), SchemaIssueKind::JsonSyntax { .. }) {
            WorkflowCodecError::MalformedJson(issue)
        } else {
            WorkflowCodecError::Schema(issue)
        }
    })
}
