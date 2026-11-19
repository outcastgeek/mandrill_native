//! Managed prefix/rest array codecs with explicit result backing.
use super::{
    IssueSegment, ParsePolicy, PreparedJsonArray, PreparedJsonArrayPool, PreparedJsonKind,
    PreparedJsonParserPool, PreparedJsonValidationPool, PureCodec, SchemaAst, SchemaIssue,
    SchemaIssueKind, SchemaResource, SharedPreparedJsonValue, ValueKind,
};
use std::sync::Arc;
/// A managed prefix element and whether its position is required.
pub struct PreparedPrefixElement<A> {
    /// Managed codec for this position.
    pub codec: PureCodec<A, SharedPreparedJsonValue>,
    /// Whether the position must be present.
    pub required: bool,
}
/// Prefix slots and remaining elements, each retaining its own fixed array/frame.
pub struct PreparedPrefixValue<A: Send + 'static, B: Send + 'static> {
    /// Declared prefix positions; absent entries are trailing None.
    pub prefix: PreparedJsonArray<Option<A>>,
    /// Elements after the prefix.
    pub rest: PreparedJsonArray<B>,
}
struct Diagnostics {
    required: Arc<str>,
    indices: Vec<Arc<str>>,
    additional: Arc<str>,
    false_value: Arc<str>,
}
impl Diagnostics {
    fn required(&self, index: usize) -> SchemaIssue {
        let Some(operand) = self.indices.get(index).map(Arc::clone) else {
            return invalid();
        };
        SchemaIssue::new(SchemaIssueKind::Constraint {
            name: Arc::clone(&self.required),
            operand,
        })
    }
    fn additional(&self) -> SchemaIssue {
        SchemaIssue::new(SchemaIssueKind::Constraint {
            name: Arc::clone(&self.additional),
            operand: Arc::clone(&self.false_value),
        })
    }
}
fn invalid() -> SchemaIssue {
    SchemaIssue::new(SchemaIssueKind::InvalidDefinition)
}
fn memory(error: crate::memory::MemoryError) -> SchemaIssue {
    SchemaIssue::new(SchemaIssueKind::Memory(error))
}
fn check_layout<A: Send + 'static, B: Send + 'static>(
    value: &PreparedPrefixValue<A, B>,
    prefix: &[PreparedPrefixElement<A>],
    has_rest: bool,
    policy: ParsePolicy,
    diagnostics: &Diagnostics,
    prefix_issue: impl Fn(SchemaIssue, usize) -> SchemaIssue,
) -> Result<(), SchemaIssue> {
    let values = value.prefix.as_slice();
    if values.len() != prefix.len() {
        return Err(invalid());
    }
    let mut absent = false;
    let mut length = value.rest.as_slice().len();
    for (index, (value, element)) in values.iter().zip(prefix).enumerate() {
        match value {
            Some(_) if !absent => length = length.saturating_add(1),
            None if !element.required => absent = true,
            _ => return Err(prefix_issue(diagnostics.required(index), index)),
        }
    }
    if (!has_rest || absent) && !value.rest.as_slice().is_empty() {
        return Err(diagnostics.additional());
    }
    if length > policy.limits.collection.get() {
        return Err(SchemaIssue::new(SchemaIssueKind::LimitExceeded {
            resource: SchemaResource::Width,
        }));
    }
    Ok(())
}
/// Construct a managed prefix/rest codec using explicit typed arrays and output backing.
pub fn json_prepared_prefix_rest<A: Send + 'static, B: Send + 'static>(
    prefix: Vec<PreparedPrefixElement<A>>,
    rest: Option<PureCodec<B, SharedPreparedJsonValue>>,
    policy: ParsePolicy,
    prefix_arrays: Arc<PreparedJsonArrayPool<Option<A>>>,
    rest_arrays: Arc<PreparedJsonArrayPool<B>>,
    output: Arc<PreparedJsonParserPool>,
    validation: Arc<PreparedJsonValidationPool>,
) -> Result<PureCodec<PreparedPrefixValue<A, B>, SharedPreparedJsonValue>, SchemaIssue> {
    if prefix.len() > policy.limits.collection.get() {
        return Err(invalid());
    }
    let mut indices = Vec::new();
    indices
        .try_reserve_exact(prefix.len())
        .map_err(|_| memory(crate::memory::MemoryError::BackingAllocationFailed))?;
    for index in 0..prefix.len() {
        indices.push(Arc::from(index.to_string()));
    }
    let diagnostics = Arc::new(Diagnostics {
        required: Arc::from("requiredPrefix"),
        indices,
        additional: Arc::from("additionalItems"),
        false_value: Arc::from("false"),
    });
    let prefix: Arc<[PreparedPrefixElement<A>]> = prefix.into();
    let decode = Arc::clone(&prefix);
    let decode_rest = rest.clone();
    let validator_prefix = Arc::clone(&prefix);
    let validator_rest = rest.clone();
    let writer_prefix = Arc::clone(&prefix);
    let writer_rest = rest.clone();
    let has_validator = prefix.iter().all(|p| p.codec.has_type_validator())
        && rest.as_ref().is_none_or(PureCodec::has_type_validator);
    let has_writer = prefix.iter().all(|p| p.codec.has_writer())
        && rest.as_ref().is_none_or(PureCodec::has_writer);
    let decode_diag = Arc::clone(&diagnostics);
    let validate_diag = Arc::clone(&diagnostics);
    let writer_diag = Arc::clone(&diagnostics);
    let result = PureCodec::with_context_ast(
        SchemaAst::opaque(),
        move |input: &SharedPreparedJsonValue, context| {
            let active = super::parse_context::policy(context, policy);
            validation.check_parse_limits(input, &active.limits)?;
            if input.view().kind() != Some(PreparedJsonKind::Array) {
                return Err(SchemaIssue::new(SchemaIssueKind::TypeMismatch {
                    expected: ValueKind::Sequence,
                    actual: super::json_codec::prepared_value_kind(input)?,
                }));
            }
            let count = input.view().len();
            let mut values = prefix_arrays.reserve().map_err(memory)?;
            let mut remaining = rest_arrays.reserve().map_err(memory)?;
            let rest_needed = if decode_rest.is_some() {
                count.saturating_sub(decode.len())
            } else {
                0
            };
            if values.capacity() < decode.len() || remaining.capacity() < rest_needed {
                return Err(memory(crate::memory::MemoryError::NoSlot));
            }
            for (index, element) in decode.iter().enumerate() {
                let value =
                    if index < count {
                        let child = input
                            .try_get_index(index)
                            .map_err(memory)?
                            .ok_or_else(invalid)?;
                        let ctx = super::parse_context::child(context, IssueSegment::Index(index))?;
                        Some(element.codec.decode_in_ref(&child, ctx.as_ref()).map_err(
                            |issue| {
                                super::prepared_issue_at(context, issue, IssueSegment::Index(index))
                            },
                        )?)
                    } else if !element.required {
                        None
                    } else {
                        return Err(super::prepared_issue_at(
                            context,
                            decode_diag.required(index),
                            IssueSegment::Index(index),
                        ));
                    };
                values
                    .push(value)
                    .map_err(|_| memory(crate::memory::MemoryError::NoSlot))?;
            }
            for index in decode.len()..count {
                let codec = decode_rest
                    .as_ref()
                    .ok_or_else(|| decode_diag.additional())?;
                let child = input
                    .try_get_index(index)
                    .map_err(memory)?
                    .ok_or_else(invalid)?;
                let ctx = super::parse_context::child(context, IssueSegment::Index(index))?;
                let value = codec.decode_in_ref(&child, ctx.as_ref()).map_err(|issue| {
                    super::prepared_issue_at(context, issue, IssueSegment::Index(index))
                })?;
                remaining
                    .push(value)
                    .map_err(|_| memory(crate::memory::MemoryError::NoSlot))?;
            }
            Ok(PreparedPrefixValue {
                prefix: values,
                rest: remaining,
            })
        },
        move |value: &PreparedPrefixValue<A, B>, context| {
            if value.prefix.as_slice().len() != prefix.len() {
                return Err(invalid());
            }
            let active = super::parse_context::policy(context, policy);
            let mut encoded = output.admit_array(active)?;
            let mut absent = false;
            for (index, (item, element)) in value
                .prefix
                .as_slice()
                .iter()
                .zip(prefix.iter())
                .enumerate()
            {
                match item {
                    Some(item) if !absent => encoded.push(|| {
                        let ctx = super::parse_context::child(context, IssueSegment::Index(index))?;
                        element
                            .codec
                            .encode_in_ref(item, ctx.as_ref())
                            .map_err(|issue| {
                                super::prepared_issue_at(context, issue, IssueSegment::Index(index))
                            })
                    })?,
                    None if !element.required => absent = true,
                    _ => return Err(invalid()),
                }
            }
            if absent && !value.rest.as_slice().is_empty() {
                return Err(invalid());
            }
            for (offset, item) in value.rest.as_slice().iter().enumerate() {
                let index = prefix.len().saturating_add(offset);
                let codec = rest.as_ref().ok_or_else(invalid)?;
                encoded.push(|| {
                    let ctx = super::parse_context::child(context, IssueSegment::Index(index))?;
                    codec.encode_in_ref(item, ctx.as_ref())
                })?;
            }
            encoded.finish()
        },
    );
    let result = if has_validator {
        result.with_context_type_validator(move |value: &PreparedPrefixValue<A, B>, context| {
            check_layout(
                value,
                &validator_prefix,
                validator_rest.is_some(),
                super::parse_context::policy(context, policy),
                &validate_diag,
                |issue, index| super::prepared_issue_at(context, issue, IssueSegment::Index(index)),
            )?;
            for (index, (item, element)) in value
                .prefix
                .as_slice()
                .iter()
                .zip(validator_prefix.iter())
                .enumerate()
            {
                if let Some(item) = item {
                    let ctx = super::parse_context::child(context, IssueSegment::Index(index))?;
                    element
                        .codec
                        .validate_in_ref(item, ctx.as_ref())
                        .map_err(|issue| {
                            super::prepared_issue_at(context, issue, IssueSegment::Index(index))
                        })?;
                }
            }
            for (offset, item) in value.rest.as_slice().iter().enumerate() {
                let index = validator_prefix.len().saturating_add(offset);
                let ctx = super::parse_context::child(context, IssueSegment::Index(index))?;
                validator_rest
                    .as_ref()
                    .ok_or_else(invalid)?
                    .validate_in_ref(item, ctx.as_ref())
                    .map_err(|issue| {
                        super::prepared_issue_at(context, issue, IssueSegment::Index(index))
                    })?;
            }
            Ok(())
        })
    } else {
        result
    };
    Ok(if has_writer {
        result.with_writer(move |value, writer| {
            check_layout(
                value,
                &writer_prefix,
                writer_rest.is_some(),
                policy,
                &writer_diag,
                |issue, index| writer.issue_at(issue, IssueSegment::Index(index)),
            )?;
            writer.begin_array()?;
            for (index, (item, element)) in value
                .prefix
                .as_slice()
                .iter()
                .zip(writer_prefix.iter())
                .enumerate()
            {
                if let Some(item) = item {
                    element
                        .codec
                        .write_into(item, writer)
                        .map_err(|issue| writer.issue_at(issue, IssueSegment::Index(index)))?;
                }
            }
            for (offset, item) in value.rest.as_slice().iter().enumerate() {
                let index = writer_prefix.len().saturating_add(offset);
                writer_rest
                    .as_ref()
                    .ok_or_else(invalid)?
                    .write_into(item, writer)
                    .map_err(|issue| writer.issue_at(issue, IssueSegment::Index(index)))?;
            }
            writer.end_array()
        })
    } else {
        result
    })
}
