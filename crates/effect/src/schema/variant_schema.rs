//! Named typed views and projections of schemas.

use super::{JsonValue, ParsePolicy, PureCodec, SchemaIssue, TaggedEnumBuilder};
use std::sync::Arc;

/// A named, typed Result view. Wire tags and envelope keys are application
/// identities, so callers must provide them rather than inheriting hidden
/// strings from the library.
///
/// # Type parameters
/// - `A`: Application value type. Bounds: `Send + 'static`.
/// - `E`: Domain-error type. Bounds: `Send + 'static`.
///
/// # Parameters
/// - `success` (`PureCodec<A, JsonValue>`): Successful application value or its codec.
/// - `failure` (`PureCodec<E, JsonValue>`): Expected error or cleanup failure retained in this outcome.
/// - `tag_field` (`impl Into<Arc<str>>`): The tag field supplied to this operation.
/// - `content_field` (`impl Into<Arc<str>>`): The content field supplied to this operation.
/// - `success_tag` (`impl Into<Arc<str>>`): The success tag supplied to this operation.
/// - `failure_tag` (`impl Into<Arc<str>>`): The failure tag supplied to this operation.
/// - `policy` (`ParsePolicy`): The caller-supplied behavior policy.
///
/// # Returns
/// Type: `Result<PureCodec<Result<A, E>, JsonValue>, SchemaIssue>`.
pub fn json_result<A: Send + 'static, E: Send + 'static>(
    success: PureCodec<A, JsonValue>,
    failure: PureCodec<E, JsonValue>,
    tag_field: impl Into<Arc<str>>,
    content_field: impl Into<Arc<str>>,
    success_tag: impl Into<Arc<str>>,
    failure_tag: impl Into<Arc<str>>,
    policy: ParsePolicy,
) -> Result<PureCodec<Result<A, E>, JsonValue>, SchemaIssue> {
    TaggedEnumBuilder::new(tag_field, content_field)
        .variant(
            success_tag,
            success,
            |value: &Result<A, E>| value.as_ref().ok(),
            Ok,
        )?
        .variant(
            failure_tag,
            failure,
            |value: &Result<A, E>| value.as_ref().err(),
            Err,
        )?
        .finish(policy)
}

/// An Option view with distinct tags also accepts a non-null payload codec;
/// it need not reserve JSON null as a sentinel for absence.
///
/// # Type parameters
/// - `A`: Application value type. Bounds: `Send + 'static`.
///
/// # Parameters
/// - `value` (`PureCodec<A, JsonValue>`): Value read, transformed, encoded, or stored by this operation.
/// - `tag_field` (`impl Into<Arc<str>>`): The tag field supplied to this operation.
/// - `content_field` (`impl Into<Arc<str>>`): The content field supplied to this operation.
/// - `some_tag` (`impl Into<Arc<str>>`): The some tag supplied to this operation.
/// - `none_tag` (`impl Into<Arc<str>>`): The none tag supplied to this operation.
/// - `policy` (`ParsePolicy`): The caller-supplied behavior policy.
///
/// # Returns
/// Type: `Result<PureCodec<Option<A>, JsonValue>, SchemaIssue>`.
pub fn json_option<A: Send + 'static>(
    value: PureCodec<A, JsonValue>,
    tag_field: impl Into<Arc<str>>,
    content_field: impl Into<Arc<str>>,
    some_tag: impl Into<Arc<str>>,
    none_tag: impl Into<Arc<str>>,
    policy: ParsePolicy,
) -> Result<PureCodec<Option<A>, JsonValue>, SchemaIssue> {
    TaggedEnumBuilder::new(tag_field, content_field)
        .variant(some_tag, value, |item: &Option<A>| item.as_ref(), Some)?
        .variant(
            none_tag,
            super::json_null(),
            |item: &Option<A>| {
                if item.is_none() { Some(&()) } else { None }
            },
            |_| None,
        )?
        .finish(policy)
}
