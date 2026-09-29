use super::{JsonValue, ParsePolicy, PureCodec, SchemaIssue, TaggedEnumBuilder};
use std::sync::Arc;

/// A named, typed Result view. Wire tags and envelope keys are application
/// identities, so callers must provide them rather than inheriting hidden
/// strings from the library.
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
